//! Win32 helpers: global mouse state, window styles, the magnifier host window.
//! All coordinates are physical desktop pixels (the process is per-monitor DPI aware).
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::Win32::UI::Magnification::*;

pub struct AppInstance(windows_sys::Win32::Foundation::HANDLE);
impl Drop for AppInstance {
    fn drop(&mut self) { unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0); } }
}

pub fn single_instance() -> Result<Option<AppInstance>, std::io::Error> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = "Local\\kr.oniclass.oniscreen.instance\0".encode_utf16().collect();
    unsafe {
        let handle = CreateMutexW(null(), 0, name.as_ptr());
        if handle.is_null() { return Err(std::io::Error::last_os_error()); }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            return Ok(None);
        }
        Ok(Some(AppInstance(handle)))
    }
}

/// 로컬 시각 (연, 월, 일, 시, 분, 초) — 파일 이름용.


pub fn space_down() -> bool {
    unsafe { (GetAsyncKeyState(0x20) as u16 & 0x8000) != 0 }
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

/// (left, right) physical button state. Honors the "swap buttons" setting.
pub fn buttons() -> (bool, bool) {
    unsafe {
        let down = |vk: i32| (GetAsyncKeyState(vk) as u16 & 0x8000) != 0;
        let swapped = GetSystemMetrics(SM_SWAPBUTTON) != 0;
        let (l, r) = (down(0x01), down(0x02));
        if swapped {
            (r, l)
        } else {
            (l, r)
        }
    }
}

/// 가상 키가 눌려 있는지(읽기 전용). `화면 고정 키`에 쓴다.


/// Tab / Shift / Ctrl 키가 눌려 있는지(도형 보조키). 읽기 전용.
pub fn modifier_keys() -> (bool, bool, bool) {
    unsafe {
        let down = |vk: i32| (GetAsyncKeyState(vk) as u16 & 0x8000) != 0;
        (down(0x09), down(0x10), down(0x11))
    }
}

/// Hide from Alt-Tab; optionally never steal focus when clicked (toolbar, overlays).
/// tao rewrites the extended style when click-through is toggled, so call this after every toggle.
pub fn style_tool_window(hwnd: isize, no_activate: bool) {
    unsafe {
        let hwnd = hwnd as HWND;
        let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        ex |= WS_EX_TOOLWINDOW as isize;
        if no_activate {
            ex |= WS_EX_NOACTIVATE as isize;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
    }
}

/// Let a window take keyboard focus (or never take it again).
pub fn set_noactivate(hwnd: isize, on: bool) {
    unsafe {
        let hwnd = hwnd as HWND;
        let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if on {
            ex |= WS_EX_NOACTIVATE as isize;
        } else {
            ex &= !(WS_EX_NOACTIVATE as isize);
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
    }
}

/// Re-assert TOPMOST so the window sits above sibling topmost windows (e.g. the overlays).
pub fn raise_topmost(hwnd: isize) {
    unsafe {
        SetWindowPos(
            hwnd as HWND,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// (left, top, right, bottom) of a window in desktop pixels.
pub fn window_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { GetWindowRect(hwnd as HWND, &mut r) } != 0;
    ok.then_some((r.left, r.top, r.right, r.bottom))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Hide only control windows (toolbar/settings) from external screen capture.
pub fn exclude_from_capture(hwnd: isize) {
    unsafe {
        SetWindowDisplayAffinity(hwnd as HWND, 0x11); // WDA_EXCLUDEFROMCAPTURE
    }
}

/// Presentation windows remain visible to Meet/OBS and other capture clients.
pub fn allow_capture(hwnd: isize) {
    unsafe { SetWindowDisplayAffinity(hwnd as HWND, 0); }
}

/// Windows magnifier excludes our windows only from its own source, not from screen sharing.
pub struct ZoomHost {
    host: HWND,
    magnifier: HWND,
    excluded: std::cell::RefCell<Vec<HWND>>,
    screen: HDC,
    mem: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    w: i32,
    h: i32,
}

unsafe extern "system" fn collect_app_window(hwnd: HWND, data: isize) -> i32 {
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == std::process::id() {
        (*(data as *mut Vec<HWND>)).push(hwnd);
    }
    1
}

fn update_zoom_filter(z: &ZoomHost) -> bool {
    let mut windows: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(collect_app_window), &mut windows as *mut _ as isize); }
    windows.sort_unstable();
    let mut previous = z.excluded.borrow_mut();
    if *previous == windows { return true; }
    let ok = unsafe { MagSetWindowFilterList(z.magnifier, MW_FILTERMODE_EXCLUDE, windows.len() as i32, windows.as_mut_ptr()) } != 0;
    if ok { *previous = windows; }
    ok
}

pub fn zoom_create(rect: (i32, i32, i32, i32)) -> Option<ZoomHost> {
    unsafe {
        if MagInitialize() == 0 { return None; }
        let (x, y, w, h) = rect;
        let host = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            wide("Static").as_ptr(), wide("OniScreen View").as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN, x, y, w, h,
            null_mut(), null_mut(), null_mut(), null(),
        );
        if host.is_null() { MagUninitialize(); return None; }
        SetLayeredWindowAttributes(host, 0, 255, LWA_ALPHA);
        let magnifier = CreateWindowExW(0, WC_MAGNIFIER, wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE, 0, 0, w, h, host, null_mut(), null_mut(), null());
        if magnifier.is_null() { DestroyWindow(host); MagUninitialize(); return None; }
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp);
        let z = ZoomHost { host, magnifier, excluded: std::cell::RefCell::new(Vec::new()), screen, mem, bmp, old, w, h };
        if !update_zoom_filter(&z) { zoom_destroy(z); return None; }
        let source = RECT { left: x, top: y, right: x + w, bottom: y + h };
        if MagSetWindowSource(magnifier, source) == 0 { zoom_destroy(z); return None; }
        ShowWindow(host, SW_SHOWNOACTIVATE);
        allow_capture(host as isize);
        SetWindowPos(host, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        Some(z)
    }
}

pub fn zoom_draw(z: &ZoomHost, r: (i32, i32, i32, i32), _smooth: bool) -> bool {
    if r.2 <= r.0 || r.3 <= r.1 || !update_zoom_filter(z) { return false; }
    let mut transform = MAGTRANSFORM { v: [0.0; 9] };
    transform.v[0] = z.w as f32 / (r.2 - r.0) as f32;
    transform.v[4] = z.h as f32 / (r.3 - r.1) as f32;
    transform.v[8] = 1.0;
    unsafe {
        let ok = MagSetWindowTransform(z.magnifier, &mut transform) != 0
            && MagSetWindowSource(z.magnifier, RECT { left: r.0, top: r.1, right: r.2, bottom: r.3 }) != 0;
        InvalidateRect(z.magnifier, null(), 0);
        ok
    }
}

/// Debug aid: write the current frame as a 32-bit BMP.
pub fn zoom_dump(z: &ZoomHost, path: &str) {
    unsafe {
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(z.host, &mut r);
        BitBlt(z.mem, 0, 0, z.w, z.h, z.screen, r.left, r.top, SRCCOPY | CAPTUREBLT);
        SelectObject(z.mem, z.old);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = z.w;
        info.bmiHeader.biHeight = -z.h;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut px = vec![0u8; (z.w * z.h * 4) as usize];
        GetDIBits(z.mem, z.bmp, 0, z.h as u32, px.as_mut_ptr() as *mut _, &mut info, DIB_RGB_COLORS);
        SelectObject(z.mem, z.bmp);
        let mut out = Vec::with_capacity(54 + px.len());
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&((54 + px.len()) as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&z.w.to_le_bytes());
        out.extend_from_slice(&(-z.h).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        out.extend_from_slice(&px);
        let _ = std::fs::write(path, out);
    }
}

pub fn zoom_destroy(z: ZoomHost) {
    unsafe {
        SelectObject(z.mem, z.old);
        DeleteObject(z.bmp);
        DeleteDC(z.mem);
        ReleaseDC(null_mut(), z.screen);
        DestroyWindow(z.host);
        MagUninitialize();
    }
}

#[cfg(test)]
mod sharing_tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn magnifier_is_shareable_and_filters_own_windows() {
        let z = zoom_create((0, 0, 160, 120)).expect("create native magnifier");
        assert!(zoom_draw(&z, (0, 0, 80, 60), true));
        pump_messages();
        unsafe {
            let mut affinity = u32::MAX;
            assert_ne!(GetWindowDisplayAffinity(z.host, &mut affinity), 0);
            assert_eq!(affinity, 0, "zoom must remain visible to external capture");
            let mut mode = u32::MAX;
            let count = MagGetWindowFilterList(z.magnifier, &mut mode, 0, null_mut());
            assert!(count > 0);
            let mut windows = vec![null_mut(); count as usize];
            assert_eq!(MagGetWindowFilterList(z.magnifier, &mut mode, count, windows.as_mut_ptr()), count);
            assert_eq!(mode, MW_FILTERMODE_EXCLUDE);
            assert!(windows.contains(&z.host), "prevent recursive self-magnification");
            let mut transform = MAGTRANSFORM { v: [0.0; 9] };
            assert_ne!(MagGetWindowTransform(z.magnifier, &mut transform), 0);
            assert_eq!(transform.v[0], 2.0);
            assert_eq!(transform.v[4], 2.0);
        }
        zoom_destroy(z);
    }
}

/// 모니터에 실제 표시된 확대·필기·포인터를 포함해 RGBA로 캡처한다.
/// 메모리로만 전달하며 파일에 쓰지 않는다. 크기는 한 축 8192, 총 1,677만 픽셀로 제한(ZoomIt/기존 엔진과 동일 한도).
pub fn grab_rect(rect: (i32, i32, i32, i32)) -> Option<(i32, i32, Vec<u8>)> {
    let (x, y, w, h) = rect;
    if w <= 0 || h <= 0 || w > 8192 || h > 8192 || (w as i64 * h as i64) > 16_777_216 {
        return None;
    }
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp);
        let mut ok = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT) != 0;
        let mut px = vec![0u8; (w * h * 4) as usize];
        if ok {
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            SelectObject(mem, old);
            ok = GetDIBits(mem, bmp, 0, h as u32, px.as_mut_ptr() as *mut _, &mut info, DIB_RGB_COLORS) != 0;
        } else {
            SelectObject(mem, old);
        }
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
        if !ok {
            return None;
        }
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2); // BGRA → RGBA
            p[3] = 255;
        }
        Some((w, h, px))
    }
}

