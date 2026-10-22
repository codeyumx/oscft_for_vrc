// @ts-check
// Renders a module's settings form from its declared schema (spec/ipc-protocol.md, "Settings schema JSON").
// `text` fields are read-only display text the module updates while it runs; `button` fields call the module.

import { api } from '../api.js';
import { errMsg, h, toast } from '../dom.js';

/** @typedef {import('../types.js').SettingsField} SettingsField */
/** @typedef {import('../types.js').ModuleSettingsView} ModuleSettingsView */

let uid = 0;

/**
 * @typedef {object} FieldControl
 * @property {SettingsField} field
 * @property {HTMLElement} row
 * @property {'input'|'display'|'button'} role  only `input` fields are edited, saved and reverted
 * @property {() => any} read       current value (undefined when invalid or not an input)
 * @property {(value: any) => void} write
 * @property {() => string | null} validate  error message or null
 * @property {HTMLButtonElement} [button]
 */

/** @param {SettingsField} f */
function stepOf(f) {
  if (f.type === 'int') return f.step && f.step >= 1 ? Math.round(f.step) : 1;
  return f.step && f.step > 0 ? f.step : 0.01;
}

/**
 * @param {SettingsField} f
 * @param {string} id
 * @param {() => void} onInput
 * @returns {{ control: HTMLElement, read: () => any, write: (v: any) => void, validate: () => string | null }}
 */
function numberControl(f, id, onInput) {
  const isInt = f.type === 'int';
  const step = stepOf(f);
  const hasRange = typeof f.min === 'number' && typeof f.max === 'number';
  const number = h('input', { type: 'number', id, step, min: f.min, max: f.max, 'aria-describedby': `${id}-hint ${id}-err` });
  const slider = hasRange ? h('input', { type: 'range', step, min: f.min, max: f.max, 'aria-label': `${f.label ?? f.key} slider`, tabIndex: -1 }) : null;
  number.addEventListener('input', () => {
    if (slider && number.value !== '' && !Number.isNaN(number.valueAsNumber)) slider.value = number.value;
    onInput();
  });
  slider?.addEventListener('input', () => {
    number.value = slider.value;
    onInput();
  });
  return {
    control: h('div', { class: 'control' }, slider, number),
    read: () => (number.value === '' ? undefined : number.valueAsNumber),
    write: (v) => {
      number.value = String(v ?? f.default ?? 0);
      if (slider) slider.value = number.value;
    },
    validate: () => {
      if (number.value === '' || Number.isNaN(number.valueAsNumber)) return 'Enter a number.';
      const v = number.valueAsNumber;
      if (isInt && !Number.isInteger(v)) return 'Enter a whole number.';
      if (typeof f.min === 'number' && v < f.min) return `Must be at least ${f.min}.`;
      if (typeof f.max === 'number' && v > f.max) return `Must be at most ${f.max}.`;
      return null;
    },
  };
}

/**
 * @param {SettingsField} f
 * @param {() => void} onInput
 * @param {(f: SettingsField, button: HTMLButtonElement) => void} onPress
 * @returns {FieldControl | null} null for a field type this version does not know (skipped)
 */
