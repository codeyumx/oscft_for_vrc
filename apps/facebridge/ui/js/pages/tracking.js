// @ts-check
// Tracking: calibration, unified correctors, parameter adjustment and data filter (see spec/mutation.md).
// Controls are built once; edits are debounced, sent with `set_mutation_config`, and the sanitized result is
// written back into the controls.

import { api } from '../api.js';
import { confirmButton, errMsg, h, runButton, toast } from '../dom.js';
import { state } from '../store.js';

/** @typedef {import('../types.js').MutationConfig} MutationConfig */
/** @typedef {import('../types.js').CalibrationData} CalibrationData */
/** @typedef {import('../types.js').AdjustGroupInfo} AdjustGroupInfo */
/** @typedef {import('../types.js').FilterChannel} FilterChannel */

const DEBOUNCE_MS = 150;
const CALIBRATION_REFRESH_MS = 1000;
const CUTOFF_MIN = 0.01;
const CUTOFF_MAX = 30;
const CUTOFF_STEPS = 1000;

/** @type {{ key: 'eyeGaze'|'eyeOpenness'|'pupil'|'expressions'|'head', label: string }[]} */
const FILTER_CATEGORIES = [
  { key: 'eyeGaze', label: 'Eye gaze' },
  { key: 'eyeOpenness', label: 'Eye openness' },
  { key: 'pupil', label: 'Pupil' },
  { key: 'expressions', label: 'Expressions' },
  { key: 'head', label: 'Head' },
];

let uid = 0;

/** @param {string} label @param {(on: boolean) => void} onChange */
function switchControl(label, onChange) {
  const id = `trk-sw-${uid++}`;
  const input = h('input', { type: 'checkbox', id });
  input.addEventListener('change', () => onChange(input.checked));
  const el = h('label', { class: 'switch', for: id }, input, h('span', { class: 'track', 'aria-hidden': 'true' }), h('span', null, label));
  return { el, input };
}

/**
 * Slider with a numeric readout.
 * @param {{ label: string, min: number, max: number, step: number, format: (v: number) => string, toSlider?: (v: number) => number, fromSlider?: (s: number) => number, onInput: (v: number) => void }} o
 */
function sliderControl(o) {
  const toSlider = o.toSlider ?? ((/** @type {number} */ v) => v);
  const fromSlider = o.fromSlider ?? ((/** @type {number} */ s) => s);
  const input = h('input', { type: 'range', min: o.min, max: o.max, step: o.step, 'aria-label': o.label });
  const out = h('output', { class: 'mono trk-readout' });
  input.addEventListener('input', () => {
    const v = fromSlider(input.valueAsNumber);
    out.textContent = o.format(v);
    o.onInput(v);
  });
  return {
    el: h('div', { class: 'control' }, input, out),
    /** @param {number} v */
    set(v) {
      input.value = String(toSlider(v));
      out.textContent = o.format(v);
    },
  };
}

/** @param {number} v */
const pct = (v) => `${Math.round(v * 100)}%`;
/** @param {number} v */
const fix2 = (v) => v.toFixed(2);

/** @param {string} id @param {string} title @param {...Node} body */
function card(id, title, ...body) {
  return h('section', { class: 'card', 'aria-labelledby': id }, h('div', { class: 'card-head' }, h('h2', { id }, title)), ...body);
}

/**
 * @param {HTMLElement} root
 * @returns {import('../main.js').PageHandle}
 */
