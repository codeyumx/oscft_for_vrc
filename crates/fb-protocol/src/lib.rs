//! Wire protocol between the FaceBridge host and `FaceBridge.ModuleHost` (`spec/ipc-protocol.md`).
//!
//! Blocking encode/decode over any `Read`/`Write`. Both directions are implemented so tests and fake
//! children can speak the child side.

mod codec;

pub use codec::{
    encode_child_message, encode_frame, encode_host_message, read_child_message, read_host_message,
    write_child_message, write_host_message,
};
use fb_tracking::TrackingFrame;
use std::io;

pub const PROTOCOL_VERSION: u16 = 1;

/// Largest accepted message length (type byte + payload), 16 MiB.
pub const MAX_MESSAGE_LEN: u32 = 16 * 1024 * 1024;

/// `Microsoft.Extensions.Logging.LogLevel` numbering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Information = 2,
    Warning = 3,
    Error = 4,
    Critical = 5,
}

impl LogLevel {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Trace,
            1 => Self::Debug,
            2 => Self::Information,
            3 => Self::Warning,
            4 => Self::Error,
            5 => Self::Critical,
            _ => return None,
        })
    }

    /// Lower-case name as shown in the UI (`"information"`, `"warning"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Information => "information",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }
}

/// Frame flag bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameFlags(pub u8);

impl FrameFlags {
    /// Eye data valid (the module claimed eye).
    pub const EYE: Self = Self(1);
    /// Expression data valid.
    pub const EXPRESSION: Self = Self(2);
    /// Head data valid (any head field non-zero).
    pub const HEAD: Self = Self(4);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for FrameFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameMessage {
    pub seq: u64,
    pub flags: FrameFlags,
    pub frame: TrackingFrame,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChildMessage {
    Hello { version: u16, token: String, pid: u32 },
    Loaded { module_type: String, supports_eye: bool, supports_expression: bool },
    LoadFailed { reason: String },
    InitResult { eye: bool, expression: bool, name: String, images: Vec<Vec<u8>> },
    Frame(Box<FrameMessage>),
    Log { level: LogLevel, event_id: i32, category: String, message: String, exception: String },
    /// Informational pop-up; never stored.
    Notify { level: LogLevel, key: String, message: String },
    /// A message: stays on the Home page until the user dismisses it or the module clears it.
    NotifyMessage { level: LogLevel, key: String, message: String },
    /// Removes stored messages (`NotifyMessage`) by key; empty = all of this module's.
    NotifyClear { key: String },
    Status { state: i8 },
    SettingsSchema { json: String },
    SettingsWrite { json: String },
    TeardownDone,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HostMessage {
    Init { eye_available: bool, expression_available: bool },
    Settings { json: String },
    Teardown,
    /// The user pressed the settings button `key`.
    SettingsAction { key: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The peer closed the connection cleanly between messages.
    #[error("connection closed")]
    Closed,
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("message length {0} exceeds the {MAX_MESSAGE_LEN} byte limit")]
    TooLarge(u64),
    #[error("unknown message type 0x{0:02x}")]
    UnknownType(u8),
    #[error("malformed message: {0}")]
    Malformed(&'static str),
}

#[cfg(test)]
mod tests;
