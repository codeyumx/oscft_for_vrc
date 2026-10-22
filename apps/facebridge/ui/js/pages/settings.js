// @ts-check
// Settings: OSC output, module host / .NET locations, module update check, and where data lives.

import { api } from '../api.js';
import { errMsg, h, runButton, toast } from '../dom.js';
import { refresh, state } from '../store.js';

/** `host:port` with an IPv4 address, hostname-free (the backend parses a socket address). */
const ADDR_RE = /^(\d{1,3}(\.\d{1,3}){3}|\[[0-9a-fA-F:.]+\]):\d{1,5}$/;

/**
 * @param {string} id
 * @param {string} label
 * @param {HTMLElement} control
 * @param {string} [hint]
 */
function field(id, label, control, hint) {
  return h(
    'div',
    { class: 'field' },
    h('label', { for: id }, label),
    h('div', { class: 'control' }, control),
    hint ? h('div', { class: 'hint', id: `${id}-hint` }, hint) : null,
  );
}

/**
 * @param {HTMLElement} root
 * @returns {import('../main.js').PageHandle}
 */
export function mountSettings(root) {
  // ---- OSC ----
  const sendAddr = h('input', { type: 'text', id: 's-addr', autocomplete: 'off', placeholder: '127.0.0.1:9000', 'aria-describedby': 's-addr-hint s-addr-err' });
  const addrErr = h('div', { class: 'error', id: 's-addr-err', hidden: true });
  const listenPort = h('input', { type: 'number', id: 's-listen', min: 0, max: 65535, step: 1, 'aria-describedby': 's-listen-hint' });
  const rateHz = h('input', { type: 'number', id: 's-rate', min: 1, max: 240, step: 1 });
  const oscquery = h('input', { type: 'checkbox', id: 's-oscquery' });
  const oscquerySwitch = h('label', { class: 'switch' }, oscquery, h('span', { class: 'track', 'aria-hidden': 'true' }), h('span', null, 'Advertise and query through OSCQuery'));
  const oscSave = h('button', { type: 'submit', class: 'primary' }, 'Apply OSC settings');
  const oscDirtyInputs = [sendAddr, listenPort, rateHz, oscquery];
  let oscDirty = false;

  // ---- host ----
  const moduleHost = h('input', { type: 'text', id: 's-host', autocomplete: 'off', placeholder: 'Automatic', 'aria-describedby': 's-host-hint' });
  const browseHost = h('button', { type: 'button' }, 'Browse\u2026');
  const dotnetRoot = h('input', { type: 'text', id: 's-dotnet', autocomplete: 'off', placeholder: 'Automatic', 'aria-describedby': 's-dotnet-hint' });
  const browseDotnet = h('button', { type: 'button' }, 'Browse\u2026');
  const hostSave = h('button', { type: 'submit', class: 'primary' }, 'Save and restart engine');
  const hostDirtyInputs = [moduleHost, dotnetRoot];
  let hostDirty = false;

  // ---- module updates ----
  const checkUpdates = h('input', { type: 'checkbox', id: 's-updates', 'aria-describedby': 's-updates-hint' });
  const checkUpdatesSwitch = h('label', { class: 'switch', for: 's-updates' }, checkUpdates, h('span', { class: 'track', 'aria-hidden': 'true' }), h('span', null, 'Check for module updates'));
  let updatesSaving = false;

  const configError = h('div', { class: 'banner error', role: 'alert', hidden: true });
  const dataDir = h('dd', { class: 'mono' });
  const versionEl = h('dd', null);

  function validateAddr() {
    const value = sendAddr.value.trim();
    let message = '';
    if (!ADDR_RE.test(value)) message = 'Use an IP address and port, for example 127.0.0.1:9000.';
    else {
      const port = Number(value.slice(value.lastIndexOf(':') + 1));
      if (port < 1 || port > 65535) message = 'The port must be between 1 and 65535.';
    }
    addrErr.hidden = message === '';
    addrErr.textContent = message;
    sendAddr.setAttribute('aria-invalid', String(message !== ''));
    return message === '';
  }

  const oscForm = h(
    'form',
    { class: 'form card', novalidate: true, 'aria-labelledby': 'osc-set-h' },
    h('h2', { id: 'osc-set-h' }, 'OSC output'),
    h('div', { class: 'field' }, h('label', { for: 's-addr' }, 'Send to'), h('div', { class: 'control' }, sendAddr), h('div', { class: 'hint', id: 's-addr-hint' }, 'Where VRChat listens for OSC. VRChat uses 127.0.0.1:9000 unless you changed it.'), addrErr),
    field('s-listen', 'Listen port', listenPort, 'UDP port for legacy avatar-change messages and replies. 0 = rely on OSCQuery only.'),
    field('s-rate', 'Output rate (Hz)', rateHz, 'How often tracking data is sent to VRChat.'),
    h('div', { class: 'field' }, h('span', { class: 'label' }, 'Discovery'), h('div', { class: 'control' }, oscquerySwitch), h('div', { class: 'hint' }, 'Lets VRChat and FaceBridge find each other automatically and read the avatar parameter list.')),
    h('div', { class: 'form-actions' }, oscSave),
  );

  const hostForm = h(
    'form',
    { class: 'form card', novalidate: true, 'aria-labelledby': 'host-set-h' },
    h('h2', { id: 'host-set-h' }, 'Module host'),
    h('p', { class: 'muted' }, 'Modules run in a separate .NET process. Leave these empty to let FaceBridge find them.'),
    h('div', { class: 'field' }, h('label', { for: 's-host' }, 'Module host executable'), h('div', { class: 'control' }, moduleHost, browseHost), h('div', { class: 'hint', id: 's-host-hint' }, 'FaceBridge.ModuleHost.exe. Automatic: FACEBRIDGE_MODULE_HOST, the module-host folder next to the app, then the development build.')),
    h('div', { class: 'field' }, h('label', { for: 's-dotnet' }, '.NET installation folder'), h('div', { class: 'control' }, dotnetRoot, browseDotnet), h('div', { class: 'hint', id: 's-dotnet-hint' }, 'Passed to the module host as DOTNET_ROOT. Only needed when .NET 10 is not installed system-wide.')),
    h('div', { class: 'form-actions' }, hostSave, h('span', { class: 'muted small' }, 'Restarts the engine and all modules.')),
  );

  const updatesCard = h(
    'section',
    { class: 'form card', 'aria-labelledby': 'updates-set-h' },
    h('h2', { id: 'updates-set-h' }, 'Module updates'),
    h('div', { class: 'field' }, h('span', { class: 'label' }, 'Updates'), h('div', { class: 'control' }, checkUpdatesSwitch), h('div', { class: 'hint', id: 's-updates-hint' }, 'Asks GitHub for the latest release of each module that is published there, and shows "Update available" on the Modules page. Checked at startup, after installs, and every 6 hours.')),
  );

  oscForm.addEventListener('submit', (e) => e.preventDefault());
  hostForm.addEventListener('submit', (e) => e.preventDefault());
  for (const input of oscDirtyInputs) input.addEventListener('input', () => (oscDirty = true));
  for (const input of oscDirtyInputs) input.addEventListener('change', () => (oscDirty = true));
  for (const input of hostDirtyInputs) input.addEventListener('input', () => (hostDirty = true));
  sendAddr.addEventListener('input', validateAddr);

  oscSave.addEventListener('click', () => {
    if (!validateAddr()) {
      sendAddr.focus();
      return;
    }
    const rate = Number(rateHz.value);
    const port = Number(listenPort.value);
    if (!Number.isInteger(rate) || rate < 1) return toast('The output rate must be a whole number of at least 1 Hz.', 'error');
    if (!Number.isInteger(port) || port < 0 || port > 65535) return toast('The listen port must be between 0 and 65535.', 'error');
    void runButton(
      oscSave,
      async () => {
        await api.setOscConfig({ sendAddr: sendAddr.value.trim(), listenPort: port, rateHz: rate, oscquery: oscquery.checked });
        oscDirty = false;
        await refresh();
      },
      'OSC settings applied',
    );
  });

  hostSave.addEventListener('click', () => {
    void runButton(
      hostSave,
      async () => {
        await api.setHostPaths({ moduleHost: moduleHost.value.trim() || null, dotnetRoot: dotnetRoot.value.trim() || null });
        hostDirty = false;
        await refresh(true);
      },
      'Settings saved; the engine is restarting',
    );
  });

  checkUpdates.addEventListener('change', async () => {
    const enabled = checkUpdates.checked;
    updatesSaving = true;
    checkUpdates.disabled = true;
    try {
      await api.setCheckModuleUpdates(enabled);
      toast(enabled ? 'Module update check turned on' : 'Module update check turned off', 'ok');
    } catch (e) {
      checkUpdates.checked = !enabled;
      toast(errMsg(e), 'error');
    } finally {
      updatesSaving = false;
      checkUpdates.disabled = false;
    }
    await refresh();
  });

  /**
   * @param {HTMLInputElement} input
   * @param {boolean} folder
   * @param {string} title
   */
  async function browse(input, folder, title) {
    try {
      const picked = await api.pickPath(folder, title);
      if (picked) {
        input.value = picked;
        hostDirty = true;
      }
    } catch (e) {
      toast(errMsg(e), 'error');
    }
  }
  browseHost.addEventListener('click', () => browse(moduleHost, false, 'Select FaceBridge.ModuleHost.exe'));
  browseDotnet.addEventListener('click', () => browse(dotnetRoot, true, 'Select the .NET installation folder'));

  root.append(
    h(
      'div',
      { class: 'page' },
      h('div', { class: 'page-head' }, h('div', null, h('h1', null, 'Settings'))),
      configError,
      oscForm,
      hostForm,
      updatesCard,
      h('section', { class: 'card', 'aria-labelledby': 'about-h' }, h('h2', { id: 'about-h' }, 'About'), h('dl', { class: 'kv' }, h('dt', null, 'Version'), versionEl, h('dt', null, 'Data folder'), dataDir)),
    ),
  );

  let filled = false;

  function update() {
    versionEl.textContent = state.version;
    dataDir.textContent = state.dataDir;
    configError.hidden = !state.configError;
    configError.textContent = state.configError ? `config.json could not be read: ${state.configError}` : '';
    oscSave.disabled = state.oscConfig === null;

    // Fill the forms from the backend, but never overwrite what the user is typing.
    const osc = state.snapshot?.oscConfig ?? state.oscConfig;
    if (osc && (!oscDirty || !filled)) {
      sendAddr.value = osc.sendAddr;
      listenPort.value = String(osc.listenPort);
      rateHz.value = String(osc.rateHz);
      oscquery.checked = osc.oscquery;
      validateAddr();
    }
    if (!hostDirty || !filled) {
      moduleHost.value = state.hostPaths.moduleHost ?? '';
      dotnetRoot.value = state.hostPaths.dotnetRoot ?? '';
    }
    if (!updatesSaving) checkUpdates.checked = state.checkModuleUpdates;
    if (osc || state.snapshot) filled = true;
  }

  return { update };
}
