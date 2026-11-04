const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);

// ---------- tabs ----------
const tabs = $('tabs'), ind = $('ind');
function placeInd(t) { ind.style.left = t.offsetLeft + 'px'; ind.style.width = t.offsetWidth + 'px'; }
function showTab(name) {
  const t = tabs.querySelector(`.tab[data-p="${name}"]`) || tabs.querySelector('.tab');
  tabs.querySelectorAll('.tab').forEach((x) => x.classList.toggle('on', x === t));
  document.querySelectorAll('.pane').forEach((p) => p.classList.toggle('on', p.id === 'p-' + t.dataset.p));
  placeInd(t);
}
tabs.querySelectorAll('.tab').forEach((t) => (t.onclick = () => showTab(t.dataset.p)));
requestAnimationFrame(() => placeInd(tabs.querySelector('.tab.on')));

// ---------- toast ----------
let toastTimer;
function toast(msg) {
  const t = $('toast');
  t.textContent = msg;
  t.classList.add('on');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove('on'), 2600);
}

// ---------- settings ----------
let S = null;
function setHint(text, cls) {
  const h = $('keyHint');
  h.textContent = text || ' ';
  h.className = 'hint' + (cls ? ' ' + cls : '');
}
async function load() {
  S = await invoke('get_settings');
  $('enabled').checked = S.enabled;
  showKey('text', S.text_key, !S.enabled);
  showKey('capture', S.capture_key, !S.capture_enabled);
  $('model').value = S.model;
  $('capture').checked = S.capture_enabled;
  $('autostart').checked = S.autostart;
  $('ver').textContent = 'v' + S.version;
  $('key').placeholder = S.has_key ? `Đã lưu (${S.key_hint}) · dán key mới để thay` : 'Dán API key vào đây';
  if (!$('key').value) setHint(S.has_key ? '✓ Đã có API key, nút Dịch với Claude đã sẵn sàng' : 'Chưa có key: đang dịch bằng Google. Cách lấy key ở tab Trợ giúp.', S.has_key ? 'ok' : '');
  document.querySelectorAll('kbd.k-text').forEach((k) => (k.textContent = S.text_key.label));
  document.querySelectorAll('kbd.k-capture').forEach((k) => (k.textContent = S.capture_key.label));
}

$('enabled').onchange = (e) => invoke('set_enabled', { on: e.target.checked }).then(load);
// ---------- shortcuts ----------
// Physical key (KeyboardEvent.code) -> Windows virtual-key code.
const CODE_VK = (() => {
  const m = {
    Space: 0x20, Insert: 0x2d, Home: 0x24, End: 0x23, PageUp: 0x21, PageDown: 0x22, Pause: 0x13, ScrollLock: 0x91,
    Semicolon: 0xba, Equal: 0xbb, Comma: 0xbc, Minus: 0xbd, Period: 0xbe, Slash: 0xbf, Backquote: 0xc0,
    BracketLeft: 0xdb, Backslash: 0xdc, BracketRight: 0xdd, Quote: 0xde,
  };
  for (let i = 0; i < 26; i++) m['Key' + String.fromCharCode(65 + i)] = 0x41 + i;
  for (let i = 0; i < 10; i++) { m['Digit' + i] = 0x30 + i; m['Numpad' + i] = 0x60 + i; }
  for (let i = 1; i <= 24; i++) m['F' + i] = 0x6f + i;
  return m;
})();
const VK_NAME = (vk) => {
  if ((vk >= 0x30 && vk <= 0x39) || (vk >= 0x41 && vk <= 0x5a)) return String.fromCharCode(vk);
  if (vk >= 0x60 && vk <= 0x69) return 'Num ' + (vk - 0x60);
  if (vk >= 0x70 && vk <= 0x87) return 'F' + (vk - 0x6f);
  const n = { 0x20: 'Space', 0x2d: 'Insert', 0x24: 'Home', 0x23: 'End', 0x21: 'PageUp', 0x22: 'PageDown', 0x13: 'Pause', 0x91: 'ScrollLock',
    0xba: ';', 0xbb: '=', 0xbc: ',', 0xbd: '-', 0xbe: '.', 0xbf: '/', 0xc0: '`', 0xdb: '[', 0xdc: '\\', 0xdd: ']', 0xde: "'" };
  return n[vk] || '?';
};
const rowOf = (which) => document.querySelector(`.keyrow[data-which="${which}"]`);
const keys = { text: { mods: 1, vk: 0 }, capture: { mods: 4, vk: 0 } };

