//! Layout of the header value that carries a request's or response's module
//! wire data, and the primitives to read and write it.
//!
//! ```text
//! ctx: <name> : <base64 bincode> [ . <name> : <base64 bincode> ]*
//! ```
//!
//! Each section carries one module's wire data. Neither `.` nor `:` is in the
//! base64 alphabet, and section names may not contain them, so the value is
//! split with plain string searches and one section is found without decoding
//! any other.
//!
//! A section's payload is its module's wire type serialized with `bincode`
//! (little endian, variable-length integers: a `u64` below 251 takes one byte,
//! one that fits 16, 32 or 64 bits takes 3, 5 or 9) and then base64-encoded.
//! `bincode` does not describe itself, so a wire type serializes every field
//! every time: use `Option` where absence has to be told apart from a value,
//! and no `skip_serializing_if` or `default`. [`describe`] prints a header's
//! sections as hex, to read one by eye.
//!
//! This crate knows nothing about HTTP or gRPC types, so a crate that cannot
//! depend on them can still read one section selectively. `rpcstack` builds the
//! typed module API (`WireIn`, `WireOut`) on top of these primitives.

/// Name of the header that carries the sections.
pub const HEADER_NAME: &str = "ctx";

/// Optional opaque scheduling key for transports that enqueue before RPC hooks.
/// Decimal u64; interpretation belongs to the selected run queue. This is
/// independent of any module's private wire schema.
pub const SCHEDULING_HEADER: &str = "x-rpcstack-scheduling";

use std::fmt::Write as _;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use bincode::Options as _;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// The `bincode` configuration of every payload. Spelled out so that a change
/// of the crate's defaults cannot change the wire format.
fn options() -> impl bincode::Options {
    bincode::DefaultOptions::new()
        .with_varint_encoding()
        .with_little_endian()
        .reject_trailing_bytes()
}

/// Payloads up to this many bytes are encoded and decoded on the stack.
const STACK_PAYLOAD: usize = 192;

/// Separates sections.
pub const SECTION_SEPARATOR: char = '.';

/// Separates a section's name from its payload.
pub const NAME_SEPARATOR: char = ':';

/// The `(name, payload)` pairs of a header value, undecoded.
#[inline]
pub fn sections(header: &str) -> impl Iterator<Item = (&str, &str)> {
    header
        .split(SECTION_SEPARATOR)
        .filter(|section| !section.is_empty())
        .map(|section| section.split_once(NAME_SEPARATOR).unwrap_or((section, "")))
}

/// The undecoded payload of the section called `name`.
///
/// Compares each section's start with `name` and skips the payloads without
/// looking into them, so the header is scanned once, for separators only.
#[inline]
pub fn find_section<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = header;
    loop {
        let end = rest
            .bytes()
            .position(|byte| byte == SECTION_SEPARATOR as u8)
            .unwrap_or(rest.len());
        if let Some(after_name) = rest[..end].strip_prefix(name) {
            if let Some(payload) = after_name.strip_prefix(NAME_SEPARATOR) {
                return Some(payload);
            }
            // A section without a payload.
            if after_name.is_empty() && !name.is_empty() {
                return Some("");
            }
        }
        rest = rest.get(end + 1..)?;
    }
}

/// Decode a section payload into `W`. The error says which step failed.
///
/// A payload that fits the stack buffer is decoded without allocating, beyond
/// what `W` itself owns.
pub fn decode_payload<W: DeserializeOwned>(payload: &str) -> Result<W, String> {
    let mut buf = [0u8; STACK_PAYLOAD];
    if let Some(bytes) = decode_payload_into(payload, &mut buf) {
        return deserialize(bytes);
    }
    let bytes = BASE64
        .decode(payload)
        .map_err(|err| format!("base64: {err}"))?;
    deserialize(&bytes)
}

fn deserialize<W: DeserializeOwned>(bytes: &[u8]) -> Result<W, String> {
    options()
        .deserialize(bytes)
        .map_err(|err| format!("bincode: {err}"))
}

