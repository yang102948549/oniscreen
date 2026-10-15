let S = loadSettings();
const persist = () => {
  const ok = saveSettings(S);
  $('#saveStatus').textContent = ok ? '저장됨 · 다음 실행에도 유지' : '저장 실패 · 다시 시도해 주세요';
  return ok;
};
const set = (patch) => { S = { ...loadSettings(), ...patch }; persist(); paint(); };
$('#save').onclick = persist;

const hc = $('#hcolors'), rc = $('#rcolors');
for (const [container, setting] of [[hc, 'haloColor'], [rc, 'rippleColor']]) {
  HALO_COLORS.forEach(c => {
    const b = document.createElement('button');
    b.className = 'dot'; b.setAttribute('role', 'radio'); b.setAttribute('aria-label', c.name);
    b.style.setProperty('--c', `rgb(${c.rgb})`); b.innerHTML = '<i></i>';
    b.onclick = () => set({ [setting]: c.rgb });
    container.appendChild(b);
  });
}

const pc = $('#pcolors'), tc = $('#tcolors');
for (const [container, setting] of [[pc, 'penColor'], [tc, 'textColor']]) {
  PEN_COLORS.forEach(c => {
    const b = document.createElement('button');
    b.className = 'dot'; b.setAttribute('role', 'radio'); b.setAttribute('aria-label', c.name); b.title = c.name;
    b.style.setProperty('--c', c.hex); b.innerHTML = '<i></i>';
    b.onclick = () => set({ [setting]: c.hex });
    container.appendChild(b);
  });
}

const zt = $('#ztarget');
(async () => {
  zt.add(new Option('자동 (작업 중인 화면)', '-1'));
  for (let i = 0; ; i++) {
    const r = await invoke('monitor_rect', { index: i });
    if (!r) break;
    zt.add(new Option(`모니터 ${i + 1} (${r.w}×${r.h})`, String(i)));
  }
  zt.value = String(S.zoomTarget);
})();

// 단축키 편집: 칸을 누르면 다음 키 조합을 받는다
const keysEl = $('#keys');
let capturing = null;
// 입력을 받는 동안은 전역 단축키를 잠시 풀어, 누른 조합이 실제 동작(확대 등)을 일으키지 않게 한다
const suspendKeys = () => invoke('set_hotkeys', { on: false, keys: [] });
const resumeKeys = () => invoke('set_hotkeys', { on: S.hotkeys, keys: hotkeyList(S.keys) });
Object.keys(KEY_LABELS).forEach((act) => {
  const row = document.createElement('div'); row.className = 'krow';
  const lab = document.createElement('label'); lab.textContent = KEY_LABELS[act];
  const btn = document.createElement('button'); btn.className = 'kbtn'; btn.dataset.act = act;
  const reset = document.createElement('button'); reset.className = 'btn kreset'; reset.setAttribute('aria-label', `${KEY_LABELS[act]} 기본값`);
  reset.innerHTML = '<svg viewBox="0 0 24 24"><path d="M4 12a8 8 0 1 0 2.5-5.8M4 4v4h4"/></svg>';
  reset.onclick = () => set({ keys: { ...S.keys, [act]: DEFAULT_KEYS[act] } });
  btn.onclick = () => { capturing = act; suspendKeys(); paint(); btn.focus(); };
  btn.onblur = () => { if (capturing === act) { capturing = null; resumeKeys(); paint(); } };
  btn.onkeydown = (e) => {
    if (capturing !== act) return;
    e.preventDefault();
    if (e.key === 'Escape') { capturing = null; resumeKeys(); paint(); return; }
    const combo = comboFromEvent(e);
    if (!combo) return; // 수정키만 눌렀거나 수정키 없이 누른 경우는 계속 기다린다
    capturing = null;
    const taken = Object.entries(S.keys).find(([a2, c]) => a2 !== act && c === combo);
    set({ keys: { ...S.keys, ...(taken ? { [taken[0]]: '' } : {}), [act]: combo } }); // 겹치면 이전 항목을 비운다
    resumeKeys();
  };
  row.append(lab, btn, reset); keysEl.appendChild(row);
});

