//! Starts before any GUI/capture initialization. Owns the entire worker process tree.
use std::{
    io::{BufRead, BufReader, Write},
    mem::{size_of, zeroed},
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Child, Command, Stdio},
    ptr::{null, null_mut},
    sync::mpsc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{JobObjects::*, Threading::*},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub fn gate() -> Result<(), String> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.trim() != "GO" {
        return Err("Worker refused: supervisor handshake missing".into());
    }
    Ok(())
}
pub fn signal(value: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

pub(crate) struct Job(pub(crate) HANDLE);
impl Job {
    pub(crate) fn new() -> Result<Self, String> {
        unsafe {
            let handle = CreateJobObjectW(null(), null());
            if handle.is_null() {
                return Err(format!("CreateJobObject: {}", GetLastError()));
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                return Err(format!("Job limits: {}", GetLastError()));
            }
            Ok(job)
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Worker {
    child: Child,
    _job: Job,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Emergency(bool);
impl Emergency {
    fn new(enabled: bool) -> Result<Self, String> {
        if enabled
            && unsafe { RegisterHotKey(null_mut(), 100, MOD_CONTROL | MOD_NOREPEAT, b'0' as u32) }
                == 0
        {
            return Err("Ctrl+0 emergency key unavailable. No worker was started.".into());
        }
        Ok(Self(enabled))
    }
    fn pressed(&self) -> bool {
        if !self.0 {
            return false;
        }
        unsafe {
            let mut msg = MSG::default();
            let mut pressed = false;
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                pressed |= msg.message == WM_HOTKEY && msg.wParam == 100;
            }
            pressed
        }
    }
}
impl Drop for Emergency {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                UnregisterHotKey(null_mut(), 100);
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub startup: Duration,
    pub response: Duration,
    pub session: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(8),
            response: Duration::from_secs(3),
            session: Duration::from_secs(300),
        }
    }
}
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Exited(i32),
    Stopped(&'static str),
}

pub fn run(args: &[String], limits: Limits, emergency: bool) -> Result<Outcome, String> {
    // No desktop API in the test path, including hotkey registration.
    let emergency = Emergency::new(emergency)?;
    let job = Job::new()?;
    let child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut worker = Worker { child, _job: job };
    unsafe {
        if AssignProcessToJobObject(worker._job.0, worker.child.as_raw_handle() as HANDLE) == 0 {
            return Err(format!(
                "Worker containment unavailable: {}. Worker never initialized.",
                GetLastError()
            ));
        }
    }
    let stdout = worker.child.stdout.take().ok_or("Worker stdout missing")?;
    let (tx, rx) = mpsc::sync_channel(32);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            // Protocol is tiny; no screen data crosses this channel.
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.len() > 64 || tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    worker
        .child
        .stdin
        .as_mut()
        .ok_or("Worker stdin missing")?
        .write_all(b"GO\n")
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let mut last = start;
    let mut ready = false;
    let mut active = None;
    loop {
        if let Some(status) = worker.child.try_wait().map_err(|e| e.to_string())? {
            return Ok(Outcome::Exited(status.code().unwrap_or(1)));
        }
        for line in rx.try_iter().take(32) {
            match line.trim() {
                "READY" => {
                    ready = true;
                    last = Instant::now();
                }
                "PING" if ready => last = Instant::now(),
                "ACTIVE" if ready => {
                    active.get_or_insert(Instant::now());
                }
                "IDLE" => active = None,
                _ => {}
            }
        }
        let reason = if emergency.pressed() {
            Some("emergency key")
        } else if !ready && start.elapsed() > limits.startup {
            Some("initialization timeout")
        } else if ready && last.elapsed() > limits.response {
            Some("UI response timeout")
        } else if active.is_some_and(|t: Instant| t.elapsed() > limits.session) {
            Some("session deadline")
        } else {
            None
        };
        if let Some(reason) = reason {
            // Explicit whole-tree termination; Drop also protects parent crash/exit.
            unsafe {
                if TerminateJobObject(worker._job.0, 73) == 0 {
                    return Err(format!("Job termination failed: {}", GetLastError()));
                }
            }
            worker.child.wait().map_err(|e| e.to_string())?;
            return Ok(Outcome::Stopped(reason));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub fn fault_worker(mode: &str) -> Result<(), String> {
    gate()?;
    if mode == "startup" {
        std::thread::sleep(Duration::from_secs(30));
        return Ok(());
    }
    signal("READY");
    if mode == "crash" {
        std::process::exit(17);
    }
    if mode == "stall" {
        std::thread::sleep(Duration::from_secs(30));
        return Ok(());
    }
    if mode == "session" {
        signal("ACTIVE");
    }
    for _ in 0..if mode == "session" { 300 } else { 3 } {
        signal("PING");
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

pub fn self_test() -> Result<(), String> {
    let limits = Limits {
        startup: Duration::from_millis(600),
        response: Duration::from_millis(400),
        session: Duration::from_millis(500),
    };
    for (mode, expected) in [
        ("healthy", Outcome::Exited(0)),
        ("startup", Outcome::Stopped("initialization timeout")),
        ("stall", Outcome::Stopped("UI response timeout")),
        ("crash", Outcome::Exited(17)),
        ("session", Outcome::Stopped("session deadline")),
    ] {
        let actual = run(&["--fault-worker".into(), mode.into()], limits, false)?;
        if actual != expected {
            return Err(format!("{mode}: expected {expected:?}, got {actual:?}"));
        }
        println!("PASS supervisor: {mode}: {actual:?}");
    }
    job_close_test()?;
    Ok(())
}

// Abrupt exit bypasses Rust destructors. Windows must close the job handle.
pub fn job_owner_fixture() -> Result<(), String> {
    gate()?;
    let job = Job::new()?;
    let child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .args(["--fault-worker", "startup"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut worker = Worker { child, _job: job };
    unsafe {
        if AssignProcessToJobObject(worker._job.0, worker.child.as_raw_handle() as HANDLE) == 0 {
            return Err("Fixture job assignment failed".into());
        }
    }
    worker
        .child
        .stdin
        .as_mut()
        .ok_or("Fixture stdin")?
        .write_all(b"GO\n")
        .map_err(|e| e.to_string())?;
    signal(&worker.child.id().to_string());
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.trim() == "EXIT" {
        std::process::exit(0);
    }
    Err("Fixture handshake ended".into())
}
fn job_close_test() -> Result<(), String> {
    let job = Job::new()?;
    let child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("--job-owner-fixture")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut owner = Worker { child, _job: job };
    unsafe {
        if AssignProcessToJobObject(owner._job.0, owner.child.as_raw_handle() as HANDLE) == 0 {
            return Err("Outer fixture job failed".into());
        }
    }
    let stdout = owner.child.stdout.take().ok_or("Fixture stdout")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    owner
        .child
        .stdin
        .as_mut()
        .ok_or("Fixture stdin")?
        .write_all(b"GO\n")
        .map_err(|e| e.to_string())?;
    let pid = rx
        .recv_timeout(Duration::from_secs(4))
        .map_err(|e| e.to_string())?
        .trim()
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return Err("Cannot observe fixture worker".into());
        }
        struct Process(HANDLE);
        impl Drop for Process {
            fn drop(&mut self) {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
        let handle = Process(handle);
        if WaitForSingleObject(handle.0, 0) != WAIT_TIMEOUT {
            return Err("Fixture exited before supervisor death".into());
        }
        owner
            .child
            .stdin
            .as_mut()
            .ok_or("Fixture stdin")?
            .write_all(b"EXIT\n")
            .map_err(|e| e.to_string())?;
        if WaitForSingleObject(handle.0, 3000) != WAIT_OBJECT_0 {
            return Err("Worker survived abrupt supervisor exit".into());
        }
    }
    println!("PASS supervisor: abrupt parent exit reaps job worker (without Rust cleanup)");
    Ok(())
}