export function mountTracking(root) {
  /** @type {MutationConfig | null} */
  let cfg = null;
  /** @type {AdjustGroupInfo[]} */
  let groups = [];
  /** @type {number | undefined} */
  let saveTimer;
  /** An edit not yet sent; while set, a response does not overwrite the controls. */
  let dirty = false;
  let destroyed = false;

  const banner = h('div', { class: 'banner info', role: 'status', hidden: true });
  /** @type {(() => void)[]} */
  const writers = [];

  /** @param {(c: MutationConfig) => void} edit */
  function change(edit) {
    if (!cfg) return;
    edit(cfg);
    dirty = true;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => void save(), DEBOUNCE_MS);
  }

  async function save() {
    if (!cfg || destroyed) return;
    dirty = false;
    try {
      const sanitized = await api.setMutationConfig(cfg);
      if (destroyed) return;
      if (!dirty) {
        cfg = sanitized;
        render();
      }
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  }

  // ---- master + calibration ----
  const master = switchControl('Apply tracking adjustments', (on) => change((c) => (c.enabled = on)));
  writers.push(() => (master.input.checked = !!cfg?.enabled));

  const calEnabled = switchControl('Calibrate shapes', (on) => change((c) => (c.calibration.enabled = on)));
  const calContinuous = switchControl('Keep learning while I use the face', (on) => change((c) => (c.calibration.continuous = on)));
  const calBlend = sliderControl({ label: 'Calibration strength', min: 0, max: 1, step: 0.01, format: pct, onInput: (v) => change((c) => (c.calibration.blend = v)) });
  writers.push(() => {
    if (!cfg) return;
    calEnabled.input.checked = cfg.calibration.enabled;
    calContinuous.input.checked = cfg.calibration.continuous;
    calBlend.set(cfg.calibration.blend);
  });

  const calProgress = h('div', { class: 'bar', role: 'progressbar', 'aria-label': 'Overall calibration progress', 'aria-valuemin': 0, 'aria-valuemax': 100 }, h('i'));
  const calProgressText = h('span', { class: 'mono' });
  const calList = h('div', { class: 'trk-shape-list' });
  const calDetails = h('details', { class: 'settings' }, h('summary', null, 'Per-shape calibration'), calList);
  const calReset = confirmButton('Reset calibration', 'Confirm reset', async () => {
    await api.resetCalibration(null);
    toast('Calibration reset.', 'ok');
    await refreshCalibration();
  });

  /** @type {Map<string, { row: HTMLElement, peak: HTMLElement, bar: HTMLElement, fill: HTMLElement }>} */
  const calRows = new Map();

  /** @param {CalibrationData} data */
  function renderCalibration(data) {
    const known = data.shapes.filter((s) => s.peak > 0);
    const mean = known.length ? known.reduce((a, s) => a + s.progress, 0) / known.length : 0;
    calProgress.firstElementChild?.setAttribute('style', `--v:${mean}`);
    calProgress.setAttribute('aria-valuenow', String(Math.round(mean * 100)));
    calProgressText.textContent = known.length ? `${pct(mean)} over ${known.length} shapes` : 'Nothing learned yet';
    if (calRows.size !== data.shapes.length) {
      calRows.clear();
      calList.replaceChildren();
      for (const s of data.shapes) {
        const peak = h('span', { class: 'mono trk-readout' });
        const fill = h('i');
        const bar = h('div', { class: 'bar', role: 'progressbar', 'aria-label': `${s.name} calibration progress`, 'aria-valuemin': 0, 'aria-valuemax': 100 }, fill);
        const reset = h('button', { type: 'button', class: 'small', 'aria-label': `Reset ${s.name}` }, 'Reset');
        reset.addEventListener('click', () => runButton(reset, async () => {
          await api.resetCalibration(s.name);
          await refreshCalibration();
        }));
        const row = h('div', { class: 'trk-row' }, h('span', { class: 'name', title: s.name }, s.name), peak, bar, reset);
        calRows.set(s.name, { row, peak, bar, fill });
        calList.append(row);
      }
    }
    for (const s of data.shapes) {
      const r = calRows.get(s.name);
      if (!r) continue;
      r.peak.textContent = `peak ${s.peak.toFixed(2)}`;
      r.fill.style.setProperty('--v', String(s.progress));
      r.bar.setAttribute('aria-valuenow', String(Math.round(s.progress * 100)));
      r.row.classList.toggle('inactive', s.peak <= 0);
    }
  }

  async function refreshCalibration() {
    try {
      const data = await api.getCalibration();
      if (!destroyed) renderCalibration(data);
    } catch {
      /* the engine may be down; the next tick retries */
    }
  }

  // ---- correctors ----
  const fixMouth = switchControl('Mouth closed cannot exceed jaw open', (on) => change((c) => (c.correctors.mouthClosedFix = on)));
  const fixLip = switchControl('Lip suck is cancelled while the lip is raised or lowered', (on) => change((c) => (c.correctors.lipSuckFix = on)));
  const fixSym = switchControl('Both eyes look up and down together', (on) => change((c) => (c.correctors.eyeLookSymmetrize = on)));
  const corEnabled = switchControl('Apply correctors', (on) => change((c) => (c.correctors.enabled = on)));
  const lidBlend = sliderControl({ label: 'Eyelid link', min: 0, max: 1, step: 0.01, format: pct, onInput: (v) => change((c) => (c.correctors.eyeLidBlend = v)) });
  writers.push(() => {
    if (!cfg) return;
    corEnabled.input.checked = cfg.correctors.enabled;
    fixMouth.input.checked = cfg.correctors.mouthClosedFix;
    fixLip.input.checked = cfg.correctors.lipSuckFix;
    fixSym.input.checked = cfg.correctors.eyeLookSymmetrize;
    lidBlend.set(cfg.correctors.eyeLidBlend);
  });

  // ---- adjustment ----
  const adjEnabled = switchControl('Apply parameter ranges', (on) => change((c) => (c.adjustment.enabled = on)));
  const shapeList = h('div', { class: 'trk-adjust' });
  const headList = h('div', { class: 'trk-adjust' });
  /** @type {Map<string, { min: ReturnType<typeof sliderControl>, max: ReturnType<typeof sliderControl>, reset: HTMLButtonElement }>} */
  const adjRows = new Map();
  writers.push(() => {
    if (!cfg) return;
    adjEnabled.input.checked = cfg.adjustment.enabled;
    for (const [key, r] of adjRows) {
      const range = cfg.adjustment.ranges[key] ?? { min: 0, max: 1 };
      r.min.set(range.min);
      r.max.set(range.max);
      r.reset.disabled = range.min === 0 && range.max === 1;
    }
  });

  const adjResetAll = h('button', { type: 'button' }, 'Reset all');
  adjResetAll.addEventListener('click', () => {
    change((c) => (c.adjustment.ranges = {}));
    for (const r of adjRows.values()) {
      r.min.set(0);
      r.max.set(1);
      r.reset.disabled = true;
    }
  });

  /** @param {AdjustGroupInfo} g */
  function buildAdjustRow(g) {
    const min = sliderControl({
      label: `${g.label} minimum`, min: 0, max: 1, step: 0.01, format: fix2,
      onInput: (v) => change((c) => (rangeOf(c, g.key).min = v)),
    });
    const max = sliderControl({
      label: `${g.label} maximum`, min: 0, max: 1, step: 0.01, format: fix2,
      onInput: (v) => change((c) => (rangeOf(c, g.key).max = v)),
    });
    const reset = h('button', { type: 'button', class: 'small', 'aria-label': `Reset ${g.label}` }, 'Reset');
    reset.addEventListener('click', () => {
      change((c) => delete c.adjustment.ranges[g.key]);
      min.set(0);
      max.set(1);
      reset.disabled = true;
    });
    adjRows.set(g.key, { min, max, reset });
    return h('div', { class: 'trk-row adjust' }, h('span', { class: 'name' }, g.label), min.el, max.el, reset);
  }

  /** @param {MutationConfig} c @param {string} key */
  function rangeOf(c, key) {
    return (c.adjustment.ranges[key] ??= { min: 0, max: 1 });
  }

  function buildGroups() {
    adjRows.clear();
    shapeList.replaceChildren(...groups.filter((g) => !g.isHead).map(buildAdjustRow));
    headList.replaceChildren(...groups.filter((g) => g.isHead).map(buildAdjustRow));
  }

  // ---- filter ----
  const fltEnabled = switchControl('Smooth tracking data', (on) => change((c) => (c.filter.enabled = on)));
  writers.push(() => (fltEnabled.input.checked = !!cfg?.filter.enabled));
  const fltRows = FILTER_CATEGORIES.map(({ key, label }) => {
    const enable = switchControl(label, (on) => change((c) => (c.filter[key].enabled = on)));
    const cutoff = sliderControl({
      label: `${label} minimum cutoff`, min: 0, max: CUTOFF_STEPS, step: 1,
      format: (v) => `${v.toFixed(2)} Hz`,
      toSlider: (v) => (Math.log(v / CUTOFF_MIN) / Math.log(CUTOFF_MAX / CUTOFF_MIN)) * CUTOFF_STEPS,
      fromSlider: (s) => CUTOFF_MIN * (CUTOFF_MAX / CUTOFF_MIN) ** (s / CUTOFF_STEPS),
      onInput: (v) => change((c) => (c.filter[key].minCutoff = v)),
    });
    const beta = sliderControl({ label: `${label} beta`, min: 0, max: 10, step: 0.01, format: fix2, onInput: (v) => change((c) => (c.filter[key].beta = v)) });
    const deriv = sliderControl({
      label: `${label} derivative cutoff`, min: 0, max: CUTOFF_STEPS, step: 1,
      format: (v) => `${v.toFixed(2)} Hz`,
      toSlider: (v) => (Math.log(v / CUTOFF_MIN) / Math.log(CUTOFF_MAX / CUTOFF_MIN)) * CUTOFF_STEPS,
      fromSlider: (s) => CUTOFF_MIN * (CUTOFF_MAX / CUTOFF_MIN) ** (s / CUTOFF_STEPS),
      onInput: (v) => change((c) => (c.filter[key].derivativeCutoff = v)),
    });
    writers.push(() => {
      if (!cfg) return;
      /** @type {FilterChannel} */
      const ch = cfg.filter[key];
      enable.input.checked = ch.enabled;
      cutoff.set(ch.minCutoff);
      beta.set(ch.beta);
      deriv.set(ch.derivativeCutoff);
    });
    return h('div', { class: 'trk-row filter' }, enable.el, h('label', { class: 'trk-lbl' }, h('span', { class: 'muted small' }, 'Min cutoff'), cutoff.el), h('label', { class: 'trk-lbl' }, h('span', { class: 'muted small' }, 'Beta'), beta.el), h('label', { class: 'trk-lbl' }, h('span', { class: 'muted small' }, 'Derivative cutoff'), deriv.el));
  });

  function render() {
    for (const w of writers) w();
  }

  root.append(
    h(
      'div',
      { class: 'page' },
      h('div', { class: 'page-head' }, h('div', null, h('h1', null, 'Tracking'), h('p', { class: 'sub' }, 'Adjust the tracking data before it is sent to VRChat. Stages run in order: calibration, correctors, parameter adjustment, filter.'))),
      banner,
      h('section', { class: 'card', 'aria-label': 'Master switch' }, master.el),
      card('trk-cal-h', 'Calibration',
        h('p', { class: 'muted' }, 'Learns how far you actually move each shape and stretches it to the full range.'),
        h('div', { class: 'trk-stack' }, calEnabled.el, calContinuous.el),
        h('div', { class: 'field' }, h('span', { class: 'label' }, 'Strength'), calBlend.el),
        h('div', { class: 'field' }, h('span', { class: 'label' }, 'Progress'), h('div', { class: 'control' }, calProgress, calProgressText)),
        h('div', { class: 'form-actions' }, calReset),
        calDetails),
      card('trk-cor-h', 'Correctors',
        h('p', { class: 'muted' }, 'Fix combinations that are physically impossible.'),
        h('div', { class: 'trk-stack' }, corEnabled.el, fixMouth.el, fixLip.el, fixSym.el),
        h('div', { class: 'field' }, h('span', { class: 'label' }, 'Eyelid link'), lidBlend.el, h('span', { class: 'hint' }, 'At 100% both eyelids always open the same amount.'))),
      card('trk-adj-h', 'Parameter adjustment',
        h('p', { class: 'muted' }, 'Choose the input range that maps to 0..1: the minimum is a dead zone, a lower maximum is extra gain.'),
        h('div', { class: 'trk-stack' }, adjEnabled.el),
        h('div', { class: 'form-actions' }, adjResetAll),
        h('h3', null, 'Face shapes'), shapeList,
        h('h3', null, 'Head pose'), headList),
      card('trk-flt-h', 'Data filter',
        h('p', { class: 'muted' }, 'One Euro smoothing. Lower min cutoff = smoother at rest, higher beta = less lag when moving.'),
        h('div', { class: 'trk-stack' }, fltEnabled.el),
        ...fltRows),
    ),
  );

  /** @type {number | undefined} */
  let calTimer;

  (async () => {
    try {
      [groups, cfg] = await Promise.all([api.getAdjustGroups(), api.getMutationConfig()]);
      if (destroyed) return;
      buildGroups();
      render();
    } catch (e) {
      banner.hidden = false;
      banner.textContent = `Could not load tracking settings: ${errMsg(e)}`;
      return;
    }
    await refreshCalibration();
    if (!destroyed) calTimer = setInterval(() => void refreshCalibration(), CALIBRATION_REFRESH_MS);
  })();

  return {
    update() {
      const offline = state.phase !== 'running';
      banner.hidden = !offline;
      if (offline) banner.textContent = 'The engine is not running. Changes are saved and apply at the next start; calibration is unavailable.';
    },
    destroy() {
      destroyed = true;
      clearTimeout(saveTimer);
      clearInterval(calTimer);
      // Flush an edit still waiting for the debounce.
      if (dirty && cfg) void api.setMutationConfig(cfg).catch(() => {});
    },
  };
}
