const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
(async () => {
const native = [];
const data = new Map();
const elements = new Map();
const element = id => {
  if (!elements.has(id)) {
    const classes = new Set();
    elements.set(id, {
    style: { setProperty() {} }, classList: { toggle(name, on) { if (on ?? !classes.has(name)) classes.add(name); else classes.delete(name); }, contains(name) { return classes.has(name); } }, dataset: {},
    setAttribute(name, value) { this[name] = value; }, addEventListener() {}, contains() { return false; },
    getAttribute(name) { return this[name] || '도구'; },
    replaceChildren(...children) { this.children = children; },
    getBoundingClientRect() { return { width: 800, bottom: 80 }; },
    querySelector(selector) { return selector === 'svg' ? { cloneNode() { return { tagName: 'svg' }; } } : null; }
  });
  }
  return elements.get(id);
};
const emitted = [];
const events = {};
const sandbox = {
  window: { __TAURI__: { core: { invoke: async (name, args) => { native.push([name, args]); } }, event: { emit: (name, payload) => emitted.push([name, payload]), listen(name, fn) { events[name] = fn; } } } },
  document: { querySelector: element, querySelectorAll: () => [] },
  localStorage: { getItem: k => data.get(k), setItem: (k, v) => data.set(k, v) },
  addEventListener() {}, ResizeObserver: class { observe() {} }, console,
};
vm.createContext(sandbox);
const run = code => vm.runInContext(code, sandbox);
run(fs.readFileSync('ui/shared.js', 'utf8'));
run(fs.readFileSync('ui/toolbar.js', 'utf8'));
// Ribbon visibility must survive repeated zoom/settings renders.
run("$('#fold').onclick()");
for (let i = 0; i < 50; i++) run('render()');
assert.equal(run('toolbarFolded'), true);
assert(element('#bar').classList.contains('folded'));
assert(element('#stack').classList.contains('folded'));
run("$('#fold').onclick()");
for (let i = 0; i < 50; i++) run('render()');
assert.equal(run('toolbarFolded'), false);
assert(!element('#stack').classList.contains('folded'));
run("$('#fold').onclick(); $('#restore').onclick()");
assert.equal(run('toolbarFolded'), false, 'the sole collapsed tool restores the toolbar');
events.hotkey({ payload: 'zout' });
assert.equal(native.at(-1)[0], 'zoom_step');
assert(native.at(-1)[1].d < 0, 'Ctrl+Down must request a negative zoom step');
run('st.zoomActive = true; st.editing = true; syncKeys();');
assert.equal(native.at(-1)[1].editing, true, 'native live zoom must be disabled during text editing');
run('st.zoomActive = false; st.editing = false;');
await run('resizeToolbar()');
await Promise.resolve();
const resizeCount = native.filter(([name]) => name === 'resize_toolbar').length;
await Promise.all(Array.from({ length: 50 }, () => run('resizeToolbar()')));
assert.equal(native.filter(([name]) => name === 'resize_toolbar').length, resizeCount, 'unchanged geometry must not resize native window again');
run("setTool('pen'); pickColor('#C43F35'); setTool('text'); pickColor('#2383E2');");
assert.equal(run('loadSettings().penColor'), '#C43F35');
assert.equal(run('loadSettings().textColor'), '#2383E2');
run("setTool('pen')");
assert.equal(emitted.filter(([n]) => n === 'state').at(-1)[1].color, '#C43F35');
run("setTool('text')");
assert.equal(emitted.filter(([n]) => n === 'state').at(-1)[1].color, '#2383E2');
await element('#zoomFixed').onclick();
assert(native.some(([name]) => name === 'begin_zoom'));
run('st.zoomActive = true; st.z = 3;');
await run('zoomIn()');
assert.equal(native.at(-1)[0], 'zoom_step');
data.set('oniscreen-settings', JSON.stringify({ ver: 2, penColor: '#20AD96', keys: { pen: 'ctrl+alt+p' } }));
assert.equal(run('loadSettings().textColor'), '#20AD96');
assert.equal(run('loadSettings().keys.pen'), 'ctrl+alt+p');
assert.equal(run("loadSettings().keys['zoom-mode']"), undefined);
data.set('oniscreen-settings', JSON.stringify({ ver: 3, zoomMode: 'live', holdKey: 'space', keys: { 'zoom-mode': 'ctrl+shift+1' } }));
assert.equal(run("loadSettings().keys['zoom-mode']"), undefined);
assert.equal(run('loadSettings().zoomMode'), undefined);
assert.equal(run('loadSettings().holdKey'), undefined);
data.set('oniscreen-settings', JSON.stringify({ ver: 3, keys: { record: 'ctrl+9', 'shot-save': 'ctrl+alt+s' } }));
assert.equal(run("loadSettings().keys.record"), undefined);
assert.equal(run("loadSettings().keys['shot-save']"), undefined);
assert(!run("hotkeyList(loadSettings().keys)").some(([, action]) => ['record', 'shot-save'].includes(action)));
data.set('oniscreen-settings', JSON.stringify({ ver: 3, haloColor: '239,120,108', keys: { zout: 'ctrl+9' } }));
assert.equal(run('loadSettings().keys.zout'), undefined);
assert.equal(run('loadSettings().rippleColor'), '239,120,108');
run("settings = loadSettings(); settings.rippleColor = '235,166,64'; saveSettings(settings); st.tool = null; pickColor('35,131,226');");
assert.equal(run('loadSettings().haloColor'), '35,131,226');
assert.equal(run('loadSettings().rippleColor'), '235,166,64');
assert.equal(run('st.tool'), null, 'pointer palette must not activate pen');
run("setTool('text'); togglePointer()");
assert.equal(run('st.tool'), null);
assert.equal(run('settings.haloOn'), true);
run("st.board = 'white'; st.zoomActive = true; togglePointer()");
assert.equal(run('st.board'), null);
assert(native.some(([n]) => n === 'zoom_exit'));
run("$('#pointerOpacity').oninput({ target: { value: '45' } }); $('#pointerSize').oninput({ target: { value: '60' } });");
assert.equal(run('loadSettings().haloOpacity'), 45);
assert.equal(run('loadSettings().haloSize'), 60);
sandbox.localStorage.setItem = () => { throw Error('unavailable'); };
assert.equal(run('saveSettings(loadSettings())'), false);
console.log('Settings checks passed: independent colors, tool switching, zoom controls, legacy zoom/hold removal, migration, save failure.');

})().catch(e => { console.error(e); process.exitCode = 1; });
