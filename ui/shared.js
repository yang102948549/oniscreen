/* 모든 창이 공유하는 설정/상수. localStorage는 같은 origin의 창끼리 공유되고 `storage` 이벤트로 동기화된다. */
const T = window.__TAURI__;
const { invoke } = T.core;
const { emit, listen } = T.event;

// 팔레트는 Oni 토큰 색에 ZoomIt의 색상 단축키(R·G·B·O·Y)를 맞춘 것이다. key = 도구가 켜져 있을 때 누르는 글자.
const PEN_COLORS = [
  { hex: '#292927', name: '검정', key: 'k' },
  { hex: '#2383E2', name: '블루', key: 'b' },
  { hex: '#C43F35', name: '레드', key: 'r' },
  { hex: '#20AD96', name: '그린', key: 'g' },
  { hex: '#EBA640', name: '오렌지', key: 'o' },
  { hex: '#F2D13B', name: '옐로', key: 'y' },
  { hex: '#FFFFFF', name: '화이트', key: 'w' },
];
// ZoomIt 도형: Shift = 선, Ctrl = 사각형, Tab = 타원, Ctrl+Shift = 화살표
const SHAPES = [
  { id: 'line', name: '선' },
  { id: 'rect', name: '사각형' },
  { id: 'ellipse', name: '타원' },
  { id: 'arrow', name: '화살표' },
];
const MIN_PEN = 2, MAX_PEN = 40;
const HALO_COLORS = [
  { rgb: '35,131,226', name: '블루' },
  { rgb: '235,166,64', name: '앰버' },
  { rgb: '239,120,108', name: '코랄' },
];


// 전역 단축키 기본값(설정에서 바꿀 수 있다). 형식은 Rust/tauri와 같다: `ctrl+alt+up`
const DEFAULT_KEYS = {
  zin: 'ctrl+1', pen: 'ctrl+2', hl: 'ctrl+3', eraser: 'ctrl+4', text: 'ctrl+5', ptr: 'ctrl+6', clear: 'ctrl+7',
  zoom: 'ctrl+alt+z', board: 'ctrl+0',
  'shot-copy': 'ctrl+alt+c', quit: 'ctrl+alt+q',
};
const KEY_LABELS = {
  zoom: '확대 켜기/끄기', zin: '일반 줌 (+)', pen: '펜', hl: '형광펜', eraser: '지우개', text: '텍스트',
  ptr: '포인터 강조', clear: '모두 지우기', board: '화이트보드/칠판', 'shot-copy': '화면 복사',
  quit: '종료',
};
// Rust(set_hotkeys)가 받는 [조합, 동작] 목록. 비어 있는 조합은 보내지 않는다.
const hotkeyList = (keys) => Object.entries(keys).filter(([, combo]) => combo).map(([act, combo]) => [combo, act]);
const keyLabel = (c) => (c || '없음').split('+').map((p) => ({ ctrl: 'Ctrl', alt: 'Alt', shift: 'Shift', up: '↑', down: '↓', left: '←', right: '→', space: 'Space' }[p] || p.toUpperCase())).join(' ');
// 키 입력 → tauri 단축키 문자열. 수정키(Ctrl/Alt/Shift)가 하나도 없으면 null (일반 입력을 가로채지 않도록).
function comboFromEvent(e) {
  let k = null;
  if (/^Key[A-Z]$/.test(e.code)) k = e.code.slice(3).toLowerCase();
  else if (/^Digit\d$/.test(e.code)) k = e.code.slice(5);
  else if (/^F\d{1,2}$/.test(e.code)) k = e.code.toLowerCase();
  else k = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right', Space: 'space', PageUp: 'pageup', PageDown: 'pagedown', Home: 'home', End: 'end' }[e.code] || null;
  if (!k || !(e.ctrlKey || e.altKey || e.shiftKey)) return null;
  return [e.ctrlKey && 'ctrl', e.altKey && 'alt', e.shiftKey && 'shift', k].filter(Boolean).join('+');
}

const SETTINGS_KEY = 'oniscreen-settings';
const SETTINGS_VER = 3;
const DEFAULTS = { haloOn: true, ripple: true, haloSize: 44, haloOpacity: 100, zoomDefault: 2, zoomStep: 0.25, zoomLadder: false, drawView: 'stroke', keys: DEFAULT_KEYS, penColor: '#292927', textColor: '#292927', zoomTarget: -1, hotkeys: true, haloColor: '35,131,226', rippleColor: '35,131,226', penWidth: 4, textSize: 28, reduceMotion: false };

function loadSettings() {
  try {
    const saved = JSON.parse(localStorage.getItem(SETTINGS_KEY) || '{}');
    // 기본 단축키/확대 방식을 바꾼 버전: 이전에 자동 저장된 값은 버린다
    if (saved.keys) { delete saved.keys.zout; delete saved.keys.record; delete saved.keys['shot-save']; }
    if (saved.keys) delete saved.keys['zoom-mode'];
    delete saved.zoomMode; delete saved.holdKey;
    // Preserve user shortcuts across upgrades; new fields receive defaults.
    if (!saved.rippleColor) saved.rippleColor = saved.haloColor || DEFAULTS.rippleColor;
    if (!saved.textColor) saved.textColor = saved.penColor || DEFAULTS.textColor;
    return { ...DEFAULTS, ...saved, keys: { ...DEFAULT_KEYS, ...(saved.keys || {}) } };
  }
  catch { return { ...DEFAULTS }; }
}
function saveSettings(s) {
  try { localStorage.setItem(SETTINGS_KEY, JSON.stringify({ ...s, ver: SETTINGS_VER })); return true; } catch { return false; }
}
const hexToRgb = (h) => `${parseInt(h.slice(1, 3), 16)},${parseInt(h.slice(3, 5), 16)},${parseInt(h.slice(5, 7), 16)}`;
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
