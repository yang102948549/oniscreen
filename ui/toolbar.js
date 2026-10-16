/* 툴바 = 앱 상태의 단일 소유자. 상태를 `state` 이벤트로 모든 오버레이에 방송한다. */
const st = {
  tool: null, color: loadSettings().penColor, shape: 'rect', hlShape: false,
  board: null, boardMon: 0,
  z: 1, zoomActive: false, zoomMon: -1, owner: -1, editing: false,
};
let settings = loadSettings();

const bar = $('#bar');
const stack = $('#stack');
const toolBtns = $$('#tools .btn');
const shapeBtns = $$('#shapes .btn');
const ptrBtn = $('#ptrBtn');
let zoomRangeDragging = false;
let toolbarFolded = false;

const DRAWING = ['pen', 'hl', 'eraser', 'text', 'shape'];
const isDrawing = () => DRAWING.includes(st.tool);

function pickColor(hex) {
  settings = loadSettings();
  if (!st.tool) {
    settings.haloColor = hex.includes(',') ? hex : hexToRgb(hex);
    saveSettings(settings); render(); return;
  }
  settings[st.tool === 'text' ? 'textColor' : 'penColor'] = hex;
  saveSettings(settings);
  if (!st.tool || st.tool === 'eraser') st.tool = 'pen';
  render(); pushState();
}

function render() {
  st.color = settings[st.tool === 'text' ? 'textColor' : 'penColor'];
  $('#zoomFixed').setAttribute('aria-pressed', String(st.zoomActive));
  $('#palette').setAttribute('aria-label', !st.tool ? '포인터 색상' : st.tool === 'text' ? '텍스트 색상' : '펜 색상');
  stack.classList.toggle('pointer-mode', !st.tool && !st.zoomActive);
  stack.classList.toggle('zoom-mode', st.zoomActive);
  stack.classList.toggle('eraser-mode', st.tool === 'eraser');
  const activeButton = st.tool === 'shape' ? document.querySelector(`#shapes [data-shape="${st.shape}"]`) : st.tool ? document.querySelector(`#tools [data-tool="${st.tool}"]`) : st.zoomActive ? $('#zoomFixed') : ptrBtn;
  $('#restore').replaceChildren(activeButton.querySelector('svg').cloneNode(true));
  $('#restore').setAttribute('aria-label', `${activeButton.getAttribute('aria-label')} · 툴바 펼치기`);
  bar.classList.toggle('folded', toolbarFolded);
  stack.classList.toggle('folded', toolbarFolded);
  $('#fold').setAttribute('aria-expanded', String(!toolbarFolded));
  $('#fold').setAttribute('aria-label', toolbarFolded ? '펼치기' : '접기');
  $('#fold .tip').textContent = toolbarFolded ? '툴바 펼치기' : '툴바 접기';
  toolBtns.forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.tool === st.tool)));
  shapeBtns.forEach((b) => b.setAttribute('aria-pressed', String(st.tool === 'shape' && b.dataset.shape === st.shape)));
  ptrBtn.setAttribute('aria-pressed', String(settings.haloOn));
  $('#hlShape').setAttribute('aria-pressed', String(st.hlShape));
  $('#boardW').setAttribute('aria-pressed', String(st.board === 'white'));
  $('#boardK').setAttribute('aria-pressed', String(st.board === 'black'));
  $$('#palette button').forEach((b, i) => {
    const c = !st.tool ? HALO_COLORS[i] : PEN_COLORS[i];
    b.dataset.color = c.rgb || c.hex;
    b.setAttribute('aria-label', c.name);
    b.querySelector('i').style.setProperty('--swatch', c.rgb ? `rgb(${c.rgb})` : c.hex);
    b.setAttribute('aria-pressed', String(b.dataset.color === (!st.tool ? settings.haloColor : st.color)));
  });
  $$('#ringPalette button').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.color === settings.rippleColor)));
  $('#pointerOpacity').value = settings.haloOpacity;
  $('#pointerOpacityValue').textContent = `${settings.haloOpacity}%`;
  $('#pointerSize').value = settings.haloSize;
  $('#pointerSizeValue').textContent = `${settings.haloSize}px`;
  $('#pw').value = settings.penWidth; $('#pwv').textContent = `${settings.penWidth}px`;
  const z = st.zoomActive ? st.z : settings.zoomDefault;
  if (!zoomRangeDragging) $('#zoomRange').value = z;
  $('#zoomValue').textContent = `${(zoomRangeDragging ? Number($('#zoomRange').value) : z).toFixed(2).replace(/\.?0+$/, '')}×`;
}


