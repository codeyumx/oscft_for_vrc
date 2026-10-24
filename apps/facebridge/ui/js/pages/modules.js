// @ts-check
// Modules: install / import / enable / reorder / uninstall, plus each module's settings form.
// Cards are keyed by module id and updated in place so open settings forms keep their unsaved edits.

import { api } from '../api.js';
import { confirmButton, h, runButton, toast, errMsg } from '../dom.js';
import { refresh, state } from '../store.js';
import { claimChips, moduleStateBadge } from './module-ui.js';
import { createSettingsForm } from './settings-form.js';

/** @typedef {import('../types.js').ModuleView} ModuleView */
/** @typedef {import('../types.js').ModuleUpdate} ModuleUpdate */
/** @typedef {import('../types.js').ModuleOptions} ModuleOptions */

/**
 * Options FaceBridge offers for every module (stored in config.json). A new option needs an entry here and a
 * field in fb-core's `ModuleOptions`. A toggle may carry a number that goes with it (shown next to the toggle).
 * @type {{ key: 'allowNotifications' | 'requireData', title: string, label: string, hint: string, number?: { key: 'dataTimeoutSecs', label: string, min: number, max: number } }[]}
 */
const MODULE_OPTIONS = [
  {
    key: 'allowNotifications',
    title: 'Notifications',
    label: 'Allow notifications from this module',
    hint: 'Shows its notifications and errors as pop-ups, and keeps its messages on the Home page. When off they only go to the Output log. A module that fails is always reported.',
  },
  {
    key: 'requireData',
    title: 'Tracking data',
    label: 'Wait for tracking data before claiming',
    number: { key: 'dataTimeoutSecs', label: 'Wait up to', min: 1, max: 120 },
    hint: 'At startup, a module that claims eye or face tracking must send changing data within this time, otherwise the part goes to the next module in line (a module without a successor keeps it). Only checked while modules start; a module that later stops sending keeps its claim. Applies the next time modules start.',
  },
];

/**
 * @param {string} id
 */
