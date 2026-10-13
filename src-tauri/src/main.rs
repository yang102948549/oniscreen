#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod recorder;
mod win;
mod zoom_anim;

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use recorder::Recorder;
use zoom_anim::ZoomAnim;

#[derive(Clone, Copy, Serialize)]
struct MonRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scale: f64,
}

enum MagCmd {
    Start { z: f32 },
    Set(f32),
    Step(f32),
    /// 휠 한 칸 (+1 확대 / -1 축소). `ladder`면 ZoomIt식 2배 사다리, 아니면 `step`만큼 선형.
    Wheel { dir: i32, ladder: bool, step: f32 },
    /// 녹화 시작(대상 모니터) / 종료. 녹화기는 확대 스레드가 소유한다.
    RecStart(usize),
    RecStop,
    Stop,
    /// 필기 중 화면 고정. `hold`면 마우스 왼쪽 버튼을 누르고 있는 동안만 고정(그 외에는 라이브로 따라간다).
    Freeze { on: bool, hold: bool },
}

/// 오버레이가 보낸 최신 필기 레이어(RGBA, 화면에 보이는 그대로). 녹화 프레임에 합성한다.
struct InkFrame {
    m: usize,
    w: i32,
    h: i32,
    rgba: Vec<u8>,
}

struct AppState {
    wheel_ladder: AtomicBool,
    /// 누르고 있는 동안 확대 화면을 멈추는 키(가상 키 코드, 0 = 없음)
    hold_vk: AtomicU32,
    wheel_step: AtomicU32,
    /// 사용자가 지정한 전역 단축키(등록된 조합) — 다시 지정할 때 먼저 해제한다
    custom_keys: Mutex<Vec<String>>,
    ink: Mutex<Option<InkFrame>>,
    mons: Mutex<Vec<MonRect>>,
    mag: Mutex<Option<mpsc::Sender<MagCmd>>>,
    /// Monitor chosen in settings for zooming (NONE = automatic).
    target: AtomicUsize,
    /// Monitor where the cursor last rested outside the toolbar (the screen being presented).
    content: AtomicUsize,
}

const NONE: usize = usize::MAX;

impl Default for AppState {
    fn default() -> Self {
        Self {
            wheel_ladder: AtomicBool::new(false),
            hold_vk: AtomicU32::new(0x20),
            wheel_step: AtomicU32::new(0.25f32.to_bits()),
            custom_keys: Mutex::new(vec![]),
            ink: Mutex::new(None),
            mons: Mutex::new(vec![]),
            mag: Mutex::new(None),
            target: AtomicUsize::new(NONE),
            content: AtomicUsize::new(NONE),
        }
    }
}

fn send_mag(app: &AppHandle, c: MagCmd) {
    if let Some(tx) = app.state::<AppState>().mag.lock().unwrap().as_ref() {
        let _ = tx.send(c);
    }
}

#[derive(Clone, Serialize)]
struct Ptr {
    x: i32,
    y: i32,
    l: bool,
    r: bool,
    /// 도형 보조키(ZoomIt): Tab = 타원 (Shift/Ctrl은 마우스 이벤트에 실려 온다)
    tab: bool,
}

const MAX_ZOOM: f32 = 8.0;

/// Desktop rectangle shown by the magnifier for a view offset `off` (monitor-local) at zoom `z`.
fn source_rect(mon: (i32, i32, i32, i32), off: (i32, i32), z: f32) -> ((i32, i32, i32, i32), f32) {
    let sw = ((mon.2 as f32 / z).round() as i32).clamp(1, mon.2);
    let sh = ((mon.3 as f32 / z).round() as i32).clamp(1, mon.3);
    let ox = off.0.clamp(0, mon.2 - sw);
    let oy = off.1.clamp(0, mon.3 - sh);
    ((mon.0 + ox, mon.1 + oy, mon.0 + ox + sw, mon.1 + oy + sh), mon.2 as f32 / sw as f32)
}

/// 점과 사각형 사이의 거리(사각형 안이면 0).
fn dist_to_rect(x: i32, y: i32, r: (i32, i32, i32, i32)) -> f32 {
    let dx = (r.0 - x).max(0).max(x - r.2 + 1) as f32;
    let dy = (r.1 - y).max(0).max(y - r.3 + 1) as f32;
    dx.hypot(dy)
}

/// 툴바에 가까워질수록 화면 이동을 줄이는 비율: 툴바 위 0, `margin` 이상 1.
fn approach_weight(dist: f32, margin: f32) -> f32 {
    (dist / margin).clamp(0.0, 1.0)
}

/// 현재 위치에서 목표 쪽으로 `w` 비율만큼 이동한다(반올림). 거의 도착했으면 목표로 맞춘다.
fn step_toward(cur: (i32, i32), target: (i32, i32), w: f32) -> (i32, i32) {
    let f = |c: i32, t: i32| c + ((t - c) as f32 * w).round() as i32;
    (f(cur.0, target.0), f(cur.1, target.1))
}

const TOOLBAR_MARGIN: f32 = 160.0;

fn monitor_at(mons: &[MonRect], x: i32, y: i32) -> Option<usize> {
    mons.iter().position(|m| x >= m.x && x < m.x + m.w && y >= m.y && y < m.y + m.h)
}

fn toolbar_rect(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    app.get_webview_window(TOOLBAR).and_then(|w| hwnd_of(&w)).and_then(win::window_rect)
}

fn over_toolbar(app: &AppHandle, x: i32, y: i32) -> bool {
    app.get_webview_window(TOOLBAR)
        .and_then(|w| hwnd_of(&w))
        .and_then(win::window_rect)
        .map(|r| x >= r.0 && x < r.2 && y >= r.1 && y < r.3)
        .unwrap_or(false)
}

