//! OSC 1.0 wire encoding and the tiny amount of decoding FaceBridge needs.
//!
//! Encoding is allocation-free in steady state: addresses are padded once ([`encode_address`]) and every message
//! is appended to a reused buffer ([`Bundle`]).
//!
//! Spec: <https://opensoundcontrol.stanford.edu/spec-1_0.html>. Strings are NUL-terminated and padded to a
//! multiple of four bytes, numbers are big-endian 32-bit, `T`/`F` carry no data.

use std::fmt;

/// A value FaceBridge sends to an avatar parameter (`f`, `i`, `T`/`F`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Float(f32),
    Int(i32),
    Bool(bool),
}

impl Value {
    const fn tag(self) -> &'static [u8; 4] {
        match self {
            Value::Float(_) => b",f\0\0",
            Value::Int(_) => b",i\0\0",
            Value::Bool(true) => b",T\0\0",
            Value::Bool(false) => b",F\0\0",
        }
    }

    /// Bytes of the argument payload (`T`/`F` have none).
    const fn payload_len(self) -> usize {
        match self {
            Value::Float(_) | Value::Int(_) => 4,
            Value::Bool(_) => 0,
        }
    }
}

fn push_padded(buf: &mut Vec<u8>, s: &[u8]) {
    buf.extend_from_slice(s);
    // 1..=4 NUL bytes: always at least the terminator, then pad to a multiple of 4.
    let pad = 4 - (s.len() % 4);
    buf.resize(buf.len() + pad, 0);
}

/// An OSC address (or any OSC string) as it appears on the wire: bytes, NUL terminator and padding.
pub fn encode_address(address: &str) -> Box<[u8]> {
    let mut v = Vec::with_capacity(address.len() + 4);
    push_padded(&mut v, address.as_bytes());
    v.into_boxed_slice()
}

/// Append one complete message (`padded_address` produced by [`encode_address`]) to `buf`.
pub fn write_message(buf: &mut Vec<u8>, padded_address: &[u8], value: Value) {
    buf.extend_from_slice(padded_address);
    buf.extend_from_slice(value.tag());
    match value {
        Value::Float(f) => buf.extend_from_slice(&f.to_be_bytes()),
        Value::Int(i) => buf.extend_from_slice(&i.to_be_bytes()),
        Value::Bool(_) => {}
    }
}

/// Append a message with a single string argument (what VRChat sends for `/avatar/change`).
pub fn write_string_message(buf: &mut Vec<u8>, address: &str, arg: &str) {
    push_padded(buf, address.as_bytes());
    buf.extend_from_slice(b",s\0\0");
    push_padded(buf, arg.as_bytes());
}

/// `#bundle\0` followed by the 64-bit time tag 1 ("immediately").
const BUNDLE_HEADER: [u8; 16] = *b"#bundle\0\0\0\0\0\0\0\0\x01";

/// Accumulates messages into one UDP datagram. Messages are wrapped in an OSC bundle with the "immediately"
/// time tag; a datagram holding a single message is emitted as a plain message (no bundle, no copy).
pub struct Bundle {
    buf: Vec<u8>,
    count: usize,
    max_len: usize,
}

