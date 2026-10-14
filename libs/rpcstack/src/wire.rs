//! Framework-owned codec for per-module wire data.
//!
//! Each policy module declares a serde `Wire` type ([`Module::Wire`]) and a
//! unique `NAME`. This module is the only place that knows how those values
//! are laid out in the `ctx` header; modules and hooks use the typed API
//! ([`WireIn`], [`WireOut`], [`peek`], and the metadata helpers) and never
//! touch header bytes, base64 or JSON.
//!
//! # Layout
//!
//! ```text
//! ctx: <name> : <base64 bincode> [ . <name> : <base64 bincode> ]*
//! ```
//!
//! Each module with wire data is one section, for example `count:AA==` for a
//! module whose wire type is a `u64` holding 0. Neither `.` nor `:` is in the
//! base64 alphabet, and module names may not contain them, so the value is
//! split with plain string searches. The primitives live in [`rpcstack_wire`],
//! which has no HTTP or gRPC dependencies, so a crate that cannot depend on
//! this one can still read a single section.
//!
//! The layout was chosen so that each section is independently addressable:
//! finding one section scans section names only and decodes nothing, and
//! decoding section X never touches sections Y and Z. The alternative, one
//! blob holding every module's data, would need a full decode to read any one
//! module's. The cost of this layout is that each payload is base64-encoded
//! separately (33% overhead on small payloads, versus one base64 pass over a
//! combined blob).
//!
//! A payload is the module's wire type serialized with `bincode`, in which a
//! small integer takes one byte: a few bytes where a self-describing format
//! took tens, and no parsing of names or numbers on the way in. `bincode` does
//! not describe itself, so a wire type serializes every field every time: use
//! `Option` where absence must be told apart from a value, and no
//! `skip_serializing_if` or `default`. The header is not readable by eye;
//! [`describe`] (or `Display` for [`WireIn`]) prints its sections as hex.
//!
//! There is nothing besides sections: every message carries exactly the
//! sections its sender's modules `put`.
//!
//! # Failure
//!
//! All binaries are assumed to be built from the same module stack, like
//! protobuf stubs. A missing section is `None`, and what that means is up to the module; a section
//! that does not match its module's type is a [`WireError`], which the hooks
//! turn into a panic, as for a malformed header. Sections for modules not
//! in the stack are ignored.

use std::fmt;
use std::ops::Deref;
use std::sync::LazyLock;

use rpcstack_wire as codec;
pub use rpcstack_wire::{describe, HEADER_NAME};
use serde::de::DeserializeOwned;
use serde::Serialize;
use smallvec::SmallVec;
use tonic::metadata::{Ascii, MetadataKey, MetadataMap, MetadataValue};

use crate::module::Module;

/// The header's name, parsed once: looking a header up by a `&str` parses and
/// validates the name every time.
const HEADER_KEY: http::HeaderName = http::HeaderName::from_static(HEADER_NAME);
static METADATA_KEY: LazyLock<MetadataKey<Ascii>> =
    LazyLock::new(|| MetadataKey::from_static(HEADER_NAME));

/// A wire section could not be read or written.
#[derive(Debug)]
pub struct WireError {
    detail: String,
}

impl WireError {
    fn new(detail: impl fmt::Display) -> Self {
        Self {
            detail: detail.to_string(),
        }
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid module wire data in header `{HEADER_NAME}`; all binaries of a deployment \
             must be built from the same policy stack: {}",
            self.detail
        )
    }
}

impl std::error::Error for WireError {}

/// Panics unless `name` can be used as a section name.
pub(crate) fn assert_valid_name(name: &str) {
    assert!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "`Module::NAME` {name:?} must be non-empty and contain only ASCII letters, digits, `_` and `-`"
    );
}

/// The inbound module wire sections of one message: a borrowed, undecoded
/// view of the `ctx` header.
#[derive(Debug, Clone, Copy, Default)]
pub struct WireIn<'a> {
    sections: &'a str,
}