function buildField(f, onInput, onPress) {
  const id = `mf-${uid++}`;
  const label = f.label || f.key;
  /** @type {{ control: HTMLElement, read: () => any, write: (v: any) => void, validate: () => string | null }} */
  let impl;
  /** @type {HTMLElement | null} */
  let labelEl = h('label', { for: id }, label);
  /** @type {'input'|'display'|'button'} */
  let role = 'input';
  /** @type {HTMLButtonElement | undefined} */
  let button;

  switch (f.type) {
    case 'int':
    case 'float':
      impl = numberControl(f, id, onInput);
      break;
    case 'bool': {
      const box = h('input', { type: 'checkbox', id, 'aria-describedby': `${id}-hint` });
      box.addEventListener('change', onInput);
      impl = {
        control: h('div', { class: 'control' }, h('label', { class: 'switch' }, box, h('span', { class: 'track', 'aria-hidden': 'true' }), h('span', null, label))),
        read: () => box.checked,
        write: (v) => void (box.checked = Boolean(v ?? f.default)),
        validate: () => null,
      };
      // The switch carries its own visible label.
      labelEl = h('span', { class: 'label', 'aria-hidden': 'true' }, label);
      break;
    }
    case 'choice': {
      const select = h('select', { id, 'aria-describedby': `${id}-hint` }, ...(f.options ?? []).map((o) => h('option', { value: o.value }, o.label ?? o.value)));
      select.addEventListener('change', onInput);
      impl = {
        control: h('div', { class: 'control' }, select),
        read: () => select.value,
        write: (v) => {
          const wanted = String(v ?? f.default ?? '');
          // A stored value that is no longer an option keeps the form honest: fall back to the first option.
          select.value = [...select.options].some((o) => o.value === wanted) ? wanted : (select.options[0]?.value ?? '');
        },
        validate: () => ((f.options?.length ?? 0) === 0 ? 'This setting declares no options.' : null),
      };
      break;
    }
    case 'string': {
      const text = h('input', { type: 'text', id, autocomplete: 'off', 'aria-describedby': `${id}-hint` });
      text.addEventListener('input', onInput);
      impl = {
        control: h('div', { class: 'control' }, text),
        read: () => text.value,
        write: (v) => void (text.value = String(v ?? f.default ?? '')),
        validate: () => null,
      };
      break;
    }
    case 'text': {
      role = 'display';
      const out = h('output', { class: 'setting-text', id, 'aria-live': 'polite', 'aria-describedby': `${id}-hint` });
      impl = {
        control: h('div', { class: 'control' }, out),
        read: () => undefined,
        write: (v) => void (out.textContent = String(v ?? f.default ?? '')),
        validate: () => null,
      };
      labelEl = h('span', { class: 'label' }, label);
      break;
    }
    case 'button': {
      role = 'button';
      const b = h('button', { type: 'button', id, 'aria-describedby': `${id}-hint` }, label);
      button = b;
      b.addEventListener('click', () => onPress(f, b));
      impl = { control: h('div', { class: 'control' }, b), read: () => undefined, write: () => {}, validate: () => null };
      // The button carries its own label; keep the grid column.
      labelEl = h('span', { class: 'label', 'aria-hidden': 'true' });
      break;
    }
    default:
      return null;
  }

  const err = h('div', { class: 'error', id: `${id}-err`, hidden: true });
  const row = h('div', { class: 'field' }, labelEl, impl.control, h('div', { class: 'hint', id: `${id}-hint`, hidden: !f.description }, f.description ?? ''), err);
  return {
    field: f,
    row,
    role,
    button,
    read: impl.read,
    write: impl.write,
    validate: () => {
      const message = impl.validate();
      err.hidden = message === null;
      err.textContent = message ?? '';
      const input = row.querySelector('input[type="number"]');
      if (input) input.setAttribute('aria-invalid', String(message !== null));
      return message;
    },
  };
}

/**
 * Creates the settings form for one module. Call `load()` to (re)fetch schema and values, `sync()` when the
 * module's values changed (keeps unsaved edits, always updates display text), `setRunning()` to enable buttons.
 * @param {string} moduleId
 */
