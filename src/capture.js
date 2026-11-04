const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dim = document.getElementById('dim');
const sel = document.getElementById('sel');
const size = document.getElementById('size');
const hint = document.getElementById('hint');

let start = null; // drag start in CSS px
let busy = false;

function reset() {
  start = null;
  busy = false;
  sel.style.display = 'none';
  size.style.display = 'none';
  dim.classList.remove('has');
  hint.style.display = '';
}

function rect(e) {
  const x = Math.min(start.x, e.clientX), y = Math.min(start.y, e.clientY);
  return { x, y, w: Math.abs(e.clientX - start.x), h: Math.abs(e.clientY - start.y) };
}

function draw(r) {
  Object.assign(sel.style, { display: 'block', left: r.x + 'px', top: r.y + 'px', width: r.w + 'px', height: r.h + 'px' });
  const dpr = window.devicePixelRatio || 1;
  size.textContent = `${Math.round(r.w * dpr)} × ${Math.round(r.h * dpr)}`;
  size.style.display = 'block';
  size.style.left = r.x + 'px';
  size.style.top = Math.max(2, r.y - 22) + 'px';
}

function cancel() {
  reset();
  invoke('capture_cancel');
}

window.addEventListener('pointerdown', (e) => {
  if (busy) return;
  if (e.button === 2) { cancel(); return; }
  if (e.button !== 0) return;
  start = { x: e.clientX, y: e.clientY };
  document.body.setPointerCapture?.(e.pointerId);
  dim.classList.add('has');
  hint.style.display = 'none';
  draw({ x: start.x, y: start.y, w: 0, h: 0 });
});

window.addEventListener('pointermove', (e) => {
  if (start && !busy) draw(rect(e));
});

window.addEventListener('pointerup', (e) => {
  if (!start || busy || e.button !== 0) return;
  const r = rect(e);
  if (r.w < 6 || r.h < 6) { reset(); return; } // a click, not a drag: keep picking
  busy = true;
  invoke('capture_region', r).finally(() => setTimeout(reset, 200));
});

window.addEventListener('keydown', (e) => { if (e.key === 'Escape') cancel(); });
window.addEventListener('contextmenu', (e) => e.preventDefault());
listen('mtrans://capture-start', reset);