impl<'a> WireIn<'a> {
    /// Split the `ctx` header of `headers` into sections. Decodes nothing; a
    /// missing header yields no sections.
    #[inline]
    pub fn from_headers(headers: &'a http::HeaderMap) -> Result<Self, WireError> {
        match headers.get(&HEADER_KEY) {
            None => Ok(Self::default()),
            Some(value) => Self::from_header_bytes(value.as_bytes()),
        }
    }

    /// Split the `ctx` header of `metadata` into sections. Decodes nothing; a
    /// missing header yields no sections.
    #[inline]
    pub fn from_metadata(metadata: &'a MetadataMap) -> Result<Self, WireError> {
        match metadata.get(&*METADATA_KEY) {
            None => Ok(Self::default()),
            Some(value) => Self::from_header_bytes(value.as_encoded_bytes()),
        }
    }

    // Bytes rather than `to_str`: the latter re-validates every byte as
    // visible ASCII, which costs more than the split itself.
    #[inline]
    fn from_header_bytes(value: &'a [u8]) -> Result<Self, WireError> {
        std::str::from_utf8(value)
            .map(|sections| Self { sections })
            .map_err(WireError::new)
    }

    /// Module `M`'s wire data, or `None` if the sender attached none; what
    /// absence means is up to the module. Only `M`'s section is decoded.
    pub fn get<M: Module>(&self) -> Result<Option<M::Wire>, WireError> {
        self.decode_present(M::NAME)
    }

    /// Module `M`'s wire data together with its section exactly as the sender
    /// encoded it, found with one scan of the header.
    pub fn get_with_encoded<M: Module>(
        &self,
    ) -> Result<Option<(M::Wire, EncodedSection)>, WireError> {
        let Some(payload) = codec::find_section(self.sections, M::NAME) else {
            return Ok(None);
        };
        let wire = codec::decode_payload(payload)
            .map_err(|err| WireError::new(format_args!("{}: {err}", M::NAME)))?;
        Ok(Some((wire, EncodedSection::new(payload))))
    }

    fn decode_present<W: DeserializeOwned>(&self, name: &str) -> Result<Option<W>, WireError> {
        let Some(payload) = codec::find_section(self.sections, name) else {
            return Ok(None);
        };
        codec::decode_payload(payload)
            .map(Some)
            .map_err(|err| WireError::new(format_args!("{name}: {err}")))
    }
}

impl fmt::Display for WireIn<'_> {
    /// The sections as [`describe`] prints them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&describe(self.sections))
    }
}

/// One section's payload, owned, exactly as a sender encoded it. A module that
/// sends a received section on unchanged keeps the one from
/// [`WireIn::get_with_encoded`] and gives it to [`WireOut::put_encoded`], so
/// the section is not encoded again. Short payloads are stored inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedSection(SmallVec<[u8; 64]>);

impl EncodedSection {
    fn new(payload: &str) -> Self {
        Self(SmallVec::from_slice(payload.as_bytes()))
    }
}

impl Deref for EncodedSection {
    type Target = [u8];

    /// The payload's bytes, which are ASCII.
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

/// The outbound module wire sections of one message, encoded.
///
/// The `ctx` header value is built in place, one `name:payload` after another,
/// and a value of up to 128 bytes never touches the heap.
#[derive(Clone, Default)]
pub struct WireOut {
    header: SmallVec<[u8; 128]>,
}

impl fmt::Debug for WireOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("WireOut")
            .field(&String::from_utf8_lossy(&self.header))
            .finish()
    }
}

impl WireOut {
    /// An empty set of sections.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sections already present in `metadata`'s `ctx` header, kept verbatim.
    pub fn from_metadata(metadata: &MetadataMap) -> Result<Self, WireError> {
        let input = WireIn::from_metadata(metadata)?;
        Ok(Self {
            header: SmallVec::from_slice(input.sections.as_bytes()),
        })
    }

    /// Set module `M`'s wire data, replacing any earlier value.
    pub fn put<M: Module>(&mut self, wire: &M::Wire) -> Result<(), WireError> {
        self.put_named(M::NAME, wire)
    }

