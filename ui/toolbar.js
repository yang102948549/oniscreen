/* 툴바 = 앱 상태의 단일 소유자. 상태를 `state` 이벤트로 모든 오버레이에 방송한다. */
const st = {
  tool: null, color: loadSettings().penColor, shape: 'rect', hlShape: false,
  board: null, boardMon: 0,
  z: 1, zoomActive: false, zoomMon: -1, owner: -1, editing: false,
  rec: false, recSecs: 0,
};
let settings = loadSettings();

const bar = $('#bar');
const stack = $('#stack');
const zs = $('#zslider');
let zDrag = false;
const toolBtns = $$('#tools .btn');
const shapeBtns = $$('#shapes .btn');
const ptrBtn = $('#ptrBtn');

const DRAWING = ['pen', 'hl', 'eraser', 'text', 'shape'];
const isDrawing = () => DRAWING.includes(st.tool);

function pickColor(hex) {
  st.color = hex;
  if (!st.tool || st.tool === 'eraser') st.tool = 'pen';
  render(); pushState();
}

function render() {
  stack.classList.toggle('has-sub', isDrawing() || !!st.board || st.rec);
  toolBtns.forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.tool === st.tool)));
  shapeBtns.forEach((b) => b.setAttribute('aria-pressed', String(st.tool === 'shape' && b.dataset.shape === st.shape)));
  ptrBtn.setAttribute('aria-pressed', String(settings.haloOn));
  $('#hlShape').setAttribute('aria-pressed', String(st.hlShape));
  $('#boardW').setAttribute('aria-pressed', String(st.board === 'white'));
  $('#boardK').setAttribute('aria-pressed', String(st.board === 'black'));
  $('#pw').value = settings.penWidth; $('#pwv').textContent = `${settings.penWidth}px`;
  const rec = $('#rec');
  rec.setAttribute('aria-pressed', String(st.rec));
  $('#recT').textContent = st.rec ? fmtTime(st.recSecs) : '녹화';
  const z = st.zoomActive ? st.z : 1;
  $('#zhold').hidden = !(st.zoomActive && st.held);
  $('#zval').textContent = `${z.toFixed(2).replace(/0$/, '')}×`;
  if (!zDrag) zs.value = z;
}
const fmtTime = (s) => `${String(Math.floor(s / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;

const pushState = () => {
  emit('state', { tool: st.tool, color: st.color, shape: st.shape, hlShape: st.hlShape, board: st.board, boardMon: st.boardMon });
  // 필기 중에도 화면은 라이브. 선을 긋는 동안만 고정하고, 텍스트는 입력 내내 고정한다
  // 설정 '필기 중 화면 동작': live = 계속 따라감 · stroke = 선 긋는 동안만 고정 · always = 도구가 켜져 있는 동안 고정
  const dv = settings.drawView;
  invoke('zoom_freeze', { on: isDrawing() && (dv !== 'live' || st.tool === 'text'), hold: st.tool !== 'text' && dv !== 'always' });
  syncKeys();
};
// 전역 키: Esc는 확대/도구가 있는 동안, Ctrl+Z는 도구가 켜진 동안, 색상 글자는 텍스트 입력 중이 아닐 때만
const syncKeys = () => invoke('grab_keys', { esc: st.zoomActive || !!st.tool || !!st.board, undo: !!st.tool, keys: isDrawing() && !st.editing, zoom: st.zoomActive });

function setTool(t) { st.tool = st.tool === t ? null : t; render(); pushState(); }
function setShape(id) {
  if (st.tool === 'shape' && st.shape === id) st.tool = null; else { st.tool = 'shape'; st.shape = id; }
  render(); pushState();
}
function togglePointer() { settings.haloOn = !settings.haloOn; saveSettings(settings); render(); }
// 확대 중이 아니면 기본 배율로 시작하고, 확대 중이면 한 단계 키운다
function zoomIn() { if (st.zoomActive) zoomStep(0.25); else invoke('begin_zoom', { z: settings.zoomDefault }); }
function toggleZoom() {
  if (st.zoomActive) invoke('zoom_exit');
  else invoke('begin_zoom', { z: settings.zoomDefault });
}
const zoomStep = (d) => invoke('zoom_step', { d: d * settings.zoomStep / 0.25 });

// 화면 대상(스크린샷·보드·녹화): 확대 중인 화면 > 마지막으로 필기한 화면 > 작업 중인 화면
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
async function toggleRecord() {
  if (st.rec) { invoke('record_toggle', { on: false }); return; }
  invoke('record_toggle', { on: true, mon: await targetMon() });
}
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
$('#shotSave').onclick = () => shot('save');
$('#rec').onclick = toggleRecord;
$('#pw').oninput = (e) => setPenWidth(+e.target.value);
ptrBtn.onclick = togglePointer;
ptrBtn.oncontextmenu = (e) => { e.preventDefault(); invoke('open_settings'); };
zs.oninput = () => { zDrag = true; invoke('zoom_set', { z: +zs.value }); };
zs.onchange = () => { zDrag = false; render(); };
$('#zin').onclick = zoomIn;
$('#zout').onclick = () => zoomStep(-0.25);
$('#undo').onclick = () => emit('ink', { op: 'undo' });
$('#clear').onclick = () => emit('ink', { op: 'clear' });
$('#gear').onclick = () => invoke('open_settings');
$('#quit').onclick = () => { if (st.rec) invoke('record_toggle', { on: false }); invoke('quit'); };
$('#fold').onclick = () => { bar.classList.toggle('folded'); stack.classList.toggle('folded'); };

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
    board: cycleBoard, 'shot-copy': () => shot('copy'), 'shot-save': () => shot('save'), record: toggleRecord,
  })[a]?.();
});

const KEY_TOOL = { p: 'pen', h: 'hl', e: 'eraser', t: 'text' };
listen('key', ({ payload: { key, ctrl } }) => {
  const k = key.toLowerCase();
  if (k === 'escape') { if (st.zoomActive) invoke('zoom_exit'); else if (st.board) setBoard(st.board); else if (st.tool) setTool(st.tool); }
  else if (ctrl && k === 'z') emit('ink', { op: 'undo' });
  else if (ctrl && k === 'c') shot('copy');
  else if (ctrl && k === 's') shot('save');
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
listen('record-state', ({ payload }) => { st.rec = !!payload.active; st.recSecs = payload.secs || 0; render(); });
// 토스트는 오버레이가 그린다. 녹화 결과(저장 경로/오류)는 대상 화면의 오버레이에 알린다.
listen('record-state', async ({ payload }) => {
  if (payload.active) return;
  const m = await targetMon();
  if (payload.path) emit('toast', { m, msg: `녹화 저장 · ${payload.path}` });
  else if (payload.error) emit('toast', { m, msg: `녹화 실패: ${payload.error}` });
});
addEventListener('storage', () => {
  const prev = settings.penColor;
  settings = loadSettings();
  if (settings.penColor !== prev) { st.color = settings.penColor; pushState(); }
  render();
});

// 창 크기를 툴바 내용에 맞춤 (아래 36px은 툴팁 여유)
new ResizeObserver(() => {
  const r = stack.getBoundingClientRect();
  invoke('resize_toolbar', { width: Math.ceil(r.width) + 16, height: Math.ceil(r.height) + 16 + 36 });
}).observe(stack);

render();
pushState();

// ───── 툴바 이동: 버튼/슬라이더가 아닌 곳을 잡고 끌면 된다(접었을 때도 동일) ─────
// 창이 포커스를 갖지 않으므로 Rust가 보내는 전역 포인터(`ptr`)로 따라간다.
let lastPtr = null, drag = null;
listen('ptr', ({ payload: p }) => {
  lastPtr = p;
  if (!drag) return;
  if (!p.l) { drag = null; bar.classList.remove('moving'); return; }
  invoke('move_toolbar', { x: drag.wx + p.x - drag.px, y: drag.wy + p.y - drag.py });
});
stack.addEventListener('pointerdown', async (e) => {
  if (e.button !== 0 || e.target.closest('button, input')) return;
  e.preventDefault();
  const pos = await invoke('toolbar_pos');
  if (!pos || !lastPtr || !lastPtr.l) return;
  drag = { px: lastPtr.x, py: lastPtr.y, wx: pos[0], wy: pos[1] };
  bar.classList.add('moving');
});


// 설정을 Rust에 반영: 확대 대상 화면, 전역 단축키 사용 여부
const applyNative = () => {
  settings = loadSettings();
  invoke('set_zoom_target', { index: settings.zoomTarget });
  invoke('set_zoom_prefs', { ladder: !!settings.zoomLadder, step: settings.zoomStep, hold: settings.holdKey });
  pushState(); // 필기 중 화면 동작 설정 반영
  invoke('set_hotkeys', { on: settings.hotkeys, keys: hotkeyList(settings.keys) });
  labelKeys();
};

// 툴팁의 단축키 표기를 설정과 맞춘다
const TIP_ACT = { zout: 'zout', zin: 'zin', ptrBtn: 'ptr', clear: 'clear', boardW: 'board', boardK: 'board', shotCopy: 'shot-copy', shotSave: 'shot-save', rec: 'record', quit: 'quit' };
function labelKeys() {
  const set = (el, act) => { const k = el?.querySelector('.tip kbd'); if (k && settings.keys[act]) k.textContent = keyLabel(settings.keys[act]); };
  Object.entries(TIP_ACT).forEach(([id, act]) => set($('#' + id), act));
  toolBtns.forEach((b) => set(b, b.dataset.tool));
}
addEventListener('storage', applyNative);
applyNative();