export function createSettingsForm(moduleId) {
  const root = h('div', { class: 'module-form' }, h('p', { class: 'muted' }, 'Loading settings\u2026'));
  /** @type {FieldControl[]} */
  let controls = [];
  /** @type {Record<string, any>} */
  let baseline = {};
  let schemaJson = '';
  let dirty = false;
  let loading = false;
  let running = false;

  const save = h('button', { type: 'button', class: 'primary' }, 'Save settings');
  const revert = h('button', { type: 'button' }, 'Revert');
  const defaults = h('button', { type: 'button' }, 'Restore defaults');
  const status = h('span', { class: 'muted small', role: 'status' });
  const inputs = () => controls.filter((c) => c.role === 'input');

  function refreshButtons() {
    const invalid = controls.some((c) => c.validate() !== null);
    save.disabled = !dirty || invalid;
    revert.disabled = !dirty;
    for (const c of controls) {
      if (c.button) {
        c.button.disabled = !running || invalid;
        c.button.title = running ? '' : 'The module is not running.';
      }
    }
  }

  function onInput() {
    dirty = true;
    status.textContent = '';
    refreshButtons();
  }

  /** Input values that differ from what the backend last reported. */
  function changed() {
    /** @type {Record<string, any>} */
    const out = {};
    for (const c of inputs()) {
      const value = c.read();
      if (value !== undefined && value !== baseline[c.field.key]) out[c.field.key] = value;
    }
    return out;
  }

  async function saveChanges() {
    const values = changed();
    await api.setModuleSettings(moduleId, values);
    Object.assign(baseline, values);
    dirty = false;
  }

  /**
   * Pending edits are saved first, so the module sees what the user typed (e.g. a pairing code).
   * @param {SettingsField} f
   * @param {HTMLButtonElement} button
   */
  async function press(f, button) {
    button.disabled = true;
    try {
      if (dirty) await saveChanges();
      await api.pressModuleButton(moduleId, f.key);
    } catch (e) {
      toast(errMsg(e), 'error');
    } finally {
      refreshButtons();
    }
  }

  /** @param {ModuleSettingsView} view */
  function render(view) {
    schemaJson = JSON.stringify(view.schema);
    controls = view.schema.fields.map((f) => buildField(f, onInput, press)).filter(/** @returns {c is FieldControl} */ (c) => c !== null);
    baseline = {};
    for (const c of controls) {
      c.write(view.values[c.field.key] ?? c.field.default);
      if (c.role === 'input') baseline[c.field.key] = c.read();
    }
    dirty = false;
    status.textContent = '';
    refreshButtons();
    const actions = inputs().length ? h('div', { class: 'form-actions' }, save, revert, defaults, status) : null;
    root.replaceChildren(
      controls.length
        ? h('form', { class: 'form', novalidate: true, onsubmit: (/** @type {Event} */ e) => e.preventDefault() }, ...controls.map((c) => c.row), actions)
        : h('p', { class: 'muted' }, 'This module declares no settings.'),
    );
  }

  save.addEventListener('click', async () => {
    save.disabled = true;
    try {
      await saveChanges();
      status.textContent = 'Saved.';
      toast('Settings saved', 'ok');
    } catch (e) {
      status.textContent = '';
      toast(errMsg(e), 'error');
    } finally {
      refreshButtons();
    }
  });
  revert.addEventListener('click', () => {
    for (const c of inputs()) c.write(baseline[c.field.key]);
    dirty = false;
    refreshButtons();
  });
  defaults.addEventListener('click', () => {
    for (const c of inputs()) c.write(c.field.default);
    onInput();
  });

  /** @param {boolean} rebuild */
  async function fetchAndApply(rebuild) {
    if (loading) return;
    loading = true;
    try {
      const view = await api.getModuleSettings(moduleId);
      if (!view) {
        controls = [];
        schemaJson = '';
        root.replaceChildren(h('p', { class: 'muted' }, 'The module has not declared its settings yet (it declares them while it starts).'));
      } else if (rebuild || JSON.stringify(view.schema) !== schemaJson) {
        if (!dirty || rebuild) render(view);
      } else {
        // Same form: display text always follows the module; inputs only when the user is not editing.
        for (const c of controls) {
          if (c.role === 'display') c.write(view.values[c.field.key]);
          else if (c.role === 'input' && !dirty) {
            c.write(view.values[c.field.key] ?? c.field.default);
            baseline[c.field.key] = c.read();
          }
        }
        refreshButtons();
      }
    } catch (e) {
      root.replaceChildren(h('p', { class: 'err-text' }, errMsg(e)));
    } finally {
      loading = false;
    }
  }

  return {
    el: root,
    get dirty() {
      return dirty;
    },
    /** Fetches schema and current values; keeps unsaved edits unless `force`. @param {boolean} [force] */
    async load(force = false) {
      if (dirty && !force) return;
      await fetchAndApply(force);
    },
    /** The module's values changed: refresh display text, and inputs the user is not editing. */
    sync() {
      return fetchAndApply(false);
    },
    /** @param {boolean} value whether the module is running (buttons need it) */
    setRunning(value) {
      if (running === value) return;
      running = value;
      refreshButtons();
    },
  };
}
