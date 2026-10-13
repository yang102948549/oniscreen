//! QUARANTINED: excluded from main.rs after desktop lockup and cursor loss.
//! Do not reconnect this module before isolated engine and recovery validation.
//! Stage-one native probe. Deliberately independent of the future Tauri UI.
//! GDI diagnostic ink is temporary; production ink is gated on capture/input validation.
use crate::geometry::{absolute, Point, Rect, Viewport, WheelZoom};
use crate::guardian::Guardian;
use std::{
    cell::Cell,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::core::BOOL;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Dwm::DwmFlush,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, Magnification::*, WindowsAndMessaging::*},
};

const TAG: usize = 0x53435250;
const FAIL: u32 = WM_APP + 1;
const ESCAPE: u32 = WM_APP + 2;
const ZOOM_WHEEL: u32 = WM_APP + 3;
const TEST_HANG: u32 = WM_APP + 4;
const FRAME_TIMER: usize = 1;
const WATCHDOG_TIMER: usize = 2;
thread_local! { static APP: Cell<*mut App> = const { Cell::new(null_mut()) }; }

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn win_error(operation: &str) -> String {
    format!("{operation}: Win32 error {}", unsafe { GetLastError() })
}
fn rect(r: Rect) -> RECT {
    RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}
fn point(p: POINT) -> Point {
    Point { x: p.x, y: p.y }
}
unsafe fn cursor() -> Point {
    let mut p = POINT::default();
    GetCursorPos(&mut p);
    point(p)
}

