use crate::{
    ChildMessage, FrameFlags, FrameMessage, HostMessage, LogLevel, MAX_MESSAGE_LEN, ProtocolError,
};
use fb_tracking::{Eyes, Head, SHAPE_COUNT, SingleEye, TrackingFrame};
use std::io::{self, Read, Write};

// Child -> host.
const HELLO: u8 = 0x01;
const LOADED: u8 = 0x02;
const LOAD_FAILED: u8 = 0x03;
const INIT_RESULT: u8 = 0x04;
const FRAME: u8 = 0x05;
const LOG: u8 = 0x06;
const NOTIFY: u8 = 0x07;
const NOTIFY_CLEAR: u8 = 0x08;
const STATUS: u8 = 0x09;
const SETTINGS_SCHEMA: u8 = 0x0A;
const SETTINGS_WRITE: u8 = 0x0B;
const TEARDOWN_DONE: u8 = 0x0C;
const NOTIFY_MESSAGE: u8 = 0x0D;
// Host -> child.
const INIT: u8 = 0x81;
const SETTINGS: u8 = 0x82;
const TEARDOWN: u8 = 0x83;
const SETTINGS_ACTION: u8 = 0x84;

// ---------------------------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------------------------

/// Appends one complete message (length prefix included) to `out`. On error `out` is left unchanged.
struct Enc<'a> {
    out: &'a mut Vec<u8>,
    start: usize,
}

impl<'a> Enc<'a> {
    fn begin(out: &'a mut Vec<u8>, ty: u8) -> Self {
        let start = out.len();
        out.extend_from_slice(&[0; 4]);
        out.push(ty);
        Self { out, start }
    }

    fn finish(self) -> Result<(), ProtocolError> {
        let len = self.out.len() - self.start - 4;
        if len as u64 > u64::from(MAX_MESSAGE_LEN) {
            self.out.truncate(self.start);
            return Err(ProtocolError::TooLarge(len as u64));
        }
        self.out[self.start..self.start + 4].copy_from_slice(&(len as u32).to_le_bytes());
        Ok(())
    }

    fn u8(&mut self, v: u8) -> &mut Self {
        self.out.push(v);
        self
    }

    fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(u8::from(v))
    }

    fn u16(&mut self, v: u16) -> &mut Self {
        self.out.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn u32(&mut self, v: u32) -> &mut Self {
        self.out.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn i32(&mut self, v: i32) -> &mut Self {
        self.out.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn u64(&mut self, v: u64) -> &mut Self {
        self.out.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn f32(&mut self, v: f32) -> &mut Self {
        self.out.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn bytes(&mut self, v: &[u8]) -> &mut Self {
        // Lengths beyond u32 are impossible to represent; `finish` would reject them anyway.
        self.u32(u32::try_from(v.len()).unwrap_or(u32::MAX));
        self.out.extend_from_slice(v);
        self
    }

    fn str(&mut self, v: &str) -> &mut Self {
        self.bytes(v.as_bytes())
    }
}

/// Encodes a child → host message, appending to `out`.
pub fn encode_child_message(m: &ChildMessage, out: &mut Vec<u8>) -> Result<(), ProtocolError> {
    match m {
        ChildMessage::Hello { version, token, pid } => {
            let mut e = Enc::begin(out, HELLO);
            e.u16(*version).str(token).u32(*pid);
            e.finish()
        }
        ChildMessage::Loaded { module_type, supports_eye, supports_expression } => {
            let mut e = Enc::begin(out, LOADED);
            e.str(module_type).bool(*supports_eye).bool(*supports_expression);
            e.finish()
        }
        ChildMessage::LoadFailed { reason } => {
            let mut e = Enc::begin(out, LOAD_FAILED);
            e.str(reason);
            e.finish()
        }
        ChildMessage::InitResult { eye, expression, name, images } => {
            let count = u16::try_from(images.len())
                .map_err(|_| ProtocolError::Malformed("too many images"))?;
            let mut e = Enc::begin(out, INIT_RESULT);
            e.bool(*eye).bool(*expression).str(name).u16(count);
            for image in images {
                e.bytes(image);
            }
            e.finish()
        }
        ChildMessage::Frame(f) => encode_frame(f, SHAPE_COUNT, out),
        ChildMessage::Log { level, event_id, category, message, exception } => {
            let mut e = Enc::begin(out, LOG);
            e.u8(*level as u8).i32(*event_id).str(category).str(message).str(exception);
            e.finish()
        }
        ChildMessage::Notify { level, key, message } => {
            let mut e = Enc::begin(out, NOTIFY);
            e.u8(*level as u8).str(key).str(message);
            e.finish()
        }
        ChildMessage::NotifyMessage { level, key, message } => {
            let mut e = Enc::begin(out, NOTIFY_MESSAGE);
            e.u8(*level as u8).str(key).str(message);
            e.finish()
        }
        ChildMessage::NotifyClear { key } => {
            let mut e = Enc::begin(out, NOTIFY_CLEAR);
            e.str(key);
            e.finish()
        }
        ChildMessage::Status { state } => {
            let mut e = Enc::begin(out, STATUS);
            e.u8(*state as u8);
            e.finish()
        }
        ChildMessage::SettingsSchema { json } => {
            let mut e = Enc::begin(out, SETTINGS_SCHEMA);
            e.str(json);
            e.finish()
        }
        ChildMessage::SettingsWrite { json } => {
            let mut e = Enc::begin(out, SETTINGS_WRITE);
            e.str(json);
            e.finish()
        }
        ChildMessage::TeardownDone => Enc::begin(out, TEARDOWN_DONE).finish(),
    }
}

/// Encodes a `Frame` carrying only the first `shape_count` shapes (`ChildMessage::Frame` always sends all
/// [`SHAPE_COUNT`]; real children may send a different count, which the decoder normalizes).
pub fn encode_frame(
    m: &FrameMessage,
    shape_count: usize,
    out: &mut Vec<u8>,
) -> Result<(), ProtocolError> {
    let count = u16::try_from(shape_count)
        .ok()
        .filter(|_| shape_count <= SHAPE_COUNT)
        .ok_or(ProtocolError::Malformed("shape count out of range"))?;
    let f = &m.frame;
    let mut e = Enc::begin(out, FRAME);
    e.u64(m.seq).u8(m.flags.bits());
    for eye in [&f.eye.left, &f.eye.right] {
        e.f32(eye.gaze_x).f32(eye.gaze_y).f32(eye.openness).f32(eye.pupil_diameter_mm);
    }
    e.f32(f.eye.max_dilation).f32(f.eye.min_dilation).f32(f.eye.left_diameter).f32(f.eye.right_diameter);
    e.f32(f.head.yaw).f32(f.head.pitch).f32(f.head.roll);
    e.f32(f.head.pos_x).f32(f.head.pos_y).f32(f.head.pos_z);
    e.u16(count);
    for s in &f.shapes[..shape_count] {
        e.f32(*s);
    }
    e.finish()
}

/// Encodes a host → child message, appending to `out`.
pub fn encode_host_message(m: &HostMessage, out: &mut Vec<u8>) -> Result<(), ProtocolError> {
    match m {
        HostMessage::Init { eye_available, expression_available } => {
            let mut e = Enc::begin(out, INIT);
            e.bool(*eye_available).bool(*expression_available);
            e.finish()
        }
        HostMessage::Settings { json } => {
            let mut e = Enc::begin(out, SETTINGS);
            e.str(json);
            e.finish()
        }
        HostMessage::Teardown => Enc::begin(out, TEARDOWN).finish(),
        HostMessage::SettingsAction { key } => {
            let mut e = Enc::begin(out, SETTINGS_ACTION);
            e.str(key);
            e.finish()
        }
    }
}

fn into_io(e: ProtocolError) -> io::Error {
    match e {
        ProtocolError::Io(e) => e,
        other => io::Error::new(io::ErrorKind::InvalidInput, other),
    }
}

pub fn write_host_message(w: &mut impl Write, m: &HostMessage) -> io::Result<()> {
    let mut buf = Vec::with_capacity(64);
    encode_host_message(m, &mut buf).map_err(into_io)?;
    w.write_all(&buf)
}

pub fn write_child_message(w: &mut impl Write, m: &ChildMessage) -> io::Result<()> {
    let mut buf = Vec::with_capacity(64);
    encode_child_message(m, &mut buf).map_err(into_io)?;
    w.write_all(&buf)
}

// ---------------------------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------------------------

struct Dec<'a> {
    buf: &'a [u8],
}

impl<'a> Dec<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ProtocolError> {
        if self.buf.len() < n {
            return Err(ProtocolError::Malformed("truncated payload"));
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        let mut a = [0; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.array::<1>()?[0])
    }

    fn bool(&mut self) -> Result<bool, ProtocolError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtocolError::Malformed("invalid bool")),
        }
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, ProtocolError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, ProtocolError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn bytes(&mut self) -> Result<&'a [u8], ProtocolError> {
        let n = self.u32()? as usize;
        self.take(n)
    }

    fn str(&mut self) -> Result<String, ProtocolError> {
        std::str::from_utf8(self.bytes()?)
            .map(str::to_owned)
            .map_err(|_| ProtocolError::Malformed("invalid UTF-8"))
    }

    fn level(&mut self) -> Result<LogLevel, ProtocolError> {
        LogLevel::from_u8(self.u8()?).ok_or(ProtocolError::Malformed("invalid log level"))
    }

    fn end(&self) -> Result<(), ProtocolError> {
        if self.buf.is_empty() { Ok(()) } else { Err(ProtocolError::Malformed("trailing bytes")) }
    }
}