/// Which monitor to zoom. The toolbar often sits on another screen than the presented content, so
/// pressing its zoom button must not zoom the toolbar's screen:
/// settings choice > cursor's screen (hotkey, cursor on the content) > last screen the cursor rested
/// on outside the toolbar > any screen without the toolbar.
fn pick_monitor(app: &AppHandle, mons: &[MonRect]) -> usize {
    let st = app.state::<AppState>();
    let fixed = st.target.load(Ordering::Relaxed);
    if fixed < mons.len() {
        return fixed;
    }
    let (cx, cy) = win::cursor_pos();
    let cur = monitor_at(mons, cx, cy).unwrap_or(0);
    if !over_toolbar(app, cx, cy) {
        return cur;
    }
    let content = st.content.load(Ordering::Relaxed);
    if content < mons.len() {
        return content;
    }
    let tb = app
        .get_webview_window(TOOLBAR)
        .and_then(|w| hwnd_of(&w))
        .and_then(win::window_rect)
        .and_then(|r| monitor_at(mons, (r.0 + r.2) / 2, (r.1 + r.3) / 2))
        .unwrap_or(cur);
    (0..mons.len()).find(|i| *i != tb).unwrap_or(cur)
}

fn start_zoom(app: &AppHandle) -> Option<(win::ZoomHost, usize, (i32, i32, i32, i32))> {
    let mons = app.state::<AppState>().mons.lock().unwrap().clone();
    let idx = pick_monitor(app, &mons);
    let m = *mons.get(idx)?;
    let mon = (m.x, m.y, m.w, m.h);
    let host = win::zoom_create(mon)?;
    // Stack: zoom view < ink overlay < toolbar / settings.
    if let Some(w) = app.get_webview_window(&format!("overlay-{idx}")) {
        if let Some(h) = hwnd_of(&w) {
            win::raise_topmost(h);
        }
    }
    raise_toolbar(app);
    Some((host, idx, mon))
}

fn emit_rec(app: &AppHandle, active: bool, secs: u32, path: Option<String>, error: Option<String>) {
    let _ = app.emit("record-state", serde_json::json!({ "active": active, "secs": secs, "path": path, "error": error }));
}

/// `동영상\OniScreen` 폴더에 `OniScreen-날짜-시간.mp4`로 저장한다.
fn recording_path() -> Result<std::path::PathBuf, String> {
    let base = std::env::var("USERPROFILE").map_err(|e| e.to_string())?;
    let dir = std::path::PathBuf::from(base).join("Videos").join("OniScreen");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let t = win::local_time();
    Ok(dir.join(format!("OniScreen-{:04}{:02}{:02}-{:02}{:02}{:02}.mp4", t.0, t.1, t.2, t.3, t.4, t.5)))
}

fn start_recording(m: MonRect) -> Result<(Recorder, win::RecCanvas), String> {
    // H.264는 가로·세로가 짝수여야 한다.
    let (w, h) = (m.w & !1, m.h & !1);
    let canvas = win::RecCanvas::new(w, h).ok_or("녹화 버퍼를 만들지 못했습니다")?;
    let rec = Recorder::start(recording_path()?, w, h)?;
    Ok((rec, canvas))
}

/// 인코더 마무리(파일 닫기)는 별도 스레드에서 해서 확대 루프를 막지 않는다.
fn finish_recording(app: &AppHandle, rec: Recorder) {
    let app = app.clone();
    thread::spawn(move || match rec.finish() {
        Ok(p) => emit_rec(&app, false, 0, Some(p.to_string_lossy().into_owned()), None),
        Err(e) => emit_rec(&app, false, 0, None, Some(e)),
    });
}

/// 필기 레이어(RGBA, 비프리멀티플라이)를 BGRA 프레임에 알파 합성한다. 레이어가 프레임보다 작으면 건너뛴다.
/// 프레임이 짝수로 잘렸을 수 있어 레이어의 행 간격은 `f.w` 기준이다.
fn blend_ink(px: &mut [u8], w: i32, h: i32, f: &InkFrame) {
    if f.w < w || f.h < h || px.len() < (w * h * 4) as usize {
        return;
    }
    for y in 0..h as usize {
        for x in 0..w as usize {
            let s = (y * f.w as usize + x) * 4;
            let a = f.rgba[s + 3] as u32;
            if a == 0 {
                continue;
            }
            let d = (y * w as usize + x) * 4;
            let mix = |src: u8, dst: u8| ((src as u32 * a + dst as u32 * (255 - a) + 127) / 255) as u8;
            px[d] = mix(f.rgba[s + 2], px[d]);
            px[d + 1] = mix(f.rgba[s + 1], px[d + 1]);
            px[d + 2] = mix(f.rgba[s], px[d + 2]);
        }
    }
}

/// 한 프레임을 합성한다: (확대된 화면 | 원본 화면) + 커서 + 필기 레이어. BGRA 위→아래.
fn compose_rec_frame(
    canvas: &mut win::RecCanvas,
    mon: &MonRect,
    zoomed: Option<(&win::ZoomHost, ((i32, i32, i32, i32), f32))>,
    ink: Option<&InkFrame>,
) -> Vec<u8> {
    let (cx, cy) = win::cursor_pos();
    match zoomed {
        Some((h, (rect, ze))) => {
            canvas.copy_zoom(h);
            canvas.draw_cursor(((cx - rect.0) as f32 * ze).round() as i32, ((cy - rect.1) as f32 * ze).round() as i32);
        }
        None => {
            canvas.copy_screen(mon.x, mon.y);
            canvas.draw_cursor(cx - mon.x, cy - mon.y);
        }
    }
    let (w, h) = (canvas.w, canvas.h);
    let px = canvas.pixels();
    if let Some(f) = ink {
        blend_ink(px, w, h, f);
    }
    px.to_vec()
}


