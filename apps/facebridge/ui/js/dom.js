// @ts-check
// Tiny DOM helpers. No framework: pages build elements with `h()` and update them in place.

/**
 * Creates an element. Props: `class`, `on<event>` handlers, `aria-*`/`data-*`/`role`/`for` as attributes,
 * everything else as a DOM property when one exists (value, checked, disabled, hidden, ...). Insertion order
 * matters for inputs: put `type`, `min`, `max`, `step` before `value`.
 * @template {keyof HTMLElementTagNameMap} K
 * @param {K} tag
 * @param {Record<string, any> | null} [props]
 * @param {...(Node | string | number | null | undefined | false)} children
 * @returns {HTMLElementTagNameMap[K]}
 */
export function h(tag, props, ...children) {
  const el = document.createElement(tag);
  /** @type {any} */
  const anyEl = el;
  if (props) {
    for (const [key, value] of Object.entries(props)) {
      if (value == null || value === false) continue;
      if (key === 'class') el.className = value;
      else if (key.length > 2 && key.startsWith('on') && typeof value === 'function') {
        el.addEventListener(key.slice(2).toLowerCase(), value);
      } else if (key.includes('-') || key === 'role' || key === 'for') {
        el.setAttribute(key, value === true ? '' : String(value));
      } else if (key in el) anyEl[key] = value;
      else el.setAttribute(key, value === true ? '' : String(value));
    }
  }
  append(el, children);
  return el;
}

/**
 * @param {Element | DocumentFragment} parent
 * @param {ReadonlyArray<Node | string | number | null | undefined | false>} children
 */
export function append(parent, children) {
  for (const child of children) {
    if (child == null || child === false) continue;
    parent.append(typeof child === 'object' ? child : String(child));
  }
}

/**
 * @param {number} ms epoch milliseconds
 * @returns {string}
 */
export function fmtTime(ms) {
  return new Date(ms).toLocaleTimeString([], { hour12: false });
}

/**
 * @param {unknown} e
 * @returns {string}
 */
export function errMsg(e) {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

/** Must match the `.toast` opacity/transform transition in styles.css. */
const POPUP_FADE_MS = 250;
const POPUP_MAX = 5;

/**
 * Pop-up in the bottom-right corner: fades in, stays for `durationMs` (the timer restarts after the pointer
 * leaves it), then fades out. The newest is at the bottom; beyond five the oldest goes first.
 * @param {{ message: string, title?: string, kind?: string, durationMs?: number }} opts  `kind`: `ok`, `info`,
 *   or a log level (`information`, `warning`, `error`, `critical`, ...)
 */
export function popup({ message, title, kind = 'info', durationMs = 5000 }) {
  const host = document.getElementById('toasts');
  if (!host) return;
  const severe = kind === 'error' || kind === 'critical';
  const close = h('button', { type: 'button', class: 'icon toast-close', 'aria-label': 'Dismiss' }, '\u2715');
  const el = h(
    'div',
    { class: `toast ${kind} entering`, role: severe ? 'alert' : 'status' },
    h('div', { class: 'toast-body' }, title ? h('div', { class: 'toast-title' }, title) : null, h('div', { class: 'toast-msg' }, message)),
    close,
  );
  /** @type {number | undefined} */
  let timer;
  let leaving = false;
  const leave = () => {
    if (leaving) return;
    leaving = true;
    clearTimeout(timer);
    el.classList.add('leaving');
    setTimeout(() => el.remove(), POPUP_FADE_MS);
  };
  const arm = () => {
    clearTimeout(timer);
    timer = setTimeout(leave, durationMs);
  };
  close.addEventListener('click', leave);
  el.addEventListener('mouseenter', () => clearTimeout(timer));
  el.addEventListener('mouseleave', arm);
  host.append(el);
  const live = [...host.children].filter((c) => !c.classList.contains('leaving'));
  for (const old of live.slice(0, Math.max(0, live.length - POPUP_MAX))) old.querySelector('.toast-close')?.dispatchEvent(new Event('click'));
  // Next frame, so the entering styles are applied first and the fade-in transitions.
  requestAnimationFrame(() => requestAnimationFrame(() => el.classList.remove('entering')));
  arm();
}

/**
 * Short feedback pop-up for the user's own actions.
 * @param {string} message
 * @param {'info'|'error'|'ok'} [kind]
 */
export function toast(message, kind = 'info') {
  popup({ message, kind, durationMs: kind === 'error' ? 9000 : 4000 });
}

/**
 * Runs an async action with a disabled button and an error toast on failure.
 * @param {HTMLButtonElement} button
 * @param {() => Promise<unknown>} action
 * @param {string} [okMessage]
 */
export async function runButton(button, action, okMessage) {
  button.disabled = true;
  try {
    await action();
    if (okMessage) toast(okMessage, 'ok');
  } catch (e) {
    toast(errMsg(e), 'error');
  } finally {
    button.disabled = false;
  }
}

/**
 * A two-step destructive button: the first click arms it ("Confirm?"), a second click within 4 s runs it.
 * @param {string} label
 * @param {string} confirmLabel
 * @param {() => Promise<unknown> | void} action
 * @returns {HTMLButtonElement}
 */
export function confirmButton(label, confirmLabel, action) {
  /** @type {number | undefined} */
  let timer;
  const button = h('button', { type: 'button', class: 'danger' }, label);
  const disarm = () => {
    clearTimeout(timer);
    button.classList.remove('confirm');
    button.textContent = label;
  };
  button.addEventListener('click', async () => {
    if (!button.classList.contains('confirm')) {
      button.classList.add('confirm');
      button.textContent = confirmLabel;
      timer = setTimeout(disarm, 4000);
      return;
    }
    disarm();
    button.disabled = true;
    try {
      await action();
    } catch (e) {
      toast(errMsg(e), 'error');
    } finally {
      button.disabled = false;
    }
  });
  button.addEventListener('blur', disarm);
  return button;
}