impl Bundle {
    pub fn new(max_len: usize) -> Self {
        let mut b = Self { buf: Vec::with_capacity(max_len), count: 0, max_len };
        b.clear();
        b
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.buf.extend_from_slice(&BUNDLE_HEADER);
        self.count = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn len(&self) -> usize {
        self.count
    }

    /// Add a message; returns `false` (and adds nothing) when the datagram would exceed the size limit, in which
    /// case the caller sends [`Bundle::packet`], calls [`Bundle::clear`] and pushes again. A message that is too
    /// large for an empty bundle is accepted anyway.
    pub fn push(&mut self, padded_address: &[u8], value: Value) -> bool {
        let size = padded_address.len() + 4 + value.payload_len();
        if self.count > 0 && self.buf.len() + 4 + size > self.max_len {
            return false;
        }
        self.buf.extend_from_slice(&(size as u32).to_be_bytes());
        write_message(&mut self.buf, padded_address, value);
        self.count += 1;
        true
    }

    /// The datagram to send, or `None` when empty.
    pub fn packet(&self) -> Option<&[u8]> {
        match self.count {
            0 => None,
            1 => Some(&self.buf[BUNDLE_HEADER.len() + 4..]),
            _ => Some(&self.buf),
        }
    }
}

/// A decoded OSC argument.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Float(f32),
    Int(i32),
    Bool(bool),
    Str(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError(pub &'static str);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed OSC packet: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

fn read_str<'a>(data: &'a [u8], pos: &mut usize) -> Result<&'a str, DecodeError> {
    let rest = data.get(*pos..).ok_or(DecodeError("truncated string"))?;
    let nul = rest.iter().position(|&b| b == 0).ok_or(DecodeError("unterminated string"))?;
    let s = std::str::from_utf8(&rest[..nul]).map_err(|_| DecodeError("string is not UTF-8"))?;
    *pos += (nul + 4) & !3;
    if *pos > data.len() {
        return Err(DecodeError("truncated string padding"));
    }
    Ok(s)
}

fn read_word(data: &[u8], pos: &mut usize) -> Result<[u8; 4], DecodeError> {
    let w: [u8; 4] = data
        .get(*pos..*pos + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or(DecodeError("truncated argument"))?;
    *pos += 4;
    Ok(w)
}

/// Decode one message (not a bundle): address and arguments (`f i T F s`; anything else is an error).
pub fn decode_message(data: &[u8]) -> Result<(String, Vec<Arg>), DecodeError> {
    let mut pos = 0;
    let address = read_str(data, &mut pos)?;
    if !address.starts_with('/') {
        return Err(DecodeError("address must start with '/'"));
    }
    // A message without a type tag string is legal in OSC 1.0 (no arguments).
    if pos >= data.len() {
        return Ok((address.to_owned(), Vec::new()));
    }
    let tags = read_str(data, &mut pos)?;
    let tags = tags.strip_prefix(',').ok_or(DecodeError("missing type tag string"))?;
    let mut args = Vec::with_capacity(tags.len());
    for tag in tags.bytes() {
        args.push(match tag {
            b'f' => Arg::Float(f32::from_be_bytes(read_word(data, &mut pos)?)),
            b'i' => Arg::Int(i32::from_be_bytes(read_word(data, &mut pos)?)),
            b'T' => Arg::Bool(true),
            b'F' => Arg::Bool(false),
            b's' => Arg::Str(read_str(data, &mut pos)?.to_owned()),
            _ => return Err(DecodeError("unsupported argument type")),
        });
    }
    Ok((address.to_owned(), args))
}

/// Decode a datagram that is a message or a (possibly nested) bundle, calling `f` for every message.
/// Undecodable elements are skipped; the first error is returned after the rest was visited.
pub fn for_each_message(data: &[u8], f: &mut impl FnMut(String, Vec<Arg>)) -> Result<(), DecodeError> {
    let Some(body) = data.strip_prefix(b"#bundle\0") else {
        let (address, args) = decode_message(data)?;
        f(address, args);
        return Ok(());
    };
    // Skip the 8-byte time tag, then `size, element` pairs.
    let mut rest = body.get(8..).ok_or(DecodeError("truncated bundle header"))?;
    let mut first_err = None;
    while !rest.is_empty() {
        let len_bytes: [u8; 4] =
            rest.get(..4).and_then(|s| s.try_into().ok()).ok_or(DecodeError("truncated bundle element size"))?;
        let len = u32::from_be_bytes(len_bytes) as usize;
        let element = rest.get(4..4 + len).ok_or(DecodeError("truncated bundle element"))?;
        if let Err(e) = for_each_message(element, f) {
            first_err.get_or_insert(e);
        }
        rest = &rest[4 + len..];
    }
    first_err.map_or(Ok(()), Err)
}

/// The avatar id carried by a `/avatar/change` message anywhere in this datagram (last one wins).
pub fn avatar_change(data: &[u8]) -> Option<String> {
    let mut id = None;
    let _ = for_each_message(data, &mut |address, args| {
        if address == "/avatar/change" {
            if let Some(Arg::Str(s)) = args.into_iter().next() {
                id = Some(s);
            }
        }
    });
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(address: &str, v: Value) -> Vec<u8> {
        let mut b = Vec::new();
        write_message(&mut b, &encode_address(address), v);
        b
    }

    #[test]
    fn float_message_bytes() {
        let got = msg("/oscillator/4/frequency", Value::Float(440.0));
        let mut want = b"/oscillator/4/frequency\0".to_vec(); // 23 chars + NUL = 24, already aligned
        want.extend_from_slice(b",f\0\0");
        want.extend_from_slice(&[0x43, 0xdc, 0x00, 0x00]); // 440.0f32 big-endian
        assert_eq!(got, want);
    }

    #[test]
    fn address_padding_always_has_terminator() {
        // A string whose length is a multiple of 4 still gets four NULs (terminator + padding to the next word).
        assert_eq!(&*encode_address("/abc"), b"/abc\0\0\0\0");
        assert_eq!(&*encode_address("/abcd"), b"/abcd\0\0\0");
        assert_eq!(&*encode_address("/abcde"), b"/abcde\0\0");
        assert_eq!(&*encode_address("/abcdef"), b"/abcdef\0");
    }

    #[test]
    fn int_and_bool_messages() {
        assert_eq!(msg("/a", Value::Int(-2)), b"/a\0\0,i\0\0\xff\xff\xff\xfe");
        assert_eq!(msg("/a", Value::Bool(true)), b"/a\0\0,T\0\0");
        assert_eq!(msg("/a", Value::Bool(false)), b"/a\0\0,F\0\0");
        assert_eq!(msg("/a", Value::Float(-1.0)), b"/a\0\0,f\0\0\xbf\x80\x00\x00");
    }

    #[test]
    fn string_message_bytes() {
        let mut b = Vec::new();
        write_string_message(&mut b, "/avatar/change", "avtr_1");
        assert_eq!(b, b"/avatar/change\0\0,s\0\0avtr_1\0\0");
    }

    #[test]
    fn single_message_bundle_is_a_plain_message() {
        let addr = encode_address("/a");
        let mut bundle = Bundle::new(1400);
        assert!(bundle.packet().is_none());
        assert!(bundle.push(&addr, Value::Bool(true)));
        assert_eq!(bundle.packet().unwrap(), b"/a\0\0,T\0\0");
    }

    #[test]
    fn multi_message_bundle_bytes() {
        let a = encode_address("/a");
        let b = encode_address("/b");
        let mut bundle = Bundle::new(1400);
        assert!(bundle.push(&a, Value::Bool(true)));
        assert!(bundle.push(&b, Value::Float(1.0)));
        let mut want = b"#bundle\0".to_vec();
        want.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]); // "immediately"
        want.extend_from_slice(&8u32.to_be_bytes());
        want.extend_from_slice(b"/a\0\0,T\0\0");
        want.extend_from_slice(&12u32.to_be_bytes());
        want.extend_from_slice(b"/b\0\0,f\0\0\x3f\x80\x00\x00");
        assert_eq!(bundle.packet().unwrap(), want);

        bundle.clear();
        assert!(bundle.packet().is_none());
    }

