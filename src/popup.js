const { invoke, Channel } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const pop = document.getElementById('pop');
let cur = 0; // id of the request on screen; late chunks from older ones are ignored
let fitQueued = false;
let lastH = 0;

// While the popup window is hidden the page is in the background, so timers and
// animation frames may be delayed a lot. Measure synchronously (layout still works)
// and send the size right away; a microtask only merges several calls in one turn.
function fit(force) {
  if (force === true) lastH = 0;
  if (fitQueued) return;
  fitQueued = true;
  queueMicrotask(() => {
    fitQueued = false;
    const h = Math.ceil(pop.getBoundingClientRect().height);
    if (h === lastH) return;
    lastH = h;
    invoke('popup_resize', { height: h });
  });
}
new ResizeObserver(() => fit()).observe(pop);

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}
const dots = () => { const d = el('span', 'load'); d.append(el('i'), el('i'), el('i')); return d; };

/** Run one translation; onText gets the whole text so far. */
function run(kind, text, seq, onText, engine = 'google') {
  const ch = new Channel();
  let acc = '';
  ch.onmessage = (chunk) => {
    if (seq !== cur) return;
    acc += chunk;
    onText(acc, false);
  };
  return invoke('translate', { text, kind, engine, onEvent: ch }).then((full) => {
    if (seq === cur) onText(full, true);
  });
}

function showError(e) {
  pop.className = 'pop';
  pop.replaceChildren();
  const row = el('div', 'err');
  row.append(el('span', 'dot'));
  if (e === 'nokey') {
    row.append(el('span', null, 'Nhập API key để dịch với Claude'));
    const b = el('button', 'lnk', 'Mở Cài đặt');
    b.onclick = () => invoke('open_settings', { tab: 'set' });
    row.append(b);
  } else {
    row.append(el('span', null, String(e || 'Có lỗi xảy ra')));
  }
  pop.append(row);
  fit(true);
}

/** Small "Dịch với Claude" link; inside an error line when no key is set. */
function claudeLink(label, onClick) {
  const b = el('button', 'lnk cl', label);
  b.title = 'Dịch lại bằng Claude: tự nhiên hơn, có phiên âm';
  b.onclick = onClick;
  return b;
}

function claudeMark() {
  return el('span', 'by', 'Bản dịch của Claude');
}

/** Footer row (bottom right) under the popup content. */
function footer(...kids) {
  const f = el('div', 'pfoot');
  f.append(...kids);
  return f;
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
  let engine = 'google';
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
  const toClaude = () => switchToClaude();
  const lineCl = claudeLink('Claude', toClaude);
  lineCl.classList.add('inline');
  lineCl.hidden = true;
  line.append(el('span', 'w', text), ipa, m, more, lineCl);
  const detail = el('div', 'detail');
  const wrap = el('div');
  const inner = el('div', 'in');
  const body = el('div');
  const foot = footer(claudeLink('Dịch với Claude', toClaude));
  inner.append(body, foot);
  wrap.append(inner);
  detail.append(wrap);
  pop.append(line, detail);
  fit(true);

  let loaded = false;
  const loadShort = () => {
    // Switching to Claude: keep the Google meaning (dimmed) until Claude answers.
    if (engine === 'claude' && m.textContent) line.classList.add('stale');
    else m.replaceChildren(dots());
    return run('short', text, seq, (t, done) => {
      line.classList.remove('stale');
      const r = parseShort(t);
      ipa.textContent = r.ipa;
      m.textContent = r.meaning;
      if (done) {
        more.hidden = false;
        lineCl.hidden = engine === 'claude';
      }
    }, engine).catch((e) => {
      if (seq !== cur) return;
      line.classList.remove('stale');
      if (e === 'nokey') showNoKey(); else showError(e);
    });
  };
  const loadDetail = () => {
    loaded = true;
    if (engine === 'claude' && body.textContent) body.classList.add('stale');
    else body.replaceChildren(dots());
    run('detail', text, seq, (t) => { body.classList.remove('stale'); renderDetail(body, t, text); }, engine).catch((e) => {
      loaded = false;
      body.classList.remove('stale');
      if (seq !== cur) return;
      if (e === 'nokey') showNoKey(); else body.replaceChildren(el('span', null, String(e)));
    });
  };
  function showNoKey() {
    // Keep the Google result, just say why Claude did not run.
    engine = 'google';
    lineCl.hidden = true;
    pop.classList.add('open');
    if (!loaded && !body.textContent) loadDetail();
    loaded = true;
    const b = el('button', 'lnk', 'Mở Cài đặt');
    b.onclick = () => invoke('open_settings', { tab: 'set' });
    foot.replaceChildren(el('span', 'by warn', 'Cần API key để dịch với Claude'), b);
  }
  function switchToClaude() {
    if (engine === 'claude') return;
    engine = 'claude';
    lineCl.hidden = true;
    foot.replaceChildren(claudeMark());
    loadShort();
    if (pop.classList.contains('open')) loadDetail();
    else loaded = false;
  }

  loadShort();
  more.onclick = () => {
    pop.classList.toggle('open');
    if (!loaded) loadDetail();
  };
}

