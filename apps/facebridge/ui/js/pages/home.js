// @ts-check
// Home: engine problems, module notifications, OSC / avatar status and a compact module overview.

import { api } from '../api.js';
import { append, fmtTime, h, runButton } from '../dom.js';
import { refresh, state } from '../store.js';
import { moduleStateBadge, claimChips } from './module-ui.js';

/** @typedef {import('../types.js').NotificationView} NotificationView */
/** @typedef {import('../types.js').ModuleView} ModuleView */

const LEVEL_LABEL = /** @type {Record<string, string>} */ ({
  trace: 'Trace',
  debug: 'Debug',
  information: 'Info',
  warning: 'Warning',
  error: 'Error',
  critical: 'Critical',
});

/** @param {NotificationView} n */
function notificationBanner(n) {
  const level = n.level.toLowerCase();
  const dismiss = h('button', { type: 'button', class: 'icon', 'aria-label': `Dismiss notification from ${n.moduleName}` }, '\u2715');
  dismiss.addEventListener('click', () => runButton(dismiss, () => api.dismissNotification(n.id).then(() => refresh())));
  return h(
    'div',
    { class: `banner ${level}`, role: level === 'error' || level === 'critical' ? 'alert' : 'status' },
    h(
      'div',
      { class: 'body' },
      h('div', { class: 'title' }, n.moduleName, h('span', { class: 'meta' }, `  ${LEVEL_LABEL[level] ?? n.level} \u00b7 ${fmtTime(n.timeMs)}`)),
      h('div', { class: 'msg' }, n.message),
    ),
    dismiss,
  );
}

function engineBanner() {
  if (state.phase === 'failed') {
    const retry = h('button', { type: 'button', class: 'primary' }, 'Retry');
    retry.addEventListener('click', () => runButton(retry, () => api.restartEngine()));
    return h(
      'div',
      { class: 'banner error', role: 'alert' },
      h(
        'div',
        { class: 'body' },
        h('div', { class: 'title' }, 'The engine could not start'),
        h('div', { class: 'msg' }, state.error ?? 'Unknown error'),
        h('div', { class: 'meta' }, 'Check the Output page and the Settings page (OSC address, module host), then retry.'),
      ),
      retry,
    );
  }
  if (state.phase === 'starting') {
    return h('div', { class: 'banner info', role: 'status' }, h('div', { class: 'body' }, h('div', { class: 'title' }, 'Starting the engine\u2026')));
  }
  return null;
}

/** @param {string} source */
function avatarSource(source) {
  return source.toLowerCase().includes('query') ? 'OSCQuery' : 'Config file';
}

function oscCard() {
  const snap = state.snapshot;
  if (!snap) return h('section', { class: 'card', 'aria-labelledby': 'osc-h' }, h('h2', { id: 'osc-h' }, 'OSC output'), h('p', { class: 'muted' }, 'Not running.'));
  const { osc, oscConfig } = snap;
  const status = osc.sending ? h('span', { class: 'badge ok' }, 'Sending') : h('span', { class: 'badge warn' }, 'Idle');
  return h(
    'section',
    { class: 'card', 'aria-labelledby': 'osc-h' },
    h('div', { class: 'card-head' }, h('h2', { id: 'osc-h' }, 'OSC output'), status),
    h(
      'dl',
      { class: 'kv' },
      h('dt', null, 'Sending to'),
      h('dd', { class: 'mono' }, osc.sendAddr),
      h('dt', null, 'Rate'),
      h('dd', null, `${osc.messagesPerSecond} messages/s (target ${oscConfig.rateHz} Hz)`),
      h('dt', null, 'VRChat'),
      h('dd', null, osc.vrchatDetected ? 'Detected' : 'Not detected'),
      h('dt', null, 'Discovery'),
      h('dd', null, oscConfig.oscquery ? 'OSCQuery' : 'Legacy only', oscConfig.listenPort ? `, listening on UDP ${oscConfig.listenPort}` : ''),
    ),
    osc.lastError ? h('p', { class: 'err-text' }, osc.lastError) : null,
  );
}

function avatarCard() {
  const avatar = state.snapshot?.avatar ?? null;
  const body = avatar
    ? h(
        'dl',
        { class: 'kv' },
        h('dt', null, 'Name'),
        h('dd', null, avatar.name ?? 'Unknown'),
        h('dt', null, 'ID'),
        h('dd', { class: 'mono' }, avatar.id),
        h('dt', null, 'Parameters'),
        h('dd', null, `${avatar.matchedParameterCount} matched of ${avatar.parameterCount} on the avatar`),
        h('dt', null, 'Source'),
        h('dd', null, avatarSource(avatar.source)),
      )
    : h('p', { class: 'muted' }, 'No avatar reported yet. FaceBridge falls back to its default parameter set until VRChat announces one.');
  return h(
    'section',
    { class: 'card', 'aria-labelledby': 'avatar-h' },
    h('div', { class: 'card-head' }, h('h2', { id: 'avatar-h' }, 'Avatar'), avatar ? h('span', { class: 'badge info' }, 'Active') : h('span', { class: 'badge' }, 'None')),
    body,
  );
}

/** @param {ModuleView} m */
function moduleSummary(m) {
  return h(
    'article',
    { class: 'card', 'aria-label': m.name },
    h('div', { class: 'card-head' }, h('h3', null, m.name, m.version ? h('span', { class: 'muted' }, ` v${m.version}`) : null), moduleStateBadge(m)),
    claimChips(m),
    m.lastError ? h('p', { class: 'err-text small' }, m.lastError) : null,
  );
}

/**
 * @param {HTMLElement} root
 * @returns {import('../main.js').PageHandle}
 */
export function mountHome(root) {
  const page = h('div', { class: 'page' });
  root.append(page);

  function update() {
    const snap = state.snapshot;
    const modules = snap?.modules ?? [];
    const notifications = snap?.notifications ?? [];
    const enabled = modules.filter((m) => m.enabled);
    const banner = engineBanner();

    page.replaceChildren();
    append(
      page,
      [
      h('div', { class: 'page-head' }, h('div', null, h('h1', null, 'Home'), h('p', { class: 'sub' }, 'Tracking status at a glance.'))),
      banner,
      notifications.length
        ? h('section', { class: 'section', 'aria-labelledby': 'notif-h' }, h('h2', { id: 'notif-h' }, 'Notifications'), h('div', { class: 'banner-list' }, ...notifications.map(notificationBanner)))
        : null,
      h('div', { class: 'grid-2' }, oscCard(), avatarCard()),
      h(
        'section',
        { class: 'section', 'aria-labelledby': 'mods-h' },
        h('h2', { id: 'mods-h' }, 'Modules'),
        modules.length === 0
          ? h(
              'div',
              { class: 'empty' },
              'No tracking modules installed yet. ',
              h('a', { href: '#/modules' }, 'Install or import one'),
              ' to start tracking.',
            )
          : h(
              'div',
              { class: 'grid-2' },
              ...(enabled.length ? enabled : modules).map(moduleSummary),
            ),
      ),
      ],
    );
  }

  return { update };
}