const pushState = () => {
  emit('state', { tool: st.tool, color: st.color, shape: st.shape, hlShape: st.hlShape, board: st.board, boardMon: st.boardMon });
  // 필기 중에도 화면은 라이브. 선을 긋는 동안만 고정하고, 텍스트는 입력 내내 고정한다
  // 설정 '필기 중 화면 동작': live = 계속 따라감 · stroke = 선 긋는 동안만 고정 · always = 도구가 켜져 있는 동안 고정
  const dv = settings.drawView;
  invoke('zoom_freeze', { on: isDrawing() && (dv !== 'live' || st.tool === 'text'), hold: st.tool !== 'text' && dv !== 'always' });
  syncKeys();
};
// 전역 키: Esc는 확대/도구가 있는 동안, Ctrl+Z는 도구가 켜진 동안, 색상 글자는 텍스트 입력 중이 아닐 때만
const syncKeys = () => invoke('grab_keys', { esc: st.zoomActive || !!st.tool || !!st.board, undo: !!st.tool, keys: isDrawing() && !st.editing, zoom: st.zoomActive, editing: st.editing });

function setTool(t) { st.tool = st.tool === t ? null : t; render(); pushState(); }
function setShape(id) {
  if (st.tool === 'shape' && st.shape === id) st.tool = null; else { st.tool = 'shape'; st.shape = id; }
  render(); pushState();
}
function togglePointer() {
  settings = loadSettings();
  if (st.tool || st.board || st.zoomActive) {
    st.tool = null; st.board = null; settings.haloOn = true;
    if (st.zoomActive) invoke('zoom_exit');
  } else settings.haloOn = !settings.haloOn;
  saveSettings(settings); render(); pushState();
}
// 확대 중이 아니면 기본 배율로 시작하고, 확대 중이면 한 단계 키운다
function zoomIn() {
  if (st.zoomActive) return zoomStep(0.25);
  return invoke('begin_zoom', { z: settings.zoomDefault });
}
function toggleZoom() {
  if (st.zoomActive) invoke('zoom_exit');
  else invoke('begin_zoom', { z: settings.zoomDefault });
}
const zoomStep = (d) => invoke('zoom_step', { d: d * settings.zoomStep / 0.25 });

// 화면 대상(스크린샷·보드): 확대 중인 화면 > 마지막으로 필기한 화면 > 작업 중인 화면
async function targetMon() {
  if (st.zoomActive && st.zoomMon >= 0) return st.zoomMon;
  if (st.owner >= 0) return st.owner;
  return invoke('content_monitor');
}
async function setBoard(kind) {
  st.board = st.board === kind ? null : kind;
  if (st.board) {
    st.boardMon = await targetMon();
    if (!isDrawing()) st.tool = 'pen'; // 보드는 바로 필기할 수 있게
  }
  render(); pushState();
}
const cycleBoard = () => setBoard(st.board === null ? 'white' : st.board === 'white' ? 'black' : 'white');
async function shot(op) { emit('shot', { op, m: await targetMon() }); }
function setPenWidth(w) {
  settings = loadSettings();
  settings.penWidth = Math.min(MAX_PEN, Math.max(MIN_PEN, Math.round(w)));
  saveSettings(settings); render();
}

