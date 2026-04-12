//! Thin wrapper over Windows Job Objects.
//!
//! A Job Object is a kernel-backed container for processes. Assigning the
//! root process places it (and its descendants by default) into the job;
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` guarantees every member is
//! SIGKILL-equivalent when the last handle to the job is released, so the
//! supervisor crashing never leaves orphans behind.
//!
//! Each identity generation gets its own unique named job. A per-identity
//! mutex plus a small temp-file registry points new spawns at the predecessor
//! job name so they can terminate it without ever reusing the same kernel
//! object. That keeps stale containers isolated from their successors.

use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, OpenJobObjectW,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{
    CreateMutexW, INFINITE, OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE, ReleaseMutex,
    WaitForSingleObject,
};
use windows::core::{HRESULT, PCWSTR};

use crate::error::JobObjectError;

const OBJECT_NAMESPACE: &str = "Local\\uncontainerizable-";
const LOCK_NAMESPACE: &str = "Local\\uncontainerizable-lock-";
const STATE_DIR: &str = "uncontainerizable/win32-job-state";

/// `JOB_OBJECT_ALL_ACCESS` = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x1F.
/// The `windows` crate does not expose it as a constant, so we carry the
/// documented value from `WinNT.h`.
const JOB_OBJECT_ALL_ACCESS: u32 = 0x001F_001F;

/// Wrapper around a Windows Job Object handle. The handle is closed on drop,
/// which (with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) terminates every
/// remaining member. Named jobs persist only while at least one handle is
/// open, so dropping the last handle both kills members and releases the name.
pub struct JobObject {
    handle: HANDLE,
    name: Option<String>,
}

impl JobObject {
    fn named(name: String) -> Result<Self, JobObjectError> {
        let wide = encode_wide(&name);
        let handle =
            unsafe { CreateJobObjectW(None, PCWSTR(wide.as_ptr())) }.map_err(|source| {
                JobObjectError::OpenOrCreate {
                    name: name.clone(),
                    source,
                }
            })?;
        configure_kill_on_close(handle, &name)?;
        Ok(Self {
            handle,
            name: Some(name),
        })
    }

    fn open_existing(name: &str) -> Result<Option<Self>, JobObjectError> {
        let wide = encode_wide(name);
        match unsafe { OpenJobObjectW(JOB_OBJECT_ALL_ACCESS, false, PCWSTR(wide.as_ptr())) } {
            Ok(handle) => Ok(Some(Self {
                handle,
                name: Some(name.to_string()),
            })),
            Err(source) if source.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => Ok(None),
            Err(source) => Err(JobObjectError::OpenExisting {
                name: name.to_string(),
                source,
            }),
        }
    }

    /// Create an unnamed job. Suitable for one-shot spawns with no identity
    /// where there is nothing to preempt and nothing to find later.
    pub fn anonymous() -> Result<Self, JobObjectError> {
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(|source| {
            JobObjectError::OpenOrCreate {
                name: "<anonymous>".into(),
                source,
            }
        })?;
        configure_kill_on_close(handle, "<anonymous>")?;
        Ok(Self { handle, name: None })
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn assign_pid(&self, pid: u32) -> Result<(), JobObjectError> {
        unsafe {
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)
                .map_err(|source| JobObjectError::AssignProcess { source })?;
            let result = AssignProcessToJobObject(self.handle, process);
            let _ = CloseHandle(process);
            result.map_err(|source| JobObjectError::AssignProcess { source })
        }
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }

    /// Enumerate the PIDs currently in the job. The Win32 API requires a
    /// caller-sized buffer; we start at 64 slots and grow until the call
    /// succeeds (`MORE_DATA` means the buffer was too small).
    pub fn members(&self) -> Result<Vec<u32>, JobObjectError> {
        let mut capacity: usize = 64;
        loop {
            let header_size = size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            let pid_size = size_of::<usize>();
            let buffer_size = header_size + pid_size * capacity.saturating_sub(1);
            let mut buffer: Vec<u8> = vec![0; buffer_size];
            let mut returned: u32 = 0;
            let result = unsafe {
                QueryInformationJobObject(
                    Some(self.handle),
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr().cast(),
                    buffer_size as u32,
                    Some(&mut returned),
                )
            };
            match result {
                Ok(()) => {
                    let header: &JOBOBJECT_BASIC_PROCESS_ID_LIST =
                        unsafe { &*buffer.as_ptr().cast() };
                    let returned_count = header.NumberOfProcessIdsInList as usize;
                    let pids: &[usize] = unsafe {
                        std::slice::from_raw_parts(header.ProcessIdList.as_ptr(), returned_count)
                    };
                    return Ok(pids.iter().map(|&p| p as u32).collect());
                }
                Err(e) => {
                    // ERROR_MORE_DATA (0x800700EA) means the buffer was too small.
                    if e.code().0 as u32 == 0x800700EA && capacity < 65_536 {
                        capacity *= 2;
                        continue;
                    }
                    return Err(JobObjectError::Query { source: e });
                }
            }
        }
    }

    pub fn terminate_all(&self) -> Result<(), JobObjectError> {
        unsafe { TerminateJobObject(self.handle, 1) }
            .map_err(|source| JobObjectError::TerminatePredecessor { source })
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

pub struct IdentityClaim {
    identity: String,
    _lock: IdentityLock,
    state_path: PathBuf,
}

impl IdentityClaim {
    pub fn acquire(identity: &str) -> Result<Self, JobObjectError> {
        Ok(Self {
            identity: identity.to_string(),
            _lock: IdentityLock::acquire(identity)?,
            state_path: state_path(identity)?,
        })
    }

    pub fn terminate_predecessor(&self) -> Result<(), JobObjectError> {
        let Some(previous_name) = read_state(&self.state_path)? else {
            return Ok(());
        };
        let Some(previous) = JobObject::open_existing(&previous_name)? else {
            return Ok(());
        };
        previous.terminate_all()?;
        Ok(())
    }

    pub fn create_successor_job(&self) -> Result<(JobObject, String), JobObjectError> {
        let name = unique_job_name(&self.identity);
        Ok((JobObject::named(name.clone())?, name))
    }

    pub fn commit(&self, job_name: &str) -> Result<(), JobObjectError> {
        fs::write(&self.state_path, job_name).map_err(|source| JobObjectError::IdentityState {
            path: self.state_path.display().to_string(),
            source,
        })
    }
}

struct IdentityLock {
    handle: HANDLE,
}

impl IdentityLock {
    fn acquire(identity: &str) -> Result<Self, JobObjectError> {
        let name = format!("{LOCK_NAMESPACE}{}", identity_key(identity));
        let wide = encode_wide(&name);
        let handle =
            unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) }.map_err(|source| {
                JobObjectError::LockAcquire {
                    name: identity.to_string(),
                    source,
                }
            })?;
        let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            Ok(Self { handle })
        } else {
            unsafe {
                let _ = CloseHandle(handle);
            }
            Err(JobObjectError::LockWait {
                name: identity.to_string(),
                status: wait.0,
            })
        }
    }
}

impl Drop for IdentityLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

fn configure_kill_on_close(handle: HANDLE, identity: &str) -> Result<(), JobObjectError> {
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&info).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(|source| JobObjectError::OpenOrCreate {
        name: format!("<configure:{identity}>"),
        source,
    })
}

fn state_path(identity: &str) -> Result<PathBuf, JobObjectError> {
    let dir = std::env::temp_dir().join(STATE_DIR);
    fs::create_dir_all(&dir).map_err(|source| JobObjectError::IdentityState {
        path: dir.display().to_string(),
        source,
    })?;
    Ok(dir.join(format!("{}.txt", identity_key(identity))))
}

fn read_state(path: &Path) -> Result<Option<String>, JobObjectError> {
    match fs::read_to_string(path) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(JobObjectError::IdentityState {
            path: path.display().to_string(),
            source,
        }),
    }
}

fn unique_job_name(identity: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!(
        "{OBJECT_NAMESPACE}{}-{}-{nanos}",
        identity_key(identity),
        std::process::id()
    )
}

fn identity_key(identity: &str) -> String {
    let preview: String = identity
        .chars()
        .take(32)
        .map(|ch| if ch == ':' { '.' } else { ch })
        .collect();
    format!("{preview}-{:016x}", fnv1a(identity.as_bytes()))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn encode_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_wide_produces_nul_terminated_string() {
        let wide = encode_wide("Local\\uncontainerizable-test");
        let decoded: String = std::char::decode_utf16(wide.iter().take(wide.len() - 1).copied())
            .map(|r| r.unwrap_or('?'))
            .collect();
        assert_eq!(decoded, "Local\\uncontainerizable-test");
        assert_eq!(*wide.last().unwrap(), 0u16);
    }

    #[test]
    fn identity_key_replaces_colon_and_adds_hash_suffix() {
        let key = identity_key("com.example:test");
        assert!(key.starts_with("com.example.test-"));
        assert_eq!(key.len(), "com.example.test-".len() + 16);
    }
}
