// @ts-check
// Small pieces shared by the Home and Modules pages.

import { h } from '../dom.js';

/** @typedef {import('../types.js').ModuleView} ModuleView */

/** @type {Record<string, { label: string, cls: string }>} */
const STATE_BADGES = {
  stopped: { label: 'Stopped', cls: '' },
  starting: { label: 'Starting', cls: 'warn' },
  initializing: { label: 'Initializing', cls: 'warn' },
  running: { label: 'Running', cls: 'ok' },
  unclaimed: { label: 'Not needed', cls: 'info' },
  nodata: { label: 'No data', cls: 'warn' },
  failed: { label: 'Failed', cls: 'danger' },
  exited: { label: 'Exited', cls: 'danger' },
};

/** @param {ModuleView} m */
export function moduleStateBadge(m) {
  const info = STATE_BADGES[m.state] ?? { label: m.state, cls: '' };
  const label = m.enabled || m.state !== 'stopped' ? info.label : 'Disabled';
  const title =
    m.state === 'unclaimed'
      ? 'Everything this module can track is already provided by a module with higher priority.'
      : m.state === 'nodata'
        ? 'The module claimed tracking but sent no live data while modules were starting, so its claim was offered to the next module. Restart modules to try again.'
        : m.status === 1
          ? 'Module reports: active'
          : m.status === 0
            ? 'Module reports: idle'
            : '';
  return h('span', { class: `badge ${info.cls}`, title }, label);
}

/** Which tracking data a module supports and which part of it is actually in use. @param {ModuleView} m */
export function claimChips(m) {
  /**
   * @param {string} label
   * @param {boolean} supported
   * @param {boolean} claimed
   */
  const chip = (label, supported, claimed) =>
    h(
      'span',
      { class: `chip${claimed ? ' on' : ''}`, title: claimed ? `${label} tracking in use` : supported ? `${label} supported, not in use` : `${label} not supported` },
      `${label}: ${claimed ? 'in use' : supported ? 'available' : 'n/a'}`,
    );
  return h('div', { class: 'chips' }, chip('Eye', m.supports.eye, m.claims.eye), chip('Face', m.supports.expression, m.claims.expression));
}