/// Owns the magnifier window and the zoom state. Live zoom follows the cursor (ZoomIt style): the
/// point under the cursor stays under the cursor, so real clicks always hit what is shown.
/// Pen tools freeze the view; the cursor over the toolbar pauses following so it can be used.
fn spawn_magnifier(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<MagCmd>();
    *app.state::<AppState>().mag.lock().unwrap() = Some(tx);
    thread::spawn(move || {
        let mut host: Option<win::ZoomHost> = None;
        let (mut mi, mut mon) = (0usize, (0, 0, 0, 0));
        // z = 목표 배율, anim = 화면에 실제 적용되는(애니메이션 중인) 배율
        let (mut z, mut frozen, mut hold) = (2.0f32, false, false);
        let mut anim = ZoomAnim::new(1.0);
        let mut closing = false;
        let clock = std::time::Instant::now();
        let mut za_prev = 1.0f32;
        let mut off = (0i32, 0i32);
        let mut applied: Option<((i32, i32), f32)> = None;
        let dump = std::env::var("ONI_DUMP").ok();
        let mut frames = 0u32;
        let mut last_move = std::time::Instant::now();
        let mut reported: Option<(bool, f32, bool, bool)> = None;
        let mut catching_up = false;
        // 녹화: (녹화기, 작업 비트맵, 대상 모니터, 다음 프레임 시각, 마지막 상태 알림 초)
        let mut rec: Option<(Recorder, win::RecCanvas, usize, std::time::Instant, u32)> = None;
        // 이번 프레임의 확대 상태(원본 사각형, 실제 배율)
        let mut last_view: Option<((i32, i32, i32, i32), f32)>;
        // Alt+휠로 확대/축소(수신 전용 Raw Input). 실패해도 다른 기능에는 영향이 없다.
        let wheel_ok = win::create_wheel_sink();
        if !wheel_ok {
            eprintln!("휠 수신 등록 실패: Alt+휠 확대를 쓸 수 없습니다");
        }

        let report = |on: bool, z: f32, ze: f32, frozen: bool, mi: usize| {
            let _ = app.emit(
                "zoom-changed",
                serde_json::json!({ "active": on, "z": z, "ze": ze, "frozen": frozen, "m": mi }),
            );
        };

        loop {
            win::pump_messages();
            let mut cmds = vec![];
            loop {
                match rx.try_recv() {
                    Ok(c) => cmds.push(c),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            let now = clock.elapsed().as_millis() as u64;
            let hold_vk = app.state::<AppState>().hold_vk.load(Ordering::Relaxed);
            let notches = if wheel_ok { win::take_wheel_notches() } else { 0 };
            if notches != 0 && host.is_some() {
                let st = app.state::<AppState>();
                let ladder = st.wheel_ladder.load(Ordering::Relaxed);
                let step = f32::from_bits(st.wheel_step.load(Ordering::Relaxed));
                for _ in 0..notches.abs().min(4) {
                    cmds.push(MagCmd::Wheel { dir: notches.signum(), ladder, step });
                }
            }
            let cur = win::cursor_pos();
            let local = ((cur.0 - mon.0) as f32, (cur.1 - mon.1) as f32);
            let mut stop = false;
            for c in cmds {
                // 목표 배율을 바꾸고 애니메이션을 이어 붙인다(진행 중이던 속도를 이어받음).
                let retarget = |nz: f32, anim: &mut ZoomAnim, z: &mut f32| {
                    *z = nz;
                    let dur = zoom_anim::duration_ms(anim.sample(now), nz, true);
                    anim.retarget(nz, now, dur);
                };
                match c {
                    MagCmd::Start { z: nz } if host.is_none() => {
                        if let Some((h, i, m)) = start_zoom(&app) {
                            host = Some(h);
                            mi = i;
                            mon = m;
                            frozen = false;
                            closing = false;
                            applied = None;
                            reported = None;
                            frames = 0;
                            z = nz.clamp(1.01, MAX_ZOOM);
                            // 1배에서 목표 배율까지 부드럽게 들어간다(ZoomIt 진입 애니메이션).
                            anim.start(1.0, z, now, zoom_anim::duration_ms(1.0, z, true));
                            za_prev = 1.0;
                        }
                    }
                    MagCmd::Set(nz) => {
                        if nz <= 1.001 {
                            stop = true;
                        } else if host.is_none() {
                            send_mag(&app, MagCmd::Start { z: nz });
                        } else {
                            closing = false;
                            retarget(nz.clamp(1.01, MAX_ZOOM), &mut anim, &mut z);
                        }
                    }
                    MagCmd::Step(d) => {
                        if host.is_some() {
                            let t = ((z + d) * 100.0).round() / 100.0;
                            if t < 1.01 {
                                stop = true;
                            } else {
                                closing = false;
                                retarget(t.clamp(1.01, MAX_ZOOM), &mut anim, &mut z);
                            }
                        } else if d > 0.0 {
                            send_mag(&app, MagCmd::Start { z: 1.0 + d });
                        }
                    }
                    MagCmd::Wheel { dir, ladder, step } => {
                        if host.is_none() {
                            if dir > 0 {
                                send_mag(&app, MagCmd::Start { z: if ladder { 2.0 } else { 1.0 + step } });
                            }
                        } else if ladder {
                            if dir > 0 {
                                closing = false;
                                retarget(zoom_anim::ladder_in(z, MAX_ZOOM), &mut anim, &mut z);
                            } else {
                                let t = zoom_anim::ladder_out(z);
                                if t <= 1.0 {
                                    stop = true;
                                } else {
                                    retarget(t, &mut anim, &mut z);
                                }
                            }
                        } else {
                            send_mag(&app, MagCmd::Step(dir as f32 * step));
                        }
                    }
                    MagCmd::RecStart(mon_idx) => {
                        if rec.is_none() {
                            let mons = app.state::<AppState>().mons.lock().unwrap().clone();
                            match mons.get(mon_idx).copied().ok_or("모니터를 찾을 수 없습니다".to_string()).and_then(start_recording) {
                                Ok((r, canvas)) => {
                                    rec = Some((r, canvas, mon_idx, std::time::Instant::now(), 0));
                                    emit_rec(&app, true, 0, None, None);
                                }
                                Err(e) => emit_rec(&app, false, 0, None, Some(e)),
                            }
                        }
                    }
                    MagCmd::RecStop => {
                        if let Some((r, _c, _i, _t, _s)) = rec.take() {
                            finish_recording(&app, r);
                        }
                    }
                    MagCmd::Stop => stop = true,
                    MagCmd::Freeze { on, hold: h } => {
                        frozen = on;
                        hold = h;
                    }
                    MagCmd::Start { .. } => {} // 이미 확대 중
                }
            }
            // 종료는 1배로 부드럽게 내려간 뒤 창을 없앤다.
            if stop && host.is_some() && !closing {
                closing = true;
                z = 1.0;
                let dur = zoom_anim::duration_ms(anim.sample(now), 1.0, true).max(1);
                anim.retarget(1.0, now, dur);
            }
            if closing && !anim.is_active() {
                if let Some(h) = host.take() {
                    win::zoom_destroy(h);
                    raise_toolbar(&app);
                    report(false, 1.0, 1.0, false, mi);
                    let _ = app.emit_to(
                        format!("overlay-{mi}"),
                        "zoom-view",
                        serde_json::json!({ "ox": 0, "oy": 0, "ze": 1.0 }),
                    );
                }
                closing = false;
                reported = None;
                anim.stop(1.0);
                thread::sleep(Duration::from_millis(8));
                continue;
            }
            if let Some(h) = host.as_ref() {
                let za = anim.sample(now).max(1.0);
                // 필기 도구가 켜져 있어도 화면은 라이브로 커서를 따라가고, 선을 긋는 동안(버튼 누름)에만 멈춘다
                let held = hold_vk != 0 && win::key_down(hold_vk as i32);
                let still = held || (frozen && (!hold || win::buttons().0));
                let on_screen = cur.0 >= mon.0 && cur.0 < mon.0 + mon.2 && cur.1 >= mon.1 && cur.1 < mon.1 + mon.3;
                if !still && on_screen {
                    let (lx, ly) = (local.0.clamp(0.0, mon.2 as f32 - 1.0), local.1.clamp(0.0, mon.3 as f32 - 1.0));
                    // 커서 아래의 원본 점이 항상 커서 아래에 오도록(보이는 포인터 = 실제 포인터, 클릭이 보이는 곳에 맞는다)
                    let target = ((lx * (1.0 - 1.0 / za)).round() as i32, (ly * (1.0 - 1.0 / za)).round() as i32);
                    // 툴바에 다가가면 화면 이동을 서서히 줄여, 툴바에 닿을 때쯤 보던 곳이 그대로 남게 한다.
                    let d = toolbar_rect(&app).map_or(f32::MAX, |r| dist_to_rect(cur.0, cur.1, r));
                    if d < TOOLBAR_MARGIN {
                        off = step_toward(off, target, approach_weight(d, TOOLBAR_MARGIN));
                        catching_up = true;
                    } else if catching_up {
                        // 툴바에서 멀어진 뒤에는 한 번에 튀지 않고 부드럽게 따라잡는다
                        off = step_toward(off, target, 0.35);
                        if (off.0 - target.0).abs() < 2 && (off.1 - target.1).abs() < 2 {
                            off = target;
                            catching_up = false;
                        }
                    } else {
                        off = target;
                    }
                } else if still && (za - za_prev).abs() > f32::EPSILON {
                    // 필기 중 배율이 바뀌어도 커서 아래 점이 고정되게 한다.
                    let s = (off.0 as f32 + local.0 / za_prev, off.1 as f32 + local.1 / za_prev);
                    off = ((s.0 - local.0 / za).round() as i32, (s.1 - local.1 / za).round() as i32);
                }
                za_prev = za;
                let (rect, ze) = source_rect(mon, off, za);
                off = (rect.0 - mon.0, rect.1 - mon.1);
                if applied != Some((off, ze)) {
                    last_move = std::time::Instant::now();
                }
                // 애니메이션·이동 중에는 프레임률 우선, 멈추면 고품질로 한 번 더 그린다.
                let smooth = !anim.is_active() && last_move.elapsed() > Duration::from_millis(120);
                let t0 = std::time::Instant::now();
                if !win::zoom_draw(h, rect, smooth) {
                    eprintln!("zoom_draw failed");
                }
                frames += 1;
                if frames == 40 || frames == 400 {
                    eprintln!("zoom frame {:?} smooth={smooth} rect {:?} ze {ze}", t0.elapsed(), rect);
                    if let Some(path) = &dump {
                        win::zoom_dump(h, path);
                    }
                }
                if applied != Some((off, ze)) {
                    applied = Some((off, ze));
                    let _ = app.emit_to(
                        format!("overlay-{mi}"),
                        "zoom-view",
                        serde_json::json!({ "ox": off.0, "oy": off.1, "ze": ze }),
                    );
                }
                // '고정됨' 표시는 지금 실제로 화면이 멈춰 있을 때만(키를 누르는 중 / 선을 긋는 중)
                if reported != Some((true, z, still, held)) {
                    reported = Some((true, z, still, held));
                    report(true, z, ze, still, mi);
                }
                last_view = Some((rect, ze));
            } else {
                last_view = None;
            }
            if let Some((r, canvas, ri, next, last_sec)) = rec.as_mut() {
                if std::time::Instant::now() >= *next {
                    *next += Duration::from_millis(1000 / recorder::FPS as u64);
                    if *next < std::time::Instant::now() {
                        *next = std::time::Instant::now(); // 밀렸으면 따라잡지 않고 현재 시각부터 다시
                    }
                    let mons = app.state::<AppState>().mons.lock().unwrap().clone();
                    if let Some(mrec) = mons.get(*ri) {
                        let zoomed = if mi == *ri { host.as_ref().zip(last_view) } else { None };
                        let state = app.state::<AppState>();
                        let ink = state.ink.lock().unwrap();
                        let frame = compose_rec_frame(canvas, mrec, zoomed, ink.as_ref().filter(|f| f.m == *ri));
                        drop(ink);
                        r.push(frame);
                    }
                }
                let secs = r.elapsed_secs();
                if secs != *last_sec {
                    *last_sec = secs;
                    emit_rec(&app, true, secs, None, None);
                }
            }
            thread::sleep(Duration::from_millis(8));
        }
    });
}

const TOOLBAR: &str = "toolbar";

fn hwnd_of(w: &WebviewWindow) -> Option<isize> {
    w.hwnd().ok().map(|h| h.0 as isize)
}

/// Keep the toolbar (and settings) above the full-screen overlays.
fn raise_toolbar(app: &AppHandle) {
    for label in [TOOLBAR, "settings"] {
        if let Some(w) = app.get_webview_window(label) {
            if let Some(h) = hwnd_of(&w) {
                win::raise_topmost(h);
            }
        }
    }
}

fn set_passthrough(w: &WebviewWindow, passthrough: bool) {
    let _ = w.set_ignore_cursor_events(passthrough);
    if let Some(h) = hwnd_of(w) {
        win::style_tool_window(h, true); // overlays never take focus: the toolbar stays on top
        win::exclude_from_capture(h);
    }
}

// ───────────────────────── commands ─────────────────────────

#[tauri::command]
fn monitor_rect(state: tauri::State<'_, AppState>, index: usize) -> Option<MonRect> {
    state.mons.lock().unwrap().get(index).copied()
}

/// Overlay `index` captures the mouse (draw / zoom) or lets it fall through to the apps below.
#[tauri::command]
async fn set_interactive(app: AppHandle, index: usize, on: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&format!("overlay-{index}")) {
        set_passthrough(&w, !on);
    }
    raise_toolbar(&app);
    Ok(())
}

/// Text tool: the overlay takes keyboard focus only while a text box is open.
#[tauri::command]
async fn focus_overlay(app: AppHandle, index: usize, on: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(&format!("overlay-{index}")) {
        if let Some(h) = hwnd_of(&w) {
            win::set_noactivate(h, !on);
        }
        if on {
            let _ = w.set_focus();
        }
    }
    raise_toolbar(&app);
    Ok(())
}

#[tauri::command]
fn begin_zoom(app: AppHandle, z: f32) {
    send_mag(&app, MagCmd::Start { z });
}

/// Slider: set an exact zoom level (starts zoom if needed, 1.0 leaves it).
#[tauri::command]
fn zoom_set(app: AppHandle, z: f32) {
    send_mag(&app, MagCmd::Set(z));
}

#[tauri::command]
fn zoom_step(app: AppHandle, d: f32) {
    send_mag(&app, MagCmd::Step(d));
}

/// 휠 한 칸. `ladder`가 켜져 있으면 ZoomIt처럼 2배씩(1→2→4→8), 아니면 `step`씩 선형으로 바꾼다.
#[tauri::command]
fn zoom_wheel(app: AppHandle, dir: i32, ladder: bool, step: f32) {
    send_mag(&app, MagCmd::Wheel { dir: dir.signum(), ladder, step });
}

#[tauri::command]
fn zoom_exit(app: AppHandle) {
    send_mag(&app, MagCmd::Stop);
}

/// Pen tools freeze the zoomed view so the ink stays where it was drawn.
#[tauri::command]
fn zoom_freeze(app: AppHandle, on: bool, hold: bool) {
    send_mag(&app, MagCmd::Freeze { on, hold });
}

/// 전역 Esc / Ctrl+Z / 색상 글자키는 필요한 동안(확대 중이거나 그리기 도구가 켜진 동안)만 잡는다. 그 외에는 다른 앱이 쓴다.
/// 색상 글자키는 ZoomIt과 같다: R 레드 · G 그린 · B 블루 · O 오렌지 · Y 옐로 · K 검정 · W 흰색.
/// 텍스트 입력 중에는 글자가 입력창으로 가야 하므로 `keys`를 끈다.
#[tauri::command]
fn grab_keys(app: AppHandle, esc: bool, undo: bool, keys: bool, zoom: bool) {
    let gs = app.global_shortcut();
    let mut table: Vec<(String, String, bool)> = vec![
        ("escape".into(), "esc".into(), esc),
        ("ctrl+z".into(), "undo".into(), undo),
        // 확대 중에만: ZoomIt처럼 Ctrl+↑/↓ 로 확대·축소 (평소에는 다른 앱이 쓴다)
        ("ctrl+up".into(), "zin".into(), zoom),
        ("ctrl+down".into(), "zout".into(), zoom),
    ];
    for c in ["r", "g", "b", "o", "y", "k", "w"] {
        table.push((c.into(), format!("key:{c}"), keys));
    }
    for (combo, action, want) in table {
        let have = gs.is_registered(combo.as_str());
        if want && !have {
            let _ = gs.on_shortcut(combo.as_str(), move |app, _sc, ev| {
                if ev.state() == ShortcutState::Pressed {
                    let _ = app.emit("hotkey", action.clone());
                }
            });
        } else if !want && have {
            let _ = gs.unregister(combo.as_str());
        }
    }
}

/// 모니터 한 대의 현재 화면(확대 전 원본). 앞 8바이트 = 너비·높이(u32 LE), 이어서 RGBA.
/// 블러·스크린샷에 쓰며 메모리/IPC로만 전달한다(파일 저장 없음).
#[tauri::command]
fn grab_screen(state: tauri::State<'_, AppState>, index: usize) -> Result<tauri::ipc::Response, String> {
    let m = *state.mons.lock().unwrap().get(index).ok_or("모니터 없음")?;
    let (w, h, px) = win::grab_rect((m.x, m.y, m.w, m.h)).ok_or("화면을 가져오지 못했습니다")?;
    let mut out = Vec::with_capacity(8 + px.len());
    out.extend_from_slice(&(w as u32).to_le_bytes());
    out.extend_from_slice(&(h as u32).to_le_bytes());
    out.extend_from_slice(&px);
    Ok(tauri::ipc::Response::new(out))
}

/// 스크린샷 PNG를 `사진\OniScreen` 폴더에 저장하고 경로를 돌려준다. 파일명은 안전한 문자만 허용한다.
#[tauri::command]
fn save_png(request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("PNG 데이터가 없습니다".into());
    };
    if data.len() < 8 || data.len() > 64 * 1024 * 1024 || &data[1..4] != b"PNG" {
        return Err("PNG가 아닙니다".into());
    }
    let name = request
        .headers()
        .get("x-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("OniScreen.png")
        .to_string();
    let safe: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect();
    if safe.is_empty() || !safe.ends_with(".png") {
        return Err("파일 이름이 올바르지 않습니다".into());
    }
    let dir = std::path::PathBuf::from(std::env::var("USERPROFILE").map_err(|e| e.to_string())?)
        .join("Pictures")
        .join("OniScreen");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(safe);
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// 스크린샷을 클립보드로 복사. 본문 = RGBA, 헤더 x-w / x-h = 크기.
#[tauri::command]
fn copy_rgba(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("이미지 데이터가 없습니다".into());
    };
    let dim = |k: &str| request.headers().get(k).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i32>().ok());
    let (w, h) = (dim("x-w").ok_or("너비 없음")?, dim("x-h").ok_or("높이 없음")?);
    if win::copy_rgba_to_clipboard(w, h, data) {
        Ok(())
    } else {
        Err("클립보드에 복사하지 못했습니다".into())
    }
}