function createCard(id) {
  const image = h('img', { class: 'module-img', alt: '', hidden: true });
  const placeholder = h('div', { class: 'module-img placeholder', 'aria-hidden': 'true' }, '?');
  const name = h('h2', null, '');
  const version = h('span', { class: 'muted small' });
  const prio = h('span', { class: 'prio' });
  const badgeSlot = h('span');
  const author = h('div', { class: 'muted small' });
  const description = h('p', null);
  const usage = h('p', { class: 'module-usage' });
  const chipsSlot = h('div');
  const error = h('p', { class: 'err-text small', role: 'alert' });
  const pageLink = h('button', { type: 'button', class: 'link small' }, 'Module page');
  const updateText = h('span', { class: 'title' });
  const releaseLink = h('button', { type: 'button', class: 'link small' }, 'View release');
  const updateBanner = h('div', { class: 'banner info', role: 'status', hidden: true }, h('div', { class: 'body' }, updateText), releaseLink);

  const enabledInput = h('input', { type: 'checkbox' });
  const enabledLabel = h('label', { class: 'switch' }, enabledInput, h('span', { class: 'track', 'aria-hidden': 'true' }), h('span', null, 'Enabled'));
  const up = h('button', { type: 'button', class: 'icon' }, '\u2191');
  const down = h('button', { type: 'button', class: 'icon' }, '\u2193');
  const uninstall = confirmButton('Uninstall', 'Confirm uninstall', async () => {
    await api.uninstallModule(id);
    toast('Module uninstalled', 'ok');
    await refresh();
  });

  /** @type {ModuleOptions | null} */
  let options = null;
  /**
   * Stores `patch` on top of the current options; `revert` puts the control back when that fails.
   * @param {Partial<ModuleOptions>} patch
   * @param {HTMLInputElement[]} controls disabled while saving
   * @param {() => void} revert
   */
  async function saveOptions(patch, controls, revert) {
    if (!options) return;
    for (const c of controls) c.disabled = true;
    try {
      await api.setModuleOptions(id, { ...options, ...patch });
      await refresh();
    } catch (e) {
      revert();
      toast(errMsg(e), 'error');
    } finally {
      for (const c of controls) c.disabled = false;
    }
  }
  const optionInputs = MODULE_OPTIONS.map((o) => {
    const input = h('input', { type: 'checkbox', 'aria-describedby': `${id}-${o.key}-hint` });
    const num = o.number;
    const number = num
      ? h('input', { type: 'number', class: 'option-seconds', min: num.min, max: num.max, step: 1, 'aria-describedby': `${id}-${o.key}-hint` })
      : null;
    input.addEventListener('change', () => {
      const wanted = input.checked;
      void saveOptions({ [o.key]: wanted }, number ? [input, number] : [input], () => {
        input.checked = !wanted;
      });
    });
    if (number && num) {
      number.addEventListener('change', () => {
        const wanted = number.valueAsNumber;
        const previous = options?.[num.key] ?? num.min;
        if (!Number.isInteger(wanted) || wanted < num.min || wanted > num.max) {
          number.value = String(previous);
          toast(`${num.label} must be a whole number from ${num.min} to ${num.max}.`, 'error');
          return;
        }
        void saveOptions({ [num.key]: wanted }, [input, number], () => {
          number.value = String(previous);
        });
      });
    }
    return { option: o, input, number };
  });
  const builtin = h(
    'section',
    { class: 'settings-part', 'aria-label': 'OSCFT options' },
    h('h3', null, 'OSCFT options'),
    ...optionInputs.map(({ option, input, number }) =>
      h(
        'div',
        { class: 'field' },
        h('span', { class: 'label', 'aria-hidden': 'true' }, option.title),
        h(
          'div',
          { class: 'control' },
          h('label', { class: 'switch' }, input, h('span', { class: 'track', 'aria-hidden': 'true' }), h('span', null, option.label)),
          number && option.number ? h('span', { class: 'muted small' }, option.number.label) : null,
          number,
          number ? h('span', { class: 'muted small' }, 'seconds') : null,
        ),
        h('div', { class: 'hint', id: `${id}-${option.key}-hint` }, option.hint),
      ),
    ),
  );
  const form = createSettingsForm(id);
  const ownTitle = h('h3', null, 'Module\u2019s own settings');
  const noOwnSettings = h('p', { class: 'muted' }, 'This module has no settings of its own.');
  const own = h('section', { class: 'settings-part' }, ownTitle, noOwnSettings, form.el);
  const settings = h('details', { class: 'settings' }, h('summary', null, 'Module settings'), h('div', { class: 'settings-body' }, builtin, own));

  const el = h(
    'article',
    { class: 'card module' },
    h(
      'div',
      { class: 'module-top' },
      image,
      placeholder,
      h(
        'div',
        { class: 'module-main' },
        h('div', { class: 'module-title' }, prio, name, version, badgeSlot),
        author,
        description,
      ),
      h('div', { class: 'module-actions' }, enabledLabel, up, down, uninstall),
    ),
    h('div', { class: 'module-body' }, updateBanner, chipsSlot, usage, pageLink, error),
    settings,
  );

  /** @type {number | null} */
  let loadedImageCount = null;
  /** @type {string | null} */
  let settingsKey = null;
  /** @type {number | null} */
  let settingsRev = null;
  /** @type {string | null} */
  let pageUrl = null;
  /** @type {string | null} */
  let releaseUrl = null;
  /** @type {string[]} */
  let currentOrder = [];

  enabledInput.addEventListener('change', async () => {
    const wanted = enabledInput.checked;
    enabledInput.disabled = true;
    try {
      await api.setModuleEnabled(id, wanted);
      await refresh();
    } catch (e) {
      enabledInput.checked = !wanted;
      toast(errMsg(e), 'error');
    } finally {
      enabledInput.disabled = false;
    }
  });
  /** @param {number} delta */
  const move = (delta) => {
    const ids = [...currentOrder];
    const from = ids.indexOf(id);
    const to = from + delta;
    if (from < 0 || to < 0 || to >= ids.length) return Promise.resolve();
    ids.splice(to, 0, ...ids.splice(from, 1));
    return api.setModuleOrder(ids).then(() => refresh());
  };
  up.addEventListener('click', () => runButton(up, () => move(-1)));
  down.addEventListener('click', () => runButton(down, () => move(1)));
  pageLink.addEventListener('click', () => {
    if (pageUrl) api.openUrl(pageUrl).catch((e) => toast(errMsg(e), 'error'));
  });
  releaseLink.addEventListener('click', () => {
    if (releaseUrl) api.openUrl(releaseUrl).catch((e) => toast(errMsg(e), 'error'));
  });
  settings.addEventListener('toggle', () => {
    if (settings.open && !form.el.hidden) void form.load();
  });

  /** @param {number} count */
  async function loadImage(count) {
    loadedImageCount = count;
    if (count === 0) {
      image.hidden = true;
      placeholder.hidden = false;
      return;
    }
    try {
      const url = await api.moduleImage(id, 0);
      if (url) {
        image.src = url;
        image.hidden = false;
        placeholder.hidden = true;
      }
    } catch {
      loadedImageCount = null;
    }
  }

  return {
    el,
    /**
     * @param {ModuleView} m
     * @param {number} index
     * @param {string[]} order ids in priority order
     * @param {boolean} engineUp
     * @param {ModuleUpdate | undefined} update newer release at the module's source, if any
     */
    update(m, index, order, engineUp, update) {
      currentOrder = order;
      el.classList.toggle('disabled', !m.enabled);
      name.textContent = m.name;
      placeholder.textContent = (m.name.trim()[0] ?? '?').toUpperCase();
      version.textContent = m.version ? `v${m.version}` : '';
      prio.textContent = `#${index + 1}`;
      prio.title = 'Priority: earlier modules claim eye/face data first';
      badgeSlot.replaceChildren(moduleStateBadge(m));
      author.textContent = m.author ? `by ${m.author}` : '';
      author.hidden = !m.author;
      description.textContent = m.description ?? '';
      description.hidden = !m.description;
      usage.textContent = m.usage ?? '';
      usage.hidden = !m.usage;
      chipsSlot.replaceChildren(claimChips(m));
      error.textContent = m.lastError ?? '';
      error.hidden = !m.lastError;
      pageUrl = m.pageUrl && /^https?:\/\//i.test(m.pageUrl) ? m.pageUrl : null;
      pageLink.hidden = pageUrl === null;
      releaseUrl = update && /^https:\/\//i.test(update.releaseUrl) ? update.releaseUrl : null;
      updateBanner.hidden = !update;
      updateText.textContent = update ? `Update available: ${update.latestVersion}${m.version ? ` (installed v${m.version})` : ''}` : '';
      releaseLink.hidden = releaseUrl === null;
      releaseLink.setAttribute('aria-label', `View the ${m.name} release on ${m.sourceRepo?.forge === 'gitlab' ? 'GitLab' : 'GitHub'}`);
      enabledInput.checked = m.enabled;
      enabledInput.disabled = !engineUp;
      up.disabled = !engineUp || index === 0;
      down.disabled = !engineUp || index === order.length - 1;
      up.setAttribute('aria-label', `Move ${m.name} up (higher priority)`);
      down.setAttribute('aria-label', `Move ${m.name} down (lower priority)`);
      uninstall.setAttribute('aria-label', `Uninstall ${m.name}`);
      uninstall.disabled = !engineUp;
      enabledInput.setAttribute('aria-label', `Enable ${m.name}`);

      if (loadedImageCount !== m.imageCount) void loadImage(m.imageCount);

      options = m.options;
      for (const { option, input, number } of optionInputs) {
        input.checked = m.options[option.key];
        input.disabled = !engineUp;
        input.setAttribute('aria-label', `${option.label} (${m.name})`);
        if (number && option.number) {
          if (document.activeElement !== number) number.value = String(m.options[option.number.key]);
          number.disabled = !engineUp || !m.options[option.key];
          number.setAttribute('aria-label', `${option.number.label} (${m.name})`);
        }
      }
      ownTitle.textContent = `${m.name} settings`;
      noOwnSettings.hidden = m.hasSettings;
      form.el.hidden = !m.hasSettings;
      form.setRunning(engineUp && (m.state === 'running' || m.state === 'initializing'));
      const key = `${m.hasSettings}:${m.state}`;
      if (key !== settingsKey) {
        settingsKey = key;
        settingsRev = m.settingsRev;
        // The schema appears once the module has started; refresh an open form (unsaved edits are kept).
        if (m.hasSettings && settings.open) void form.load();
      } else if (m.settingsRev !== settingsRev) {
        settingsRev = m.settingsRev;
        // The module changed its values (e.g. a status text): update the open form in place.
        if (m.hasSettings && settings.open) void form.sync();
      }
    },
  };
}

