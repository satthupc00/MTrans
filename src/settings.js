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
  $('hotkey').classList.toggle('off', !S.enabled);
  $('hotkey').querySelectorAll('button').forEach((b) => b.classList.toggle('on', b.dataset.v === S.hotkey));
  $('model').value = S.model;
  $('source').querySelectorAll('button').forEach((b) => b.classList.toggle('on', b.dataset.v === S.source));
  $('sourceHint').textContent = {
    mix: 'Google cho bản dịch nhanh, Claude khi bấm More. Khuyên dùng.',
    google: 'Chỉ dùng Google: nhanh, miễn phí, không cần API key, không có phiên âm.',
    claude: 'Chỉ dùng Claude: dịch tự nhiên nhất, có phiên âm, chậm hơn và tốn credits.',
  }[S.source];
  document.querySelectorAll('.claudeOnly').forEach((g) => g.classList.toggle('dim', S.source === 'google'));
  $('capture').checked = S.capture_enabled;
  $('autostart').checked = S.autostart;
  $('ver').textContent = 'v' + S.version;
  $('key').placeholder = S.has_key ? `Đã lưu (${S.key_hint}) · dán key mới để thay` : 'Dán API key vào đây';
  if (!$('key').value) setHint(S.has_key ? '✓ Đã có API key' : 'Chưa có API key. Xem tab Trợ giúp để lấy key.', S.has_key ? 'ok' : '');
  document.querySelectorAll('kbd:not(.fixed)').forEach((k) => (k.textContent = S.hotkey === 'alt' ? 'Alt' : 'Ctrl'));
}

$('enabled').onchange = (e) => invoke('set_enabled', { on: e.target.checked }).then(load);
$('hotkey').querySelectorAll('button').forEach((b) => (b.onclick = () => invoke('set_hotkey', { hotkey: b.dataset.v }).then(load)));
$('source').querySelectorAll('button').forEach((b) => (b.onclick = () => invoke('set_source', { source: b.dataset.v }).then(load)));
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
  if (e.key !== 'Escape') return;
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