function paint() {
  $$('.kbtn', keysEl).forEach((b) => { b.classList.toggle('rec', capturing === b.dataset.act); b.textContent = capturing === b.dataset.act ? '키를 누르세요…' : keyLabel(S.keys[b.dataset.act]); });
  zt.value = String(S.zoomTarget);
  $$('.dot', tc).forEach((d, i) => d.setAttribute('aria-checked', String(PEN_COLORS[i].hex === S.textColor)));
  $('#hk').checked = S.hotkeys;
  $('#haloOn').checked = S.haloOn;
  $('#ripple').checked = S.ripple;
  $$('.dot', rc).forEach((d, i) => d.setAttribute('aria-checked', String(HALO_COLORS[i].rgb === S.rippleColor)));
  $('#reduce').checked = S.reduceMotion;
  $('#size').value = S.haloSize; $('#sizeV').textContent = `${S.haloSize}px`;
  $('#opac').value = S.haloOpacity; $('#opacV').textContent = `${S.haloOpacity}%`;
  $('#zdef').value = S.zoomDefault; $('#zdefV').textContent = `${S.zoomDefault}×`;
  $('#zladder').checked = !!S.zoomLadder;
  $('#drawView').value = S.drawView;
  $('#zstep').value = S.zoomStep; $('#zstepV').textContent = `${S.zoomStep}×`;
  $('#ts').value = S.textSize; $('#tsV').textContent = `${S.textSize}px`;
  $('#pw').value = S.penWidth; $('#pwV').textContent = `${S.penWidth}px`;
  $$('.dot', pc).forEach((d, i) => d.setAttribute('aria-checked', String(PEN_COLORS[i].hex === S.penColor)));
  $$('.dot', hc).forEach((d, i) => d.setAttribute('aria-checked', String(HALO_COLORS[i].rgb === S.haloColor)));
}

$('#haloOn').onchange = (e) => set({ haloOn: e.target.checked });
$('#ripple').onchange = (e) => set({ ripple: e.target.checked });
$('#reduce').onchange = (e) => set({ reduceMotion: e.target.checked });
$('#size').oninput = (e) => set({ haloSize: +e.target.value });
$('#opac').oninput = (e) => set({ haloOpacity: +e.target.value });
$('#zdef').oninput = (e) => set({ zoomDefault: +e.target.value });
$('#drawView').onchange = (e) => set({ drawView: e.target.value });
$('#zladder').onchange = (e) => set({ zoomLadder: e.target.checked });
$('#zstep').oninput = (e) => set({ zoomStep: +e.target.value });
$('#ts').oninput = (e) => set({ textSize: +e.target.value });
zt.onchange = () => set({ zoomTarget: +zt.value });
$('#hk').onchange = (e) => set({ hotkeys: e.target.checked });
$('#pw').oninput = (e) => set({ penWidth: +e.target.value });
$('#quit').onclick = () => invoke('quit');
addEventListener('storage', () => { S = loadSettings(); paint(); });
paint();

T.app.getVersion().then(version => { $('#currentVersion').textContent = `현재 버전 ${version}`; }).catch(() => { $('#currentVersion').textContent = '버전 확인 불가'; });

$('#checkUpdate').onclick = async () => {
  const button = $('#checkUpdate'), status = $('#updateStatus');
  button.disabled = true; $('#downloadUpdate').hidden = true;
  status.textContent = '최신 버전 확인 중…';
  try {
    const result = await invoke('check_update');
    status.textContent = result.available ? `새 버전 ${result.latest}을 받을 수 있습니다.` : '최신 버전입니다.';
    $('#currentVersion').textContent = `현재 버전 ${result.current}`;
    $('#downloadUpdate').hidden = !result.available;
  } catch (error) { status.textContent = `확인하지 못했습니다 · ${error}`; }
  finally { button.disabled = false; }
};
$('#downloadUpdate').onclick = async () => {
  const button = $('#downloadUpdate'); button.disabled = true;
  try {
    await invoke('download_update');
    $('#updateStatus').textContent = '브라우저에서 설치 파일을 내려받은 뒤 실행해 주세요.';
  } catch (error) { $('#updateStatus').textContent = `다운로드를 열지 못했습니다 · ${error}`; }
  finally { button.disabled = false; }
};