/**
 * @param {HTMLElement} root
 * @returns {import('../main.js').PageHandle}
 */
export function mountModules(root) {
  const installZip = h('button', { type: 'button', role: 'menuitem' }, 'Install from VRCFT module .zip\u2026');
  const installFolder = h('button', { type: 'button', role: 'menuitem' }, 'Install from folder\u2026');
  const importVrcft = h('button', { type: 'button', role: 'menuitem' }, 'Import from VRCFaceTracking');
  const installMenu = h('div', { class: 'menu', role: 'menu', hidden: true }, installZip, installFolder, importVrcft);
  const install = h('button', { type: 'button', class: 'primary', 'aria-haspopup': 'menu', 'aria-expanded': 'false' }, 'Install \u25BE');
  const installDropdown = h('div', { class: 'dropdown' }, install, installMenu);
  /** @param {boolean} open */
  const setMenuOpen = (open) => {
    installMenu.hidden = !open;
    install.setAttribute('aria-expanded', String(open));
    if (open) installZip.focus();
  };
  install.addEventListener('click', () => setMenuOpen(installMenu.hidden));
  // Choosing an item, clicking elsewhere or pressing Escape closes the menu.
  installMenu.addEventListener('click', () => setMenuOpen(false));
  const onDocClick = (/** @type {MouseEvent} */ e) => {
    if (!installDropdown.contains(/** @type {Node} */ (e.target))) setMenuOpen(false);
  };
  document.addEventListener('click', onDocClick);
  installDropdown.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !installMenu.hidden) {
      setMenuOpen(false);
      install.focus();
    }
  });
  const restart = h('button', { type: 'button', class: 'primary' }, '\u21BB Restart modules');
  const notice = h('div', { class: 'banner warning', role: 'status', hidden: true });
  const list = h('div', { class: 'banner-list', 'aria-live': 'off' });
  const empty = h('div', { class: 'empty', hidden: true }, 'No modules installed. Install a module .zip, or import the modules you already use in VRCFaceTracking.');

  installZip.addEventListener('click', () =>
    runButton(installZip, async () => {
      const id = await api.installModule('zip');
      if (id) toast('Module installed', 'ok');
      await refresh();
    }),
  );
  installFolder.addEventListener('click', () =>
    runButton(installFolder, async () => {
      const id = await api.installModule('folder');
      if (id) toast('Module installed', 'ok');
      await refresh();
    }),
  );
  importVrcft.addEventListener('click', () =>
    runButton(importVrcft, async () => {
      const ids = await api.importVrcftModules();
      toast(ids.length ? `Imported ${ids.length} module${ids.length === 1 ? '' : 's'} from VRCFaceTracking` : 'No VRCFaceTracking modules found to import', ids.length ? 'ok' : 'info');
      await refresh();
    }),
  );
  restart.addEventListener('click', () => runButton(restart, async () => { await api.restartModules(); await refresh(); }, 'Modules restarting'));

  root.append(
    h(
      'div',
      { class: 'page' },
      h(
        'div',
        { class: 'page-head' },
        h('div', null, h('div', { class: 'title-row' }, h('h1', null, 'Modules'), installDropdown), h('p', { class: 'sub' }, 'Modules try to connect in the order they are listed.')),
        restart,
      ),
      notice,
      list,
      empty,
    ),
  );

  /** @type {Map<string, ReturnType<typeof createCard>>} */
  const cards = new Map();

  function update() {
    const engineUp = state.phase === 'running' && state.snapshot !== null;
    for (const b of [install, installZip, installFolder, importVrcft, restart]) b.disabled = !engineUp;
    if (!engineUp) setMenuOpen(false);
    notice.hidden = engineUp;
    notice.textContent = state.phase === 'starting' ? 'The engine is starting\u2026' : 'The engine is not running, so modules cannot be managed. See the Home page.';

    const modules = state.snapshot?.modules ?? [];
    const updates = new Map(state.moduleUpdates.map((u) => [u.moduleId, u]));
    const order = modules.map((m) => m.id);
    const seen = new Set(order);
    for (const [id, card] of cards) {
      if (!seen.has(id)) {
        card.el.remove();
        cards.delete(id);
      }
    }
    modules.forEach((m, i) => {
      let card = cards.get(m.id);
      if (!card) {
        card = createCard(m.id);
        cards.set(m.id, card);
      }
      card.update(m, i, order, engineUp, updates.get(m.id));
      if (list.children[i] !== card.el) list.insertBefore(card.el, list.children[i] ?? null);
    });
    empty.hidden = modules.length > 0;
  }

  return { update, destroy: () => document.removeEventListener('click', onDocClick) };
}
