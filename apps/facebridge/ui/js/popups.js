// @ts-check
// Pop-ups for what modules need the user to see, on every page: their notices (informational, only ever shown
// here), their messages (also kept on Home until dismissed), their error log entries, and modules that fail with
// an error. Notifications and error logs respect the module's "Allow notifications" option (the engine already
// drops notifications from muted modules); failures always show.

import { popup } from './dom.js';
import { onLog, onNotice, onState, state } from './store.js';

/** @typedef {import('./types.js').LogEntry} LogEntry */

/** On the first snapshot, notifications younger than this still pop up (they arrived while the UI loaded). */
const RECENT_MS = 30_000;
/** The same message from the same module pops up at most once in this window (a log line and its notification). */
const DEDUPE_MS = 60_000;
/** Long enough to read; a hovered pop-up stays. */
const DURATION_MS = /** @type {Record<string, number>} */ ({ information: 6000, warning: 10000, error: 15000, critical: 15000 });
const MAX_MESSAGE = 400;

/** Keys of messages already shown (ids restart with each engine, so the key includes time and module). */
const seenNotifications = new Set();
/** @type {Map<string, string>} module id -> last run state */
const lastStates = new Map();
/** @type {Map<string, number>} `${moduleId}\n${message}` -> when it was shown */
const recent = new Map();
let primed = false;

/**
 * @param {string} moduleId
 * @param {string} title
 * @param {string} level
 * @param {string} message
 */
function show(moduleId, title, level, message) {
  const now = Date.now();
  for (const [key, at] of recent) if (now - at > DEDUPE_MS) recent.delete(key);
  const key = `${moduleId}\n${message}`;
  if (recent.has(key)) return;
  recent.set(key, now);
  const text = message.length > MAX_MESSAGE ? `${message.slice(0, MAX_MESSAGE)}\u2026` : message;
  popup({ title, message: text, kind: level, durationMs: DURATION_MS[level] ?? 8000 });
}

function onSnapshot() {
  const snap = state.snapshot;
  if (!snap) return;
  const now = Date.now();
  for (const n of snap.notifications) {
    const key = `${n.moduleId}\n${n.id}\n${n.timeMs}`;
    if (seenNotifications.has(key)) continue;
    seenNotifications.add(key);
    if (primed || now - n.timeMs < RECENT_MS) show(n.moduleId, n.moduleName, n.level.toLowerCase(), n.message);
  }
  for (const m of snap.modules) {
    const previous = lastStates.get(m.id);
    lastStates.set(m.id, m.state);
    if (primed && previous !== undefined && previous !== m.state && m.state === 'failed' && m.lastError) {
      show(m.id, `${m.name} stopped`, 'error', m.lastError);
    }
  }
  primed = true;
}

/** @param {LogEntry} entry */
function onLogEntry(entry) {
  const level = entry.level.toLowerCase();
  if (!entry.moduleId || (level !== 'error' && level !== 'critical')) return;
  // Failures are shown from the module state, muted notifications stay in the log.
  if (entry.category === 'engine' || entry.category === 'notification') return;
  const m = state.snapshot?.modules.find((x) => x.id === entry.moduleId);
  if (m && !m.options.allowNotifications) return;
  // The first line is the message; an exception's stack trace stays in the Output log.
  show(entry.moduleId, m?.name ?? 'Module', level, entry.message.split('\n', 1)[0]);
}

export function initPopups() {
  onState(onSnapshot);
  onLog(onLogEntry);
  onNotice((n) => show(n.moduleId, n.moduleName, n.level.toLowerCase(), n.message));
}
