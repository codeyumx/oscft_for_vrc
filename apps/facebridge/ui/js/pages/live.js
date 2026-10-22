// @ts-check
// Live: eye gaze / openness and all expression weights, fed by `frame` events (~20 Hz) while this page is open.

import { api } from '../api.js';
import { h, toast, errMsg } from '../dom.js';
import { expressionNames, lastFrame, onFrame, state } from '../store.js';

/** @typedef {import('../types.js').EyeFrame} EyeFrame */
/** @typedef {import('../types.js').FrameView} FrameView */

const SVG_NS = 'http://www.w3.org/2000/svg';
const GROUPS = ['Eye', 'Brow', 'Nose', 'Cheek', 'Jaw', 'Mouth', 'Lip', 'Tongue'];
/** A weight at or below this counts as "inactive" for the filter. */
const ACTIVE_THRESHOLD = 0.01;
/** With the filter on, a shape stays shown this long after its last active frame, then fades out. */
const HOLD_MS = 5000;
/** Must match the `.shape-row` opacity transition in styles.css. */
const FADE_MS = 800;

/** @param {number} v @param {number} lo @param {number} hi */
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

let svgUid = 0;

/**
 * @param {string} tag
 * @param {Record<string, string | number>} attrs
 * @param {...SVGElement} children
 */
function svg(tag, attrs, ...children) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  el.append(...children);
  return el;
}

/** One eye: lids open by `openness`, iris follows gaze. @param {string} title */
function createEyeWidget(title) {
  const clipId = `eye-clip-${svgUid++}`;
  const lens = svg('ellipse', { cx: 50, cy: 30, rx: 40, ry: 22 });
  const clip = svg('clipPath', { id: clipId }, lens);
  const iris = svg('circle', { class: 'iris', cx: 50, cy: 30, r: 13 });
  const pupil = svg('circle', { class: 'pupil', cx: 50, cy: 30, r: 5 });
  const sclera = svg('ellipse', { class: 'sclera', cx: 50, cy: 30, rx: 40, ry: 22 });
  const lid = svg('ellipse', { class: 'lid', cx: 50, cy: 30, rx: 40, ry: 22 });
  const clipped = svg('g', { 'clip-path': `url(#${clipId})` }, iris, pupil);
  const root = svg('svg', { class: 'eye-svg', viewBox: '0 0 100 60', role: 'img', 'aria-label': title }, svg('defs', {}, clip), sclera, clipped, lid);

  const readout = h('dl', { class: 'kv' });
  const gaze = h('dd', { class: 'mono' });
  const open = h('dd', { class: 'mono' });
  const pupilMm = h('dd', { class: 'mono' });
  readout.append(h('dt', null, 'Gaze'), gaze, h('dt', null, 'Openness'), open, h('dt', null, 'Pupil'), pupilMm);

  const el = h('section', { class: 'card', 'aria-label': title }, h('h2', null, title), /** @type {any} */ (root), readout);

  return {
    el,
    /** @param {EyeFrame} e */
    update(e) {
      // Eyelid opening shrinks both the clip window and the lid outline.
      const ry = Math.max(1.2, 22 * clamp(e.openness, 0, 1));
      for (const node of [lens, sclera, lid]) node.setAttribute('ry', String(ry));
      const gx = clamp(e.gazeX, -1, 1);
      const gy = clamp(e.gazeY, -1, 1);
      const cx = 50 + gx * 22;
      const cy = 30 - gy * 10;
      for (const node of [iris, pupil]) {
        node.setAttribute('cx', cx.toFixed(2));
        node.setAttribute('cy', cy.toFixed(2));
      }
      gaze.textContent = `x ${e.gazeX.toFixed(2)}, y ${e.gazeY.toFixed(2)}`;
      open.textContent = `${Math.round(clamp(e.openness, 0, 1) * 100)}%`;
      pupilMm.textContent = e.pupilDiameterMm > 0 ? `${e.pupilDiameterMm.toFixed(1)} mm` : '\u2013';
    },
  };
}

/** @param {string} name */
function groupOf(name) {
  return GROUPS.find((g) => name.startsWith(g)) ?? 'Other';
}

/**
 * @param {HTMLElement} root
 * @returns {import('../main.js').PageHandle}
 */
