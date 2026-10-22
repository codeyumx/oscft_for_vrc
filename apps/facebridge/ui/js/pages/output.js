// @ts-check
// Output: engine and module log with a level filter.

import { fmtTime, h } from '../dom.js';
import { logs, onLog, onLogReset, state } from '../store.js';

/** @typedef {import('../types.js').LogEntry} LogEntry */

const LEVEL_RANK = /** @type {Record<string, number>} */ ({ trace: 0, debug: 1, information: 2, warning: 3, error: 4, critical: 5 });
const MAX_ROWS = 1000;
const STICK_TO_BOTTOM_PX = 24;

/** @param {string} level */
const rank = (level) => LEVEL_RANK[level.toLowerCase()] ?? LEVEL_RANK.information;

/** @param {string} level */
const levelLabel = (level) => (level.toLowerCase() === 'information' ? 'info' : level.toLowerCase());

/**
 * @param {HTMLElement} root
 * @returns {import('../main.js').PageHandle}
 */
export function mountOutput(root) {
  const level = h(
    'select',
    { id: 'log-level' },
    h('option', { value: '0' }, 'All levels'),
    h('option', { value: '2' }, 'Information and above'),
    h('option', { value: '3' }, 'Warnings and errors'),
    h('option', { value: '4' }, 'Errors only'),
  );
  level.value = '2';
  const search = h('input', { type: 'text', id: 'log-search', placeholder: 'Filter text', autocomplete: 'off' });
  const clear = h('button', { type: 'button' }, 'Clear');
  const count = h('span', { class: 'muted small', role: 'status' });
  const list = h('div', { class: 'log-list', role: 'log', 'aria-label': 'Log output', tabIndex: 0 });

  /** @type {LogEntry | null} */
  let clearedAt = null;

  /** @param {string | null} moduleId */
  function sourceName(moduleId) {
    if (!moduleId) return 'engine';
    return state.snapshot?.modules.find((m) => m.id === moduleId)?.name ?? moduleId;
  }

  /** @param {LogEntry} e */
  function visible(e) {
    if (rank(e.level) < Number(level.value)) return false;
    const needle = search.value.trim().toLowerCase();
    if (!needle) return true;
    return `${e.message} ${e.category} ${sourceName(e.moduleId)}`.toLowerCase().includes(needle);
  }

  /** @param {LogEntry} e */
  function row(e) {
    const lvl = e.level.toLowerCase();
    return h(
      'div',
      { class: `log-row ${lvl}`, title: e.category },
      h('span', { class: 'time' }, fmtTime(e.timeMs)),
      h('span', { class: 'lvl' }, levelLabel(e.level)),
      h('span', { class: 'cat' }, sourceName(e.moduleId)),
      h('span', { class: 'msg' }, e.message),
    );
  }

  function atBottom() {
    return list.scrollHeight - list.scrollTop - list.clientHeight <= STICK_TO_BOTTOM_PX;
  }

  function updateCount() {
    count.textContent = `${list.childElementCount} shown`;
  }

  function renderAll() {
    const start = clearedAt ? logs.lastIndexOf(clearedAt) + 1 : 0;
    const frag = document.createDocumentFragment();
    for (let i = start; i < logs.length; i++) {
      const e = /** @type {LogEntry} */ (logs[i]);
      if (visible(e)) frag.append(row(e));
    }
    list.replaceChildren(frag);
    list.scrollTop = list.scrollHeight;
    updateCount();
  }

  level.addEventListener('change', renderAll);
  search.addEventListener('input', renderAll);
  clear.addEventListener('click', () => {
    clearedAt = logs[logs.length - 1] ?? null;
    renderAll();
  });

  const stopLog = onLog((entry) => {
    if (!visible(entry)) return;
    const stick = atBottom();
    list.append(row(entry));
    while (list.childElementCount > MAX_ROWS) list.firstElementChild?.remove();
    if (stick) list.scrollTop = list.scrollHeight;
    updateCount();
  });
  const stopReset = onLogReset(() => {
    clearedAt = null;
    renderAll();
  });

  root.append(
    h(
      'div',
      { class: 'page' },
      h(
        'div',
        { class: 'page-head' },
        h('div', null, h('h1', null, 'Output'), h('p', { class: 'sub' }, 'Messages from the engine and from your tracking modules.')),
        h(
          'div',
          { class: 'toolbar' },
          h('label', { for: 'log-level', class: 'muted' }, 'Level'),
          level,
          h('label', { for: 'log-search', class: 'muted' }, 'Search'),
          search,
          clear,
          count,
        ),
      ),
      list,
    ),
  );
  renderAll();

  return {
    // Rows are appended by log events; a state change only matters for module display names.
    update() {},
    destroy() {
      stopLog();
      stopReset();
    },
  };
}
