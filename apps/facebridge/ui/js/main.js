// @ts-check
// Entry point: routing, sidebar, and wiring the store to the active page.

import { h } from './dom.js';
import { initPopups } from './popups.js';
import { backendError, init, onState, state } from './store.js';
import { mountHome } from './pages/home.js';
import { mountLive } from './pages/live.js';
import { mountModules } from './pages/modules.js';
import { mountOutput } from './pages/output.js';
import { mountTracking } from './pages/tracking.js';
import { mountSettings } from './pages/settings.js';

/**
 * @typedef {object} PageHandle
 * @property {() => void} update  called on every store change
 * @property {() => void} [destroy]
 */

/** @type {{ id: string, label: string, mount: (root: HTMLElement) => PageHandle }[]} */
const PAGES = [
  { id: 'home', label: 'Home', mount: mountHome },
  { id: 'modules', label: 'Modules', mount: mountModules },
  { id: 'live', label: 'Live', mount: mountLive },
  { id: 'tracking', label: 'Tracking', mount: mountTracking },
  { id: 'output', label: 'Output', mount: mountOutput },
  { id: 'settings', label: 'Settings', mount: mountSettings },
];

const main = /** @type {HTMLElement} */ (document.getElementById('main'));
const nav = /** @type {HTMLElement} */ (document.getElementById('nav'));
const sidebarStatus = /** @type {HTMLElement} */ (document.getElementById('sidebar-status'));
const brandVersion = /** @type {HTMLElement} */ (document.getElementById('brand-version'));

/** @type {PageHandle | null} */
let current = null;
let currentId = '';
/** @type {Map<string, HTMLAnchorElement>} */
const navLinks = new Map();
/** @type {HTMLSpanElement | null} */
let homeBadge = null;

function routeId() {
  const id = location.hash.replace(/^#\/?/, '');
  return PAGES.some((p) => p.id === id) ? id : 'home';
}

function navigate() {
  const id = routeId();
  if (id === currentId) return;
  current?.destroy?.();
  current = null;
  currentId = id;
  main.replaceChildren();
  const page = /** @type {typeof PAGES[number]} */ (PAGES.find((p) => p.id === id));
  document.title = `${page.label} - OSCFT for VRC`;
  for (const [pid, link] of navLinks) {
    if (pid === id) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  }
  current = page.mount(main);
  current.update();
  main.scrollTop = 0;
}

function buildNav() {
  for (const page of PAGES) {
    const link = h('a', { href: `#/${page.id}` }, h('span', null, page.label));
    if (page.id === 'home') {
      homeBadge = h('span', { class: 'nav-badge', hidden: true, 'aria-label': 'notifications' });
      link.append(homeBadge);
    }
    navLinks.set(page.id, link);
    nav.append(h('li', null, link));
  }
}

function updateChrome() {
  brandVersion.textContent = state.version ? `v${state.version}` : '';
  const snapshot = state.snapshot;

  const notifications = snapshot?.notifications ?? [];
  if (homeBadge) {
    homeBadge.hidden = notifications.length === 0;
    homeBadge.textContent = String(notifications.length);
    const severe = notifications.some((n) => n.level === 'error' || n.level === 'critical');
    homeBadge.classList.toggle('error', severe);
  }

  /** @type {[string, string][]} */
  const rows = [];
  if (backendError) rows.push(['danger', 'Backend unreachable']);
  else if (state.phase === 'running') rows.push(['ok', 'Engine running']);
  else if (state.phase === 'starting') rows.push(['warn pulse', 'Engine starting']);
  else if (state.phase === 'failed') rows.push(['danger', 'Engine failed']);
  else rows.push(['', 'Engine stopped']);
  if (snapshot) {
    rows.push(snapshot.osc.sending ? ['ok', `OSC ${snapshot.osc.messagesPerSecond} msg/s`] : ['warn', 'OSC idle']);
  }
  sidebarStatus.replaceChildren(
    ...rows.map(([cls, text]) => h('div', null, h('span', { class: `dot ${cls}` }), ` ${text}`)),
  );
}

buildNav();
window.addEventListener('hashchange', navigate);
onState(() => {
  updateChrome();
  current?.update();
});
initPopups();
// Block navigation away from the app (e.g. a dropped file or a stray link) so the webview never leaves the UI.
window.addEventListener('dragover', (e) => e.preventDefault());
window.addEventListener('drop', (e) => e.preventDefault());

navigate();
updateChrome();
void init();
