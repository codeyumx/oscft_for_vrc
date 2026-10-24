use super::*;
use fb_tracking::SHAPE_COUNT;
use std::io::Cursor;

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

const HELLO_HEX: &str = "2b 00 00 00 01 01 00 20 00 00 00 30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66 30 31 32 33 34 35 36 37 38 39 61 62 63 64 65 66 92 10 00 00";
const INIT_HEX: &str = "03 00 00 00 81 01 00";
const NOTIFY_HEX: &str = "27 00 00 00 07 03 0a 00 00 00 6e 6f 2d 68 65 61 64 73 65 74 13 00 00 00 4e 6f 20 68 65 61 64 73 65 74 20 61 6e 73 77 65 72 65 64";
const FRAME_HEX: &str = "60 00 00 00 05 07 00 00 00 00 00 00 00 03 00 00 80 3e 00 00 00 bf 00 00 80 3f 00 00 60 40 00 00 00 00 00 00 00 00 00 00 40 3f 00 00 80 40 00 00 c0 40 00 00 00 40 00 00 60 40 00 00 80 40 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 03 00 00 00 00 00 00 00 00 3f 00 00 80 3f";

fn hello() -> ChildMessage {
    ChildMessage::Hello { version: 1, token: "0123456789abcdef0123456789abcdef".into(), pid: 4242 }
}

fn notify() -> ChildMessage {
    ChildMessage::Notify {
        level: LogLevel::Warning,
        key: "no-headset".into(),
        message: "No headset answered".into(),
    }
}

fn vector_frame() -> FrameMessage {
    let mut frame = TrackingFrame::default();
    frame.eye.left = fb_tracking::SingleEye { gaze_x: 0.25, gaze_y: -0.5, openness: 1.0, pupil_diameter_mm: 3.5 };
    frame.eye.right = fb_tracking::SingleEye { gaze_x: 0.0, gaze_y: 0.0, openness: 0.75, pupil_diameter_mm: 4.0 };
    frame.eye.max_dilation = 6.0;
    frame.eye.min_dilation = 2.0;
    frame.eye.left_diameter = 3.5;
    frame.eye.right_diameter = 4.0;
    frame.shapes[1] = 0.5;
    frame.shapes[2] = 1.0;
    FrameMessage { seq: 7, flags: FrameFlags::EYE | FrameFlags::EXPRESSION, frame }
}

fn read_child(bytes: &[u8]) -> Result<ChildMessage, ProtocolError> {
    read_child_message(&mut Cursor::new(bytes), &mut Vec::new())
}

#[test]
fn hello_vector() {
    let mut out = Vec::new();
    write_child_message(&mut out, &hello()).unwrap();
    assert_eq!(out, hex(HELLO_HEX));
    assert_eq!(read_child(&out).unwrap(), hello());
}

#[test]
fn init_vector() {
    let m = HostMessage::Init { eye_available: true, expression_available: false };
    let mut out = Vec::new();
    write_host_message(&mut out, &m).unwrap();
    assert_eq!(out, hex(INIT_HEX));
    assert_eq!(read_host_message(&mut Cursor::new(&out), &mut Vec::new()).unwrap(), m);
}

#[test]
fn settings_action_vector() {
    let m = HostMessage::SettingsAction { key: "pair".into() };
    let mut out = Vec::new();
    write_host_message(&mut out, &m).unwrap();
    assert_eq!(out, hex("09 00 00 00 84 04 00 00 00 70 61 69 72"));
    assert_eq!(read_host_message(&mut Cursor::new(&out), &mut Vec::new()).unwrap(), m);
}

#[test]
fn notify_vector() {
    let mut out = Vec::new();
    write_child_message(&mut out, &notify()).unwrap();
    assert_eq!(out, hex(NOTIFY_HEX));
    assert_eq!(read_child(&out).unwrap(), notify());
}

