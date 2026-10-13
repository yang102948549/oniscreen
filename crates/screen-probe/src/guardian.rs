//! QUARANTINED with the legacy engine; not compiled into the containment build.
//! Independent recovery process: never shares the capture thread or its input hook.
use std::{
    io::{BufRead, BufReader, Write},
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::Threading::*,
    UI::{Input::KeyboardAndMouse::*, Magnification::*, WindowsAndMessaging::*},
};

pub const RECOVERY_EXIT: u32 = 73;
pub struct Guardian(Child);
impl Guardian {
    pub fn start(hwnd: HWND) -> Result<Self, String> {
        let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args([
                "--guardian",
                &std::process::id().to_string(),
                &(hwnd as usize).to_string(),
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Recovery process could not start: {e}"))?;
        let stdout = child.stdout.take().ok_or("Recovery pipe unavailable")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let ready = rx.recv_timeout(Duration::from_secs(4));
        if !matches!(ready, Ok(ref text) if text.trim() == "READY") {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Independent recovery unavailable (Ctrl+0 may be in use): {ready:?}"
            ));
        }
        Ok(Self(child))
    }
    pub fn alive(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}
impl Drop for Guardian {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn run(pid: u32, hwnd: HWND) -> Result<(), String> {
    unsafe {
        let mut owner = 0;
        GetWindowThreadProcessId(hwnd, &mut owner);
        if pid == std::process::id() || owner != pid {
            return Err("Invalid recovery target".into());
        }
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if handle.is_null() {
            return Err("Cannot open recovery target".into());
        }
        struct Process(HANDLE);
        impl Drop for Process {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let process = Process(handle);
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        if QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut length) == 0 {
            return Err("Cannot identify recovery target".into());
        }
        let target_path = std::fs::canonicalize(String::from_utf16_lossy(&path[..length as usize]))
            .map_err(|e| e.to_string())?;
        let self_path = std::fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if target_path != self_path {
            return Err("Recovery target is not this executable".into());
        }
        if RegisterHotKey(null_mut(), 100, MOD_CONTROL | MOD_NOREPEAT, b'0' as u32) == 0 {
            return Err(format!("Ctrl+0 registration failed: {}", GetLastError()));
        }
        struct Hotkey;
        impl Drop for Hotkey {
            fn drop(&mut self) {
                unsafe {
                    UnregisterHotKey(null_mut(), 100);
                }
            }
        }
        let _hotkey = Hotkey;
        println!("READY");
        let _ = std::io::stdout().flush();
        let start = Instant::now();
        let mut failures = 0;
        loop {
            if WaitForSingleObject(process.0, 0) != WAIT_TIMEOUT {
                break;
            }
            let mut msg = MSG::default();
            let mut emergency = false;
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_HOTKEY && msg.wParam == 100 {
                    emergency = true;
                }
            }
            if start.elapsed() > Duration::from_secs(1) {
                let mut result = 0;
                if SendMessageTimeoutW(
                    hwnd,
                    WM_NULL,
                    0,
                    0,
                    SMTO_ABORTIFHUNG | SMTO_BLOCK,
                    250,
                    &mut result,
                ) == 0
                {
                    failures += 1;
                } else {
                    failures = 0;
                }
            }
            if emergency || failures >= 4 {
                if TerminateProcess(process.0, RECOVERY_EXIT) == 0 {
                    return Err("Failed to terminate unresponsive probe".into());
                }
                WaitForSingleObject(process.0, 3000);
                // The failed capture/input thread has exited before touching Magnification.
                if MagInitialize() != 0 {
                    MagShowSystemCursor(1);
                    MagUninitialize();
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}