function showKey(which, k, off) {
  keys[which] = { mods: k.mods, vk: k.vk };
  const row = rowOf(which);
  row.classList.toggle('off', !!off);
  row.querySelector('.kmods').value = String(k.mods);
  const box = row.querySelector('.kbox');
  if (box.classList.contains('rec')) return;
  box.classList.toggle('empty', !k.vk);
  row.querySelector('.kv').textContent = k.vk ? VK_NAME(k.vk) : '× 2';
}

function keyHint(which, text, cls) {
  const h = rowOf(which).nextElementSibling;
  h.textContent = text || '';
  h.className = 'hint khint' + (cls ? ' ' + cls : '');
}

async function applyKey(which, mods, vk) {
  try {
    const warn = await invoke('set_hotkey', { which, mods, vk });
    keyHint(which, warn ? warn : '✓ Đã lưu', warn ? 'warn' : 'ok');
  } catch (e) {
    keyHint(which, String(e), 'bad');
  }
  await load();
}

function stopRec(box) {
  if (!box.classList.contains('rec')) return;
  box.classList.remove('rec');
  invoke('set_hooks_paused', { on: false });
}

document.querySelectorAll('.keyrow').forEach((row) => {
  const which = row.dataset.which;
  const sel = row.querySelector('.kmods');
  const box = row.querySelector('.kbox');
  sel.onchange = () => applyKey(which, Number(sel.value), keys[which].vk);
  box.onclick = (e) => {
    if (e.target.classList.contains('kx')) {
      stopRec(box);
      applyKey(which, keys[which].mods, 0);
      return;
    }
    if (box.classList.contains('rec')) return;
    box.classList.add('rec');
    row.querySelector('.kv').textContent = 'Nhấn phím…';
    keyHint(which, 'Nhấn phím muốn dùng. Esc để huỷ, Backspace để xoá.');
    invoke('set_hooks_paused', { on: true });
  };
  box.onblur = () => { if (box.classList.contains('rec')) { stopRec(box); keyHint(which, ''); load(); } };
  box.addEventListener('keydown', (e) => {
    if (!box.classList.contains('rec')) return;
    e.preventDefault();
    e.stopPropagation();
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) return; // pick these in the dropdown
    stopRec(box);
    if (e.key === 'Escape') { keyHint(which, ''); load(); return; }
    if (e.key === 'Backspace' || e.key === 'Delete') { applyKey(which, keys[which].mods, 0); return; }
    const vk = CODE_VK[e.code];
    if (!vk) { keyHint(which, 'Phím này không dùng được, hãy chọn phím khác', 'bad'); load(); return; }
    applyKey(which, keys[which].mods, vk);
  });
});
$('capture').onchange = (e) => invoke('set_capture_enabled', { on: e.target.checked }).then(load);
$('model').onchange = (e) => invoke('set_model', { model: e.target.value }).then(() => toast('Đã đổi model'));
$('autostart').onchange = (e) =>
  invoke('set_autostart', { on: e.target.checked }).catch((err) => { toast('Không đổi được: ' + err); load(); });