    #[test]
    fn bundle_respects_size_limit() {
        let addr = encode_address("/abcdefghijklmnopqrs"); // 20 chars -> 24 bytes
        let element = 4 + addr.len() + 4 + 4;
        let mut bundle = Bundle::new(16 + 2 * element);
        assert!(bundle.push(&addr, Value::Float(0.0)));
        assert!(bundle.push(&addr, Value::Float(0.0)));
        assert!(!bundle.push(&addr, Value::Float(0.0)));
        assert_eq!(bundle.len(), 2);
        assert_eq!(bundle.packet().unwrap().len(), 16 + 2 * element);
        // an oversize message is still accepted into an empty bundle
        let mut tiny = Bundle::new(8);
        assert!(tiny.push(&addr, Value::Float(0.0)));
    }

    #[test]
    fn decode_round_trip_and_bundle() {
        let a = encode_address("/avatar/parameters/X");
        let mut bundle = Bundle::new(1400);
        bundle.push(&a, Value::Float(0.5));
        bundle.push(&a, Value::Int(7));
        bundle.push(&a, Value::Bool(false));
        let mut seen = Vec::new();
        for_each_message(bundle.packet().unwrap(), &mut |addr, args| seen.push((addr, args))).unwrap();
        assert_eq!(
            seen,
            vec![
                ("/avatar/parameters/X".to_owned(), vec![Arg::Float(0.5)]),
                ("/avatar/parameters/X".to_owned(), vec![Arg::Int(7)]),
                ("/avatar/parameters/X".to_owned(), vec![Arg::Bool(false)]),
            ]
        );
    }

    #[test]
    fn avatar_change_plain_and_nested_bundle() {
        let mut plain = Vec::new();
        write_string_message(&mut plain, "/avatar/change", "avtr_abc");
        assert_eq!(avatar_change(&plain).as_deref(), Some("avtr_abc"));

        // bundle { float message, bundle { avatar change } }
        let mut inner = BUNDLE_HEADER.to_vec();
        inner.extend_from_slice(&(plain.len() as u32).to_be_bytes());
        inner.extend_from_slice(&plain);
        let other = msg("/avatar/parameters/Y", Value::Float(1.0));
        let mut outer = BUNDLE_HEADER.to_vec();
        outer.extend_from_slice(&(other.len() as u32).to_be_bytes());
        outer.extend_from_slice(&other);
        outer.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        outer.extend_from_slice(&inner);
        assert_eq!(avatar_change(&outer).as_deref(), Some("avtr_abc"));

        assert_eq!(avatar_change(&other), None);
        assert_eq!(avatar_change(b"garbage"), None);
        assert_eq!(avatar_change(b""), None);
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(decode_message(b"/a\0\0,f\0\0\x3f").is_err());
        assert!(decode_message(b"/a\0\0,s\0\0abc").is_err());
    }
}