#[derive(Clone, Debug)]
struct Monitor {
    bounds: Rect,
    name: String,
    primary: bool,
}
unsafe extern "system" fn enum_monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let mut info: MONITORINFOEXW = zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(h, &mut info as *mut _ as *mut MONITORINFO) != 0 {
        let r = info.monitorInfo.rcMonitor;
        let name = String::from_utf16_lossy(
            &info.szDevice[..info.szDevice.iter().position(|v| *v == 0).unwrap_or(32)],
        );
        (*(data as *mut Vec<Monitor>)).push(Monitor {
            bounds: Rect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            },
            name,
            primary: info.monitorInfo.dwFlags & 1 != 0,
        });
    }
    1
}
unsafe fn monitors() -> Vec<Monitor> {
    let mut result = Vec::new();
    EnumDisplayMonitors(
        null_mut(),
        null(),
        Some(enum_monitor),
        &mut result as *mut _ as isize,
    );
    result.sort_by_key(|m: &Monitor| (!m.primary, m.name.clone()));
    result
}
unsafe fn desktop() -> Rect {
    let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
    Rect {
        left,
        top,
        right: left + GetSystemMetrics(SM_CXVIRTUALSCREEN),
        bottom: top + GetSystemMetrics(SM_CYVIRTUALSCREEN),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Idle,
    Frozen,
    Live,
}

struct Snapshot {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
}
impl Snapshot {
    unsafe fn capture(bounds: Rect) -> Result<Self, String> {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return Err(win_error("GetDC"));
        }
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, bounds.width(), bounds.height());
        if dc.is_null() || bitmap.is_null() {
            if !dc.is_null() {
                DeleteDC(dc);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            ReleaseDC(null_mut(), screen);
            return Err(win_error("snapshot allocation"));
        }
        let old = SelectObject(dc, bitmap);
        let ok = BitBlt(
            dc,
            0,
            0,
            bounds.width(),
            bounds.height(),
            screen,
            bounds.left,
            bounds.top,
            SRCCOPY | CAPTUREBLT,
        );
        ReleaseDC(null_mut(), screen);
        let frame = Self { dc, bitmap, old };
        if ok == 0 {
            return Err(win_error("BitBlt snapshot"));
        }
        Ok(frame)
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

struct App {
    guardian: Option<Guardian>,
    guardian_test: bool,
    wheel: WheelZoom,
    control: HWND,
    host: HWND,
    mag: HWND,
    overlay: HWND,
    mouse_hook: HHOOK,
    keyboard_hook: HHOOK,
    registered: Vec<i32>,
    initialized: bool,
    monitor: Monitor,
    displays: Vec<Monitor>,
    desktop: Rect,
    view: Viewport,
    mode: Mode,
    pointer: bool,
    ink: bool,
    drawing: bool,
    virtual_cursor: Point,
    cursor_hidden: bool,
    injected_buttons: u8,
    strokes: Vec<Vec<Point>>,
    pulse: Option<Instant>,
    snapshot: Option<Snapshot>,
    frames: u64,
    started: Instant,
    smoke_until: Option<Instant>,
    session_until: Option<Instant>,
    status: String,
}

impl App {
    unsafe fn new(monitor: Monitor) -> Self {
        let p = monitor.bounds.clamp(cursor());
        Self {
            guardian: None,
            guardian_test: false,
            wheel: WheelZoom::default(),
            control: null_mut(),
            host: null_mut(),
            mag: null_mut(),
            overlay: null_mut(),
            mouse_hook: null_mut(),
            keyboard_hook: null_mut(),
            registered: vec![],
            initialized: false,
            displays: monitors(),
            desktop: desktop(),
            view: Viewport::new(monitor.bounds, 2.0, p),
            monitor,
            mode: Mode::Idle,
            pointer: false,
            ink: false,
            drawing: false,
            virtual_cursor: p,
            cursor_hidden: false,
            injected_buttons: 0,
            strokes: vec![],
            pulse: None,
            snapshot: None,
            frames: 0,
            started: Instant::now(),
            smoke_until: None,
            session_until: None,
            status: "Ready. No capture or input hooks active.".into(),
        }
    }
    unsafe fn setup(&mut self, smoke: bool) -> Result<(), String> {
        if MagInitialize() == 0 {
            return Err(win_error("MagInitialize"));
        }
        self.initialized = true;
        let instance = GetModuleHandleW(null());
        let class = wide("ScreenProbeWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(win_error("RegisterClassW"));
        }
        self.control = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Screen Probe - UNVERIFIED / Stage 1").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            690,
            470,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if self.control.is_null() {
            return Err(win_error("control window"));
        }
        let r = self.monitor.bounds;
        self.host = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            class.as_ptr(),
            wide("ScreenProbe magnified desktop").as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            r.left,
            r.top,
            r.width(),
            r.height(),
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if self.host.is_null() {
            return Err(win_error("magnifier host"));
        }
        if SetLayeredWindowAttributes(self.host, 0, 255, LWA_ALPHA) == 0 {
            return Err(win_error("host transparency"));
        }
        self.mag = CreateWindowExW(
            0,
            wide("Magnifier").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            r.width(),
            r.height(),
            self.host,
            null_mut(),
            instance,
            null(),
        );
        if self.mag.is_null() {
            return Err(win_error("Magnifier control"));
        }
        self.overlay = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            class.as_ptr(),
            wide("ScreenProbe diagnostic ink").as_ptr(),
            WS_POPUP,
            r.left,
            r.top,
            r.width(),
            r.height(),
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if self.overlay.is_null() {
            return Err(win_error("diagnostic overlay"));
        }
        if SetLayeredWindowAttributes(self.overlay, 0, 190, LWA_COLORKEY | LWA_ALPHA) == 0 {
            return Err(win_error("overlay transparency"));
        }
        let mut exclude = [self.host, self.overlay];
        if MagSetWindowFilterList(
            self.mag,
            MW_FILTERMODE_EXCLUDE,
            exclude.len() as i32,
            exclude.as_mut_ptr(),
        ) == 0
        {
            return Err(win_error("MagSetWindowFilterList"));
        }
        self.update_source()?;
        if !smoke {
            // All shortcuts must register before the app can intercept any input.
            for (id, key) in [
                (1, b'1' as u32),
                (2, b'2' as u32),
                (3, b'3' as u32),
                (4, b'4' as u32),
                (5, b'5' as u32),
                (6, b'6' as u32),
                (7, b'7' as u32),
                (8, b'8' as u32),
                (9, b'9' as u32),
            ] {
                if RegisterHotKey(self.control, id, MOD_CONTROL | MOD_NOREPEAT, key) == 0 {
                    let code = GetLastError();
                    let reason = match code {
                        ERROR_HOTKEY_ALREADY_REGISTERED =>
                            "Shortcut already registered. Close another probe instance or the conflicting utility.",
                        ERROR_ACCESS_DENIED =>
                            "Access denied. The desktop session or execution sandbox does not allow global hotkeys. Run in a normal interactive desktop terminal.",
                        _ => "Unable to register the global shortcut.",
                    };
                    return Err(format!(
                        "{reason} RegisterHotKey: id {id}, Win32 error {code}"
                    ));
                }
                self.registered.push(id);
            }
            if let Some(other) = self.displays.iter().find(|m| m.name != self.monitor.name) {
                SetWindowPos(
                    self.control,
                    null_mut(),
                    other.bounds.left + 40,
                    other.bounds.top + 40,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            ShowWindow(self.control, SW_SHOW);
        }
        if SetTimer(self.control, WATCHDOG_TIMER, 1000, None) == 0 {
            return Err(win_error("watchdog timer"));
        }
        Ok(())
    }
    unsafe fn set_status(&mut self, text: String) {
        println!("{text}");
        self.status = text;
        InvalidateRect(self.control, null(), 1);
    }
    unsafe fn hooks(&mut self) -> Result<(), String> {
        if !self.guardian.as_mut().is_some_and(|g| g.alive()) {
            return Err("Independent recovery is unavailable; activation refused.".into());
        }
        if self.mouse_hook.is_null() {
            self.mouse_hook =
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), GetModuleHandleW(null()), 0);
            if self.mouse_hook.is_null() {
                return Err(win_error("mouse hook"));
            }
            self.keyboard_hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_proc),
                GetModuleHandleW(null()),
                0,
            );
            if self.keyboard_hook.is_null() {
                return Err(win_error("keyboard hook"));
            }
        }
        if SetTimer(self.control, FRAME_TIMER, 16, None) == 0 {
            return Err(win_error("frame timer"));
        }
        self.session_until = Some(Instant::now() + Duration::from_secs(300));
        Ok(())
    }
    unsafe fn hide_system_cursor(&mut self, hide: bool) {
        if self.cursor_hidden != hide {
            if MagShowSystemCursor(i32::from(!hide)) != 0 {
                self.cursor_hidden = hide;
            } else {
                PostMessageW(self.control, FAIL, 0, 0);
            }
        }
    }
    unsafe fn update_source(&mut self) -> Result<(), String> {
        let z = self.view.zoom;
        let mut transform = MAGTRANSFORM {
            v: [z, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 1.0],
        };
        if MagSetWindowTransform(self.mag, &mut transform) == 0 {
            return Err(win_error("MagSetWindowTransform"));
        }
        if MagSetWindowSource(self.mag, rect(self.view.source)) == 0 {
            return Err(win_error("MagSetWindowSource"));
        }
        InvalidateRect(self.mag, null(), 0);
        Ok(())
    }
    unsafe fn start(&mut self, mode: Mode) -> Result<(), String> {
        // Fail before installing hooks, changing cursor state, or showing an
        // overlay. Live mode hung the UI thread on the user's desktop. It must
        // not be exposed again until capture/input isolation is validated.
        if mode == Mode::Live {
            return Err("Live zoom is disabled after a confirmed hang. Input/capture isolation and independent recovery must be verified before re-enabling it.".into());
        }
        let same = self.mode == mode;
        self.stop();
        if same {
            self.set_status("Zoom stopped; input restored.".into());
            return Ok(());
        }
        self.virtual_cursor = self.monitor.bounds.clamp(cursor());
        self.wheel = WheelZoom::default();
        self.view = Viewport::new(self.monitor.bounds, self.view.zoom, self.virtual_cursor);
        // Hide every probe surface before capturing the frozen frame.
        if mode == Mode::Frozen {
            // ShowWindow(SW_HIDE) precedes compositor presentation. Wait for the
            // next present so switching from live cannot capture our stale overlay.
            let result = DwmFlush();
            if result < 0 {
                return Err(format!("DwmFlush before snapshot: HRESULT {result:#x}"));
            }
            self.snapshot = Some(Snapshot::capture(self.monitor.bounds)?);
            ShowWindow(self.mag, SW_HIDE);
        } else {
            ShowWindow(self.mag, SW_SHOWNA);
        }
        self.update_source()?;
        self.hooks()?;
        self.mode = mode;
        self.pointer = true;
        self.frames = 0;
        self.started = Instant::now();
        ShowWindow(self.host, SW_SHOWNA);
        SetWindowPos(
            self.overlay,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        if mode == Mode::Live {
            self.hide_system_cursor(true);
            self.inject(
                self.view.to_source(self.virtual_cursor),
                MOUSEEVENTF_MOVE,
                0,
            )?;
        }
        self.set_status(format!(
            "{mode:?} at {:.2}x. Scroll wheel to zoom. Ctrl+0: independent emergency exit.",
            self.view.zoom
        ));
        Ok(())
    }
    unsafe fn stop(&mut self) {
        let was_live = self.mode == Mode::Live;
        self.mode = Mode::Idle;
        self.pointer = false;
        self.ink = false;
        self.drawing = false;
        self.session_until = None;
        if !self.control.is_null() {
            KillTimer(self.control, FRAME_TIMER);
        }
        if !self.mouse_hook.is_null() {
            UnhookWindowsHookEx(self.mouse_hook);
            self.mouse_hook = null_mut();
        }
        if !self.keyboard_hook.is_null() {
            UnhookWindowsHookEx(self.keyboard_hook);
            self.keyboard_hook = null_mut();
        }
        // Release exactly the buttons that this probe injected, never arbitrary keyboard input.
        let releases = [
            (1, MOUSEEVENTF_LEFTUP, 0),
            (2, MOUSEEVENTF_RIGHTUP, 0),
            (4, MOUSEEVENTF_MIDDLEUP, 0),
            (8, MOUSEEVENTF_XUP, XBUTTON1 as u32),
            (16, MOUSEEVENTF_XUP, XBUTTON2 as u32),
        ];
        for (bit, flag, data) in releases {
            if self.injected_buttons & bit != 0 {
                let _ = self.inject(cursor(), flag, data);
            }
        }
        self.injected_buttons = 0;
        if self.cursor_hidden {
            MagShowSystemCursor(1);
            self.cursor_hidden = false;
        }
        if !self.overlay.is_null() {
            ShowWindow(self.overlay, SW_HIDE);
        }
        if !self.host.is_null() {
            ShowWindow(self.host, SW_HIDE);
        }
        if was_live {
            SetCursorPos(self.virtual_cursor.x, self.virtual_cursor.y);
        }
        self.snapshot = None;
        self.strokes.clear();
    }
    unsafe fn inject(&self, p: Point, flags: u32, data: u32) -> Result<(), String> {
        let (dx, dy) = absolute(p, self.desktop);
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags
                        | MOUSEEVENTF_MOVE
                        | MOUSEEVENTF_ABSOLUTE
                        | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: TAG,
                },
            },
        };
        if SendInput(1, &input, size_of::<INPUT>() as i32) != 1 {
            return Err(win_error(
                "SendInput (may be blocked by target integrity level)",
            ));
        }
        Ok(())
    }
    unsafe fn hotkey(&mut self, id: usize) -> Result<(), String> {
        match id {
            1 => self.start(Mode::Frozen)?,
            2 => self.start(Mode::Live)?,
            3 => {
                self.pointer = !self.pointer;
                if self.pointer {
                    if self.mode != Mode::Live {
                        self.virtual_cursor = cursor();
                    }
                    self.hooks()?;
                    ShowWindow(self.overlay, SW_SHOWNA);
                } else if self.mode == Mode::Idle && !self.ink {
                    self.stop();
                }
            }
            4 => {
                self.ink = !self.ink;
                self.drawing = false;
                self.virtual_cursor = if self.mode == Mode::Live {
                    self.virtual_cursor
                } else {
                    cursor()
                };
                self.hooks()?;
                ShowWindow(self.overlay, SW_SHOWNA);
                self.set_status(format!(
                    "Diagnostic pen {}. Esc returns to interaction. Ctrl+5 clears ink.",
                    if self.ink { "ON" } else { "OFF" }
                ));
            }
            5 => {
                self.strokes.clear();
                InvalidateRect(self.overlay, null(), 0);
            }
            6 | 7 => {
                self.change_zoom(if id == 7 { 0.25 } else { -0.25 })?;
            }
            8 => {
                self.stop();
                self.set_status(
                    "EMERGENCY RELEASE: overlays hidden, hooks removed, cursor restored.".into(),
                );
            }
            9 => {
                self.stop();
                PostQuitMessage(0);
            }
            _ => {}
        }
        Ok(())
    }
    unsafe fn change_zoom(&mut self, delta: f32) -> Result<(), String> {
        self.view
            .set_zoom(self.view.zoom + delta, self.virtual_cursor);
        if self.mode == Mode::Live {
            self.update_source()?;
            self.inject(
                self.view.to_source(self.virtual_cursor),
                MOUSEEVENTF_MOVE,
                0,
            )?;
        } else if self.mode == Mode::Frozen {
            InvalidateRect(self.host, null(), 0);
        }
        self.set_status(format!(
            "Zoom {:.2}x | Wheel: zoom | Ctrl+8: release | Ctrl+0: emergency exit",
            self.view.zoom
        ));
        Ok(())
    }
    unsafe fn tick(&mut self) -> Result<(), String> {
        if self.mode != Mode::Live {
            self.virtual_cursor = cursor();
        }
        if self.mode != Mode::Idle
            && self.monitor.bounds.contains(self.virtual_cursor)
            && !self.drawing
            && self.injected_buttons == 0
        {
            let p = self.virtual_cursor;
            let r = self.monitor.bounds;
            let dx = if p.x < r.left + 24 {
                -8
            } else if p.x >= r.right - 24 {
                8
            } else {
                0
            };
            let dy = if p.y < r.top + 24 {
                -8
            } else if p.y >= r.bottom - 24 {
                8
            } else {
                0
            };
            let before = self.view.source;
            self.view.pan(dx, dy);
            if self.view.source != before {
                if self.mode == Mode::Live {
                    self.inject(self.view.to_source(p), MOUSEEVENTF_MOVE, 0)?;
                } else {
                    InvalidateRect(self.host, null(), 0);
                }
            }
        }
        if self.mode == Mode::Live {
            self.update_source()?;
            self.frames += 1;
        }
        InvalidateRect(self.overlay, null(), 0);
        Ok(())
    }
    fn ink_point(&self, p: Point) -> Point {
        if self.mode == Mode::Frozen {
            self.view.to_source(p)
        } else {
            p
        }
    }
    fn display_point(&self, p: Point) -> Point {
        if self.mode == Mode::Frozen {
            Point {
                x: ((p.x - self.view.source.left) as f32 * self.view.zoom).round() as i32,
                y: ((p.y - self.view.source.top) as f32 * self.view.zoom).round() as i32,
            }
        } else {
            Point {
                x: p.x - self.monitor.bounds.left,
                y: p.y - self.monitor.bounds.top,
            }
        }
    }
    unsafe fn paint(&self, hwnd: HWND) {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        if hwnd == self.control {
            let text = format!("STAGE 1: Capture and input feasibility probe\n\nMonitor: {}  {} x {}  origin ({}, {})\n\nCtrl+1           Frozen zoom | wheel: zoom\nCtrl+2           Live zoom DISABLED\nCtrl+3           Pointer\nCtrl+4           Pen / interaction\nCtrl+5           Clear ink\nCtrl+6 / 7     Zoom down / up\nEsc                 Pen off, then zoom off\nCtrl+8           Release\nCtrl+0           EMERGENCY EXIT (independent process)\nCtrl+9           Quit\n\n{}\n\nLive zoom validation FAILED. Do not use in a lesson.\nAn independent process monitors UI responsiveness.",
                self.monitor.name, self.monitor.bounds.width(), self.monitor.bounds.height(), self.monitor.bounds.left, self.monitor.bounds.top, self.status);
            let mut bounds = RECT {
                left: 20,
                top: 15,
                right: 650,
                bottom: 440,
            };
            let font = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
            SetBkMode(dc, TRANSPARENT as i32);
            DrawTextW(
                dc,
                wide(&text).as_ptr(),
                -1,
                &mut bounds,
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(dc, font);
        } else if hwnd == self.host {
            if let Some(snapshot) = &self.snapshot {
                let r = self.monitor.bounds;
                let s = self.view.source;
                SetStretchBltMode(dc, HALFTONE);
                SetBrushOrgEx(dc, 0, 0, null_mut());
                StretchBlt(
                    dc,
                    0,
                    0,
                    r.width(),
                    r.height(),
                    snapshot.dc,
                    s.left - r.left,
                    s.top - r.top,
                    s.width(),
                    s.height(),
                    SRCCOPY,
                );
            }
        } else if hwnd == self.overlay {
            // Buffered chroma-key diagnostic overlay. Production uses Direct2D/DirectWrite.
            let r = self.monitor.bounds;
            let back = CreateCompatibleDC(dc);
            let bitmap = CreateCompatibleBitmap(dc, r.width(), r.height());
            if !back.is_null() && !bitmap.is_null() {
                let old_bitmap = SelectObject(back, bitmap);
                PatBlt(back, 0, 0, r.width(), r.height(), BLACKNESS);
                let pen = CreatePen(PS_SOLID, 4, 0x00_70_9a_ff);
                let old_pen = SelectObject(back, pen);
                for stroke in &self.strokes {
                    if let Some(first) = stroke.first() {
                        let first = self.display_point(*first);
                        MoveToEx(back, first.x, first.y, null_mut());
                        for p in stroke.iter().skip(1) {
                            let p = self.display_point(*p);
                            LineTo(back, p.x, p.y);
                        }
                    }
                }
                SelectObject(back, old_pen);
                DeleteObject(pen);
                SetBkMode(back, TRANSPARENT as i32);
                SetTextColor(back, 0x00_ff_ff_ff);
                let font = CreateFontW(
                    30,
                    0,
                    0,
                    0,
                    600,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    DEFAULT_PITCH as u32,
                    wide("Malgun Gothic").as_ptr(),
                );
                let old_font = SelectObject(back, font);
                let label = wide("공유 검증 · Screen Probe · 한글 ABC 123");
                TextOutW(back, 28, 28, label.as_ptr(), label.len() as i32 - 1);
                SelectObject(back, old_font);
                DeleteObject(font);
                if (self.pointer || self.mode == Mode::Live) && r.contains(self.virtual_cursor) {
                    let x = self.virtual_cursor.x - r.left;
                    let y = self.virtual_cursor.y - r.top;
                    let brush = CreateSolidBrush(0x00_55_cc_ff);
                    let old_brush = SelectObject(back, brush);
                    let old_pen = SelectObject(back, GetStockObject(NULL_PEN));
                    let pulse = self.pulse.map(|t| t.elapsed().as_secs_f32()).unwrap_or(1.0);
                    let radius = if pulse < 0.3 {
                        22 + (pulse * 50.0) as i32
                    } else {
                        22
                    };
                    Ellipse(back, x - radius, y - radius, x + radius, y + radius);
                    SelectObject(back, old_brush);
                    SelectObject(back, old_pen);
                    DeleteObject(brush);
                    if self.mode == Mode::Live {
                        let pen = CreatePen(PS_SOLID, 2, 0x00_ff_ff_ff);
                        let old_pen = SelectObject(back, pen);
                        MoveToEx(back, x - 6, y, null_mut());
                        LineTo(back, x + 7, y);
                        MoveToEx(back, x, y - 6, null_mut());
                        LineTo(back, x, y + 7);
                        SelectObject(back, old_pen);
                        DeleteObject(pen);
                    }
                }
                BitBlt(dc, 0, 0, r.width(), r.height(), back, 0, 0, SRCCOPY);
                SelectObject(back, old_bitmap);
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !back.is_null() {
                DeleteDC(back);
            }
        }
        EndPaint(hwnd, &ps);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            self.stop();
            for id in &self.registered {
                UnregisterHotKey(self.control, *id);
            }
            if !self.control.is_null() {
                KillTimer(self.control, WATCHDOG_TIMER);
            }
            // Remove the pointer before destruction can re-enter the window procedure.
            APP.with(|p| p.set(null_mut()));
            for hwnd in [self.overlay, self.host, self.control] {
                if !hwnd.is_null() {
                    DestroyWindow(hwnd);
                }
            }
            if self.initialized {
                MagUninitialize();
            }
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let app = APP.with(|p| p.get());
    if app.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    match msg {
        WM_PAINT => {
            (*app).paint(hwnd);
            return 0;
        }
        WM_ERASEBKGND if hwnd != (*app).control => return 1,
        WM_MOUSEACTIVATE if hwnd != (*app).control => return MA_NOACTIVATE as isize,
        WM_NCHITTEST if hwnd != (*app).control => return HTTRANSPARENT as isize,
        WM_HOTKEY => {
            if let Err(e) = (*app).hotkey(w) {
                (*app).stop();
                (*app).set_status(e);
            }
            return 0;
        }
        WM_TIMER if w == FRAME_TIMER => {
            if let Err(e) = (*app).tick() {
                (*app).stop();
                (*app).set_status(e);
            }
            return 0;
        }
        WM_TIMER if w == WATCHDOG_TIMER => {
            if !(*app).guardian_test && (*app).guardian.as_mut().is_some_and(|g| !g.alive()) {
                (*app).stop();
                (*app).set_status(
                    "Recovery process exited. Closing to avoid unprotected operation.".into(),
                );
                PostQuitMessage(0);
                return 0;
            }
            let now = Instant::now();
            if (*app).smoke_until.is_some_and(|t| now >= t) {
                PostQuitMessage(0);
            }
            if (*app).session_until.is_some_and(|t| now >= t) {
                (*app).stop();
                (*app).set_status(
                    "5-minute probe session expired; input restored. Toggle a mode to restart."
                        .into(),
                );
            }
            return 0;
        }
        ESCAPE => {
            if (*app).ink {
                (*app).ink = false;
                (*app).drawing = false;
                (*app).set_status("Interaction mode".into());
            } else {
                (*app).stop();
                (*app).set_status("Stopped by Esc".into());
            }
            return 0;
        }
        ZOOM_WHEEL => {
            if (*app).mode != Mode::Idle {
                let steps = (*app).wheel.steps(w as i32);
                if steps != 0 {
                    if let Err(error) = (*app).change_zoom(steps as f32 * 0.25) {
                        (*app).stop();
                        (*app).set_status(error);
                    }
                }
            }
            return 0;
        }
        TEST_HANG if (*app).guardian_test => {
            // Hidden-window fault injection: no capture, hook or cursor change.
            std::thread::sleep(Duration::from_secs(15));
            PostQuitMessage(99);
            return 0;
        }
        FAIL => {
            (*app).stop();
            (*app).set_status(
                "Input/cursor operation failed. Input restored; see validation guide.".into(),
            );
            return 0;
        }
        WM_DISPLAYCHANGE | WM_POWERBROADCAST => {
            (*app).stop();
            (*app).set_status(
                "Display/power change: released. Restart the probe to refresh monitor geometry."
                    .into(),
            );
            // Quit prevents reactivation with stale monitor coordinates.
            PostQuitMessage(0);
            return 0;
        }
        WM_CLOSE if hwnd == (*app).control => {
            (*app).stop();
            PostQuitMessage(0);
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, w, l)
}

unsafe extern "system" fn keyboard_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = &*(l as *const KBDLLHOOKSTRUCT);
        let app = APP.with(|p| p.get());
        if !app.is_null()
            && event.vkCode == VK_ESCAPE as u32
            && event.flags & LLKHF_INJECTED == 0
            && (*app).monitor.bounds.contains((*app).virtual_cursor)
            && ((*app).mode != Mode::Idle || (*app).ink)
        {
            if w as u32 == WM_KEYDOWN {
                PostMessageW((*app).control, ESCAPE, 0, 0);
            }
            return 1;
        }
    }
    CallNextHookEx(null_mut(), code, w, l)
}

unsafe extern "system" fn mouse_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let event = &*(l as *const MSLLHOOKSTRUCT);
    // Never transform our own injection, or other accessibility software's injection.
    if event.dwExtraInfo == TAG || event.flags & LLMHF_INJECTED != 0 {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let app_ptr = APP.with(|p| p.get());
    if app_ptr.is_null() {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let app = &mut *app_ptr;
    let msg = w as u32;
    let physical = point(event.pt);
    if msg == WM_MOUSEMOVE {
        if app.mode == Mode::Live && app.monitor.bounds.contains(app.virtual_cursor) {
            let real = cursor();
            let candidate = Point {
                x: app.virtual_cursor.x + physical.x - real.x,
                y: app.virtual_cursor.y + physical.y - real.y,
            };
            // Do not let the virtual cursor enter gaps between physically arranged displays.
            app.virtual_cursor = if app.displays.iter().any(|m| m.bounds.contains(candidate)) {
                candidate
            } else {
                app.monitor.bounds.clamp(candidate)
            };
        } else {
            app.virtual_cursor = physical;
        }
    }
    let inside = app.monitor.bounds.contains(app.virtual_cursor);
    if app.mode == Mode::Live {
        app.hide_system_cursor(inside);
    }
    if app.drawing && msg == WM_LBUTTONUP {
        app.drawing = false;
        return 1;
    }
    if !inside && app.injected_buttons == 0 {
        if app.mode == Mode::Live && msg == WM_MOUSEMOVE && physical != app.virtual_cursor {
            if app.inject(app.virtual_cursor, MOUSEEVENTF_MOVE, 0).is_err() {
                PostMessageW(app.control, FAIL, 0, 0);
            }
            return 1;
        }
        return CallNextHookEx(null_mut(), code, w, l);
    }
    if inside && app.mode != Mode::Idle && msg == WM_MOUSEWHEEL {
        let delta = (event.mouseData >> 16) as i16 as i32;
        // No magnification, rendering or input injection inside the hook.
        PostMessageW(app.control, ZOOM_WHEEL, delta as usize, 0);
        return 1;
    }
    if msg == WM_LBUTTONDOWN {
        app.pulse = Some(Instant::now());
    }
    if app.ink && inside {
        if msg == WM_LBUTTONDOWN {
            app.drawing = true;
            app.strokes.push(vec![app.ink_point(app.virtual_cursor)]);
            return 1;
        }
        if msg == WM_MOUSEMOVE && app.drawing {
            let p = app.ink_point(app.virtual_cursor);
            if let Some(stroke) = app.strokes.last_mut() {
                if stroke.last() != Some(&p) {
                    stroke.push(p);
                }
            }
            if app.mode != Mode::Live {
                return CallNextHookEx(null_mut(), code, w, l);
            }
        }
        if msg == WM_LBUTTONUP {
            app.drawing = false;
            return 1;
        }
    }
    if app.mode == Mode::Frozen {
        // Frozen presentation must not click invisible underlying applications.
        return if msg == WM_MOUSEMOVE {
            CallNextHookEx(null_mut(), code, w, l)
        } else {
            1
        };
    }
    if app.mode != Mode::Live {
        return CallNextHookEx(null_mut(), code, w, l);
    }
    let (flag, data, bit, down) = match msg {
        WM_MOUSEMOVE => (MOUSEEVENTF_MOVE, 0, 0, false),
        WM_LBUTTONDOWN => (MOUSEEVENTF_LEFTDOWN, 0, 1, true),
        WM_LBUTTONUP => (MOUSEEVENTF_LEFTUP, 0, 1, false),
        WM_RBUTTONDOWN => (MOUSEEVENTF_RIGHTDOWN, 0, 2, true),
        WM_RBUTTONUP => (MOUSEEVENTF_RIGHTUP, 0, 2, false),
        WM_MBUTTONDOWN => (MOUSEEVENTF_MIDDLEDOWN, 0, 4, true),
        WM_MBUTTONUP => (MOUSEEVENTF_MIDDLEUP, 0, 4, false),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let data = event.mouseData >> 16;
            (
                if msg == WM_XBUTTONDOWN {
                    MOUSEEVENTF_XDOWN
                } else {
                    MOUSEEVENTF_XUP
                },
                data,
                if data == XBUTTON1 as u32 { 8 } else { 16 },
                msg == WM_XBUTTONDOWN,
            )
        }
        WM_MOUSEWHEEL => (
            MOUSEEVENTF_WHEEL,
            ((event.mouseData >> 16) as i16 as i32) as u32,
            0,
            false,
        ),
        WM_MOUSEHWHEEL => (
            MOUSEEVENTF_HWHEEL,
            ((event.mouseData >> 16) as i16 as i32) as u32,
            0,
            false,
        ),
        _ => return CallNextHookEx(null_mut(), code, w, l),
    };
    let target = if inside {
        app.view.to_source(app.virtual_cursor)
    } else {
        app.virtual_cursor
    };
    if app.inject(target, flag, data).is_err() {
        PostMessageW(app.control, FAIL, 0, 0);
    } else if down {
        app.injected_buttons |= bit;
    } else {
        app.injected_buttons &= !bit;
    }
    1
}

pub fn run() -> Result<(), String> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().is_some_and(|v| v == "--guardian") {
            if args.len() != 3 {
                return Err("Invalid guardian arguments".into());
            }
            let pid = args[1].parse::<u32>().map_err(|e| e.to_string())?;
            let hwnd = args[2].parse::<usize>().map_err(|e| e.to_string())? as HWND;
            return crate::guardian::run(pid, hwnd);
        }
        if args.as_slice() == ["--restore-cursor"] {
            if MagInitialize() == 0 {
                return Err(win_error("cursor recovery: MagInitialize"));
            }
            let restored = MagShowSystemCursor(1);
            let error = GetLastError();
            MagUninitialize();
            if restored == 0 {
                return Err(format!("cursor recovery failed: Win32 error {error}"));
            }
            println!("System cursor visibility restored. No windows or input hooks installed.");
            return Ok(());
        }
        if args.iter().any(|a| a == "--help") {
            println!("screen-probe [--list-monitors | --smoke-test | --restore-cursor | --monitor N]\nDefault monitor: 1 (primary first). --smoke-test creates HIDDEN windows, no hooks or synthetic input.\n--restore-cursor restores the system cursor after terminating a hung probe.\nLive zoom is DISABLED following a confirmed hang.\nThis is stage-one validation software, not the completed product.");
            return Ok(());
        }
        let displays = monitors();
        if displays.is_empty() {
            return Err("No interactive display detected".into());
        }
        for (i, m) in displays.iter().enumerate() {
            println!(
                "{}: {} {}x{} at ({}, {}){}",
                i + 1,
                m.name,
                m.bounds.width(),
                m.bounds.height(),
                m.bounds.left,
                m.bounds.top,
                if m.primary { " primary" } else { "" }
            );
        }
        if args.iter().any(|a| a == "--list-monitors") {
            return Ok(());
        }
        let mut index = 0;
        let mut smoke = false;
        let mut guardian_test = false;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--smoke-test" => smoke = true,
                "--guardian-test" => {
                    smoke = true;
                    guardian_test = true;
                }
                "--monitor" => {
                    i += 1;
                    index = args
                        .get(i)
                        .and_then(|s| s.parse::<usize>().ok())
                        .and_then(|n| n.checked_sub(1))
                        .ok_or("--monitor requires a 1-based number")?;
                }
                other => return Err(format!("Unknown argument: {other}")),
            }
            i += 1;
        }
        let selected = displays
            .get(index)
            .ok_or("Monitor index out of range")?
            .clone();
        let mut app = Box::new(App::new(selected));
        app.guardian_test = guardian_test;
        APP.with(|p| p.set(&mut *app));
        app.setup(smoke)?;
        if !smoke || guardian_test {
            app.guardian = Some(Guardian::start(app.control)?);
            println!("Independent recovery ready. Ctrl+0: emergency exit.");
        }
        if guardian_test {
            println!("TEST: blocking hidden UI thread; guardian must terminate this process with code 73.");
            PostMessageW(app.control, TEST_HANG, 0, 0);
        }
        if smoke && !guardian_test {
            app.smoke_until = Some(Instant::now() + Duration::from_millis(1200));
            println!("PASS: MagInitialize, HWND creation, filter list, transform, source. No hooks installed.");
        }
        let mut msg: MSG = zeroed();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result == -1 {
                return Err(win_error("GetMessageW"));
            }
            if result == 0 {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let frames = app.frames;
        let elapsed = app.started.elapsed().as_secs_f64();
        drop(app);
        println!("Cleanup complete. Source refresh requests: {frames}; elapsed: {elapsed:.2}s. This is NOT measured rendered FPS.");
        if smoke {
            println!("PASS: hidden-window smoke test and normal cleanup. Capture compatibility and input remain UNTESTED.");
        }
        Ok(())
    }
}
