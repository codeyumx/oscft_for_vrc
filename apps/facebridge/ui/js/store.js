// @ts-check
// Single source of truth for the UI: the latest backend snapshot, the log buffer and the live frame.
// Pages subscribe and re-render (or update in place) when it changes.

import { api, listen } from './api.js';
import { errMsg } from './dom.js';

/** @typedef {import('./types.js').AppState} AppState */
/** @typedef {import('./types.js').LogEntry} LogEntry */
/** @typedef {import('./types.js').FrameView} FrameView */
/** @typedef {import('./types.js').EngineEvent} EngineEvent */

const MAX_LOGS = 1000;
/** Coalesces bursts of engine events into one snapshot fetch. */
const REFRESH_DELAY_MS = 120;

/** @type {AppState} */
export const state = {
  phase: 'starting',
  error: null,
  snapshot: null,
  dataDir: '',
  version: '',
  hostPaths: { moduleHost: null, dotnetRoot: null },
  oscConfig: null,
  configError: null,
  checkModuleUpdates: true,
  checkAppUpdates: true,
  appUpdate: null,
  moduleUpdates: [],
  closeToTray: true,
};

/** @type {LogEntry[]} */
export const logs = [];
/** @type {string[]} */
export let expressionNames = [];
/** @type {FrameView | null} */
export let lastFrame = null;
/** Set when the backend could not be reached (distinct from the engine failing to start). */
export let backendError = /** @type {string | null} */ (null);

/** @type {Set<() => void>} */
const stateListeners = new Set();
/** @type {Set<(entry: LogEntry) => void>} */
const logListeners = new Set();
/** @type {Set<(frame: FrameView) => void>} */
const frameListeners = new Set();
/** @type {Set<() => void>} */
const logResetListeners = new Set();
/** @type {Set<(n: import('./types.js').NotificationView) => void>} */
const noticeListeners = new Set();

/**
 * @template {Set<any>} S
 * @param {S} set
 * @param {S extends Set<infer F> ? F : never} fn
 * @returns {() => void}
 */
function add(set, fn) {
  set.add(fn);
  return () => void set.delete(fn);
}

export const onState = (/** @type {() => void} */ fn) => add(stateListeners, fn);
export const onLog = (/** @type {(entry: LogEntry) => void} */ fn) => add(logListeners, fn);
export const onLogReset = (/** @type {() => void} */ fn) => add(logResetListeners, fn);
export const onFrame = (/** @type {(frame: FrameView) => void} */ fn) => add(frameListeners, fn);
/** Informational module notifications: delivered once, never part of the snapshot. */
export const onNotice = (/** @type {(n: import('./types.js').NotificationView) => void} */ fn) => add(noticeListeners, fn);

function emitState() {
  for (const fn of [...stateListeners]) fn();
}

/** @param {import('./types.js').EngineSnapshot} snapshot */
function seedLogs(snapshot) {
  logs.length = 0;
  logs.push(...snapshot.logs.slice(-MAX_LOGS));
  for (const fn of [...logResetListeners]) fn();
}

/**
 * Fetches the backend state. `reseedLogs` replaces the log buffer with the engine's ring buffer; used on the
 * first fetch and whenever a new engine has just come up (within one engine, log events append).
 * @param {boolean} [reseedLogs]
 */
export async function refresh(reseedLogs = false) {
  try {
    const next = await api.getState();
    backendError = null;
    const firstSnapshot = state.snapshot === null;
    Object.assign(state, next);
    if (next.snapshot && (reseedLogs || firstSnapshot)) seedLogs(next.snapshot);
  } catch (e) {
    backendError = errMsg(e);
  }
  emitState();
}

/** @type {number | undefined} */
let refreshTimer;
export function scheduleRefresh() {
  if (refreshTimer !== undefined) return;
  refreshTimer = setTimeout(() => {
    refreshTimer = undefined;
    void refresh();
  }, REFRESH_DELAY_MS);
}

/** @param {LogEntry} entry */
function pushLog(entry) {
  logs.push(entry);
  if (logs.length > MAX_LOGS) logs.splice(0, logs.length - MAX_LOGS);
  for (const fn of [...logListeners]) fn(entry);
}

export async function init() {
  await Promise.all([
    listen('engine-state', () => void refresh(true)),
    listen(
      'engine-event',
      /** @param {EngineEvent} event */ (event) => {
        if (event.kind === 'log') pushLog(event.entry);
        else if (event.kind === 'notice') for (const fn of [...noticeListeners]) fn(event.notification);
        else scheduleRefresh();
      },
    ),
    listen('updates', () => scheduleRefresh()),
    listen(
      'frame',
      /** @param {FrameView} frame */ (frame) => {
        lastFrame = frame;
        for (const fn of [...frameListeners]) fn(frame);
      },
    ),
  ]);
  try {
    expressionNames = await api.expressionNames();
  } catch (e) {
    backendError = errMsg(e);
  }
  await refresh();
}
