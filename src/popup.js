const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const pop = document.getElementById('pop');
let cur = 0; // id of the request on screen; late chunks from older ones are ignored
let fitQueued = false;

function fit() {
  if (fitQueued) return;
  fitQueued = true;
  requestAnimationFrame(() => {
    fitQueued = false;
    invoke('popup_resize', { height: Math.ceil(pop.getBoundingClientRect().height) });
  });
}
new ResizeObserver(fit).observe(pop);

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}
const dots = () => { const d = el('span', 'load'); d.append(el('i'), el('i'), el('i')); return d; };

/** Run one translation; onText gets the whole text so far. */
function run(kind, text, seq, onText) {
  const ch = new Channel();
  let acc = '';
  ch.onmessage = (chunk) => {
    if (seq !== cur) return;
    acc += chunk;
    onText(acc, false);
  };
  return invoke('translate', { text, kind, onEvent: ch }).then((full) => {
    if (seq === cur) onText(full, true);
  });
}

function showError(e) {
  pop.className = 'pop';
  pop.replaceChildren();
  const row = el('div', 'err');
  row.append(el('span', 'dot'));
  if (e === 'nokey') {
    row.append(el('span', null, 'Chưa có API key'));
    const b = el('button', 'lnk', 'Mở Cài đặt');
    b.onclick = () => invoke('open_settings', { tab: 'set' });
    row.append(b);
  } else {
    row.append(el('span', null, String(e || 'Có lỗi xảy ra')));
  }
  pop.append(row);
  fit();
}

// ---------- single word ----------

function parseShort(t) {
  const line = t.trim().split('\n')[0];
  const i = line.indexOf('|');
  if (i < 0) return { ipa: '', meaning: line.trim() };
  return { ipa: line.slice(0, i).trim(), meaning: line.slice(i + 1).trim() };
}

function highlight(sentence, word) {
  const frag = document.createDocumentFragment();
  const lower = sentence.toLowerCase(), w = word.toLowerCase();
  let i = 0, j;
  while (w && (j = lower.indexOf(w, i)) >= 0) {
    frag.append(sentence.slice(i, j), el('b', null, sentence.slice(j, j + w.length)));
    i = j + w.length;
  }
  frag.append(sentence.slice(i));
  return frag;
}

function renderDetail(box, text, word) {
  box.replaceChildren();
  let ol = null, first = true, ex = null;
  for (const raw of text.split('\n')) {
    const l = raw.trim();
    if (!l) continue;
    if (l.startsWith('#')) {
      const pos = el('div', 'pos');
      pos.append(el('span', 'chip', l.replace(/^#+\s*/, '')));
      ol = el('ol');
      pos.append(ol);
      box.append(pos);
    } else if (/^[-*•]/.test(l)) {
      if (!ol) { const pos = el('div', 'pos'); ol = el('ol'); pos.append(ol); box.append(pos); }
      const li = el('li');
      const txt = l.replace(/^[-*•]\s*/, '');
      li.append(first ? el('b', null, txt) : txt);
      first = false;
      ol.append(li);
    } else if (/^EN\s*:/i.test(l)) {
      ex = el('div', 'ex');
      const en = el('div', 'en');
      en.append(highlight(l.replace(/^EN\s*:\s*/i, ''), word));
      ex.append(en);
      box.append(ex);
    } else if (/^VI\s*:/i.test(l)) {
      if (!ex) { ex = el('div', 'ex'); box.append(ex); }
      ex.append(el('div', 'vi', l.replace(/^VI\s*:\s*/i, '')));
    }
  }
}

function startWord(text, seq) {
  pop.className = 'pop';
  pop.replaceChildren();
  const line = el('div', 'line');
  const ipa = el('span', 'ipa');
  const m = el('span', 'm');
  m.append(dots());
  const more = el('button', 'more');
  more.title = 'Xem thêm nghĩa và ví dụ';
  more.innerHTML = 'More<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><path d="M6 9l6 6 6-6"/></svg>';
  more.hidden = true;
  line.append(el('span', 'w', text), ipa, m, more);
  const detail = el('div', 'detail');
  const wrap = el('div');
  const inner = el('div', 'in');
  wrap.append(inner);
  detail.append(wrap);
  pop.append(line, detail);
  fit();

  run('short', text, seq, (t, done) => {
    const r = parseShort(t);
    ipa.textContent = r.ipa;
    m.textContent = r.meaning;
    if (done) more.hidden = false;
  }).catch((e) => seq === cur && showError(e));

  let loaded = false;
  more.onclick = () => {
    pop.classList.toggle('open');
    if (loaded) return;
    loaded = true;
    inner.replaceChildren(dots());
    run('detail', text, seq, (t) => renderDetail(inner, t, text)).catch((e) => {
      loaded = false;
      inner.replaceChildren(el('span', null, e === 'nokey' ? 'Chưa có API key' : String(e)));
    });
  };
}

// ---------- sentence ----------

function startSentence(text, seq) {
  pop.className = 'pop';
  pop.replaceChildren();
  const box = el('div', 'sent');
  box.append(dots());
  pop.append(box);
  fit();
  run('sentence', text, seq, (t, done) => {
    box.textContent = t;
    if (!done) box.append(el('span', 'caret'));
  }).catch((e) => seq === cur && showError(e));
}

listen('mtrans://show', ({ payload }) => {
  cur = payload.seq;
  if (payload.mode === 'word') startWord(payload.text, payload.seq);
  else startSentence(payload.text, payload.seq);
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') invoke('popup_hide');
});
document.addEventListener('contextmenu', (e) => e.preventDefault());
