// @ts-check
// Shapes of the JSON exchanged with the Rust backend (see spec/engine-api.md and src-tauri/src/view.rs).
// This file only carries JSDoc typedefs; it is imported for type checking and has no runtime content.

/** @typedef {{ eye: boolean, expression: boolean }} Claims */

/** @typedef {'stopped'|'starting'|'initializing'|'running'|'unclaimed'|'nodata'|'failed'|'exited'} ModuleRunState */

/**
 * @typedef {object} ModuleView
 * @property {string} id
 * @property {string} name
 * @property {string|null} version
 * @property {string|null} author
 * @property {string|null} description
 * @property {string|null} usage
 * @property {string|null} pageUrl
 * @property {{forge: 'github', path: string} | {forge: 'gitlab', host: string, path: string} | null} sourceRepo  where the module is released (GitHub or GitLab)
 * @property {ModuleOptions} options  options FaceBridge offers for every module
 * @property {boolean} enabled
 * @property {ModuleRunState} state
 * @property {Claims} supports
 * @property {Claims} claims
 * @property {number|null} status  VRCFT ModuleState: -1 uninitialized, 0 idle, 1 active
 * @property {string|null} lastError
 * @property {boolean} hasSettings
 * @property {number} settingsRev  changes whenever the module's setting values change
 * @property {number} imageCount
 */

/** @typedef {{ allowNotifications: boolean, requireData: boolean, dataTimeoutSecs: number }} ModuleOptions */

/**
 * @typedef {object} NotificationView
 * @property {number} id
 * @property {string} moduleId
 * @property {string} moduleName
 * @property {string} level
 * @property {string} key
 * @property {string} message
 * @property {number} timeMs
 */

/**
 * @typedef {object} LogEntry
 * @property {number} timeMs
 * @property {string|null} moduleId
 * @property {string} level
 * @property {string} category
 * @property {string} message
 */

/**
 * @typedef {object} AvatarInfo
 * @property {string} id
 * @property {string|null} name
 * @property {number} parameterCount
 * @property {number} matchedParameterCount
 * @property {string} source
 */

/**
 * @typedef {object} OscStatus
 * @property {string} sendAddr
 * @property {boolean} sending
 * @property {number} messagesPerSecond
 * @property {string|null} lastError
 * @property {boolean} vrchatDetected
 */

/** @typedef {{ sendAddr: string, listenPort: number, rateHz: number, oscquery: boolean }} OscConfig */

/**
 * @typedef {object} EngineSnapshot
 * @property {ModuleView[]} modules
 * @property {NotificationView[]} notifications
 * @property {AvatarInfo|null} avatar
 * @property {OscStatus} osc
 * @property {OscConfig} oscConfig
 * @property {LogEntry[]} logs
 */

/** @typedef {{ moduleHost: string|null, dotnetRoot: string|null }} HostPaths */

/** @typedef {'starting'|'running'|'failed'|'stopped'} Phase */

/**
 * @typedef {object} AppState
 * @property {Phase} phase
 * @property {string|null} error
 * @property {EngineSnapshot|null} snapshot
 * @property {string} dataDir
 * @property {string} version
 * @property {HostPaths} hostPaths
 * @property {OscConfig|null} oscConfig  persisted OSC settings (available even when the engine is down)
 * @property {string|null} configError  why config.json could not be read
 * @property {boolean} checkModuleUpdates  whether module sources are checked for new releases
 * @property {boolean} checkAppUpdates  whether OSCFT for VRC's own repository is checked for new releases
 * @property {AppUpdate|null} appUpdate  a newer OSCFT for VRC release than this build (null while the app check is off)
 * @property {ModuleUpdate[]} moduleUpdates  installed modules with a newer release (empty while the module check is off)
 * @property {boolean} closeToTray  whether closing the window hides it to the tray instead of quitting
 */

/**
 * @typedef {object} AppUpdate
 * @property {string} latestVersion  release tag as published, e.g. "v0.4.0"
 * @property {string} releaseUrl  the release's GitHub page
 */

/**
 * @typedef {object} ModuleUpdate
 * @property {string} moduleId
 * @property {string} latestVersion  release tag as published, e.g. "v0.7.0"
 * @property {string} releaseUrl
 */

/**
 * @typedef {object} EyeFrame
 * @property {number} gazeX
 * @property {number} gazeY
 * @property {number} openness
 * @property {number} pupilDiameterMm
 */

/**
 * @typedef {object} FrameView
 * @property {EyeFrame} left
 * @property {EyeFrame} right
 * @property {number[]} shapes
 * @property {{ yaw: number, pitch: number, roll: number }} head
 * @property {boolean} eyeClaimed
 * @property {boolean} expressionClaimed
 */

/**
 * @typedef {object} SettingsOption
 * @property {string} value
 * @property {string} label
 */

/**
 * @typedef {object} SettingsField
 * @property {string} key
 * @property {string} [label]
 * @property {string} [description]
 * @property {'int'|'float'|'bool'|'string'|'choice'|'text'|'button'} type  `text`: read-only display text written by the module; `button`: calls the module
 * @property {any} [default]
 * @property {number} [min]
 * @property {number} [max]
 * @property {number} [step]
 * @property {SettingsOption[]} [options]
 */

/** @typedef {{ schema: { fields: SettingsField[] }, values: Record<string, any> }} ModuleSettingsView */

/** @typedef {{ kind: 'modulesChanged'|'notificationsChanged'|'avatarChanged'|'oscStatusChanged'|'mutationChanged' } | { kind: 'log', entry: LogEntry } | { kind: 'notice', notification: NotificationView }} EngineEvent */

/** @typedef {{ enabled: boolean, minCutoff: number, beta: number, derivativeCutoff: number }} FilterChannel */
/** @typedef {{ min: number, max: number }} AdjustRange */

/**
 * @typedef {object} MutationConfig  see spec/mutation.md
 * @property {boolean} enabled
 * @property {{ enabled: boolean, blend: number, continuous: boolean }} calibration
 * @property {{ enabled: boolean, mouthClosedFix: boolean, lipSuckFix: boolean, eyeLidBlend: number, eyeLookSymmetrize: boolean }} correctors
 * @property {{ enabled: boolean, ranges: Record<string, AdjustRange> }} adjustment  missing groups mean 0..1
 * @property {{ enabled: boolean, eyeGaze: FilterChannel, eyeOpenness: FilterChannel, pupil: FilterChannel, expressions: FilterChannel, head: FilterChannel }} filter
 */

/** @typedef {{ shapes: { name: string, peak: number, progress: number }[] }} CalibrationData */

/** @typedef {{ key: string, label: string, isHead: boolean }} AdjustGroupInfo */

export {};
