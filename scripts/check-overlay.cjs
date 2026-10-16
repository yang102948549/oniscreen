const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
(async () => {
  const events = {}, calls = [], globals = {};
  let frame; const drawn = [];
  const timers = [];
  const ctx = new Proxy({}, { get: (o, k) => o[k] || (() => {}) });
  ctx.fillText = (text, x, y) => drawn.push({ text, x, y, font: ctx.font });
  ctx.measureText = text => ({ width: text.length * 20 });
  const elements = {};
  function element(id) {
    return elements[id] ||= {
      style: { setProperty() {} }, classList: { add() {}, remove() {}, toggle() {} },
      handlers: {}, textContent: '', innerText: '',
      addEventListener(k, f) { this.handlers[k] = f; },
      getContext() { return ctx; },
      getBoundingClientRect() { return { left: 0, top: 0, width: 1280, height: 720 }; },
      setPointerCapture() {}, focus() {}, contains() { return false; },
    };
  }
  const sandbox = {
    T: { window: { getCurrentWindow: () => ({ label: 'overlay-0' }) } },
    invoke: async (name, args) => { calls.push([name, args]); return name === 'monitor_rect' ? { x: -1920, y: 0, w: 1920, h: 1080 } : undefined; },
    listen: (k, f) => { events[k] = f; }, emit() {},
    $: element, devicePixelRatio: 1.5, innerWidth: 1280,
    loadSettings: () => ({ textSize: 28, penWidth: 4, haloOn: false }),
    saveSettings() {},
    PEN_COLORS: [{ hex: '#292927' }], MIN_PEN: 2, MAX_PEN: 40,
    hexToRgb: () => '0,0,0', RIPPLE_RIGHT: '',
    requestAnimationFrame(f) { frame = f; }, performance, setTimeout: fn => { timers.push(fn); return timers.length; }, clearTimeout() {},
    addEventListener: (k, f) => { globals[k] = f; },
    document: { activeElement: null }, console,
  };
  vm.runInNewContext(fs.readFileSync('ui/overlay.js', 'utf8'), sandbox);
  await new Promise(setImmediate);
  assert(calls.some(([n, p]) => n === 'set_interactive' && !p.on), 'startup must explicitly enable click-through');
  events['zoom-changed']({ payload: { active: true, m: 0, ze: 2 } });
  events['zoom-view']({ payload: { ox: 100, oy: 50, ze: 2 } });
  assert(calls.some(([name, p]) => name === 'set_interactive' && p.on));
  let prevented = 0;
  const key = { code: 'Space', key: ' ', repeat: false, preventDefault() { prevented++; }, stopPropagation() {} };
  globals.keydown(key); globals.keydown({ ...key, repeat: true });
  assert.equal(prevented, 2);
  assert.equal(calls.filter(([n]) => n === 'zoom_follow').length, 1);
  globals.keyup(key);
  assert.equal(calls.filter(([n]) => n === 'zoom_follow').at(-1)[1].on, false);
  events.state({ payload: { tool: 'text', color: '#292927' } });
  const click = { clientX: 300, clientY: 200, button: 0, preventDefault() {} };
  await element('#ink').handlers.pointerdown(click);
  await new Promise(setImmediate);
  assert.equal(element('#textEditor').style.left, '300px');
  assert.equal(element('#textEditor').style.top, '200px');
  const actions = element('#textActions');
  const draftDrag = { ...click, pointerId: 7, stopPropagation() {}, target: { closest() { return null; } } };
  element('#tbox').innerText = '입력 중';
  actions.handlers.pointerdown(draftDrag);
  element('#tbox').handlers.blur();
  while (timers.length) timers.shift()();
  assert.equal(element('#textEditor').hidden, false, 'blur during a drag must not commit the draft');
  actions.handlers.pointermove({ clientX: 380, clientY: 240, pointerId: 7 });
  assert.equal(element('#textEditor').style.left, '380px', 'the header moves text while typing');
  assert.equal(element('#textEditor').style.top, '240px');
  assert.equal(element('#tbox').innerText, '입력 중', 'dragging must preserve the draft');
  actions.handlers.pointermove({ ...click, pointerId: 7 });
  actions.handlers.pointerup();
  actions.handlers.pointermove({ clientX: 700, clientY: 700, pointerId: 7 });
  assert.equal(element('#textEditor').style.left, '300px', 'movement must end on pointerup');
  const previous = calls.filter(([n]) => n === 'zoom_follow').length;
  globals.keydown(key);
  assert.equal(calls.filter(([n]) => n === 'zoom_follow').length, previous, 'Space must remain text while editing');
  const handle = element('#textResize');
  handle.handlers.pointerdown({ button: 0, pointerId: 2, clientX: 300, clientY: 200, preventDefault() {}, stopPropagation() {} });
  handle.handlers.pointermove({ clientX: 940, clientY: 560 });
  assert.equal(element('#tbox').style.fontSize, '42px');
  handle.handlers.pointerup();
  element('#textDelete').handlers.click();
  assert.equal(element('#textEditor').hidden, true);
  frame(); assert.equal(drawn.length, 0, 'deleted draft must not become ink');
  await element('#ink').handlers.pointerdown(click);
  await new Promise(setImmediate);
  element('#tbox').innerText = '크기 확인';
  handle.handlers.keydown({ key: 'ArrowUp', preventDefault() {}, stopPropagation() {} });
  assert.equal(element('#tbox').style.fontSize, '44px');
  element('#tbox').handlers.keydown({ key: 'Enter', preventDefault() {}, stopPropagation() {} });
  frame();
  assert.equal(drawn.at(-1).text, '크기 확인');
  assert.equal(drawn.at(-1).x, 325);
  assert.equal(drawn.at(-1).y, 200);
  assert(drawn.at(-1).font.includes('33px'), 'committed font must preserve resized display size at 150% DPI and 2x zoom');
  await element('#ink').handlers.pointerdown(click);
  await new Promise(setImmediate);
  assert.equal(element('#tbox').textContent, '크기 확인', 'clicking existing text selects it for editing');
  const move = element('#textMove');
  move.handlers.pointerdown({ ...click, pointerId: 3, stopPropagation() {} });
  move.handlers.pointermove({ clientX: 380, clientY: 240, pointerId: 3 });
  move.handlers.pointerup();
  assert.equal(element('#textEditor').style.left, '380px');
  assert.equal(element('#textEditor').style.top, '240px');
  element('#tbox').handlers.keydown({ key: 'Enter', preventDefault() {}, stopPropagation() {} });
  frame();
  assert.equal(drawn.at(-1).x, 385, 'moving 80 CSS px at 150% DPI and 2x zoom moves 60 source px');
  assert.equal(drawn.at(-1).y, 230);
  events.ink({ payload: { op: 'undo' } }); frame();
  assert.equal(drawn.at(-1).x, 325, 'undo restores original text position');
  assert.equal(drawn.at(-1).y, 200);

  await element('#ink').handlers.pointerdown(click);
  await new Promise(setImmediate);
  events.ink({ payload: { op: 'clear' } });
  assert.equal(element('#textEditor').hidden, true, 'clear must close and clear the active text editor');
  const clearedCount = drawn.length; frame();
  assert.equal(drawn.length, clearedCount, 'clear must remove the text currently being edited');
  events.ink({ payload: { op: 'undo' } }); frame();
  assert.equal(drawn.at(-1).text, '크기 확인', 'undo clear restores the edited text');

  globals.keydown(key);
  assert.equal(calls.filter(([n]) => n === 'zoom_follow').at(-1)[1].on, true);
  globals.blur();
  assert.equal(calls.filter(([n]) => n === 'zoom_follow').at(-1)[1].on, false, 'losing focus must stop tracking');
  element('#ink').handlers.pointerdown({ ...click, button: 1, pointerId: 1 });
  element('#ink').handlers.pointermove({ clientX: 320, clientY: 210 });
  const pan = calls.find(([n]) => n === 'zoom_pan')[1];
  assert.equal(pan.dx, 30); assert.equal(pan.dy, 15);
  globals.keydown(key);
  events['zoom-changed']({ payload: { active: false, m: 0, ze: 1 } });
  assert.equal(calls.filter(([n]) => n === 'zoom_follow').at(-1)[1].on, false, 'zoom exit must stop tracking');
  const beforeIdleSpace = calls.length;
  globals.keydown(key);
  assert.equal(calls.length, beforeIdleSpace, 'Space outside zoom must not start tracking');
  assert(calls.some(([n, p]) => n === 'focus_overlay' && !p.on));
  events.state({ payload: { tool: null, board: null, color: '#292927' } });
  await new Promise(setImmediate);
  assert.equal(calls.filter(([n]) => n === 'set_interactive').at(-1)[1].on, false);
  // A pending drawing-mode IPC must finish before the pointer-mode IPC.
  let releaseInput;
  const originalInvoke = sandbox.invoke;
  sandbox.invoke = (name, args) => {
    if (name === 'set_interactive' && args.on) {
      calls.push([name, args]);
      return new Promise(resolve => { releaseInput = resolve; });
    }
    return originalInvoke(name, args);
  };
  events.state({ payload: { tool: 'pen', color: '#292927' } });
  events.state({ payload: { tool: null, color: '#292927' } });
  releaseInput();
  await new Promise(setImmediate);
  assert.equal(calls.filter(([n]) => n === 'set_interactive').at(-1)[1].on, false, 'rapid return to pointer must restore click-through');
  console.log('Overlay checks passed: DPI coordinates, focus, Space hold/release/repeat/blur/text, pan, text resize/delete/commit.');
})().catch(e => { console.error(e); process.exitCode = 1; });
