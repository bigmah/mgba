// Pulls frames from the Rust emulation thread (`/emu/frame`) and blits them
// onto <canvas id="gba-screen">. Only new frames are transferred: the request
// carries the last sequence number we drew and Rust answers 204 if unchanged.
window.__gba = (() => {
  let running = false;
  let inflight = false;
  let seq = 0;
  let ctx = null;
  let img = null;
  let ticks = 0;
  let drawn = 0;

  function fit() {
    const canvas = document.getElementById('gba-screen');
    const stage = document.getElementById('gba-stage');
    if (!canvas || !stage) return;
    const w = canvas.width || 240, h = canvas.height || 160;
    let scale = Math.min(stage.clientWidth / w, stage.clientHeight / h);
    if (canvas.dataset.integer === 'true' && scale >= 1) scale = Math.floor(scale);
    canvas.style.width = `${Math.round(w * scale)}px`;
    canvas.style.height = `${Math.round(h * scale)}px`;
    const overlay = document.getElementById('gba-overlay');
    if (overlay) {
      overlay.style.width = canvas.style.width;
      overlay.style.height = canvas.style.height;
      overlay.style.setProperty('--px', `${scale}px`);
    }
  }

  let lastTick = 0;
  function tick() {
    if (!running) return;
    ticks++;
    lastTick = performance.now();
    requestAnimationFrame(tick);
    pull();
  }
  // WebKit can stop firing rAF (e.g. occluded window); keep frames flowing anyway.
  setInterval(() => {
    if (running && performance.now() - lastTick > 100) pull();
  }, 16);

  function pull() {
    if (inflight) return;
    const canvas = document.getElementById('gba-screen');
    if (!canvas) return;
    inflight = true;
    fetch(`/emu/frame?s=${seq}`, { cache: 'no-store' })
      .then((r) => (r.status === 200 ? r.arrayBuffer() : null))
      .then((buf) => {
        inflight = false;
        if (!buf || buf.byteLength < 16) return;
        const dv = new DataView(buf);
        const w = dv.getUint32(0, true);
        const h = dv.getUint32(4, true);
        seq = Number(dv.getBigUint64(8, true));
        if (canvas.width !== w || canvas.height !== h) {
          canvas.width = w;
          canvas.height = h;
          img = null;
          fit();
        }
        if (!ctx || ctx.canvas !== canvas) {
          ctx = canvas.getContext('2d', { alpha: false });
          img = null;
          fit();
        }
        if (!img) img = ctx.createImageData(w, h);
        img.data.set(new Uint8ClampedArray(buf, 16, w * h * 4));
        ctx.putImageData(img, 0, 0);
        drawn++;
      })
      .catch(() => {
        inflight = false;
      });
  }

  window.addEventListener('resize', fit);

  return {
    start() {
      seq = 0;
      ctx = null;
      if (!running) {
        running = true;
        requestAnimationFrame(tick);
      }
      setTimeout(fit, 0);
      const root = document.getElementById('player-root');
      if (root) root.focus();
    },
    stop() {
      running = false;
    },
    fit,
    debug() {
      return { running, seq, ticks, drawn, inflight };
    },
    focus() {
      const root = document.getElementById('player-root');
      if (root) root.focus();
    },
  };
})();

// Library helpers
window.__lib = {
  jump(letter) {
    const el = document.getElementById(`letter-${letter}`);
    if (el) el.scrollIntoView({ behavior: 'smooth', block: 'start' });
  },
  focusSearch() {
    const el = document.getElementById('search');
    if (el) { el.focus(); el.select(); }
  },
  top() {
    const el = document.getElementById('library-scroll');
    if (el) el.scrollTo({ top: 0, behavior: 'smooth' });
  },
};
