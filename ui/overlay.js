/* 모니터 하나를 덮는 투명 오버레이.
 * - 후광/클릭 파문: Rust가 전역 마우스를 폴링해 보내는 `ptr` 이벤트로 그린다(시스템 커서는 건드리지 않음).
 * - 필기: 입력을 받을 때만 클릭 통과를 끈다.
 * - 확대는 Rust가 화면 전체에 적용한다. 오버레이는 항상 1배 좌표계이고 잉크도 같이 확대된다.
 * 좌표계: 모니터 로컬 물리 픽셀.
 *
 * ZoomIt(PowerToys)에서 옮겨온 그리기 규칙
 *  - 펜/형광펜 + Shift = 선, Ctrl = 사각형, Tab = 타원, Ctrl+Shift = 화살표 (마우스를 누른 채 보조키)
 *  - 형광펜 도형은 반투명으로 채운다
 *  - Ctrl + 휠 = 펜 굵기, 화이트보드/칠판, 화면(확대 포함) 복사 */
(async () => {
  const m = Number(T.window.getCurrentWindow().label.split('-')[1]);
  const mon = await invoke('monitor_rect', { index: m });
  const cv = $('#ink'), ctx = cv.getContext('2d');
  const halo = $('#halo');
  const W = mon.w, H = mon.h;
  let dpr = devicePixelRatio || 1;
  cv.width = W; cv.height = H;
  const fit = () => { dpr = devicePixelRatio || 1; cv.style.width = `${W / dpr}px`; cv.style.height = `${H / dpr}px`; dirty = true; };

  let S = loadSettings();
  const tool = { id: null, color: PEN_COLORS[0].hex, shape: 'rect', board: null };
  const DRAW = ['pen', 'hl', 'eraser', 'text', 'shape'];
  const FONT = '"Noto Sans KR", "Malgun Gothic", sans-serif';
  const HL_ALPHA = 0.35;

  const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
  // 확대는 Rust가 그린다. 오버레이(잉크)는 확대되지 않으므로 같은 뷰로 직접 변환해 그린다.
  // 화면 = (원본 - o) * z. 확대 화면의 클릭을 같은 뷰의 원본 좌표로 역변환한다.
  const view = { active: false, z: 1, ox: 0, oy: 0 };
  const toSrc = (x, y) => (view.active ? [x / view.z + view.ox, y / view.z + view.oy] : [x, y]);
  const zf = () => (view.active ? view.z : 1);
  const frozen = () => DRAW.includes(tool.id);

  // ───── 잉크 ─────
  // stroke = { tool:'pen'|'hl'|'text', kind:'free'|'line'|'rect'|'ellipse'|'arrow', color, w, pts | a,b }
  const strokes = [];
  const undoStack = [];    // {add:stroke} | {erase:[...]} | {clear:[...]}
  let cur = null, ownsInk = false, erasing = false;
  let dirty = true;

  // ───── 화면 스냅샷(스크린샷) ─────
  // Rust가 실제 표시된 확대·잉크·포인터를 포함한 화면을 돌려준다.
  async function grab() {
    const buf = await invoke('grab_screen', { index: m });
    const dv = new DataView(buf);
    const w = dv.getUint32(0, true), h = dv.getUint32(4, true);
    const raw = document.createElement('canvas'); raw.width = w; raw.height = h;
    raw.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(buf, 8, w * h * 4), w, h), 0, 0);
    return { raw, w, h };
  }
  // ───── 그리기 ─────
  const textLines = (s) => s.text.split('\n');
  const drawText = (g, s) => {
    g.save();
    g.font = `500 ${s.size}px ${FONT}`; g.fillStyle = s.color; g.textBaseline = 'top';
    textLines(s).forEach((ln, i) => g.fillText(ln, s.x, s.y + i * s.size * 1.25));
    g.restore();
  };
  const textBox = (s) => {
    ctx.save(); ctx.font = `500 ${s.size}px ${FONT}`;
    const w = Math.max(...textLines(s).map((ln) => ctx.measureText(ln).width));
    ctx.restore();
    return [s.x, s.y, s.x + w, s.y + textLines(s).length * s.size * 1.25];
  };
  // 화살촉: ZoomIt DrawArrow와 같은 비율(길이 2.5w, 폭 1.5w), 끝점이 촉 끝
  function arrowHead(a, b, w) {
    let dx = b[0] - a[0], dy = b[1] - a[1];
    const len = Math.hypot(dx, dy) || 1; dx /= len; dy /= len;
    const L = w * 2.5, Wd = w * 1.5;
    const mx = b[0] - L * dx, my = b[1] - L * dy;
    return [[mx - dy * Wd, my + dx * Wd], b, [mx + dy * Wd, my - dx * Wd]];
  }
  function shapePath(g, s) {
    const [a, b] = [s.a, s.b];
    g.beginPath();
    if (s.kind === 'rect') g.rect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
    else if (s.kind === 'ellipse') {
      const rx = Math.abs(b[0] - a[0]) / 2, ry = Math.abs(b[1] - a[1]) / 2;
      g.ellipse((a[0] + b[0]) / 2, (a[1] + b[1]) / 2, Math.max(rx, 0.5), Math.max(ry, 0.5), 0, 0, Math.PI * 2);
    } else {
      g.moveTo(a[0], a[1]); g.lineTo(b[0], b[1]);
      if (s.kind === 'arrow') { const [l, t, r] = arrowHead(a, b, s.w); g.moveTo(l[0], l[1]); g.lineTo(t[0], t[1]); g.lineTo(r[0], r[1]); }
    }
  }
  function freePath(g, p) {
    g.beginPath(); g.moveTo(p[0], p[1]);
    if (p.length === 2) g.lineTo(p[0], p[1]);
    for (let i = 2; i < p.length - 2; i += 2) g.quadraticCurveTo(p[i], p[i + 1], (p[i] + p[i + 2]) / 2, (p[i + 1] + p[i + 3]) / 2);
    if (p.length >= 4) g.lineTo(p[p.length - 2], p[p.length - 1]);
  }
  function drawStroke(g, s) {
    if (s.tool === 'text') return drawText(g, s);
    g.save();
    g.lineCap = 'round'; g.lineJoin = 'round'; g.lineWidth = s.w;
    const closed = s.kind === 'rect' || s.kind === 'ellipse';
    {
      g.strokeStyle = g.fillStyle = s.color;
      if (s.tool === 'hl') g.globalAlpha = HL_ALPHA;
    }
    if (s.kind === 'free') { freePath(g, s.pts); g.stroke(); }
    else {
      shapePath(g, s);
      // 형광펜은 사각형·타원을 안쪽까지 채운다(ZoomIt DrawHighlightedShape)
      if (closed && s.tool === 'hl') g.fill();
      else g.stroke();
    }
    g.restore();
  }
  function drawBoard(g) {
    if (!tool.board) return;
    g.save(); g.setTransform(1, 0, 0, 1, 0, 0);
    g.fillStyle = tool.board === 'black' ? '#000' : '#fff';
    g.fillRect(0, 0, W, H); g.restore();
  }
  function render() {
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, W, H);
    drawBoard(ctx);
    ctx.setTransform(zf(), 0, 0, zf(), -view.ox * zf(), -view.oy * zf());
    strokes.forEach((s) => drawStroke(ctx, s));
    if (cur) drawStroke(ctx, cur);
  }
  (function loop() {
    if (dirty) { dirty = false; render(); }
    requestAnimationFrame(loop);
  })();

  // ───── 지우개 판정 ─────
  const distSeg = (px, py, ax, ay, bx, by) => {
    const dx = bx - ax, dy = by - ay, l2 = dx * dx + dy * dy;
    const t = l2 ? clamp(((px - ax) * dx + (py - ay) * dy) / l2, 0, 1) : 0;
    return Math.hypot(px - (ax + t * dx), py - (ay + t * dy));
  };
  const polyline = (s) => {
    if (s.kind === 'free') return s.pts;
    const [a, b] = [s.a, s.b];
    if (s.kind === 'rect') return [a[0], a[1], b[0], a[1], b[0], b[1], a[0], b[1], a[0], a[1]];
    if (s.kind === 'ellipse') {
      const cx = (a[0] + b[0]) / 2, cy = (a[1] + b[1]) / 2, rx = Math.abs(b[0] - a[0]) / 2, ry = Math.abs(b[1] - a[1]) / 2, o = [];
      for (let i = 0; i <= 48; i++) o.push(cx + rx * Math.cos(i / 48 * 6.2832), cy + ry * Math.sin(i / 48 * 6.2832));
      return o;
    }
    return [a[0], a[1], b[0], b[1]];
  };
  const hits = (s, x, y, r) => {
    if (s.tool === 'text') { const b = textBox(s); return x >= b[0] - r && x <= b[2] + r && y >= b[1] - r && y <= b[3] + r; }
    // 채운 도형은 안쪽도 맞는 것으로 본다
    if (s.tool === 'hl' && (s.kind === 'rect' || s.kind === 'ellipse')) {
      const x0 = Math.min(s.a[0], s.b[0]) - r, x1 = Math.max(s.a[0], s.b[0]) + r, y0 = Math.min(s.a[1], s.b[1]) - r, y1 = Math.max(s.a[1], s.b[1]) + r;
      if (s.kind === 'rect') return x >= x0 && x <= x1 && y >= y0 && y <= y1;
      const rx = (x1 - x0) / 2, ry = (y1 - y0) / 2;
      return ((x - (x0 + x1) / 2) / rx) ** 2 + ((y - (y0 + y1) / 2) / ry) ** 2 <= 1;
    }
    const p = polyline(s), rr = r + s.w / 2;
    if (p.length === 2) return Math.hypot(x - p[0], y - p[1]) <= rr;
    for (let i = 0; i + 3 < p.length; i += 2) if (distSeg(x, y, p[i], p[i + 1], p[i + 2], p[i + 3]) <= rr) return true;
    return false;
  };
  const eraseAt = (x, y) => {
    const r = 14 * dpr / zf(), gone = [];
    for (let i = strokes.length - 1; i >= 0; i--) if (hits(strokes[i], x, y, r)) gone.push(strokes.splice(i, 1)[0]);
    if (gone.length) { (eraseAt.batch ||= []).push(...gone); dirty = true; }
  };

  // ───── 입력 모드 (클릭 통과 on/off) ─────
  let interactive = null, syncingInteractive = false;
  async function syncInteractive() {
    if (syncingInteractive) return;
    syncingInteractive = true;
    try {
      // Serialize native changes; apply the newest mode after any pending change.
      while (true) {
        const want = view.active || frozen() || !!tool.board;
        if (want === interactive) break;
        await invoke('set_interactive', { index: m, on: want });
        interactive = want;
      }
    } catch (error) { interactive = null; console.error('Overlay input sync failed', error); }
    finally { syncingInteractive = false; }
  }

  // ───── 토스트 / 펜 굵기 표시 ─────
  const toastEl = $('#toast');
  let toastT = 0;
  function toast(msg) {
    toastEl.textContent = msg; toastEl.classList.add('on');
    clearTimeout(toastT); toastT = setTimeout(() => toastEl.classList.remove('on'), 2600);
  }
  const penHud = $('#penhud');
  let hudT = 0;
  function setPenWidth(w) {
    S = loadSettings();
    S.penWidth = clamp(Math.round(w), MIN_PEN, MAX_PEN);
    saveSettings(S);
    penHud.style.setProperty('--d', `${S.penWidth}px`);
    penHud.classList.add('on'); clearTimeout(hudT); hudT = setTimeout(() => penHud.classList.remove('on'), 800);
    emit('pen-width', { w: S.penWidth });
  }

  // ───── 텍스트 입력 ─────
  // 입력하는 동안만 오버레이가 포커스를 받는다(그 외에는 툴바가 항상 위에 있도록 포커스를 갖지 않음).
  const tbox = $('#tbox'), textEditor = $('#textEditor'), textResize = $('#textResize');
  let textDrag = null;
  let editing = null; // { x, y } 원본 좌표
  async function openText(e) {
    commitText();
    stopFollowing();
    const [x, y] = local(e);
    editing = { x, y, size: S.textSize, color: tool.color };
    textEditor.hidden = false; $('#textSizeMenu').open = false; $('#textFontSize').value = editing.size;
    $('#textSizeLabel').textContent = `${editing.size}px`;
    tbox.textContent = '';
    tbox.style.cssText = `display:block;font-size:${S.textSize}px;color:${tool.color}`;
    positionText();
    emit('text-editing', { on: true });
    await invoke('focus_overlay', { index: m, on: true });
    if (editing) tbox.focus({ preventScroll: true });
  }
  function closeText(commit) {
    if (!editing) return;
    const { x, y, size, color } = editing; editing = null;
    textDrag = null; textEditor.hidden = true;
    const text = tbox.innerText.replace(/\n+$/, '');
    tbox.style.display = 'none'; tbox.textContent = '';
    emit('text-editing', { on: false });
    invoke('focus_overlay', { index: m, on: view.active });
    if (commit && text.trim()) {
      const t = { tool: 'text', color, size: size * dpr / zf(), x, y, text };
      strokes.push(t); undoStack.push({ add: t }); ownsInk = true; emit('ink-owner', { m }); dirty = true;
    }
  }
  const commitText = () => closeText(true);
  tbox.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); commitText(); }
  });
  tbox.addEventListener('blur', () => { const current = editing; setTimeout(() => { if (editing === current && !textEditor.contains(document.activeElement)) commitText(); }, 0); });
  listen('text-cancel', () => closeText(false));
  for (const id of ['#textDelete']) {
    $(id).addEventListener('pointerdown', e => e.preventDefault());
  }
  $('#textDelete').addEventListener('click', () => closeText(false));
  function resizeText(size) {
    if (!editing) return;
    editing.size = clamp(Math.round(size), 14, 144);
    tbox.style.fontSize = `${editing.size}px`;
    $('#textFontSize').value = editing.size;
    $('#textSizeLabel').textContent = `${editing.size}px`;
  }
  function saveTextSize() {
    if (!editing) return;
    S = { ...loadSettings(), textSize: editing.size }; saveSettings(S);
  }
  $('#textFontSize').addEventListener('input', e => { resizeText(Number(e.target.value)); saveTextSize(); });
  textResize.addEventListener('pointerdown', e => {
    if (e.button !== 0 || !editing) return;
    e.preventDefault(); e.stopPropagation();
    const r = tbox.getBoundingClientRect();
    textDrag = { x: e.clientX, y: e.clientY, w: Math.max(r.width, 40), h: Math.max(r.height, 28), size: editing.size };
    textResize.setPointerCapture(e.pointerId);
  });
  textResize.addEventListener('pointermove', e => {
    if (!textDrag || !editing) return;
    const dx = (e.clientX - textDrag.x) / textDrag.w, dy = (e.clientY - textDrag.y) / textDrag.h;
    resizeText(textDrag.size * (1 + (Math.abs(dx) > Math.abs(dy) ? dx : dy)));
  });
  const finishTextResize = () => { if (!textDrag) return; textDrag = null; saveTextSize(); tbox.focus({ preventScroll: true }); };
  textResize.addEventListener('pointerup', finishTextResize);
  textResize.addEventListener('pointercancel', finishTextResize);
  textResize.addEventListener('lostpointercapture', finishTextResize);
  textResize.addEventListener('keydown', e => {
    if (!editing || !['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key)) return;
    e.preventDefault(); e.stopPropagation();
    resizeText(editing.size + (['ArrowUp', 'ArrowRight'].includes(e.key) ? 2 : -2)); saveTextSize();
  });

  // ───── 포인터 이벤트(필기) ─────
  const local = (e) => {
    const r = cv.getBoundingClientRect();
    return toSrc((e.clientX - r.left) * W / r.width, (e.clientY - r.top) * H / r.height);
  };
  function positionText() {
    if (!editing) return;
    const r = cv.getBoundingClientRect();
    const x = r.left + (editing.x - view.ox) * zf() * r.width / W;
    const y = r.top + (editing.y - view.oy) * zf() * r.height / H;
    textEditor.style.left = `${x}px`;
    textEditor.style.top = `${y}px`;
    textEditor.classList.toggle('actions-below', y < 40);
    tbox.style.maxWidth = `${Math.max(24, innerWidth - x - 4)}px`;
  }
  let pan = null, spaceFollowing = false;

  let lastPtr = { tab: false };

  // 보조키로 도형 결정: Ctrl+Shift = 화살표, Ctrl = 사각형, Shift = 선, Tab = 타원. 도구가 '도형'이면 고른 도형.
  const kindFor = (e) => {
    const c = e.ctrlKey, s = e.shiftKey, tab = lastPtr.tab;
    if (c && s) return 'arrow';
    if (c) return 'rect';
    if (s) return 'line';
    if (tab) return 'ellipse';
    return null;
  };
  const strokeWidth = (t) => (t === 'hl' ? Math.max(22, S.penWidth * 3) : S.penWidth) * dpr / zf();

  cv.addEventListener('pointerdown', (e) => {
    const [x, y] = local(e);
    if (e.button === 1 && view.active) { e.preventDefault(); commitText(); pan = [e.clientX, e.clientY]; cv.setPointerCapture(e.pointerId); return; }
    if (e.button === 2) { invoke('zoom_exit'); return; }
    if (e.button !== 0 || !frozen()) return;
    if (tool.id === 'text') { e.preventDefault(); openText(e); return; }
    cv.setPointerCapture(e.pointerId);
    emit('ink-owner', { m }); ownsInk = true;
    if (tool.id === 'eraser') { erasing = true; eraseAt.batch = []; eraseAt(x, y); return; }
    // 보조키가 있으면 보조키의 도형, 없으면 '도형' 도구에서 고른 도형, 둘 다 없으면 자유 곡선
    const kind = kindFor(e) || (tool.id === 'shape' ? tool.shape : 'free');
    const inkTool = tool.id === 'shape' ? (tool.hlShape ? 'hl' : 'pen') : tool.id;
    const base = { tool: inkTool, kind, color: tool.color, w: strokeWidth(inkTool) };
    cur = kind === 'free' ? { ...base, pts: [x, y] } : { ...base, a: [x, y], b: [x, y] };
    dirty = true;
  });
  cv.addEventListener('pointermove', (e) => {
    if (pan) { invoke('zoom_pan', { dx: (e.clientX - pan[0]) * dpr, dy: (e.clientY - pan[1]) * dpr }); pan = [e.clientX, e.clientY]; return; }
    const [x, y] = local(e);
    if (cur) {
      if (cur.kind === 'free') {
        for (const ev of (e.getCoalescedEvents?.() || [e])) cur.pts.push(...local(ev));
      } else cur.b = [x, y];
      dirty = true;
    } else if (erasing) eraseAt(x, y);
  });
  const endStroke = () => {
    pan = null;
    if (cur) {
      const degenerate = cur.kind !== 'free' && cur.kind !== 'line' && cur.kind !== 'arrow' && Math.hypot(cur.b[0] - cur.a[0], cur.b[1] - cur.a[1]) < 2;
      if (!degenerate) { strokes.push(cur); undoStack.push({ add: cur }); }
      cur = null; dirty = true;
    }
    if (erasing) { erasing = false; if (eraseAt.batch?.length) undoStack.push({ erase: eraseAt.batch }); eraseAt.batch = null; }
  };
  cv.addEventListener('pointerup', () => { endStroke(); invoke('raise_toolbar_cmd'); });
  cv.addEventListener('pointercancel', endStroke);
  cv.addEventListener('contextmenu', (e) => e.preventDefault());
  cv.addEventListener('wheel', (e) => {
    // Ctrl+휠 = 펜 굵기 (ZoomIt). 그냥 휠 = 확대/축소
    if (e.ctrlKey && frozen()) { e.preventDefault(); setPenWidth(S.penWidth + (e.deltaY < 0 ? 1 : -1)); return; }
    if (!view.active) return;
    e.preventDefault();
    invoke('zoom_wheel', { dir: e.deltaY < 0 ? 1 : -1, ladder: !!S.zoomLadder, step: S.zoomStep });
  }, { passive: false });

  listen('zoom-changed', ({ payload: p }) => {
    const mine = p.active && p.m === m;
    const entering = mine && !view.active;
    const leaving = !mine && view.active;
    view.active = mine; view.z = mine ? p.ze : 1;
    if (!mine) { view.ox = view.oy = 0; stopFollowing(); }
    syncInteractive();
    if (entering || leaving) invoke('focus_overlay', { index: m, on: mine || !!editing });
    positionText();
    dirty = true;
  });
  listen('zoom-view', ({ payload: p }) => {
    view.ox = p.ox; view.oy = p.oy;
    if (view.active && p.ze) view.z = p.ze; // 애니메이션 중에도 잉크가 화면과 같은 배율로 따라간다
    positionText();
    dirty = true;
  });

  // ───── 상태/잉크 명령 ─────
  await listen('state', ({ payload }) => {
    if (payload.tool !== 'text') commitText();
    tool.id = payload.tool; tool.color = payload.color;
    tool.shape = payload.shape || 'rect'; tool.hlShape = !!payload.hlShape; tool.board = payload.board && payload.boardMon === m ? payload.board : null;
    cv.style.cursor = tool.id === 'text' ? 'text' : 'default';
    dirty = true;
    syncInteractive(); applyHalo();
  });
  listen('ink-owner', ({ payload }) => { ownsInk = payload.m === m; });
  listen('ink', ({ payload }) => {
    if (payload.op === 'clear') {
      if (strokes.length) { undoStack.push({ clear: strokes.splice(0) }); dirty = true; }
    } else if (payload.op === 'undo' && ownsInk) {
      const op = undoStack.pop(); if (!op) return;
      if (op.add) { const i = strokes.indexOf(op.add); if (i >= 0) strokes.splice(i, 1); }
      else strokes.push(...(op.erase || op.clear));
      dirty = true;
    }
  });

  // ───── 스크린샷 (확대·잉크·보드 포함, 화면에 보이는 그대로) ─────
  async function composeShot() {
    // The shared screen already contains zoom and ink: do not apply them twice.
    return (await grab()).raw;
  }
  listen('shot', async ({ payload }) => {
    if (payload.m !== m) return;
    try {
      const c = await composeShot();
      if (payload.op === 'copy') {
        const px = c.getContext('2d').getImageData(0, 0, W, H).data;
        await invoke('copy_rgba', new Uint8Array(px.buffer), { headers: { 'x-w': String(W), 'x-h': String(H) } });
        toast('스크린샷을 클립보드에 복사했습니다');
      }
    } catch (err) { toast(`스크린샷 실패: ${err}`); }
  });
  listen('toast', ({ payload }) => { if (payload.m === m) toast(payload.msg); });

  // ───── 키 입력 → 툴바로 전달 ─────
  function stopFollowing() {
    if (!spaceFollowing) return;
    spaceFollowing = false;
    invoke('zoom_follow', { on: false });
  }
  addEventListener('keydown', (e) => {
    if (editing) return;
    if (view.active && e.code === 'Space') {
      e.preventDefault(); e.stopPropagation();
      if (!e.repeat && !spaceFollowing && !e.ctrlKey && !e.altKey && !e.metaKey) {
        spaceFollowing = true;
        invoke('zoom_follow', { on: true });
      }
      return;
    }
    emit('key', { key: e.key, ctrl: e.ctrlKey, shift: e.shiftKey });
  });
  addEventListener('keyup', e => {
    if (e.code !== 'Space') return;
    if (view.active && !editing) { e.preventDefault(); e.stopPropagation(); }
    stopFollowing();
  });
  addEventListener('blur', stopFollowing);
  cv.addEventListener('auxclick', e => e.preventDefault());

  // ───── 포인터 후광 + 클릭 파문 ─────
  let prev = { l: false, r: false };
  const penDown = () => frozen() && prev.l;
  function applyHalo() {
    halo.style.setProperty('--s', `${S.haloSize}px`);
    halo.style.setProperty('--ho', S.haloOpacity / 100);
    halo.style.setProperty('--hc', penDown() ? hexToRgb(tool.color) : S.haloColor);
    if (!S.haloOn) halo.classList.remove('on');
  }
  function ripple(x, y, rgb) {
    const r = document.createElement('i');
    r.className = 'ripple' + (S.reduceMotion ? ' calm' : '');
    r.style.cssText = `left:${x}px;top:${y}px;--rc:${rgb}`;
    document.body.appendChild(r);
    setTimeout(() => r.remove(), 520);
  }
  listen('ptr', ({ payload: p }) => {
    lastPtr = p;
    const lx = p.x - mon.x, ly = p.y - mon.y;
    const inside = lx >= 0 && ly >= 0 && lx < W && ly < H;
    const show = inside && S.haloOn;
    // 확대 화면은 항상 커서 아래 점이 커서 위치에 오도록 움직이므로, 후광은 실제 포인터 좌표에 그대로 그린다
    const cx = lx / dpr, cy = ly / dpr;
    if (inside) {
      halo.style.transform = `translate(${cx}px,${cy}px)`; halo.classList.toggle('on', show);
      penHud.style.transform = `translate(${cx}px,${cy}px)`;
      if (S.ripple) {
        if (p.l && !prev.l) ripple(cx, cy, S.rippleColor);
        if (p.r && !prev.r) ripple(cx, cy, S.rippleColor);
      }
    }
    prev = { l: p.l, r: p.r };
    halo.classList.toggle('on', show);
    halo.classList.toggle('down', show && (p.l || p.r));
    applyHalo();
  });

  addEventListener('storage', () => {
    S = loadSettings();
    applyHalo();
  });
  addEventListener('resize', fit);
  fit();
  await syncInteractive();
  emit('state-request');
})();