/// 녹화 시작/종료. `mon` = 녹화할 모니터(생략하면 마지막으로 작업한 화면).
#[tauri::command]
fn record_toggle(app: AppHandle, on: bool, mon: Option<usize>) {
    if on {
        let st = app.state::<AppState>();
        *st.ink.lock().unwrap() = None; // 이전 녹화의 필기 레이어가 남지 않게
        let n = st.mons.lock().unwrap().len();
        let idx = mon.filter(|i| *i < n).unwrap_or_else(|| {
            let c = st.content.load(Ordering::Relaxed);
            if c < n { c } else { 0 }
        });
        send_mag(&app, MagCmd::RecStart(idx));
    } else {
        send_mag(&app, MagCmd::RecStop);
    }
}

/// 오버레이의 필기 레이어를 녹화 합성용으로 보관한다. 본문 = RGBA, 헤더 x-w / x-h / x-m.
#[tauri::command]
fn record_ink(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("데이터 없음".into());
    };
    let n = |k: &str| request.headers().get(k).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i32>().ok());
    let (w, h, m) = (n("x-w").ok_or("w")?, n("x-h").ok_or("h")?, n("x-m").ok_or("m")?);
    if w <= 0 || h <= 0 || m < 0 || data.len() != (w as usize) * (h as usize) * 4 || data.len() > 134_217_728 {
        return Err("잘못된 레이어 크기".into());
    }
    *app.state::<AppState>().ink.lock().unwrap() = Some(InkFrame { m: m as usize, w, h, rgba: data.clone() });
    Ok(())
}