/// Decode a section payload into `buf`, without allocating, and return the
/// decoded bytes. `None` means this decoder does not handle the payload (it is
/// not valid base64, or does not fit `buf`); [`decode_payload`] is the general
/// decoder and says why.
///
/// For a reader that handles the decoded bytes itself, on a path where a heap
/// allocation and the general decoder's setup per message matter. It accepts
/// exactly what the general decoder accepts, and decodes to the same bytes.
pub fn decode_payload_into<'b>(payload: &str, buf: &'b mut [u8]) -> Option<&'b [u8]> {
    let bytes = payload.as_bytes();
    if bytes.len() % 4 != 0 || bytes.len() / 4 * 3 > buf.len() {
        return None;
    }
    // The last group may be padded; every group before it is not.
    let (body, last) = bytes.split_at(bytes.len().saturating_sub(4));
    let mut invalid = 0;
    let mut written = 0;
    for (group, out) in body.chunks_exact(4).zip(buf.chunks_exact_mut(3)) {
        let (a, b, c, d) = (
            BASE64_VALUE[usize::from(group[0])],
            BASE64_VALUE[usize::from(group[1])],
            BASE64_VALUE[usize::from(group[2])],
            BASE64_VALUE[usize::from(group[3])],
        );
        invalid |= a | b | c | d;
        let bits = u32::from(a) << 18 | u32::from(b) << 12 | u32::from(c) << 6 | u32::from(d);
        out.copy_from_slice(&bits.to_be_bytes()[1..]);
        written += 3;
    }
    if invalid & INVALID != 0 {
        return None;
    }
    if let [c0, c1, c2, c3] = *last {
        let (a, b) = (BASE64_VALUE[usize::from(c0)], BASE64_VALUE[usize::from(c1)]);
        let (c, d) = (BASE64_VALUE[usize::from(c2)], BASE64_VALUE[usize::from(c3)]);
        let tail = &mut buf[written..];
        match (c2 == b'=', c3 == b'=') {
            (false, false) if (a | b | c | d) & INVALID == 0 => {
                let bits =
                    u32::from(a) << 18 | u32::from(b) << 12 | u32::from(c) << 6 | u32::from(d);
                tail[..3].copy_from_slice(&bits.to_be_bytes()[1..]);
                written += 3;
            }
            // Two bytes, whose last character must carry no stray bits.
            (false, true) if (a | b | c) & INVALID == 0 && c & 0b11 == 0 => {
                let bits = u32::from(a) << 18 | u32::from(b) << 12 | u32::from(c) << 6;
                tail[..2].copy_from_slice(&bits.to_be_bytes()[1..3]);
                written += 2;
            }
            (true, true) if (a | b) & INVALID == 0 && b & 0b1111 == 0 => {
                let bits = u32::from(a) << 18 | u32::from(b) << 12;
                tail[0] = bits.to_be_bytes()[1];
                written += 1;
            }
            _ => return None,
        }
    }
    Some(&buf[..written])
}

const INVALID: u8 = 0x80;

/// The value of each character of the standard base64 alphabet, or `INVALID`.
const BASE64_VALUE: [u8; 256] = {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut table = [INVALID; 256];
    let mut at = 0;
    while at < alphabet.len() {
        table[alphabet[at] as usize] = at as u8;
        at += 1;
    }
    table
};

/// Encode `wire` as a section payload.
pub fn encode_payload<W: Serialize>(wire: &W) -> Result<String, String> {
    encode_payload_with(wire, |payload| {
        String::from_utf8(payload.to_vec()).expect("base64 is ASCII")
    })
}

/// Encode `wire` as a section payload and hand the payload, which is ASCII, to
/// `use_payload`. A value that serializes to a few hundred bytes at most is
/// encoded on the stack, so the caller decides where the bytes go and nothing
/// is allocated for them here.
pub fn encode_payload_with<W: Serialize, R>(
    wire: &W,
    use_payload: impl FnOnce(&[u8]) -> R,
) -> Result<R, String> {
    let mut raw = [0u8; STACK_PAYLOAD];
    let mut writer: &mut [u8] = &mut raw;
    if options().serialize_into(&mut writer, wire).is_ok() {
        let written = STACK_PAYLOAD - writer.len();
        let mut text = [0u8; STACK_PAYLOAD / 3 * 4];
        let len = BASE64
            .encode_slice(&raw[..written], &mut text)
            .expect("the buffer holds the base64 of a full stack buffer");
        return Ok(use_payload(&text[..len]));
    }
    // Too long for the stack buffer, or not serializable: the heap says which.
    let bytes = options()
        .serialize(wire)
        .map_err(|err| format!("bincode: {err}"))?;
    Ok(use_payload(BASE64.encode(bytes).as_bytes()))
}