toolBtns.forEach((b) => (b.onclick = () => setTool(b.dataset.tool)));
shapeBtns.forEach((b) => (b.onclick = () => setShape(b.dataset.shape)));
$('#hlShape').onclick = () => { st.hlShape = !st.hlShape; if (st.tool !== 'shape') { st.tool = 'shape'; } render(); pushState(); };
$('#boardW').onclick = () => setBoard('white');
$('#boardK').onclick = () => setBoard('black');
$('#shotCopy').onclick = () => shot('copy');
$$('#palette button').forEach(b => b.onclick = () => pickColor(b.dataset.color));
$$('#ringPalette button').forEach(b => b.onclick = () => { settings = loadSettings(); settings.rippleColor = b.dataset.color; saveSettings(settings); render(); });
$('#pw').oninput = (e) => setPenWidth(+e.target.value);
for (const [id, key] of [['#pointerOpacity', 'haloOpacity'], ['#pointerSize', 'haloSize']]) {
  $(id).oninput = e => { settings = loadSettings(); settings[key] = Number(e.target.value); saveSettings(settings); render(); };
}
ptrBtn.onclick = togglePointer;
ptrBtn.oncontextmenu = (e) => { e.preventDefault(); invoke('open_settings'); };
$('#zoomFixed').onclick = toggleZoom;
$('#zoomOut').onclick = () => zoomStep(-0.25);
$('#zoomIn').onclick = () => zoomStep(0.25);
$('#zoomExit').onclick = () => invoke('zoom_exit');
$('#zoomRange').oninput = e => {
  zoomRangeDragging = true;
  settings = loadSettings(); settings.zoomDefault = Number(e.target.value); saveSettings(settings);
  if (st.zoomActive) invoke('zoom_set', { z: settings.zoomDefault });
  render();
};
$('#zoomRange').onchange = () => { zoomRangeDragging = false; render(); };
$('#zoomRange').addEventListener('pointercancel', () => { zoomRangeDragging = false; render(); });
$('#undo').onclick = () => emit('ink', { op: 'undo' });
$('#eraseSelected').onclick = () => { st.tool = 'eraser'; render(); pushState(); };
$('#clear').onclick = () => emit('ink', { op: 'clear' });
$('#gear').onclick = () => invoke('open_settings');
$('#quit').onclick = () => invoke('quit');
$('#fold').onclick = () => {
  toolbarFolded = !toolbarFolded;
  if (toolbarFolded) popupMenus.forEach(menu => { menu.open = false; });
  render(); resizeToolbar();
};
const restoreButton = $('#restore');
let restoreDrag = null, restoreDragged = false;
restoreButton.onclick = (e) => {
  if (restoreDragged && e?.detail !== 0) return;
  toolbarFolded = false; render(); resizeToolbar();
};
restoreButton.addEventListener('pointerdown', e => {
  if (e.button !== 0) return;
  restoreDragged = false;
  restoreDrag = { x: e.clientX, y: e.clientY, id: e.pointerId };
  restoreButton.setPointerCapture(e.pointerId);
});
restoreButton.addEventListener('pointermove', e => {
  if (!restoreDrag || e.pointerId !== restoreDrag.id ||
      Math.hypot(e.clientX - restoreDrag.x, e.clientY - restoreDrag.y) < 5) return;
  const id = restoreDrag.id;
  restoreDrag = null; restoreDragged = true;
  e.preventDefault();
  if (restoreButton.hasPointerCapture(id)) restoreButton.releasePointerCapture(id);
  T.window.getCurrentWindow().startDragging().catch(console.error);
});
for (const event of ['pointerup', 'pointercancel', 'lostpointercapture']) {
  restoreButton.addEventListener(event, () => { restoreDrag = null; });
}

// 글자 단축키(ZoomIt): 색상 R G B O Y K W. 기본 색은 설정에서 고른다.
const COLOR_KEY = Object.fromEntries(PEN_COLORS.map((c) => [c.key, c.hex]));
function letterKey(k) { if (COLOR_KEY[k]) pickColor(COLOR_KEY[k]); }

// 전역 단축키(Rust) / 오버레이 키 입력
listen('hotkey', (e) => {
  const a = e.payload;
  if (typeof a === 'string' && a.startsWith('key:')) { if (isDrawing() && !st.editing) letterKey(a.slice(4)); return; }
  ({
    zoom: toggleZoom, zin: zoomIn, zout: () => zoomStep(-0.25),
    esc: () => {
      if (st.editing) emit('text-cancel');
      else if (st.zoomActive) invoke('zoom_exit');
      else if (st.board) setBoard(st.board);
      else if (st.tool) setTool(st.tool);
    },
    undo: () => { if (!st.editing) emit('ink', { op: 'undo' }); },
    pen: () => setTool('pen'), hl: () => setTool('hl'), eraser: () => setTool('eraser'), text: () => setTool('text'),
    ptr: togglePointer, clear: () => emit('ink', { op: 'clear' }),
    board: cycleBoard, 'shot-copy': () => shot('copy'),
  })[a]?.();
});

const KEY_TOOL = { p: 'pen', h: 'hl', e: 'eraser', t: 'text' };
listen('key', ({ payload: { key, ctrl } }) => {
  const k = key.toLowerCase();
  if (k === 'escape') { if (st.zoomActive) invoke('zoom_exit'); else if (st.board) setBoard(st.board); else if (st.tool) setTool(st.tool); }
  else if (ctrl && k === 'z') emit('ink', { op: 'undo' });
  else if (ctrl && k === 'c') shot('copy');
  else if (!ctrl && KEY_TOOL[k]) setTool(KEY_TOOL[k]);
  else if (!ctrl && COLOR_KEY[k] && isDrawing()) letterKey(k);
  else if (!ctrl && k === 'm') togglePointer();
  else if (!ctrl && k === 'delete') emit('ink', { op: 'clear' });
  else if (!ctrl && (k === '+' || k === '=')) zoomStep(0.25);
  else if (!ctrl && (k === '-' || k === '_')) zoomStep(-0.25);
});