/// Reads one message body into `buf`; returns the type byte (payload = `buf[1..]`).
fn read_body(r: &mut impl Read, buf: &mut Vec<u8>) -> Result<u8, ProtocolError> {
    let mut header = [0u8; 4];
    let mut got = 0;
    while got < header.len() {
        match r.read(&mut header[got..]) {
            Ok(0) if got == 0 => return Err(ProtocolError::Closed),
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let len = u32::from_le_bytes(header);
    if len == 0 {
        return Err(ProtocolError::Malformed("empty message"));
    }
    if len > MAX_MESSAGE_LEN {
        return Err(ProtocolError::TooLarge(u64::from(len)));
    }
    buf.clear();
    buf.resize(len as usize, 0);
    r.read_exact(buf)?;
    Ok(buf[0])
}

/// Blocking read of one child → host message. `buf` is a reusable scratch buffer.
pub fn read_child_message(
    r: &mut impl Read,
    buf: &mut Vec<u8>,
) -> Result<ChildMessage, ProtocolError> {
    let ty = read_body(r, buf)?;
    let mut d = Dec { buf: &buf[1..] };
    let m = match ty {
        HELLO => ChildMessage::Hello { version: d.u16()?, token: d.str()?, pid: d.u32()? },
        LOADED => ChildMessage::Loaded {
            module_type: d.str()?,
            supports_eye: d.bool()?,
            supports_expression: d.bool()?,
        },
        LOAD_FAILED => ChildMessage::LoadFailed { reason: d.str()? },
        INIT_RESULT => {
            let eye = d.bool()?;
            let expression = d.bool()?;
            let name = d.str()?;
            let count = d.u16()?;
            let mut images = Vec::with_capacity(usize::from(count).min(64));
            for _ in 0..count {
                images.push(d.bytes()?.to_vec());
            }
            ChildMessage::InitResult { eye, expression, name, images }
        }
        FRAME => ChildMessage::Frame(Box::new(decode_frame(&mut d)?)),
        LOG => ChildMessage::Log {
            level: d.level()?,
            event_id: d.i32()?,
            category: d.str()?,
            message: d.str()?,
            exception: d.str()?,
        },
        NOTIFY => ChildMessage::Notify { level: d.level()?, key: d.str()?, message: d.str()? },
        NOTIFY_MESSAGE => ChildMessage::NotifyMessage { level: d.level()?, key: d.str()?, message: d.str()? },
        NOTIFY_CLEAR => ChildMessage::NotifyClear { key: d.str()? },
        STATUS => ChildMessage::Status { state: d.u8()? as i8 },
        SETTINGS_SCHEMA => ChildMessage::SettingsSchema { json: d.str()? },
        SETTINGS_WRITE => ChildMessage::SettingsWrite { json: d.str()? },
        TEARDOWN_DONE => ChildMessage::TeardownDone,
        other => return Err(ProtocolError::UnknownType(other)),
    };
    d.end()?;
    Ok(m)
}

fn decode_frame(d: &mut Dec<'_>) -> Result<FrameMessage, ProtocolError> {
    let seq = d.u64()?;
    let flags = FrameFlags(d.u8()?);
    let eye = |d: &mut Dec<'_>| -> Result<SingleEye, ProtocolError> {
        Ok(SingleEye {
            gaze_x: d.f32()?,
            gaze_y: d.f32()?,
            openness: d.f32()?,
            pupil_diameter_mm: d.f32()?,
        })
    };
    let left = eye(d)?;
    let right = eye(d)?;
    let eyes = Eyes {
        left,
        right,
        max_dilation: d.f32()?,
        min_dilation: d.f32()?,
        left_diameter: d.f32()?,
        right_diameter: d.f32()?,
    };
    let head = Head {
        yaw: d.f32()?,
        pitch: d.f32()?,
        roll: d.f32()?,
        pos_x: d.f32()?,
        pos_y: d.f32()?,
        pos_z: d.f32()?,
    };
    let count = usize::from(d.u16()?);
    let raw = d.take(count * 4)?;
    let mut frame = TrackingFrame { eye: eyes, head, ..TrackingFrame::default() };
    for (slot, c) in frame.shapes.iter_mut().zip(raw.chunks_exact(4)) {
        *slot = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    Ok(FrameMessage { seq, flags, frame })
}

/// Blocking read of one host → child message.
pub fn read_host_message(r: &mut impl Read, buf: &mut Vec<u8>) -> Result<HostMessage, ProtocolError> {
    let ty = read_body(r, buf)?;
    let mut d = Dec { buf: &buf[1..] };
    let m = match ty {
        INIT => HostMessage::Init { eye_available: d.bool()?, expression_available: d.bool()? },
        SETTINGS => HostMessage::Settings { json: d.str()? },
        TEARDOWN => HostMessage::Teardown,
        SETTINGS_ACTION => HostMessage::SettingsAction { key: d.str()? },
        other => return Err(ProtocolError::UnknownType(other)),
    };
    d.end()?;
    Ok(m)
}