/// 지금 작업 중인 화면(마지막으로 마우스를 두고 쉰 모니터). 스크린샷·녹화·보드의 기본 대상.
#[tauri::command]
fn content_monitor(app: AppHandle) -> usize {
    let st = app.state::<AppState>();
    let n = st.mons.lock().unwrap().len();
    let c = st.content.load(Ordering::Relaxed);
    if c < n { c } else { 0 }
}

/// Settings: which monitor zoom applies to (-1 = automatic).
#[tauri::command]
fn set_zoom_target(state: tauri::State<'_, AppState>, index: i32) {
    state.target.store(if index < 0 { NONE } else { index as usize }, Ordering::Relaxed);
}

/// 확대 동작 설정: 휠 단계, 화면 고정 키(space / shift / ctrl / f8 / none).
#[tauri::command]
fn set_zoom_prefs(state: tauri::State<'_, AppState>, ladder: bool, step: f32, hold: String) {
    let vk = match hold.as_str() {
        "space" => 0x20,
        "shift" => 0x10,
        "ctrl" => 0x11,
        "f8" => 0x77,
        _ => 0,
    };
    state.hold_vk.store(vk, Ordering::Relaxed);
    state.wheel_ladder.store(ladder, Ordering::Relaxed);
    state.wheel_step.store(step.clamp(0.1, 2.0).to_bits(), Ordering::Relaxed);
}