$('eye').onclick = () => { const k = $('key'); k.type = k.type === 'password' ? 'text' : 'password'; };
async function saveKey() {
  const btn = $('saveKey');
  const key = $('key').value.trim();
  btn.disabled = true;
  setHint('Đang kiểm tra…');
  try {
    if (key) {
      await invoke('save_api_key', { key });
      $('key').value = '';
      $('key').type = 'password';
    } else {
      await invoke('test_saved_key');
    }
    await load();
    setHint('✓ Key hoạt động', 'ok');
  } catch (e) {
    setHint(String(e), 'bad');
  } finally {
    btn.disabled = false;
  }
}
$('saveKey').onclick = saveKey;
$('key').addEventListener('keydown', (e) => { if (e.key === 'Enter') saveKey(); });

// ---------- updates ----------
let checking = false;
function showUpdate(info) {
  const upd = $('upd');
  upd.classList.remove('spin', 'ok', 'has');
  $('latest').hidden = true;
  $('newv').hidden = true;
  $('doUpdate').hidden = true;
  if (info.has_update) {
    upd.classList.add('has');
    upd.title = `Kiểm tra cập nhật · Có bản mới v${info.latest}`;
    $('newv').textContent = `Có bản mới v${info.latest}`;
    $('newv').hidden = false;
    $('doUpdate').hidden = false;
  } else {
    upd.classList.add('ok');
    upd.title = 'Kiểm tra cập nhật · Đã là bản mới nhất';
    $('latest').hidden = false;
  }
}
async function checkUpdate(manual) {
  if (checking) return;
  checking = true;
  const upd = $('upd');
  upd.classList.remove('ok', 'has');
  upd.classList.add('spin');
  try {
    showUpdate(await invoke('check_update'));
  } catch (e) {
    upd.classList.remove('spin');
    upd.title = 'Kiểm tra cập nhật';
    if (manual) toast(String(e));
  } finally {
    checking = false;
  }
}
$('upd').onclick = () => checkUpdate(true);
$('doUpdate').onclick = async () => {
  const b = $('doUpdate');
  b.disabled = true;
  b.textContent = 'Đang tải…';
  try {
    await invoke('install_update');
  } catch (e) {
    toast(String(e));
    b.disabled = false;
    b.textContent = 'Cập nhật';
  }
};

// ---------- what's new ----------
async function openWhatsNew() {
  const body = $('wnBody');
  body.replaceChildren();
  try {
    const wn = await (await fetch('whatsnew.json')).json();
    const d = wn.date ? wn.date.split('-').reverse().join('/') : '';
    $('wnVer').textContent = `v${wn.version}${d ? ' · ' + d : ''}`;
    const box = document.createElement('div');
    box.className = 'wn';
    for (const [k, title] of [['new', 'Mới'], ['improved', 'Cải thiện'], ['fixed', 'Sửa lỗi']]) {
      if (!wn[k] || !wn[k].length) continue;
      const h = document.createElement('h5');
      h.textContent = title;
      const ul = document.createElement('ul');
      for (const line of wn[k]) { const li = document.createElement('li'); li.textContent = line; ul.append(li); }
      box.append(h, ul);
    }
    body.append(box);
  } catch {
    body.textContent = 'Không đọc được nội dung.';
  }
  $('modal').hidden = false;
}
function closeModal() {
  const m = $('modal');
  if (m.hidden || m.classList.contains('closing')) return;
  m.classList.add('closing');
  setTimeout(() => { m.hidden = true; m.classList.remove('closing'); }, 160);
}
$('whatsnew').onclick = openWhatsNew;
$('wnClose').onclick = closeModal;
$('modal').onclick = (e) => { if (e.target === $('modal')) closeModal(); };

// ---------- window ----------
$('close').onclick = () => invoke('settings_hide');
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape' || document.querySelector('.kbox.rec')) return;
  if (!$('modal').hidden) closeModal();
  else invoke('settings_hide');
});
document.addEventListener('contextmenu', (e) => { if (e.target.tagName !== 'INPUT') e.preventDefault(); });

listen('mtrans://tab', ({ payload }) => { showTab(payload); load(); });
listen('mtrans://settings-changed', load);
listen('mtrans://update', ({ payload }) => showUpdate(payload));
listen('mtrans://check-update', () => checkUpdate(true));
window.addEventListener('focus', load);

load();