/// RGBA 픽셀을 클립보드에 비트맵(CF_DIB)으로 복사한다.
pub fn copy_rgba_to_clipboard(w: i32, h: i32, rgba: &[u8]) -> bool {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    if w <= 0 || h <= 0 || rgba.len() != (w as usize) * (h as usize) * 4 {
        return false;
    }
    unsafe {
        let hdr = std::mem::size_of::<BITMAPINFOHEADER>();
        let mem = GlobalAlloc(GMEM_MOVEABLE, hdr + rgba.len());
        if mem.is_null() {
            return false;
        }
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            GlobalFree(mem);
            return false;
        }
        let mut info: BITMAPINFOHEADER = std::mem::zeroed();
        info.biSize = hdr as u32;
        info.biWidth = w;
        info.biHeight = h; // 아래→위
        info.biPlanes = 1;
        info.biBitCount = 32;
        info.biCompression = BI_RGB;
        std::ptr::copy_nonoverlapping(&info as *const _ as *const u8, ptr, hdr);
        let dst = ptr.add(hdr);
        let stride = w as usize * 4;
        for row in 0..h as usize {
            let src = &rgba[(h as usize - 1 - row) * stride..][..stride];
            let out = std::slice::from_raw_parts_mut(dst.add(row * stride), stride);
            for (o, p) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                o.copy_from_slice(&[p[2], p[1], p[0], 255]); // RGBA → BGRA
            }
        }
        GlobalUnlock(mem);
        if OpenClipboard(null_mut()) == 0 {
            GlobalFree(mem);
            return false;
        }
        EmptyClipboard();
        let ok = !SetClipboardData(8 /* CF_DIB */, mem as _).is_null();
        CloseClipboard();
        if !ok {
            GlobalFree(mem); // 클립보드가 소유권을 받지 못한 경우에만 해제
        }
        ok
    }
}