/// 전역 단축키를 (조합, 동작) 목록으로 다시 등록한다. `on`이 꺼져 있으면 모두 해제한다.
/// 조합은 `ctrl+alt+up` 같은 tauri 형식이며, 설정 화면에서 사용자가 지정한다. 수정키 없는 조합은 거부한다.
#[tauri::command]
fn set_hotkeys(app: AppHandle, on: bool, keys: Vec<(String, String)>) {
    let gs = app.global_shortcut();
    let state = app.state::<AppState>();
    let mut reg = state.custom_keys.lock().unwrap();
    for c in reg.drain(..) {
        let _ = gs.unregister(c.as_str());
    }
    if !on {
        return;
    }
    for (combo, action) in keys {
        let has_mod = combo.contains("ctrl") || combo.contains("alt") || combo.contains("shift");
        if combo.is_empty() || !has_mod || gs.is_registered(combo.as_str()) {
            continue;
        }
        let act = action.clone();
        match gs.on_shortcut(combo.as_str(), move |app, _sc, ev| {
            if ev.state() == ShortcutState::Pressed {
                if act == "quit" {
                    app.exit(0);
                    return;
                }
                let _ = app.emit("hotkey", act.clone());
            }
        }) {
            Ok(()) => reg.push(combo),
            Err(e) => eprintln!("hotkey {combo}: {e}"),
        }
    }
}

