//! One-shot capture in a disposable process; UI polls and never waits on GDI.
use crate::{
    engine::Frame,
    geometry::Rect,
    supervisor::{self, Job},
};
use std::{
    io::Write,
    mem::{size_of, zeroed},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command, Stdio},
    ptr::null_mut,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{JobObjects::*, Threading::*},
};

pub struct Task {
    child: Child,
    _job: Job,
    rx: mpsc::Receiver<Result<Frame, String>>,
    start: Instant,
    timeout: Duration,
}
impl Task {
    pub fn start(args: &[String], timeout: Duration) -> Result<Self, String> {
        let job = Job::new()?;
        let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| e.to_string())?;
        unsafe {
            if AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) == 0 {
                let error = GetLastError();
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Capture job assignment: {error}"));
            }
        }
        let (tx, rx) = mpsc::channel();
        let stdout = child.stdout.take().ok_or("No capture stdout")?;
        let mut task = Self {
            child,
            _job: job,
            rx,
            start: Instant::now(),
            timeout,
        };
        std::thread::spawn(move || {
            let _ = tx.send(Frame::read(stdout));
        });
        task.child
            .stdin
            .as_mut()
            .ok_or("No capture gate")?
            .write_all(b"GO\n")
            .map_err(|e| e.to_string())?;
        Ok(task)
    }
    pub fn desktop(bounds: Rect) -> Result<Self, String> {
        Self::start(
            &[
                "--capture-worker".into(),
                bounds.left.to_string(),
                bounds.top.to_string(),
                bounds.width().to_string(),
                bounds.height().to_string(),
            ],
            Duration::from_secs(4),
        )
    }
    pub fn poll(&mut self) -> Option<Result<Frame, String>> {
        if self.start.elapsed() > self.timeout {
            return Some(Err("Capture timeout; worker discarded".into()));
        }
        match self.child.try_wait() {
            Ok(Some(status)) if !status.success() => Some(Err(format!("Capture exited: {status}"))),
            Ok(Some(_)) => match self.rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(_) => Some(Err("Capture channel closed".into())),
            },
            Ok(None) => None,
            Err(e) => Some(Err(e.to_string())),
        }
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn worker(bounds: Rect) -> Result<(), String> {
    supervisor::gate()?;
    unsafe {
        // Only this explicitly requested worker may read the real desktop.
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
        let mut frame = Frame::new(bounds.width(), bounds.height())?;
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return Err("Capture GetDC failed".into());
        }
        struct Screen(HDC);
        impl Drop for Screen {
            fn drop(&mut self) {
                unsafe {
                    ReleaseDC(null_mut(), self.0);
                }
            }
        }
        let screen = Screen(screen);
        let dc = CreateCompatibleDC(screen.0);
        if dc.is_null() {
            return Err("Capture DC failed".into());
        }
        struct Dc(HDC);
        impl Drop for Dc {
            fn drop(&mut self) {
                unsafe {
                    DeleteDC(self.0);
                }
            }
        }
        let dc = Dc(dc);
        let info = bitmap_info(frame.width, frame.height);
        let mut pixels = null_mut();
        let bitmap = CreateDIBSection(screen.0, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if bitmap.is_null() {
            return Err("Capture bitmap failed".into());
        }
        struct Bitmap(HBITMAP);
        impl Drop for Bitmap {
            fn drop(&mut self) {
                unsafe {
                    DeleteObject(self.0);
                }
            }
        }
        let bitmap = Bitmap(bitmap);
        let old = SelectObject(dc.0, bitmap.0);
        let success = BitBlt(
            dc.0,
            0,
            0,
            frame.width,
            frame.height,
            screen.0,
            bounds.left,
            bounds.top,
            SRCCOPY | CAPTUREBLT,
        );
        // Flush GDI before CPU reading the DIB pixels.
        let flushed = GdiFlush();
        SelectObject(dc.0, old);
        if success == 0 || flushed == 0 {
            return Err("Capture BitBlt/GdiFlush failed".into());
        }
        let count = frame.pixels.len();
        frame
            .pixels
            .copy_from_slice(std::slice::from_raw_parts(pixels as *const u32, count));
        frame.write(std::io::stdout().lock())
    }
}
pub fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { zeroed() }
        },
        ..unsafe { zeroed() }
    }
}
pub fn fixture_worker(mode: &str) -> Result<(), String> {
    supervisor::gate()?;
    match mode {
        "good" => Frame::fixture(320, 200)?.write(std::io::stdout().lock()),
        "truncated" => std::io::stdout()
            .write_all(b"SPF2")
            .map_err(|e| e.to_string()),
        "stall" => {
            std::thread::sleep(Duration::from_secs(30));
            Ok(())
        }
        _ => Err("Unknown fixture".into()),
    }
}
pub fn self_test() -> Result<(), String> {
    for mode in ["good", "truncated", "stall"] {
        let mut task = Task::start(
            &["--capture-fixture".into(), mode.into()],
            Duration::from_millis(800),
        )?;
        let result = loop {
            if let Some(result) = task.poll() {
                break result;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if (mode == "good") != result.is_ok() {
            return Err(format!("Capture fixture failed: {mode}"));
        }
        drop(task);
        println!("PASS capture subprocess: {mode}");
    }
    Ok(())
}