/// Alt+휠 수신. 훅이나 입력 주입 없이 Raw Input의 "수신만"(INPUTSINK)으로 휠 값을 읽는다.
/// 휠 입력은 아래 앱에도 그대로 전달된다(가로채지 않음). Alt를 누른 휠만 센다.
static WHEEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

unsafe extern "system" fn wheel_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    use windows_sys::Win32::UI::Input::{GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTHEADER, RID_INPUT, RIM_TYPEMOUSE};
    if msg == 0x00FF {
        // WM_INPUT
        let mut raw: RAWINPUT = std::mem::zeroed();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        let n = GetRawInputData(lp as HRAWINPUT, RID_INPUT, &mut raw as *mut _ as *mut _, &mut size, std::mem::size_of::<RAWINPUTHEADER>() as u32);
        if n != u32::MAX && raw.header.dwType == RIM_TYPEMOUSE {
            let m = raw.data.mouse;
            // RI_MOUSE_WHEEL = 0x0400 (usButtonFlags)
            let flags = m.Anonymous.Anonymous.usButtonFlags;
            if flags & 0x0400 != 0 && (GetAsyncKeyState(0x12) as u16 & 0x8000) != 0 {
                let delta = m.Anonymous.Anonymous.usButtonData as i16 as i32;
                WHEEL.fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// 메시지 전용 창을 만들고 마우스 Raw Input을 수신 등록한다. 호출한 스레드가 `pump_messages`를 돌려야 한다.
pub fn create_wheel_sink() -> bool {
    use windows_sys::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK};
    unsafe {
        let class = wide("OniScreenWheelSink");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wheel_proc);
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), null_mut(), null());
        if hwnd.is_null() {
            return false;
        }
        let dev = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
        RegisterRawInputDevices(&dev, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32) != 0
    }
}

/// 마지막 호출 이후 누적된 Alt+휠 눈금 수(+ = 위로 = 확대).
pub fn take_wheel_notches() -> i32 {
    WHEEL.swap(0, std::sync::atomic::Ordering::Relaxed) / 120
}

pub fn pump_messages() {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