/// Toolbar dragging is driven from JS with the global pointer feed (the window never takes focus).
#[tauri::command]
fn toolbar_pos(app: AppHandle) -> Option<(i32, i32)> {
    let p = app.get_webview_window(TOOLBAR)?.outer_position().ok()?;
    Some((p.x, p.y))
}

#[tauri::command]
fn move_toolbar(app: AppHandle, x: i32, y: i32) {
    if let Some(w) = app.get_webview_window(TOOLBAR) {
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

#[tauri::command]
fn raise_toolbar_cmd(app: AppHandle) {
    raise_toolbar(&app);
}

#[tauri::command]
fn resize_toolbar(app: AppHandle, width: f64, height: f64) {
    if let Some(w) = app.get_webview_window(TOOLBAR) {
        let _ = w.set_size(LogicalSize::new(width, height));
    }
}

#[tauri::command]
async fn open_settings(app: AppHandle) -> Result<(), String> {
    show_settings(&app);
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn show_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("OniScreen 설정")
        .inner_size(880.0, 600.0)
        .min_inner_size(720.0, 420.0)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .center()
        .build();
    if let Ok(w) = built {
        if let Some(h) = hwnd_of(&w) {
            win::exclude_from_capture(h);
            win::raise_topmost(h);
        }
    }
}

// ───────────────────────── setup ─────────────────────────

fn create_overlays(app: &AppHandle, mons: &[MonRect]) -> tauri::Result<()> {
    for (i, m) in mons.iter().enumerate() {
        let w = WebviewWindowBuilder::new(
            app,
            format!("overlay-{i}"),
            WebviewUrl::App("overlay.html".into()),
        )
        .title("OniScreen Overlay")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
        let (pos, size) = (
            PhysicalPosition::new(m.x, m.y),
            PhysicalSize::new(m.w as u32, m.h as u32),
        );
        w.set_position(pos)?;
        w.set_size(size)?;
        set_passthrough(&w, true);
        if let Some(h) = hwnd_of(&w) {
            win::exclude_from_capture(h);
        }
        w.show()?;
        // A DPI change while moving between monitors can rescale the window; pin it again.
        let w2 = w.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(400));
            let _ = w2.set_position(pos);
            let _ = w2.set_size(size);
        });
    }
    Ok(())
}

fn create_toolbar(app: &AppHandle, primary: Option<MonRect>) -> tauri::Result<()> {
    let (w_l, h_l) = (680.0, 96.0);
    let w = WebviewWindowBuilder::new(app, TOOLBAR, WebviewUrl::App("toolbar.html".into()))
        .title("OniScreen")
        .inner_size(w_l, h_l)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .build()?;
    if let Some(m) = primary {
        let (pw, ph) = ((w_l * m.scale) as i32, (h_l * m.scale) as i32);
        w.set_position(PhysicalPosition::new(
            m.x + (m.w - pw) / 2,
            m.y + m.h - ph - (60.0 * m.scale) as i32,
        ))?;
    }
    if let Some(h) = hwnd_of(&w) {
        win::style_tool_window(h, true);
        win::exclude_from_capture(h);
        win::raise_topmost(h);
    }
    Ok(())
}