    /// Set module `M`'s section to `payload`, an encoded payload obtained
    /// from [`WireIn::get_with_encoded`] for a section of the same type.
    /// Nothing is validated: a payload that is not `M::Wire` is a
    /// [`WireError`] for whoever reads it.
    pub fn put_encoded<M: Module>(&mut self, payload: &EncodedSection) {
        self.set(M::NAME, payload);
    }

    fn put_named<W: Serialize>(&mut self, name: &'static str, wire: &W) -> Result<(), WireError> {
        codec::encode_payload_with(wire, |payload| self.set(name, payload))
            .map_err(|err| WireError::new(format_args!("{name}: {err}")))
    }

    /// The byte range of the section called `name`, name and payload.
    fn find(&self, name: &str) -> Option<std::ops::Range<usize>> {
        let mut start = 0;
        for section in self.header.split(|&byte| byte == b'.') {
            let end = start + section.len();
            if section.split(|&byte| byte == b':').next() == Some(name.as_bytes()) {
                return Some(start..end);
            }
            start = end + 1;
        }
        None
    }

    /// Write `name:payload`: in place of the section of that name if there is
    /// one, else after the last section.
    fn set(&mut self, name: &str, payload: &[u8]) {
        if let Some(range) = self.find(name) {
            let mut section: SmallVec<[u8; 128]> = SmallVec::new();
            section.extend_from_slice(name.as_bytes());
            section.push(b':');
            section.extend_from_slice(payload);
            let at = range.start;
            self.header.drain(range);
            self.header.insert_many(at, section);
            return;
        }
        self.header.reserve(name.len() + payload.len() + 2);
        if !self.header.is_empty() {
            self.header.push(codec::SECTION_SEPARATOR as u8);
        }
        self.header.extend_from_slice(name.as_bytes());
        self.header.push(codec::NAME_SEPARATOR as u8);
        self.header.extend_from_slice(payload);
    }

    /// The `ctx` header value these sections make.
    pub fn header_value(&self) -> String {
        String::from_utf8(self.header.to_vec()).expect("base64, `.`, `:` and names are ASCII")
    }

    /// Make these sections the `ctx` header of `metadata`, replacing whatever
    /// it carried; with no sections the header is removed.
    #[inline]
    pub fn install(&self, metadata: &mut MetadataMap) {
        if self.header.is_empty() {
            metadata.remove(&*METADATA_KEY);
            return;
        }
        let value = MetadataValue::<Ascii>::try_from(&self.header[..])
            .expect("base64, `.`, `:` and names are ASCII");
        metadata.insert(&*METADATA_KEY, value);
    }
}

/// Decode only module `M`'s section from `headers`, without decoding any
/// other module's data. `Ok(None)` if the header or the section is absent.
pub fn peek<M: Module>(headers: &http::HeaderMap) -> Result<Option<M::Wire>, WireError> {
    WireIn::from_headers(headers)?.get::<M>()
}

/// Module `M`'s wire data in `metadata`, if any.
pub fn get_from_metadata<M: Module>(metadata: &MetadataMap) -> Result<Option<M::Wire>, WireError> {
    WireIn::from_metadata(metadata)?.get::<M>()
}

/// Set module `M`'s wire data in `metadata`, keeping other modules' data.
pub fn set_in_metadata<M: Module>(
    metadata: &mut MetadataMap,
    wire: &M::Wire,
) -> Result<(), WireError> {
    let mut out = WireOut::from_metadata(metadata)?;
    out.put::<M>(wire)?;
    out.install(metadata);
    Ok(())
}

