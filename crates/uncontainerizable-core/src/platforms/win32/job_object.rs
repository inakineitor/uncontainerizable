//! Thin wrapper over a Windows Job Object.
//!
//! A Job Object is a kernel-backed container for processes. Assigning the
//! root process places it (and its descendants by default) into the job;
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` guarantees every member is SIGKILL-equivalent
//! when the last handle to the job is released, so the supervisor crashing
//! never leaves orphans behind.
//!
//! Identity preemption uses the named-object namespace: a prior job with the
//! same `Local\\uncontainerizable-<identity>` name is opened, terminated, and
//! closed before creating the fresh one. Opening-by-name is atomic at the
//! object-manager level, so concurrent spawns serialize.

use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, OpenJobObjectW,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};
use windows::core::PCWSTR;

use crate::error::JobObjectError;

const OBJECT_NAMESPACE: &str = "Local\\uncontainerizable-";

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
    #[allow(dead_code)]
    name: Option<String>,
}

impl JobObject {
    /// Open an existing named job, terminate every process in it, close the
    /// handle to release the name, then create a fresh job with that name.
    pub fn open_or_replace(identity: &str) -> Result<Self, JobObjectError> {
        let wide = encode_name(identity);
        let name_ptr = PCWSTR(wide.as_ptr());

        let existing = unsafe { OpenJobObjectW(JOB_OBJECT_ALL_ACCESS, false, name_ptr).ok() };
        if let Some(handle) = existing {
            unsafe {
                let _ = TerminateJobObject(handle, 1);
                let _ = CloseHandle(handle);
            }
        }

        let handle = unsafe { CreateJobObjectW(None, name_ptr) }.map_err(|source| {
            JobObjectError::OpenOrCreate {
                name: identity.to_string(),
                source,
            }
        })?;
        configure_kill_on_close(handle, identity)?;
        Ok(Self {
            handle,
            name: Some(identity.to_string()),
        })
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

fn encode_name(identity: &str) -> Vec<u16> {
    let name = format!("{OBJECT_NAMESPACE}{identity}");
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_name_produces_nul_terminated_local_namespace() {
        let wide = encode_name("com.example.test");
        let decoded: String = std::char::decode_utf16(wide.iter().take(wide.len() - 1).copied())
            .map(|r| r.unwrap_or('?'))
            .collect();
        assert_eq!(decoded, "Local\\uncontainerizable-com.example.test");
        assert_eq!(*wide.last().unwrap(), 0u16);
    }
}
