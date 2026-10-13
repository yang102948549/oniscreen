//! Win32 helpers: global mouse state, window styles, the magnifier host window.
//! All coordinates are physical desktop pixels (the process is per-monitor DPI aware).
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 로컬 시각 (연, 월, 일, 시, 분, 초) — 파일 이름용.
pub fn local_time() -> (i32, i32, i32, i32, i32, i32) {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        (t.wYear as i32, t.wMonth as i32, t.wDay as i32, t.wHour as i32, t.wMinute as i32, t.wSecond as i32)
    }
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
pub fn key_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

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

/// Exclude a window from screen capture, so the zoom loop can keep grabbing the desktop
/// without our toolbar / ink / halo (or the zoom window itself) showing up in the frames.
pub fn exclude_from_capture(hwnd: isize) {
    unsafe {
        SetWindowDisplayAffinity(hwnd as HWND, 0x11); // WDA_EXCLUDEFROMCAPTURE
    }
}

/// Live zoom: a click-through topmost window covering one monitor. Each frame the visible part of
/// that monitor is grabbed from the screen (GDI, physical pixels, no DPI virtualisation) and
/// stretched into the window. Everything stays inside the one monitor, whatever the DPI mix.
pub struct ZoomHost {
    host: HWND,
    screen: HDC,
    mem: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    w: i32,
    h: i32,
}

pub fn zoom_create(rect: (i32, i32, i32, i32)) -> Option<ZoomHost> {
    unsafe {
        let (x, y, w, h) = rect;
        let host = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            wide("Static").as_ptr(),
            wide("OniScreen View").as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        );
        if host.is_null() {
            return None;
        }
        SetLayeredWindowAttributes(host, 0, 255, LWA_ALPHA);
        exclude_from_capture(host as isize);
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp);
        ShowWindow(host, SW_SHOWNOACTIVATE);
        SetWindowPos(host, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        Some(ZoomHost { host, screen, mem, bmp, old, w, h })
    }
}

/// Draw desktop rectangle `(left, top, right, bottom)` magnified to fill the whole window.
pub fn zoom_draw(z: &ZoomHost, r: (i32, i32, i32, i32), smooth: bool) -> bool {
    unsafe {
        // HALFTONE looks best but is slow; while the view moves, favour frame rate.
        SetStretchBltMode(z.mem, if smooth { HALFTONE } else { COLORONCOLOR });
        SetBrushOrgEx(z.mem, 0, 0, null_mut());
        let ok = StretchBlt(z.mem, 0, 0, z.w, z.h, z.screen, r.0, r.1, r.2 - r.0, r.3 - r.1, SRCCOPY) != 0;
        let dc = GetDC(z.host);
        BitBlt(dc, 0, 0, z.w, z.h, z.mem, 0, 0, SRCCOPY);
        ReleaseDC(z.host, dc);
        ok
    }
}

/// Debug aid: write the current frame as a 32-bit BMP.
pub fn zoom_dump(z: &ZoomHost, path: &str) {
    unsafe {
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
    }
}

/// 모니터 한 대의 화면을 RGBA(위→아래)로 가져온다. 우리 창은 캡처에서 제외되어 있어 확대 중에도 원본 화면이 잡힌다.
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

/// 녹화 프레임을 합성하는 작업 비트맵(32bpp, 위→아래). 화면이나 확대 결과를 복사하고 커서를 덧그린다.
/// 읽기 전용 API만 쓴다: 시스템 커서는 `GetCursorInfo`로 읽기만 하고 숨기거나 바꾸지 않는다.
pub struct RecCanvas {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub w: i32,
    pub h: i32,
}

impl RecCanvas {
    pub fn new(w: i32, h: i32) -> Option<Self> {
        unsafe {
            let screen = GetDC(null_mut());
            let dc = CreateCompatibleDC(screen);
            ReleaseDC(null_mut(), screen);
            if dc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut core::ffi::c_void = null_mut();
            let bmp = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bmp);
            Some(Self { dc, bmp, old, bits: bits as *mut u8, w, h })
        }
    }

    /// 데스크톱의 `(x, y)`에서 `w×h`를 복사한다(우리 창은 캡처에서 제외되어 보이지 않음).
    pub fn copy_screen(&mut self, x: i32, y: i32) -> bool {
        unsafe {
            let screen = GetDC(null_mut());
            let ok = BitBlt(self.dc, 0, 0, self.w, self.h, screen, x, y, SRCCOPY | CAPTUREBLT) != 0;
            ReleaseDC(null_mut(), screen);
            ok
        }
    }

    /// 확대 창이 마지막으로 그린 확대 화면을 복사한다.
    pub fn copy_zoom(&mut self, z: &ZoomHost) -> bool {
        unsafe { BitBlt(self.dc, 0, 0, self.w.min(z.w), self.h.min(z.h), z.mem, 0, 0, SRCCOPY) != 0 }
    }

    /// 화면 좌표 `(cx, cy)`가 이 프레임에서 `(px, py)`에 놓이도록 현재 커서를 덧그린다.
    pub fn draw_cursor(&mut self, px: i32, py: i32) {
        unsafe {
            let mut ci: CURSORINFO = std::mem::zeroed();
            ci.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut ci) == 0 || ci.flags & CURSOR_SHOWING == 0 || ci.hCursor.is_null() {
                return;
            }
            let mut ii: ICONINFO = std::mem::zeroed();
            if GetIconInfo(ci.hCursor, &mut ii) == 0 {
                return;
            }
            DrawIconEx(self.dc, px - ii.xHotspot as i32, py - ii.yHotspot as i32, ci.hCursor, 0, 0, 0, null_mut(), DI_NORMAL);
            if !ii.hbmMask.is_null() {
                DeleteObject(ii.hbmMask);
            }
            if !ii.hbmColor.is_null() {
                DeleteObject(ii.hbmColor);
            }
        }
    }

    /// BGRA 픽셀(`w*h*4`). GDI 그리기가 끝난 뒤에 읽는다.
    pub fn pixels(&mut self) -> &mut [u8] {
        unsafe {
            GdiFlush();
            std::slice::from_raw_parts_mut(self.bits, (self.w * self.h * 4) as usize)
        }
    }
}

impl Drop for RecCanvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
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
