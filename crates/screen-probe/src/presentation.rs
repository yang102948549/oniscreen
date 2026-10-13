//! Disposable presentation process. Only ordinary window input; never hooks or injection.
use crate::{
    capture::{self, Task},
    engine::{Canvas, Frame, INK},
    geometry::{Point, Rect},
    supervisor,
};
use std::{
    cell::RefCell,
    mem::zeroed,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::{
    core::BOOL,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, Magnification::*, WindowsAndMessaging::*},
    },
};

const CLASS: &str = "ScreenProbeIsolatedV2";
const TIMER: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02a3;
const CLOSE_ID: usize = 50;
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Idle,
    Capturing,
    Frozen,
    Live,
}
struct Ui {
    window: HWND,
    mag: HWND,
    overlay: HWND,
    close: HWND,
    buttons: Vec<HWND>,
    initialized: bool,
    mode: Mode,
    canvas: Canvas,
    frame: Option<Frame>,
    capture: Option<Task>,
    capture_at: Option<Instant>,
    status: String,
    hidden: bool,
    registered: Vec<i32>,
    last_ping: Instant,
}
thread_local! {static STATE:RefCell<Option<Ui>>=const {RefCell::new(None)};}
fn state<T>(f: impl FnOnce(&mut Ui) -> T) -> Option<T> {
    STATE.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
}
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn error(name: &str) -> String {
    format!("{name}: Win32 {}", unsafe { GetLastError() })
}
fn native_rect(r: Rect) -> RECT {
    RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}
unsafe extern "system" fn enum_monitor(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let mut info: MONITORINFOEXW = zeroed();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(h, &mut info as *mut _ as *mut MONITORINFO) != 0 {
        let r = info.monitorInfo.rcMonitor;
        let name = String::from_utf16_lossy(
            &info.szDevice[..info.szDevice.iter().position(|v| *v == 0).unwrap_or(32)],
        );
        (*(data as *mut Vec<(bool, String, Rect)>)).push((
            info.monitorInfo.dwFlags & 1 != 0,
            name,
            Rect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            },
        ));
    }
    1
}
pub fn monitors() -> Vec<(bool, String, Rect)> {
    let mut list: Vec<(bool, String, Rect)> = vec![];
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(enum_monitor),
            &mut list as *mut _ as isize,
        );
    }
    list.sort_by_key(|(primary, name, _)| (!*primary, name.clone()));
    list
}