listen('zoom-changed', ({ payload }) => {
  st.zoomActive = payload.active; st.z = payload.z; st.held = !!payload.frozen; st.zoomMon = payload.active ? payload.m : -1; render();
  syncKeys();
});
listen('state-request', pushState);
listen('text-editing', ({ payload }) => { st.editing = payload.on; syncKeys(); });
listen('ink-owner', ({ payload }) => { st.owner = payload.m; });
listen('pen-width', () => { settings = loadSettings(); render(); });
addEventListener('storage', () => {
  settings = loadSettings();
  render(); pushState();
});

// 창 크기를 툴바 내용에 맞춤 (아래 36px은 툴팁 여유)
let resizePending = false;
let resizeRunning = false;
let lastToolbarSize = '';
async function resizeToolbar() {
  resizePending = true;
  if (resizeRunning) return;
  resizeRunning = true;
  try {
    while (resizePending) {
      resizePending = false;
      const r = stack.getBoundingClientRect();
      const popups = toolbarFolded ? [] : $$('.width-menu[open] .width-options, .zoom-menu[open] .zoom-options, .pointer-size-menu[open] .pointer-size-options');
      const rects = popups.map(p => p.getBoundingClientRect());
      const right = Math.max(r.right ?? r.width + 8, ...rects.map(p => p.right));
      const bottom = Math.max(r.bottom, ...rects.map(p => p.bottom));
      const width = Math.ceil(right) + 8, height = Math.ceil(bottom) + 44;
      const key = `${width}:${height}`;
      if (key === lastToolbarSize) continue;
      await invoke('resize_toolbar', { width, height });
      lastToolbarSize = key;
    }
  } catch (error) { console.error('Toolbar resize failed', error); }
  finally { resizeRunning = false; }
}
new ResizeObserver(resizeToolbar).observe(stack);
const popupMenus = [document.querySelector('.width-menu'), document.querySelector('.pointer-size-menu')];
for (const menu of popupMenus) {
  menu.addEventListener('toggle', () => {
    if (menu.open) popupMenus.forEach(other => { if (other !== menu) other.open = false; });
    resizeToolbar();
  });
}
new ResizeObserver(resizeToolbar).observe(document.querySelector('.width-options'));
addEventListener('pointerdown', e => popupMenus.forEach(menu => { if (!menu.contains(e.target)) menu.open = false; }));
addEventListener('keydown', e => { if (e.key === 'Escape') popupMenus.forEach(menu => { menu.open = false; }); });



render();
pushState();

// ───── 툴바 이동: 버튼/슬라이더가 아닌 곳을 잡고 끌면 된다(접었을 때도 동일) ─────
// Windows가 드래그 수명과 포인터 캡처를 관리한다.
stack.addEventListener('pointerdown', (e) => {
  if (e.button !== 0 || e.target.closest('button, input, summary, details')) return;
  e.preventDefault();
  T.window.getCurrentWindow().startDragging().catch(console.error);
});

// 설정을 Rust에 반영: 확대 대상 화면, 전역 단축키 사용 여부
const applyNative = () => {
  settings = loadSettings();
  invoke('set_zoom_target', { index: settings.zoomTarget });
  invoke('set_zoom_prefs', { ladder: !!settings.zoomLadder, step: settings.zoomStep });
  pushState(); // 필기 중 화면 동작 설정 반영
  invoke('set_hotkeys', { on: settings.hotkeys, keys: hotkeyList(settings.keys) });
  labelKeys();
};

// 툴팁의 단축키 표기를 설정과 맞춘다
const TIP_ACT = { ptrBtn: 'ptr', eraseSelected: 'eraser', clear: 'clear', boardW: 'board', boardK: 'board', shotCopy: 'shot-copy', quit: 'quit' };
function labelKeys() {
  const set = (el, act) => { const k = el?.querySelector('.tip kbd'); if (k && settings.keys[act]) k.textContent = keyLabel(settings.keys[act]); };
  Object.entries(TIP_ACT).forEach(([id, act]) => set($('#' + id), act));
  toolBtns.forEach((b) => set(b, b.dataset.tool));
}
addEventListener('storage', applyNative);
applyNative();