/// Poll the global cursor + buttons and fan them out to every overlay.
/// Polling (instead of a low-level hook) keeps the real system cursor untouched.
fn spawn_pointer_feed(app: AppHandle) {
    thread::spawn(move || {
        let mut last = (i32::MIN, i32::MIN, false, false, false);
        let mut tick = 0u32;
        let mut rest: (usize, std::time::Instant) = (NONE, std::time::Instant::now());
        loop {
            tick = tick.wrapping_add(1);
            if tick % 25 == 0 {
                raise_toolbar(&app);
            }
            let (x, y) = win::cursor_pos();
            let (l, r) = win::buttons();
            let tab = win::modifier_keys().0;
            if tick % 5 == 0 {
                let mons = app.state::<AppState>().mons.lock().unwrap().clone();
                let here = if over_toolbar(&app, x, y) { NONE } else { monitor_at(&mons, x, y).unwrap_or(NONE) };
                if here != rest.0 {
                    rest = (here, std::time::Instant::now());
                } else if here != NONE && rest.1.elapsed() > Duration::from_millis(500) {
                    app.state::<AppState>().content.store(here, Ordering::Relaxed);
                }
            }
            if (x, y, l, r, tab) != last {
                last = (x, y, l, r, tab);
                let _ = app.emit("ptr", Ptr { x, y, l, r, tab });
            }
            thread::sleep(Duration::from_millis(8));
        }
    });
}


fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            monitor_rect,
            set_interactive,
            begin_zoom,
            focus_overlay,
            toolbar_pos,
            set_zoom_target,
            set_hotkeys,
            set_zoom_prefs,
            move_toolbar,
            raise_toolbar_cmd,
            zoom_set,
            zoom_step,
            zoom_wheel,
            grab_screen,
            save_png,
            copy_rgba,
            record_toggle,
            record_ink,
            content_monitor,
            grab_keys,
            zoom_exit,
            zoom_freeze,
            resize_toolbar,
            open_settings,
            quit
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let mons: Vec<MonRect> = app
                .available_monitors()?
                .iter()
                .map(|m| MonRect {
                    x: m.position().x,
                    y: m.position().y,
                    w: m.size().width as i32,
                    h: m.size().height as i32,
                    scale: m.scale_factor(),
                })
                .collect();
            {
                let st = app.state::<AppState>();
                *st.mons.lock().unwrap() = mons.clone();
            }
            create_overlays(&handle, &mons)?;
            let primary = app.primary_monitor()?.and_then(|p| {
                mons.iter()
                    .find(|m| m.x == p.position().x && m.y == p.position().y)
                    .copied()
            });
            create_toolbar(&handle, primary.or_else(|| mons.first().copied()))?;

            let settings = MenuItem::with_id(app, "settings", "설정", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("OniScreen")
                .menu(&menu)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "quit" => app.exit(0),
                    "settings" => show_settings(app),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            spawn_magnifier(handle.clone());
            spawn_pointer_feed(handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("OniScreen 실행 중 오류")
        .run(|_app, _ev| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_rect_keeps_view_inside_monitor() {
        let mon = (1920, 0, 1920, 1080);
        let (r, ze) = source_rect(mon, (5000, 5000), 2.0);
        assert_eq!(r, (1920 + 960, 540, 1920 + 1920, 1080));
        assert!((ze - 2.0).abs() < 1e-3);
        let (r0, _) = source_rect(mon, (-50, -50), 4.0);
        assert_eq!((r0.0, r0.1), (1920, 0));
    }

    #[test]
    fn ink_blend_is_alpha_over_bgra() {
        // 2x1 프레임: 파랑(BGRA) 두 픽셀. 필기: 첫 픽셀 불투명 빨강, 둘째 50% 흰색
        let mut px = vec![255, 0, 0, 255, 255, 0, 0, 255];
        let ink = InkFrame { m: 0, w: 2, h: 1, rgba: vec![255, 0, 0, 255, 255, 255, 255, 128] };
        blend_ink(&mut px, 2, 1, &ink);
        assert_eq!(&px[0..3], &[0, 0, 255]); // 빨강 (BGR)
        assert!(px[3] == 255);
        assert_eq!(px[4], 255); // 파랑 위 50% 흰색 → B 255
        assert!(px[5] > 120 && px[5] < 136, "G={}", px[5]);
        assert!(px[6] > 120 && px[6] < 136, "R={}", px[6]);
    }

    #[test]
    fn ink_smaller_than_frame_is_ignored() {
        let mut px = vec![9u8; 4 * 4];
        let ink = InkFrame { m: 0, w: 1, h: 1, rgba: vec![0, 0, 0, 255] };
        blend_ink(&mut px, 2, 2, &ink);
        assert!(px.iter().all(|b| *b == 9));
    }
}

#[cfg(test)]
mod toolbar_approach_tests {
    use super::*;

    #[test]
    fn distance_is_zero_inside_and_grows_outside() {
        let r = (100, 100, 300, 160);
        assert_eq!(dist_to_rect(150, 120, r), 0.0);
        assert_eq!(dist_to_rect(100, 100, r), 0.0);
        assert_eq!(dist_to_rect(50, 130, r), 50.0);
        assert!((dist_to_rect(60, 60, r) - 40f32.hypot(40.0)).abs() < 1e-3);
    }

    #[test]
    fn view_slows_down_near_toolbar_and_stops_on_it() {
        assert_eq!(approach_weight(0.0, 160.0), 0.0);
        assert_eq!(approach_weight(80.0, 160.0), 0.5);
        assert_eq!(approach_weight(400.0, 160.0), 1.0);
        // 툴바 위: 목표가 멀어도 그 자리에 남는다
        assert_eq!(step_toward((10, 10), (500, 500), 0.0), (10, 10));
        // 절반 속도, 완전 속도
        assert_eq!(step_toward((0, 0), (100, 40), 0.5), (50, 20));
        assert_eq!(step_toward((0, 0), (100, 40), 1.0), (100, 40));
    }
}