// ---------- sentence ----------

function startSentence(text, seq) {
  pop.className = 'pop';
  pop.replaceChildren();
  const box = el('div', 'sent');
  box.append(dots());
  const foot = footer();
  pop.append(box, foot);
  fit(true);
  const go = (engine) => {
    foot.replaceChildren();
    if (engine === 'claude' && box.textContent) box.classList.add('stale');
    else box.replaceChildren(dots());
    run('sentence', text, seq, (t, done) => {
      box.classList.remove('stale');
      box.textContent = t;
      if (!done) box.append(el('span', 'caret'));
      else foot.replaceChildren(engine === 'claude' ? claudeMark() : claudeLink('Dịch với Claude', () => go('claude')));
    }, engine).catch((e) => {
      if (seq !== cur) return;
      box.classList.remove('stale');
      if (e === 'nokey') {
        const b = el('button', 'lnk', 'Mở Cài đặt');
        b.onclick = () => invoke('open_settings', { tab: 'set' });
        foot.replaceChildren(el('span', 'by warn', 'Cần API key để dịch với Claude'), b);
      } else showError(e);
    });
  };
  go('google');
}

// ---------- screen region (image) ----------

const IMG_ICON = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="M21 15l-5-5L5 21"/></svg>';

function startImage(seq, engine) {
  pop.className = 'pop';
  pop.replaceChildren();
  const wait = el('div', 'sent wait');
  wait.append(dots(), el('span', null, engine === 'claude' ? 'Claude đang đọc ảnh…' : 'Đang đọc chữ trong ảnh…'));
  pop.append(wait);
  fit(true);

  const ocr = el('div', 'ocr');
  ocr.innerHTML = IMG_ICON;
  const ocrText = el('span');
  ocr.append(ocrText);
  const box = el('div', 'sent');
  const foot = el('div', 'pfoot');
  const retry = claudeLink('Dịch với Claude', () => { cur += 0.001; startImage(cur, 'claude'); });
  retry.title = 'Gửi ảnh cho Claude đọc và dịch lại';
  foot.append(engine === 'claude' ? claudeMark() : retry);
  let shown = false;
  const show = () => {
    if (shown) return;
    shown = true;
    pop.replaceChildren(ocr, box, foot);
  };

  const ch = new Channel();
  let acc = '';
  ch.onmessage = (m) => {
    if (seq !== cur) return;
    if (m.t === 'ocr') {
      show();
      ocrText.textContent = m.text;
      box.replaceChildren(dots());
      return;
    }
    acc += m.text;
    show();
    if (engine === 'claude') {
      // Claude answers "original\n---\ntranslation".
      const i = acc.indexOf('\n---');
      if (i < 0) {
        ocrText.textContent = acc.trim();
        box.replaceChildren(dots());
      } else {
        ocrText.textContent = acc.slice(0, i).trim();
        box.textContent = acc.slice(i).replace(/^\n-+\s*/, '');
        box.append(el('span', 'caret'));
      }
    } else {
      box.textContent = acc;
    }
  };
  invoke('translate_image', { engine, onEvent: ch })
    .then(() => {
      if (seq !== cur) return;
      box.querySelector('.caret')?.remove();
      if (/^\(không có chữ\)/i.test(acc.trim())) showError('Không thấy chữ trong vùng này');
    })
    .catch((e) => {
      if (seq !== cur) return;
      if (e === 'notext') {
        showError('Không thấy chữ trong vùng này');
        // Still offer Claude, it reads stylised text better.
        if (engine !== 'claude') pop.append(foot);
      } else if (e === 'nokey' && engine === 'claude') {
        showError('nokey');
      } else showError(e);
    });
}

listen('mtrans://show', ({ payload }) => {
  cur = payload.seq;
  if (payload.mode === 'word') startWord(payload.text, payload.seq);
  else if (payload.mode === 'image') startImage(payload.seq, 'auto');
  else startSentence(payload.text, payload.seq);
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') invoke('popup_hide');
});
document.addEventListener('contextmenu', (e) => e.preventDefault());