/// A readable form of a header value for debugging: one line per section with
/// its name and the payload's bytes in hex, or the reason the payload is not
/// base64. The framework cannot name the fields, because a payload does not
/// describe itself; to see them, decode the section as the module's wire type.
pub fn describe(header: &str) -> String {
    let mut text = String::new();
    for (name, payload) in sections(header) {
        if !text.is_empty() {
            text.push('\n');
        }
        match BASE64.decode(payload) {
            Err(err) => {
                let _ = write!(text, "{name}: not base64 ({err})");
            }
            Ok(bytes) => {
                let _ = write!(text, "{name} ({} bytes):", bytes.len());
                for byte in bytes {
                    let _ = write!(text, " {byte:02x}");
                }
            }
        }
    }
    text
}

/// Append `name:payload` to `header`.
#[inline]
pub fn push_section(header: &mut String, name: &str, payload: &str) {
    if !header.is_empty() {
        header.push(SECTION_SEPARATOR);
    }
    header.push_str(name);
    header.push(NAME_SEPARATOR);
    header.push_str(payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_find_sections() {
        let mut header = String::new();
        push_section(&mut header, "a", &encode_payload(&7u32).unwrap());
        push_section(&mut header, "b", &encode_payload(&"x").unwrap());
        assert_eq!(header.matches('.').count(), 1);
        assert_eq!(
            decode_payload::<u32>(find_section(&header, "a").unwrap()),
            Ok(7)
        );
        assert_eq!(
            decode_payload::<String>(find_section(&header, "b").unwrap()),
            Ok("x".to_owned())
        );
        assert_eq!(find_section(&header, "c"), None);
    }

    /// `find_section` against what splitting the header into sections finds,
    /// on headers made of names, empty sections and stray separators.
    #[test]
    fn find_section_agrees_with_splitting_the_header() {
        const PIECES: [&str; 7] = ["a", "ab", "b", ".", ":", "x", ""];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..50_000 {
            let header: String = (0..next() % 10)
                .map(|_| PIECES[(next() % 7) as usize])
                .collect();
            for name in ["a", "ab", "b", "x", "abx"] {
                let split = sections(&header)
                    .find_map(|(section, payload)| (section == name).then_some(payload));
                assert_eq!(find_section(&header, name), split, "{header:?} {name:?}");
            }
        }
    }

    #[test]
    fn decode_into_agrees_with_the_general_decoder_for_every_padding() {
        for len in 0..80usize {
            let payload = encode_payload(&"x".repeat(len)).unwrap();
            let mut buf = [0u8; 128];
            let decoded = decode_payload_into(&payload, &mut buf).unwrap();
            assert_eq!(decoded, BASE64.decode(&payload).unwrap(), "{len}");
        }
    }

    /// Strings over the base64 alphabet plus `=` and a few other bytes, whatever
    /// their length or padding: the decoder accepts exactly what the general one
    /// does, and decodes it to the same bytes.
    #[test]
    fn decode_into_accepts_exactly_what_the_general_decoder_accepts() {
        const CHARS: &[u8] = b"AAAABQgz09+/=Zz-_ !\xff";
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut accepted = 0;
        for _ in 0..200_000 {
            let len = (next() % 12) as usize * 4;
            // Mostly valid characters, with `=` at the end and the odd stray byte.
            let mut text: Vec<u8> = (0..len).map(|_| CHARS[(next() % 12) as usize]).collect();
            if next() % 4 == 0 && len >= 4 {
                let pad = 1 + (next() % 2) as usize;
                text[len - pad..].fill(b'=');
            }
            if next() % 16 == 0 && len > 0 {
                text[(next() as usize) % len] = CHARS[(next() % CHARS.len() as u64) as usize];
            }
            let Ok(payload) = String::from_utf8(text) else {
                continue;
            };
            let mut buf = [0u8; 64];
            let general = BASE64.decode(&payload);
            match (decode_payload_into(&payload, &mut buf), general) {
                (Some(bytes), Ok(expected)) => {
                    assert_eq!(bytes, expected, "{payload:?}");
                    accepted += 1;
                }
                (None, Err(_)) => {}
                (mine, general) => panic!("{payload:?}: {mine:?} against {general:?}"),
            }
        }
        assert!(
            accepted > 1_000,
            "the test accepted only {accepted} payloads"
        );
    }

    #[test]
    fn decode_into_declines_what_does_not_fit_or_is_not_base64() {
        let mut buf = [0u8; 8];
        assert_eq!(decode_payload_into("!!!!", &mut buf), None);
        assert_eq!(decode_payload_into("AAA", &mut buf), None);
        assert_eq!(decode_payload_into("AA=A", &mut buf), None);
        assert_eq!(decode_payload_into("A===", &mut buf), None);
        assert_eq!(decode_payload_into("=AAA", &mut buf), None);
        assert_eq!(decode_payload_into(&"A".repeat(40), &mut buf), None);
        assert_eq!(decode_payload_into("", &mut buf), Some(&[][..]));
    }

    #[test]
    fn decode_reports_the_failing_step() {
        assert!(decode_payload::<u32>("!!")
            .unwrap_err()
            .starts_with("base64"));
        assert!(decode_payload::<u32>("")
            .unwrap_err()
            .starts_with("bincode"));
    }

    #[test]
    fn trailing_bytes_are_an_error() {
        let mut payload = BASE64.encode([7u8, 0]);
        assert!(decode_payload::<u8>(&payload)
            .unwrap_err()
            .starts_with("bincode"));
        payload = BASE64.encode([7u8]);
        assert_eq!(decode_payload::<u8>(&payload), Ok(7));
    }

    #[test]
    fn integers_take_the_fewest_bytes() {
        let bytes = |value: u64| {
            BASE64
                .decode(encode_payload(&value).unwrap())
                .unwrap()
                .len()
        };
        assert_eq!(bytes(0), 1);
        assert_eq!(bytes(250), 1);
        assert_eq!(bytes(251), 3);
        assert_eq!(bytes(65_535), 3);
        assert_eq!(bytes(65_536), 5);
        assert_eq!(bytes(u64::from(u32::MAX) + 1), 9);
    }

    /// Zero and absent differ, because an `Option` always writes its tag.
    #[test]
    fn an_absent_option_differs_from_zero() {
        let absent = encode_payload(&(5u8, None::<u64>)).unwrap();
        let zero = encode_payload(&(5u8, Some(0u64))).unwrap();
        assert_ne!(absent, zero);
        assert_eq!(decode_payload::<(u8, Option<u64>)>(&absent), Ok((5, None)));
        assert_eq!(decode_payload::<(u8, Option<u64>)>(&zero), Ok((5, Some(0))));
    }

    #[test]
    fn values_longer_than_the_stack_buffer_round_trip() {
        for len in [0usize, 100, STACK_PAYLOAD - 1, STACK_PAYLOAD, 500, 5_000] {
            let value: Vec<u8> = (0..len).map(|at| at as u8).collect();
            let payload = encode_payload(&value).unwrap();
            assert_eq!(decode_payload::<Vec<u8>>(&payload), Ok(value), "{len}");
        }
    }

    #[test]
    fn encode_with_hands_over_the_ascii_payload() {
        assert_eq!(
            encode_payload_with(&7u8, |payload| payload.to_vec()).unwrap(),
            b"Bw=="
        );
    }

    #[test]
    fn describe_shows_each_section_as_hex() {
        let mut header = String::new();
        push_section(&mut header, "a", &encode_payload(&(7u8, 300u64)).unwrap());
        push_section(&mut header, "b", "!!");
        let text = describe(&header);
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("a (4 bytes): 07 fb 2c 01"));
        assert!(lines.next().unwrap().starts_with("b: not base64"));
        assert_eq!(lines.next(), None);
    }
}