unsafe fn create(
    class: &[u16],
    name: &str,
    style: u32,
    ex: u32,
    parent: HWND,
    id: usize,
    r: Rect,
) -> Result<HWND, String> {
    let hwnd = CreateWindowExW(
        ex,
        class.as_ptr(),
        wide(name).as_ptr(),
        style,
        r.left,
        r.top,
        r.width(),
        r.height(),
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    if hwnd.is_null() {
        Err(error("CreateWindowEx"))
    } else {
        Ok(hwnd)
    }
}
fn status(text: String) {
    eprintln!("{text}");
    if let Some(hwnd) = state(|s| {
        s.status = text;
        s.window
    }) {
        unsafe {
            InvalidateRect(hwnd, null(), 1);
        }
    }
}

unsafe fn stop() {
    let Some((hwnd, mag, overlay, initialized, close, buttons, capture, hidden)) = state(|s| {
        s.mode = Mode::Idle;
        s.canvas.end();
        s.frame = None;
        s.capture_at = None;
        (
            s.window,
            std::mem::replace(&mut s.mag, null_mut()),
            std::mem::replace(&mut s.overlay, null_mut()),
            std::mem::replace(&mut s.initialized, false),
            s.close,
            s.buttons.clone(),
            s.capture.take(),
            s.hidden,
        )
    }) else {
        return;
    };
    // Drop disposable capture outside the state borrow. No callback owns &mut Ui.
    drop(capture);
    if !overlay.is_null() {
        DestroyWindow(overlay);
    }
    if !mag.is_null() {
        DestroyWindow(mag);
    }
    if initialized {
        MagUninitialize();
    }
    ShowWindow(close, SW_HIDE);
    SetWindowLongPtrW(hwnd, GWL_STYLE, WS_OVERLAPPEDWINDOW as isize);
    SetWindowPos(
        hwnd,
        null_mut(),
        0,
        0,
        760,
        430,
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
    );
    for button in buttons {
        ShowWindow(button, if hidden { SW_HIDE } else { SW_SHOWNA });
    }
    if !hidden {
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    InvalidateRect(hwnd, null(), 1);
    supervisor::signal("IDLE");
}
unsafe fn fullscreen() -> Result<(), String> {
    if state(|s| s.hidden).unwrap_or(true) {
        return Ok(());
    }
    let (hwnd, close, buttons, r) =
        state(|s| (s.window, s.close, s.buttons.clone(), s.canvas.view.monitor))
            .ok_or("UI missing")?;
    for button in buttons {
        ShowWindow(button, SW_HIDE);
    }
    SetWindowLongPtrW(
        hwnd,
        GWL_STYLE,
        (WS_POPUP | WS_SYSMENU | WS_CLIPCHILDREN) as isize,
    );
    // Normal z-order: Alt+Tab and system windows remain usable. Never TOPMOST.
    if SetWindowPos(
        hwnd,
        HWND_TOP,
        r.left,
        r.top,
        r.width(),
        r.height(),
        SWP_FRAMECHANGED | SWP_SHOWWINDOW,
    ) == 0
    {
        return Err(error("Presentation placement"));
    }
    SetWindowPos(
        close,
        HWND_TOP,
        r.width() - 170,
        8,
        155,
        36,
        SWP_SHOWWINDOW | SWP_NOACTIVATE,
    );
    SetFocus(hwnd);
    Ok(())
}
unsafe fn start_frozen(fixture: bool) -> Result<(), String> {
    if !fixture && state(|s| s.hidden).unwrap_or(true) {
        return Err("Real capture forbidden in offscreen test mode".into());
    }
    stop();
    let (hwnd, bounds) = state(|s| {
        s.canvas = Canvas::new(s.canvas.view.monitor);
        s.mode = Mode::Capturing;
        (s.window, s.canvas.view.monitor)
    })
    .ok_or("UI missing")?;
    supervisor::signal("ACTIVE");
    if fixture {
        let frame = Frame::fixture(bounds.width(), bounds.height())?;
        state(|s| {
            s.frame = Some(frame);
            s.mode = Mode::Frozen;
        });
        fullscreen()?;
    } else {
        ShowWindow(hwnd, SW_HIDE);
        // Allow the window to disappear without blocking on DwmFlush.
        state(|s| s.capture_at = Some(Instant::now() + Duration::from_millis(250)));
    }
    Ok(())
}
unsafe fn start_live() -> Result<(), String> {
    if state(|s| s.hidden).unwrap_or(true) {
        return Err("Magnification forbidden in offscreen test mode".into());
    }
    stop();
    supervisor::signal("ACTIVE");
    let (hwnd, bounds) = state(|s| (s.window, s.canvas.view.monitor)).ok_or("UI missing")?;
    // Already supervised. No cursor state APIs, input transforms or input injection.
    if MagInitialize() == 0 {
        return Err(error("MagInitialize"));
    }
    state(|s| s.initialized = true);
    let local = Rect {
        left: 0,
        top: 0,
        right: bounds.width(),
        bottom: bounds.height(),
    };
    let mag = create(
        &wide("Magnifier"),
        "",
        WS_CHILD | WS_DISABLED,
        0,
        hwnd,
        0,
        local,
    )?;
    state(|s| s.mag = mag);
    let overlay = create(
        &wide(CLASS),
        "Ink",
        WS_CHILD,
        WS_EX_LAYERED | WS_EX_TRANSPARENT,
        hwnd,
        0,
        local,
    )?;
    state(|s| s.overlay = overlay);
    if SetLayeredWindowAttributes(overlay, 0, 255, LWA_COLORKEY) == 0 {
        return Err(error("Ink transparency"));
    }
    let mut excluded = [hwnd, overlay];
    if MagSetWindowFilterList(mag, MW_FILTERMODE_EXCLUDE, 2, excluded.as_mut_ptr()) == 0 {
        return Err(error("Mag filter"));
    }
    state(|s| {
        s.canvas = Canvas::new(bounds);
        s.mode = Mode::Live;
    });
    update_live()?;
    ShowWindow(mag, SW_SHOWNA);
    ShowWindow(overlay, SW_SHOWNA);
    fullscreen()?;
    Ok(())
}
unsafe fn update_live() -> Result<(), String> {
    let Some((mag, view)) = state(|s| (s.mag, s.canvas.view)).filter(|(mag, _)| !mag.is_null())
    else {
        return Ok(());
    };
    let z = view.zoom;
    let mut transform = MAGTRANSFORM {
        v: [z, 0., 0., 0., z, 0., 0., 0., 1.],
    };
    if MagSetWindowTransform(mag, &mut transform) == 0
        || MagSetWindowSource(mag, native_rect(view.source)) == 0
    {
        return Err(error("Live source update"));
    }
    InvalidateRect(mag, null(), 0);
    Ok(())
}
unsafe fn redraw() {
    if let Some((hwnd, overlay)) = state(|s| (s.window, s.overlay)) {
        InvalidateRect(if overlay.is_null() { hwnd } else { overlay }, null(), 0);
    }
}
unsafe fn command(id: usize) -> Result<(), String> {
    let active = state(|s| s.mode != Mode::Idle).unwrap_or(false);
    match id {
        1 => {
            if active {
                stop();
            } else {
                start_frozen(false)?;
            }
        }
        2 => {
            if active {
                stop();
            } else {
                start_live()?;
            }
        }
        3 => start_frozen(true)?,
        4 => {
            state(|s| {
                s.canvas.end();
                s.canvas.pen = !s.canvas.pen;
            });
        }
        5 => {
            state(|s| s.canvas.clear());
        }
        6 | 7 => {
            state(|s| {
                let m = s.canvas.view.monitor;
                let p = Point {
                    x: m.left + m.width() / 2,
                    y: m.top + m.height() / 2,
                };
                s.canvas.zoom(if id == 6 { -0.25 } else { 0.25 }, p);
            });
            update_live()?;
        }
        8 | CLOSE_ID => stop(),
        9 => PostQuitMessage(0),
        10 => {
            state(|s| s.canvas.undo());
        }
        _ => {}
    }
    redraw();
    Ok(())
}
unsafe fn tick() -> Result<(), String> {
    let begin = state(|s| {
        if s.capture_at.is_some_and(|t| Instant::now() >= t) {
            s.capture_at = None;
            Some(s.canvas.view.monitor)
        } else {
            None
        }
    })
    .flatten();
    if let Some(bounds) = begin {
        if state(|s| s.hidden).unwrap_or(true) {
            return Err("Desktop capture forbidden in offscreen mode".into());
        }
        let task = Task::desktop(bounds)?;
        state(|s| s.capture = Some(task));
    }
    let captured = state(|s| s.capture.as_mut().and_then(|task| task.poll())).flatten();
    if let Some(result) = captured {
        let frame = result?;
        let old = state(|s| {
            s.frame = Some(frame);
            s.mode = Mode::Frozen;
            s.capture.take()
        });
        drop(old);
        fullscreen()?;
    }
    // Native rendering can block only this disposable worker, never the supervisor.
    // Timer refresh also updates moving content when the mouse is stationary.
    update_live()?;
    if state(|s| {
        if s.last_ping.elapsed() >= Duration::from_millis(200) {
            s.last_ping = Instant::now();
            true
        } else {
            false
        }
    })
    .unwrap_or(false)
    {
        supervisor::signal("PING");
    }
    Ok(())
}
unsafe fn blit(dc: HDC, frame: &Frame) {
    let info = capture::bitmap_info(frame.width, frame.height);
    SetDIBitsToDevice(
        dc,
        0,
        0,
        frame.width as u32,
        frame.height as u32,
        0,
        0,
        0,
        frame.height as u32,
        frame.pixels.as_ptr() as _,
        &info,
        DIB_RGB_COLORS,
    );
}
unsafe fn paint(hwnd: HWND, dc: HDC) {
    let Some((mode, window, frame, label)) = state(|s| {
        let frame = if s.mode == Mode::Frozen {
            s.frame.as_ref().and_then(|f| s.canvas.render(f).ok())
        } else if s.mode == Mode::Live && hwnd == s.overlay {
            Frame::new(
                s.canvas.view.monitor.width(),
                s.canvas.view.monitor.height(),
            )
            .ok()
            .map(|mut f| {
                s.canvas.ink(&mut f);
                f
            })
        } else {
            None
        };
        let label = if s.mode == Mode::Idle {
            format!("확대 · 필기 / 격리 실행 버전\n\nCtrl+1 정지 확대    Ctrl+2 라이브 확대\n가상화면 연습은 실제 화면을 캡처하지 않습니다.\n\n확대 중: 휠 / + - 배율, 방향키 이동\nP 또는 Ctrl+4 펜 전환 · Z 되돌리기 · C 지우기\nEsc 또는 오른쪽 위 버튼: 확대 종료\nCtrl+0: 독립 감시 프로세스에서 강제 종료\n\n라이브 화면은 보기·필기 전용입니다. 원래 앱 조작은 Esc로 돌아갑니다.\n5분 후 독립 감시에서 세션을 종료합니다.\n\n{}",s.status)
        } else {
            format!("{:.2}x  |  펜 {}  |  휠 +/- 확대 · 방향키 이동 · P 펜 · Z 취소 · C 지우기 · Esc 종료",s.canvas.view.zoom,if s.canvas.pen {"켜짐"}else {"꺼짐"})
        };
        (s.mode, s.window, frame, label)
    }) else {
        return;
    };
    if let Some(frame) = frame {
        blit(dc, &frame);
    } else {
        let mut r = RECT::default();
        GetClientRect(hwnd, &mut r);
        FillRect(dc, &r, GetStockObject(WHITE_BRUSH) as HBRUSH);
    }
    if hwnd == window || mode == Mode::Live {
        SetBkMode(dc, OPAQUE as i32);
        SetBkColor(dc, 0x002c241c);
        SetTextColor(dc, 0x00ffffff);
        let old = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
        let mut r = RECT {
            left: 16,
            top: if mode == Mode::Idle { 75 } else { 12 },
            right: if mode == Mode::Idle {
                725
            } else {
                state(|s| s.canvas.view.monitor.width() - 180).unwrap_or(500)
            },
            bottom: 410,
        };
        DrawTextW(
            dc,
            wide(&label).as_ptr(),
            -1,
            &mut r,
            DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    }
}
unsafe fn input(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) {
    let local = Point {
        x: (l as u16 as i16) as i32,
        y: ((l as u32 >> 16) as u16 as i16) as i32,
    };
    state(|s| {
        if !matches!(s.mode, Mode::Frozen | Mode::Live) {
            return;
        }
        let m = s.canvas.view.monitor;
        let p = Point {
            x: m.left + local.x,
            y: m.top + local.y,
        };
        match msg {
            WM_LBUTTONDOWN => {
                if local.y > 50 {
                    s.canvas.begin(p);
                }
            }
            WM_MOUSEMOVE => {
                if w & 1 != 0 {
                    s.canvas.extend(p);
                } else {
                    s.canvas.end();
                }
            }
            _ => s.canvas.end(),
        }
    });
    if msg == WM_LBUTTONDOWN && !state(|s| s.hidden).unwrap_or(true) {
        SetFocus(state(|s| s.window).unwrap_or(hwnd));
    }
    if msg == WM_MOUSEMOVE {
        let mut track = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        TrackMouseEvent(&mut track);
    }
    redraw();
}
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let handled = match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            paint(hwnd, dc);
            EndPaint(hwnd, &ps);
            Some(Ok(()))
        }
        WM_PRINTCLIENT => {
            paint(hwnd, w as HDC);
            Some(Ok(()))
        }
        WM_ERASEBKGND => return 1,
        WM_HOTKEY | WM_COMMAND => Some(command(w & 0xffff)),
        WM_TIMER if w == TIMER => Some(tick()),
        WM_CLOSE => {
            PostQuitMessage(0);
            Some(Ok(()))
        }
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP | WM_MOUSELEAVE | WM_KILLFOCUS => {
            input(hwnd, msg, w, l);
            Some(Ok(()))
        }
        WM_ACTIVATE if w & 0xffff == WA_INACTIVE as usize => {
            state(|s| s.canvas.end());
            Some(Ok(()))
        }
        WM_DISPLAYCHANGE | WM_POWERBROADCAST => {
            stop();
            status("화면 구성/전원 변경: 프로그램을 다시 실행해 주세요.".into());
            PostQuitMessage(0);
            Some(Ok(()))
        }
        WM_MOUSEWHEEL => {
            let mut p = POINT {
                x: (l as u16 as i16) as i32,
                y: ((l as u32 >> 16) as u16 as i16) as i32,
            };
            ScreenToClient(hwnd, &mut p);
            state(|s| {
                let m = s.canvas.view.monitor;
                s.canvas.wheel(
                    (w >> 16) as u16 as i16 as i32,
                    Point {
                        x: m.left + p.x,
                        y: m.top + p.y,
                    },
                );
            });
            redraw();
            Some(update_live())
        }
        WM_KEYDOWN => {
            let id = match w as u16 {
                VK_ESCAPE => Some(8),
                VK_ADD | VK_OEM_PLUS => Some(7),
                VK_SUBTRACT | VK_OEM_MINUS => Some(6),
                0x50 => Some(4),
                0x43 => Some(5),
                0x5a => Some(10),
                _ => None,
            };
            if let Some(id) = id {
                Some(command(id))
            } else if [VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN].contains(&(w as u16)) {
                state(|s| {
                    s.canvas.end();
                    s.canvas.view.pan(
                        if w == VK_LEFT as usize {
                            -24
                        } else if w == VK_RIGHT as usize {
                            24
                        } else {
                            0
                        },
                        if w == VK_UP as usize {
                            -24
                        } else if w == VK_DOWN as usize {
                            24
                        } else {
                            0
                        },
                    );
                });
                redraw();
                Some(update_live())
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(result) = handled {
        if let Err(e) = result {
            stop();
            status(e);
        }
        return 0;
    }
    DefWindowProcW(hwnd, msg, w, l)
}

pub fn run(index: usize, hidden: bool) -> Result<(), String> {
    supervisor::gate()?;
    unsafe {
        if !hidden {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let bounds = if hidden {
            Rect {
                left: 0,
                top: 0,
                right: 640,
                bottom: 360,
            }
        } else {
            monitors().get(index).ok_or("Monitor index out of range")?.2
        };
        // Validate bounds before any presentation allocation.
        drop(Frame::new(bounds.width(), bounds.height())?);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            ..zeroed()
        };
        // The class-name allocation must live through RegisterClassW.
        let class = wide(CLASS);
        let wc = WNDCLASSW {
            lpszClassName: class.as_ptr(),
            ..wc
        };
        if RegisterClassW(&wc) == 0 {
            return Err(error("RegisterClass"));
        }
        let window = create(
            &class,
            "Screen Probe — 확대와 필기",
            WS_OVERLAPPEDWINDOW,
            0,
            null_mut(),
            0,
            Rect {
                left: 100,
                top: 100,
                right: 860,
                bottom: 530,
            },
        )?;
        STATE.with(|cell| {
            *cell.borrow_mut() = Some(Ui {
                window,
                mag: null_mut(),
                overlay: null_mut(),
                close: null_mut(),
                buttons: vec![],
                initialized: false,
                mode: Mode::Idle,
                canvas: Canvas::new(bounds),
                frame: None,
                capture: None,
                capture_at: None,
                status: "준비됨. 확대 버튼을 누르기 전에는 캡처하지 않습니다.".into(),
                hidden,
                registered: vec![],
                last_ping: Instant::now(),
            })
        });
        let result = (|| -> Result<(), String> {
            for (id, name, x) in [
                (1, "정지 확대", 16),
                (2, "라이브 확대", 175),
                (3, "가상화면 연습", 334),
                (9, "종료", 510),
            ] {
                let button = create(
                    &wide("BUTTON"),
                    name,
                    WS_CHILD | if hidden { 0 } else { WS_VISIBLE },
                    0,
                    window,
                    id,
                    Rect {
                        left: x,
                        top: 20,
                        right: x + 145,
                        bottom: 56,
                    },
                )?;
                state(|s| s.buttons.push(button));
            }
            let close = create(
                &wide("BUTTON"),
                "확대 종료 (Esc)",
                WS_CHILD,
                0,
                window,
                CLOSE_ID,
                Rect {
                    left: 460,
                    top: 8,
                    right: 625,
                    bottom: 44,
                },
            )?;
            state(|s| s.close = close);
            if !hidden {
                for id in [1, 2, 4, 5, 6, 7, 8, 9] {
                    if RegisterHotKey(
                        window,
                        id,
                        MOD_CONTROL | MOD_NOREPEAT,
                        (b'0' + id as u8) as u32,
                    ) == 0
                    {
                        return Err(error("Shortcut unavailable; activation refused"));
                    }
                    state(|s| s.registered.push(id));
                }
            }
            if SetTimer(window, TIMER, 33, None) == 0 {
                return Err(error("UI heartbeat timer"));
            }
            supervisor::signal("READY");
            if hidden {
                return hidden_test(window);
            }
            ShowWindow(window, SW_SHOW);
            let mut msg = MSG::default();
            loop {
                let result = GetMessageW(&mut msg, null_mut(), 0, 0);
                if result == -1 {
                    return Err(error("GetMessage"));
                }
                if result == 0 {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            Ok(())
        })();
        stop();
        if let Some(ids) = state(|s| s.registered.clone()) {
            for id in ids {
                UnregisterHotKey(window, id);
            }
        }
        KillTimer(window, TIMER);
        STATE.with(|s| *s.borrow_mut() = None);
        DestroyWindow(window);
        result
    }
}

unsafe fn hidden_test(window: HWND) -> Result<(), String> {
    // Real window procedure + real GDI output, but the window stays hidden.
    // No monitor enumeration, screen DC, Magnification, global keys or synthetic input.
    if start_live().is_ok() || start_frozen(false).is_ok() {
        return Err("Offscreen desktop access guard failed".into());
    }
    start_frozen(true)?;
    let packed = |x: i32, y: i32| ((y as u32) << 16 | x as u32) as isize;
    SendMessageW(window, WM_LBUTTONDOWN, 1, packed(220, 180));
    SendMessageW(window, WM_MOUSEMOVE, 1, packed(380, 230));
    SendMessageW(window, WM_LBUTTONUP, 0, packed(380, 230));
    SendMessageW(window, WM_KEYDOWN, VK_OEM_PLUS as usize, 0);
    if state(|s| s.canvas.strokes.len() != 1 || s.canvas.view.zoom != 2.25).unwrap_or(true) {
        return Err("Hidden input/zoom dispatch failed".into());
    }
    supervisor::signal("PING");
    let dc = CreateCompatibleDC(null_mut());
    if dc.is_null() {
        return Err(error("Offscreen DC"));
    }
    let info = capture::bitmap_info(640, 360);
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() {
        DeleteDC(dc);
        return Err(error("Offscreen bitmap"));
    }
    let old = SelectObject(dc, bitmap);
    SendMessageW(window, WM_PRINTCLIENT, dc as usize, 0);
    GdiFlush();
    let pixels = std::slice::from_raw_parts(bits as *const u32, 640 * 360);
    let frozen_ink = pixels.iter().any(|p| p & 0xffffff == INK);
    // Exercise the real transparent ink surface with synthetic content only.
    let overlay = create(
        &wide(CLASS),
        "Hidden ink",
        WS_CHILD,
        WS_EX_LAYERED | WS_EX_TRANSPARENT,
        window,
        0,
        Rect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 360,
        },
    )?;
    state(|s| {
        s.overlay = overlay;
        s.mode = Mode::Live;
    });
    let transparent = SetLayeredWindowAttributes(overlay, 0, 255, LWA_COLORKEY) != 0;
    SendMessageW(overlay, WM_PRINTCLIENT, dc as usize, 0);
    GdiFlush();
    let pixels = std::slice::from_raw_parts(bits as *const u32, 640 * 360);
    let live_ink =
        pixels.iter().any(|p| p & 0xffffff == INK) && pixels[640 * 360 - 1] & 0xffffff == 0;
    state(|s| s.mode = Mode::Frozen);
    SelectObject(dc, old);
    DeleteObject(bitmap);
    DeleteDC(dc);
    if !frozen_ink || !live_ink || !transparent {
        return Err("Frozen/live-overlay offscreen GDI output failed".into());
    }
    SendMessageW(window, WM_KEYDOWN, 0x5a, 0);
    if state(|s| !s.canvas.strokes.is_empty()).unwrap_or(true) {
        return Err("Undo dispatch failed".into());
    }
    if IsWindowVisible(window) != 0
        || state(|s| s.initialized || !s.registered.is_empty()).unwrap_or(true)
    {
        return Err("Hidden test isolation violation".into());
    }
    SendMessageW(window, WM_KEYDOWN, VK_ESCAPE as usize, 0);
    if state(|s| s.mode != Mode::Idle || s.frame.is_some()).unwrap_or(true)
        || IsWindowVisible(window) != 0
    {
        return Err("Hidden Escape cleanup failed".into());
    }
    eprintln!("PASS hidden UI: zoom, pen, undo, Escape, frozen GDI and transparent live-ink surface; no desktop capture or Magnification calls.");
    Ok(())
}