export function mountLive(root) {
  const left = createEyeWidget('Left eye');
  const right = createEyeWidget('Right eye');
  const status = h('div', { class: 'banner info', role: 'status' });
  const onlyActive = h('input', { type: 'checkbox', id: 'only-active' });
  const filter = h('label', { class: 'switch', for: 'only-active' }, onlyActive, h('span', { class: 'track', 'aria-hidden': 'true' }), h('span', null, 'Only active shapes'));
  const sourceSelect = h('select', { id: 'live-source', 'aria-label': 'Frame source' }, h('option', { value: 'output' }, 'Output'), h('option', { value: 'raw' }, 'Raw'));
  const head = h('dd', { class: 'mono' });
  const shapesHost = h('div', { class: 'section' });

  /** @type {{ row: HTMLElement, fill: HTMLElement, bar: HTMLElement, val: HTMLElement, lastActive: number }[]} */
  let rows = [];
  let builtFor = 0;

  function buildShapes() {
    builtFor = expressionNames.length;
    rows = [];
    /** @type {Map<string, HTMLElement>} */
    const groups = new Map();
    expressionNames.forEach((name, index) => {
      const g = groupOf(name);
      let grid = groups.get(g);
      if (!grid) {
        grid = h('div', { class: 'shape-grid' });
        groups.set(g, grid);
      }
      const fill = h('i');
      const bar = h('div', { class: 'bar', role: 'presentation' }, fill);
      const val = h('span', { class: 'val' }, '0.00');
      const row = h('div', { class: 'shape-row inactive', 'data-index': index }, h('span', { class: 'name', title: name }, name), bar, val);
      grid.append(row);
      rows[index] = { row, fill, bar, val, lastActive: -Infinity };
    });
    const order = [...GROUPS, 'Other'].filter((g) => groups.has(g));
    shapesHost.replaceChildren(
      ...order.map((g) => h('div', { class: 'shape-group' }, h('h3', null, g), /** @type {HTMLElement} */ (groups.get(g)))),
    );
  }

  /** @param {FrameView} f */
  function render(f) {
    left.update(f.left);
    right.update(f.right);
    head.textContent = `yaw ${f.head.yaw.toFixed(2)}, pitch ${f.head.pitch.toFixed(2)}, roll ${f.head.roll.toFixed(2)}`;
    const hideInactive = onlyActive.checked;
    const now = performance.now();
    const n = Math.min(rows.length, f.shapes.length);
    for (let i = 0; i < n; i++) {
      const w = f.shapes[i];
      const r = rows[i];
      if (!r) continue;
      const active = Math.abs(w) > ACTIVE_THRESHOLD;
      if (active) r.lastActive = now;
      r.fill.style.setProperty('--v', String(clamp(Math.abs(w), 0, 1)));
      r.bar.classList.toggle('neg', w < 0);
      r.val.textContent = w.toFixed(2);
      r.row.classList.toggle('inactive', !active);
      const idle = now - r.lastActive;
      r.row.classList.toggle('fading', hideInactive && idle >= HOLD_MS);
      r.row.hidden = hideInactive && idle >= HOLD_MS + FADE_MS;
    }
  }

  function updateStatus() {
    const modules = state.snapshot?.modules ?? [];
    const eye = modules.find((m) => m.claims.eye);
    const face = modules.find((m) => m.claims.expression);
    if (state.phase !== 'running') {
      status.textContent = 'The engine is not running, so there is no tracking data.';
    } else if (!eye && !face) {
      status.textContent = 'No module is providing tracking data yet. Enable a module on the Modules page.';
    } else {
      status.textContent = `Eye tracking: ${eye ? eye.name : 'no module'}. Face tracking: ${face ? face.name : 'no module'}.`;
    }
  }

  sourceSelect.addEventListener('change', () => {
    const raw = sourceSelect.value === 'raw';
    api
      .setLiveRaw(raw)
      .then(() => (raw ? api.latestRawFrame() : api.latestFrame()))
      .then(render)
      .catch((e) => toast(errMsg(e), 'error'));
  });

  onlyActive.addEventListener('change', () => {
    if (lastFrame) render(lastFrame);
  });

  root.append(
    h(
      'div',
      { class: 'page' },
      h('div', { class: 'page-head' }, h('div', null, h('h1', null, 'Live'), h('p', { class: 'sub' }, 'Output is what FaceBridge sends to VRChat; Raw is the merged module data before the Tracking adjustments.')), h('label', { class: 'live-source' }, h('span', { class: 'muted small' }, 'Show '), sourceSelect)),
      status,
      h('div', { class: 'eyes' }, left.el, right.el),
      h('section', { class: 'card', 'aria-label': 'Head pose' }, h('h2', null, 'Head'), h('dl', { class: 'kv' }, h('dt', null, 'Rotation'), head)),
      h('section', { class: 'section', 'aria-labelledby': 'shapes-h' }, h('div', { class: 'card-head' }, h('h2', { id: 'shapes-h' }, 'Expressions'), filter), shapesHost),
    ),
  );

  // Frames are pushed by the backend only while the page is open and the window visible.
  const stop = onFrame(render);
  api.setLiveRaw(false).catch(() => {});
  api.setLive(true).catch((e) => toast(errMsg(e), 'error'));
  api
    .latestFrame()
    .then((f) => {
      if (expressionNames.length !== builtFor) buildShapes();
      render(f);
    })
    .catch(() => {});

  return {
    update() {
      if (expressionNames.length !== builtFor) buildShapes();
      updateStatus();
      if (lastFrame) render(lastFrame);
    },
    destroy() {
      stop();
      api.setLive(false).catch(() => {});
    },
  };
}