#[test]
fn frame_vector_with_three_shapes() {
    let mut out = Vec::new();
    encode_frame(&vector_frame(), 3, &mut out).unwrap();
    assert_eq!(out, hex(FRAME_HEX));
    // The decoder zero-fills the missing shapes.
    match read_child(&out).unwrap() {
        ChildMessage::Frame(f) => assert_eq!(*f, vector_frame()),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn full_frame_round_trips_with_all_shapes() {
    let mut m = vector_frame();
    for (i, s) in m.frame.shapes.iter_mut().enumerate() {
        *s = i as f32 / 100.0;
    }
    m.frame.head.yaw = -1.5;
    m.frame.head.pos_z = 9.0;
    m.flags = m.flags | FrameFlags::HEAD;
    let msg = ChildMessage::Frame(Box::new(m));
    let mut out = Vec::new();
    write_child_message(&mut out, &msg).unwrap();
    assert_eq!(read_child(&out).unwrap(), msg);
}

#[test]
fn extra_shapes_are_ignored() {
    // Re-frame the vector as 90 shapes: 88 real + 2 extra.
    let mut body = hex(FRAME_HEX)[4..].to_vec();
    let count_pos = body.len() - 3 * 4 - 2;
    body[count_pos..count_pos + 2].copy_from_slice(&90u16.to_le_bytes());
    body.extend(std::iter::repeat_n(0u8, (90 - 3) * 4));
    let n = body.len() as u32;
    let mut bytes = n.to_le_bytes().to_vec();
    bytes.extend(body);
    match read_child(&bytes).unwrap() {
        ChildMessage::Frame(f) => {
            assert_eq!(f.frame.shapes.len(), SHAPE_COUNT);
            assert_eq!(f.frame.shapes[1], 0.5);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn every_message_round_trips() {
    let msgs = vec![
        hello(),
        ChildMessage::Loaded { module_type: "Ns.Type".into(), supports_eye: true, supports_expression: false },
        ChildMessage::LoadFailed { reason: "boom \u{1F4A5}".into() },
        ChildMessage::InitResult { eye: true, expression: true, name: "Mod".into(), images: vec![vec![1, 2, 3], vec![]] },
        ChildMessage::Log {
            level: LogLevel::Critical,
            event_id: -7,
            category: "cat".into(),
            message: "msg".into(),
            exception: String::new(),
        },
        notify(),
        ChildMessage::NotifyMessage { level: LogLevel::Error, key: "k".into(), message: "Pairing lost".into() },
        ChildMessage::NotifyClear { key: String::new() },
        ChildMessage::Status { state: -1 },
        ChildMessage::SettingsSchema { json: "{\"fields\":[]}".into() },
        ChildMessage::SettingsWrite { json: "{}".into() },
        ChildMessage::TeardownDone,
    ];
    let mut wire = Vec::new();
    for m in &msgs {
        write_child_message(&mut wire, m).unwrap();
    }
    let mut cur = Cursor::new(wire);
    let mut buf = Vec::new();
    for m in &msgs {
        assert_eq!(&read_child_message(&mut cur, &mut buf).unwrap(), m);
    }
    assert!(matches!(read_child_message(&mut cur, &mut buf), Err(ProtocolError::Closed)));

    let host = [
        HostMessage::Init { eye_available: false, expression_available: true },
        HostMessage::Settings { json: "{\"a\":1}".into() },
        HostMessage::Teardown,
        HostMessage::SettingsAction { key: "pair".into() },
    ];
    let mut wire = Vec::new();
    for m in &host {
        write_host_message(&mut wire, m).unwrap();
    }
    let mut cur = Cursor::new(wire);
    for m in &host {
        assert_eq!(&read_host_message(&mut cur, &mut buf).unwrap(), m);
    }
}

#[test]
fn oversize_length_is_rejected_without_reading_payload() {
    let bytes = (MAX_MESSAGE_LEN + 1).to_le_bytes();
    assert!(matches!(read_child(&bytes), Err(ProtocolError::TooLarge(n)) if n == u64::from(MAX_MESSAGE_LEN) + 1));
    // Exactly at the limit is accepted as a length (then fails as truncated I/O, not TooLarge).
    let bytes = MAX_MESSAGE_LEN.to_le_bytes();
    assert!(matches!(read_child(&bytes), Err(ProtocolError::Io(_))));
}

#[test]
fn encoding_oversize_message_fails_and_leaves_buffer_untouched() {
    let m = ChildMessage::SettingsWrite { json: "x".repeat(MAX_MESSAGE_LEN as usize) };
    let mut out = vec![9u8];
    assert!(matches!(encode_child_message(&m, &mut out), Err(ProtocolError::TooLarge(_))));
    assert_eq!(out, [9]);
}

#[test]
fn malformed_input_is_rejected() {
    let frame = |body: &[u8]| {
        let mut v = (body.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(body);
        v
    };
    // Unknown type, in either direction.
    assert!(matches!(read_child(&frame(&[0x55])), Err(ProtocolError::UnknownType(0x55))));
    assert!(matches!(
        read_host_message(&mut Cursor::new(frame(&[0x01])), &mut Vec::new()),
        Err(ProtocolError::UnknownType(0x01))
    ));
    // Zero length.
    assert!(matches!(read_child(&0u32.to_le_bytes()), Err(ProtocolError::Malformed(_))));
    // Truncated payload (Hello with only a version).
    assert!(matches!(read_child(&frame(&[0x01, 1, 0])), Err(ProtocolError::Malformed(_))));
    // String length beyond the payload.
    assert!(matches!(
        read_child(&frame(&[0x03, 0xff, 0xff, 0xff, 0x7f, b'a'])),
        Err(ProtocolError::Malformed(_))
    ));
    // Invalid UTF-8.
    assert!(matches!(
        read_child(&frame(&[0x03, 1, 0, 0, 0, 0xff])),
        Err(ProtocolError::Malformed("invalid UTF-8"))
    ));
    // Invalid bool.
    assert!(matches!(
        read_host_message(&mut Cursor::new(frame(&[0x81, 2, 0])), &mut Vec::new()),
        Err(ProtocolError::Malformed("invalid bool"))
    ));
    // Invalid log level.
    assert!(matches!(
        read_child(&frame(&[0x08 - 1, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
        Err(ProtocolError::Malformed("invalid log level"))
    ));
    // Trailing bytes.
    assert!(matches!(read_child(&frame(&[0x0c, 0])), Err(ProtocolError::Malformed("trailing bytes"))));
    // Frame whose shape_count promises more floats than present.
    let mut body = hex(FRAME_HEX)[4..].to_vec();
    let count_pos = body.len() - 3 * 4 - 2;
    body[count_pos] = 4;
    assert!(matches!(read_child(&frame(&body)), Err(ProtocolError::Malformed(_))));
    // Connection dropped mid-header and mid-body.
    assert!(matches!(read_child(&[0x03, 0]), Err(ProtocolError::Io(_))));
    assert!(matches!(read_child(&[0x03, 0, 0, 0, 0x81]), Err(ProtocolError::Io(_))));
    // Clean EOF.
    assert!(matches!(read_child(&[]), Err(ProtocolError::Closed)));
}

#[test]
fn flags_and_levels() {
    let f = FrameFlags::EYE | FrameFlags::HEAD;
    assert!(f.contains(FrameFlags::EYE) && f.contains(FrameFlags::HEAD) && !f.contains(FrameFlags::EXPRESSION));
    assert_eq!(LogLevel::from_u8(3), Some(LogLevel::Warning));
    assert_eq!(LogLevel::from_u8(6), None);
    assert_eq!(LogLevel::Information.as_str(), "information");
}