/// A `ctx` header value carrying module `M`'s wire data, for building inbound
/// requests by hand (tests, tools).
pub fn header_value_with<M: Module>(wire: &M::Wire) -> Result<String, WireError> {
    let mut out = WireOut::new();
    out.put::<M>(wire)?;
    Ok(out.header_value())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_with(pairs: &[(&str, &str)]) -> http::HeaderMap {
        let mut value = String::new();
        for (name, payload) in pairs {
            codec::push_section(&mut value, name, payload);
        }
        let mut headers = http::HeaderMap::new();
        headers.insert(HEADER_NAME, value.parse().unwrap());
        headers
    }

    #[test]
    fn no_sections_is_an_empty_header() {
        assert_eq!(WireOut::new().header_value(), "");
        let mut metadata = MetadataMap::new();
        metadata.insert(HEADER_NAME, "a:NQ==".parse().unwrap());
        WireOut::new().install(&mut metadata);
        assert!(metadata.get(HEADER_NAME).is_none());
    }

    #[test]
    fn named_sections_round_trip_and_replace() {
        let mut out = WireOut::new();
        out.put_named("a", &7u32).unwrap();
        out.put_named("b", &"x".to_string()).unwrap();
        out.put_named("a", &8u32).unwrap();
        let value = out.header_value();
        assert_eq!(value.matches('.').count(), 1);

        let input = WireIn { sections: &value };
        assert_eq!(input.decode_present::<u32>("a").unwrap(), Some(8));
        assert_eq!(
            input.decode_present::<String>("b").unwrap().as_deref(),
            Some("x")
        );
    }

    #[test]
    fn replacing_a_section_keeps_its_place_and_its_neighbours() {
        let mut out = WireOut::new();
        // A name that starts another's must not be mistaken for it.
        for name in ["ab", "a", "abc"] {
            out.put_named(name, &1u8).unwrap();
        }
        let before = out.header_value();
        out.put_named("a", &(1u64 << 40)).unwrap();
        let after = out.header_value();
        assert_ne!(before, after);
        assert!(after.starts_with("ab:AQ==.a:"), "{after}");
        assert!(after.ends_with(".abc:AQ=="), "{after}");
        assert_eq!(after.matches('.').count(), 2);
    }

    #[test]
    fn long_values_spill_past_the_inline_buffer() {
        let mut out = WireOut::new();
        out.put_named("big", &vec![7u8; 400]).unwrap();
        out.put_named("small", &1u8).unwrap();
        let value = out.header_value();
        let input = WireIn { sections: &value };
        assert_eq!(
            input.decode_present::<Vec<u8>>("big").unwrap(),
            Some(vec![7u8; 400])
        );
        assert_eq!(input.decode_present::<u8>("small").unwrap(), Some(1));
        assert_eq!(input.to_string().lines().count(), 2, "one line per section");
    }

    #[test]
    fn install_replaces_the_previous_header() {
        let mut metadata = MetadataMap::new();
        metadata.insert(HEADER_NAME, "old:NQ==".parse().unwrap());
        let mut out = WireOut::new();
        out.put_named("new", &1u32).unwrap();
        out.install(&mut metadata);
        let value = metadata.get(HEADER_NAME).unwrap().to_str().unwrap();
        assert!(value.starts_with("new:") && !value.contains("old"));
    }

    #[test]
    fn absent_section_is_none() {
        let headers = map_with(&[]);
        let input = WireIn::from_headers(&headers).unwrap();
        assert_eq!(input.decode_present::<u64>("missing").unwrap(), None);
    }

    #[test]
    fn selective_decode_ignores_other_sections() {
        let headers = map_with(&[("good", "Kg=="), ("bad", "!!not base64!!")]);
        let input = WireIn::from_headers(&headers).unwrap();
        assert_eq!(input.decode_present::<u32>("good").unwrap(), Some(42));
        assert!(input.decode_present::<u32>("bad").is_err());
    }

    #[test]
    fn names_are_validated() {
        assert_valid_name("rajomon");
        assert_valid_name("pred-admission_2");
        assert!(std::panic::catch_unwind(|| assert_valid_name("a.b")).is_err());
        assert!(std::panic::catch_unwind(|| assert_valid_name("a:b")).is_err());
        assert!(std::panic::catch_unwind(|| assert_valid_name("")).is_err());
    }
}
