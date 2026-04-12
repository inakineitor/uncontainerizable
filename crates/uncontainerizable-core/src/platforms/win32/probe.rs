//! Windows probe capture. Reads the executable path from the process
//! via `QueryFullProcessImageNameW`. No bundle-id analogue on Windows.

use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

use crate::error::ProbeError;
use crate::probe::{Probe, SupportedPlatform};

pub async fn capture_probe(pid: u32) -> Result<Probe, ProbeError> {
    let mut probe = Probe::new(pid, SupportedPlatform::Windows);
    probe.executable_path = exe_path(pid).ok();
    Ok(probe)
}

fn exe_path(pid: u32) -> Result<PathBuf, ProbeError> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(|e| {
            ProbeError::Subprocess {
                command: "OpenProcess".into(),
                message: e.to_string(),
            }
        })?;
        let mut buffer: Vec<u16> = vec![0; 1024];
        let mut size: u32 = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        result.map_err(|e| ProbeError::Subprocess {
            command: "QueryFullProcessImageNameW".into(),
            message: e.to_string(),
        })?;
        buffer.truncate(size as usize);
        let os = std::ffi::OsString::from_wide(&buffer);
        Ok(PathBuf::from(os))
    }
}
