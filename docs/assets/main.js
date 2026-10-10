document.querySelectorAll('.tab-btn').forEach((btn) => {
  btn.addEventListener('click', () => {
    const id = btn.dataset.tab;
    document.querySelectorAll('.tab-btn').forEach((b) => b.classList.remove('active'));
    document.querySelectorAll('.tab-panel').forEach((p) => p.classList.remove('active'));
    btn.classList.add('active');
    document.getElementById(id)?.classList.add('active');
  });
});

document.querySelectorAll('.faq-q').forEach((btn) => {
  btn.addEventListener('click', () => {
    const item = btn.closest('.faq-item');
    const open = item.classList.contains('open');
    document.querySelectorAll('.faq-item').forEach((i) => i.classList.remove('open'));
    if (!open) item.classList.add('open');
  });
});

// Agent loop event stream: reveal one event at a time while the section is on
// screen, moving the runner to that event's stop on the ring. Without this
// script (or with reduced motion) the section stays in its finished state.
(() => {
  const stage = document.querySelector('[data-engine]');
  if (!stage || !('IntersectionObserver' in window)) return;
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;

  const rows = [...stage.querySelectorAll('.ev')];
  const toggle = stage.querySelector('.engine-toggle');
  const STOP_ANGLE = { read: 0, think: 90, act: 180, verify: 270 };
  const STEP_MS = 1400;
  const HOLD_MS = 4500;
  const RESET_MS = 900;
  let step = 0;
  let cycle = 0;
  let timer = 0;
  let onScreen = false;
  let paused = false;

  // step -1 clears the stream between runs; the runner stays where it finished.
  const show = (index) => {
    step = index;
    rows.forEach((row, n) => {
      row.classList.toggle('is-on', n <= index);
      row.classList.toggle('is-now', n === index);
      row.classList.toggle('is-past', n < index);
    });
    const { stop = 'read', lap = '1' } = rows[index]?.dataset ?? {};
    stage.dataset.lap = lap;
    stage.dataset.stop = stop;
    // Angles only ever grow, so the runner never spins backwards on restart.
    const angle = lap === 'done' ? 720 : (Number(lap) - 1) * 360 + STOP_ANGLE[stop];
    stage.style.setProperty('--engine-angle', `${cycle * 720 + angle}deg`);
  };

  const advance = () => {
    if (step === rows.length - 1) {
      cycle += 1;
      show(-1);
      timer = setTimeout(advance, RESET_MS);
      return;
    }
    show(step + 1);
    timer = setTimeout(advance, step === rows.length - 1 ? HOLD_MS : STEP_MS);
  };

  const sync = () => {
    clearTimeout(timer);
    if (onScreen && !paused) timer = setTimeout(advance, STEP_MS);
  };

  stage.classList.add('is-live');
  show(0);
  toggle.hidden = false;
  toggle.addEventListener('click', () => {
    paused = !paused;
    stage.classList.toggle('is-paused', paused);
    sync();
  });
  new IntersectionObserver(([entry]) => {
    onScreen = entry.isIntersecting;
    sync();
  }, { threshold: 0.3 }).observe(stage);
})();
