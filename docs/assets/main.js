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

// Hero motion: split the title into characters so each can bounce in (see the
// Hero motion block in styles.css), hop a character on hover, and spin the
// wordmark wisps on hover. i18n.js replaces the title's markup when the
// language changes, so the split reruns and the entrance replays.
(() => {
  const title = document.querySelector('.hero h1');
  if (!title) return;
  const motion = matchMedia('(prefers-reduced-motion: no-preference)');

  const split = () => {
    if (title.querySelector('.hero-char')) return;
    title.classList.remove('is-split', 'is-settled');
    title.setAttribute('aria-label', title.innerText.replace(/\s+/g, ' ').trim());
    let index = 0;
    // Words stay unbreakable so English never wraps mid-word; Chinese lines
    // are already set by the <br>.
    const wrap = (text) => {
      const out = document.createDocumentFragment();
      text.split(/(\s+)/).forEach((part) => {
        if (!part) return;
        if (/^\s+$/.test(part)) {
          out.append(part);
          return;
        }
        const word = document.createElement('span');
        word.className = 'hero-word';
        word.setAttribute('aria-hidden', 'true');
        for (const ch of part) {
          const span = document.createElement('span');
          span.className = 'hero-char';
          span.style.setProperty('--i', index++);
          span.textContent = ch;
          word.append(span);
        }
        out.append(word);
      });
      return out;
    };
    [...title.childNodes].forEach((node) => {
      if (node.nodeType === Node.TEXT_NODE) node.replaceWith(wrap(node.textContent));
    });
    title.classList.add('is-split');
  };

  split();
  new MutationObserver(split).observe(title, { childList: true });

  // Hops only start once the entrance has finished, so removing .is-hopping
  // never re-triggers the entrance bounce.
  title.addEventListener('animationend', (event) => {
    if (event.animationName === 'hero-char-hop') {
      event.target.classList.remove('is-hopping');
      return;
    }
    const chars = title.querySelectorAll('.hero-char');
    if (event.animationName === 'hero-char-bounce' && event.target === chars[chars.length - 1]) {
      title.classList.add('is-settled');
    }
  });
  title.addEventListener('pointerover', (event) => {
    const ch = event.target.closest?.('.hero-char');
    if (!ch || !motion.matches || !title.classList.contains('is-settled')) return;
    ch.classList.add('is-hopping');
  });

  // Hovering or clicking a swirl (the wordmark's p wisps, the nav logo) spins
  // it. The spin is additive, so it layers on top of the CSS orbit and never
  // snaps the wisps back to their resting angle; each click adds another turn.
  const spin = (parts, turns) => {
    if (!motion.matches || typeof Element.prototype.animate !== 'function') return;
    parts.forEach((part, n) => part.animate(
      [{ transform: 'rotate(0deg)' }, { transform: `rotate(${(n === 1 ? -360 : 360) * turns}deg)` }],
      { duration: 1000 + turns * 300 + n * 150, easing: 'cubic-bezier(0.34, 1.3, 0.64, 1)', composite: 'add' },
    ));
  };

  const wordmark = document.querySelector('.hero-wordmark');
  const wisps = wordmark ? [...wordmark.querySelectorAll('.wm-orbit')] : [];
  let hoverSpin = 0;
  wordmark?.addEventListener('pointerenter', () => {
    if (performance.now() < hoverSpin) return;
    hoverSpin = performance.now() + 1500;
    spin(wisps, 1);
  });
  wordmark?.addEventListener('click', () => spin(wisps, 2));

  const navLogo = document.querySelector('.site-nav .brand img');
  navLogo?.addEventListener('click', () => spin([navLogo], 2));
})();
