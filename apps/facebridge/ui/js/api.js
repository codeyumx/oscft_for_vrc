// @ts-check
// Typed wrappers over the Tauri commands registered in src-tauri/src/lib.rs.

/** @typedef {import('./types.js').AppState} AppState */
/** @typedef {import('./types.js').FrameView} FrameView */
/** @typedef {import('./types.js').ModuleSettingsView} ModuleSettingsView */
/** @typedef {import('./types.js').OscConfig} OscConfig */
/** @typedef {import('./types.js').HostPaths} HostPaths */
/** @typedef {import('./types.js').MutationConfig} MutationConfig */
/** @typedef {import('./types.js').CalibrationData} CalibrationData */
/** @typedef {import('./types.js').AdjustGroupInfo} AdjustGroupInfo */

/** @type {any} */
const tauri = /** @type {any} */ (window).__TAURI__;

if (!tauri) {
  throw new Error('window.__TAURI__ is missing: this UI must run inside the OSCFT for VRC desktop app.');
}

/**
 * @template T
 * @param {string} command
 * @param {Record<string, unknown>} [args]
 * @returns {Promise<T>}
 */
const invoke = (command, args) => tauri.core.invoke(command, args);

/**
 * Subscribes to a backend event; resolves to an unlisten function.
 * @template T
 * @param {string} event
 * @param {(payload: T) => void} handler
 * @returns {Promise<() => void>}
 */
export const listen = (event, handler) => tauri.event.listen(event, (/** @type {{ payload: T }} */ e) => handler(e.payload));

export const api = {
  /** @returns {Promise<AppState>} */
  getState: () => invoke('get_state'),
  /** @returns {Promise<string[]>} */
  expressionNames: () => invoke('expression_names_list'),
  /**
   * Opens a file/folder dialog and installs the picked module.
   * @param {'zip'|'folder'} kind
   * @returns {Promise<string|null>} new module id, or null when cancelled
   */
  installModule: (kind) => invoke('install_module', { kind }),
  /** @returns {Promise<string[]>} */
  importVrcftModules: () => invoke('import_vrcft_modules'),
  /** @param {string} id */
  uninstallModule: (id) => invoke('uninstall_module', { id }),
  /**
   * @param {string} id
   * @param {boolean} enabled
   */
  setModuleEnabled: (id, enabled) => invoke('set_module_enabled', { id, enabled }),
  /** @param {string[]} ids */
  setModuleOrder: (ids) => invoke('set_module_order', { ids }),
  restartModules: () => invoke('restart_modules'),
  /**
   * @param {string} id
   * @returns {Promise<ModuleSettingsView|null>}
   */
  getModuleSettings: (id) => invoke('get_module_settings', { id }),
  /**
   * @param {string} id
   * @param {Record<string, any>} values
   */
  setModuleSettings: (id, values) => invoke('set_module_settings', { id, values }),
  /**
   * @param {string} id
   * @param {import('./types.js').ModuleOptions} options
   */
  setModuleOptions: (id, options) => invoke('set_module_options', { id, options }),
  /**
   * Tells the running module that the user pressed its settings button `key`.
   * @param {string} id
   * @param {string} key
   */
  pressModuleButton: (id, key) => invoke('press_module_button', { id, key }),
  /**
   * @param {string} id
   * @param {number} index
   * @returns {Promise<string|null>} data URL
   */
  moduleImage: (id, index) => invoke('module_image', { id, index }),
  /** @param {number} id */
  dismissNotification: (id) => invoke('dismiss_notification', { id }),
  /** @returns {Promise<FrameView>} */
  latestFrame: () => invoke('latest_frame'),
  /** @returns {Promise<FrameView>} the merged input before calibration, correction, adjustment and filtering */
  latestRawFrame: () => invoke('latest_raw_frame'),
  /** @param {boolean} raw  whether pushed `frame` events carry the raw input instead of the output */
  setLiveRaw: (raw) => invoke('set_live_raw', { raw }),
  /** @returns {Promise<MutationConfig>} */
  getMutationConfig: () => invoke('get_mutation_config'),
  /**
   * @param {MutationConfig} config
   * @returns {Promise<MutationConfig>} the sanitized config
   */
  setMutationConfig: (config) => invoke('set_mutation_config', { config }),
  /** @returns {Promise<CalibrationData>} */
  getCalibration: () => invoke('get_calibration'),
  /** @param {string|null} shape  Expression name, or null for all shapes */
  resetCalibration: (shape) => invoke('reset_calibration', { shape }),
  /** @returns {Promise<AdjustGroupInfo[]>} */
  getAdjustGroups: () => invoke('get_adjust_groups'),
  /** @param {boolean} live */
  setLive: (live) => invoke('set_live', { live }),
  /** @param {OscConfig} osc */
  setOscConfig: (osc) => invoke('set_osc_config', { osc }),
  /** @param {HostPaths} paths */
  setHostPaths: (paths) => invoke('set_host_paths', { paths }),
  /** @param {boolean} enabled */
  setCheckModuleUpdates: (enabled) => invoke('set_check_module_updates', { enabled }),
  /** @param {boolean} enabled */
  setCheckAppUpdates: (enabled) => invoke('set_check_app_updates', { enabled }),
  /** @param {boolean} enabled */
  setCloseToTray: (enabled) => invoke('set_close_to_tray', { enabled }),
  restartEngine: () => invoke('restart_engine'),
  /**
   * @param {boolean} folder
   * @param {string} title
   * @returns {Promise<string|null>}
   */
  pickPath: (folder, title) => invoke('pick_path', { folder, title }),
  /**
   * Opens an http(s) URL in the default browser.
   * @param {string} url
   */
  openUrl: (url) => invoke('plugin:opener|open_url', { url }),
};
