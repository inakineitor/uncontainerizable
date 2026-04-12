//! Windows quit-ladder stages.
//!
//! Two stages:
//! * `wm_close_root`: Post `WM_CLOSE` to every top-level window owned by
//!   the root PID (the most common case: a GUI app). Non-terminal.
//! * `terminate_job`: `TerminateJobObject(handle, 1)`. Terminal, kernel-level.
//!
//! The terminal stage holds an `Arc<JobObject>` captured from the container
//! at spawn time; the non-terminal stage only needs the PID and resolves
//! windows through `EnumWindows`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};
use windows::core::BOOL;

use crate::container::{Container, Stage};
use crate::error::StageError;

use super::job_object::JobObject;

pub fn win32_stages(job: Arc<JobObject>) -> Vec<Arc<dyn Stage>> {
    vec![Arc::new(WmCloseRoot), Arc::new(TerminateJob { job })]
}

pub struct WmCloseRoot;

#[async_trait]
impl Stage for WmCloseRoot {
    fn name(&self) -> &str {
        "wm_close_root"
    }
    fn is_terminal(&self) -> bool {
        false
    }
    fn max_wait(&self) -> Duration {
        Duration::from_secs(3)
    }
    async fn execute(&self, c: &dyn Container) -> Result<(), StageError> {
        post_wm_close_to_pid(c.pid())
    }
}

pub struct TerminateJob {
    job: Arc<JobObject>,
}

#[async_trait]
impl Stage for TerminateJob {
    fn name(&self) -> &str {
        "terminate_job"
    }
    fn is_terminal(&self) -> bool {
        true
    }
    fn max_wait(&self) -> Duration {
        Duration::from_millis(500)
    }
    async fn execute(&self, _c: &dyn Container) -> Result<(), StageError> {
        self.job.terminate_all().map_err(StageError::JobObject)?;
        Ok(())
    }
}

struct EnumCtx {
    target_pid: u32,
    windows: Vec<HWND>,
}

extern "system" fn collect_windows(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let ctx = &mut *(lparam.0 as *mut EnumCtx);
        let mut pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == ctx.target_pid && IsWindowVisible(hwnd).as_bool() {
            ctx.windows.push(hwnd);
        }
    }
    BOOL(1)
}

fn post_wm_close_to_pid(pid: u32) -> Result<(), StageError> {
    let mut ctx = EnumCtx {
        target_pid: pid,
        windows: Vec::new(),
    };
    let lparam = LPARAM(std::ptr::from_mut(&mut ctx) as isize);
    let _ = unsafe { EnumWindows(Some(collect_windows), lparam) };
    for hwnd in &ctx.windows {
        unsafe {
            let _ = PostMessageW(Some(*hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    Ok(())
}
