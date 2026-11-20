//! The implemented segment versions, lookup by (code, version), and the
//! raw fallback.
//!
//! "Parse what you can, keep the rest" (docs/architecture.md 2.1): a segment that
//! has no definition, or whose definition rejects it, becomes
//! [`ParsedSegment::Raw`]: the tokenized segment ([`RawSegment`], the type
//! the hand-written parser already uses) plus the [`CodecError`] that says
//! why. It does not fail the message, and the caller sees the error.

use crate::codec::defs::*;
use crate::codec::error::{CodecError, ErrorKind};
use crate::codec::schema::FieldSchema;
use crate::codec::segment::{Segment, SegmentHeader};
use crate::codec::wire;
use crate::parser::{DEG, RawSegment};

/// One implemented segment version.
#[derive(Debug, Clone, Copy)]
pub struct SegmentDef {
    /// Segment code, e.g. `HKSAL`.
    pub code: &'static str,
    /// Segment version.
    pub version: u16,
    decode: fn(&[DEG]) -> Result<TypedSegment, CodecError>,
    schema: &'static [FieldSchema],
}

impl SegmentDef {
    /// Reads a segment body (the data elements after the header) with this
    /// definition.
    pub fn decode_body(&self, body: &[DEG]) -> Result<TypedSegment, CodecError> {
        (self.decode)(body)
    }

    /// The data elements after the header, in wire order
    /// ([`Segment::SCHEMA`]).
    pub fn schema(&self) -> &'static [FieldSchema] {
        self.schema
    }
}

fn decode_as<T: Segment + Into<TypedSegment>>(body: &[DEG]) -> Result<TypedSegment, CodecError> {
    T::decode_body(body).map(Into::into)
}

/// Lists every derived segment once: builds [`TypedSegment`] and the
/// definition table. Add a new segment version here.
macro_rules! registry {
    ($($ty:ident),* $(,)?) => {
        /// A segment read into its typed definition, one variant per
        /// implemented segment version.
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum TypedSegment {
            $($ty($ty),)*
        }

        $(impl From<$ty> for TypedSegment {
            fn from(v: $ty) -> Self {
                TypedSegment::$ty(v)
            }
        })*

        impl TypedSegment {
            /// Segment code of the definition.
            pub fn code(&self) -> &'static str {
                match self {
                    $(TypedSegment::$ty(_) => <$ty as Segment>::CODE,)*
                }
            }

            /// Segment version of the definition.
            pub fn version(&self) -> u16 {
                match self {
                    $(TypedSegment::$ty(_) => <$ty as Segment>::VERSION,)*
                }
            }

            /// Writes the data elements after the header.
            pub fn encode_body(&self) -> Result<Vec<DEG>, CodecError> {
                match self {
                    $(TypedSegment::$ty(v) => v.encode_body(),)*
                }
            }
        }

        static DEFINITIONS: &[SegmentDef] = &[
            $(SegmentDef {
                code: <$ty as Segment>::CODE,
                version: <$ty as Segment>::VERSION,
                decode: decode_as::<$ty>,
                schema: <$ty as Segment>::SCHEMA,
            },)*
        ];
    };
}

registry!(
    HnHbk3, HnHbs1, HnShk4, HnSha2, HnVsk3, HnVsd1, HiRmg2, HiRms2, HkIdn2, HkVvb3, HkSyn3, HiSyn4,
    HkEnd1, HkTan6, HkTan7, HiTan6, HiTan7, HkSal7, HiTans6, HiTans7, HiBpa3, HiPins1, HiUpa4,
    HiUpd6, HkSpa1, HiSpa1, HiSals5, HiSals6, HiSals7, HkKaz5, HkKaz6, HkKaz7, HiKaz5, HiKaz6,
    HiKaz7, HiKazs5, HiKazs6, HiKazs7, HkWpd5, HkWpd6, HiWpd5, HiWpd6, HiWpds5, HiWpds6, HkWdu5,
    HiWdu5, HiSal5, HiSal6, HiSal7, HkSal5, HkSal6, HkCaz1, HiCaz1, HiCazs1, HkTan5, HiTan1,
    HiTan2, HiTan3, HiTan4, HiTan5, HiTans1, HiTans2, HiTans3, HiTans4, HiTans5, HiUpd4, HkSal8,
    HiSal8, HiSals8, HiSpas1, HiSpas2, HiSpas3, HkCcs1, HiCcss1, HkIpz1, HkIpz2, HiIpz1, HiIpz2,
    HiIpzs1, HiIpzs2, HkIps1, HiIps1, HiIpss1, HkVpp1, HiVpp1, HiVpps1, HkVpa1, HiVpas1
);

/// All implemented segment versions.
pub fn definitions() -> &'static [SegmentDef] {
    DEFINITIONS
}

/// The definition for a segment code and version, if implemented.
pub fn lookup(code: &str, version: u16) -> Option<&'static SegmentDef> {
    DEFINITIONS
        .iter()
        .find(|d| d.code == code && d.version == version)
}

/// The implemented versions of a segment code, ascending (empty if none).
pub fn versions(code: &str) -> Vec<u16> {
    let mut v: Vec<u16> = DEFINITIONS
        .iter()
        .filter(|d| d.code == code)
        .map(|d| d.version)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// A segment as the codec read it: typed, or raw with the reason.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedSegment {
    /// Read with its definition.
    ///
    /// The segment is boxed: with the read-path definitions the typed
    /// variants are much larger than a raw segment.
    Typed {
        header: SegmentHeader,
        segment: Box<TypedSegment>,
    },
    /// Kept as tokenized; `error` says why it is not typed: no definition
    /// for its code and version ([`ErrorKind::UnknownSegment`]), a malformed
    /// header, or a rule its definition found broken.
    Raw {
        segment: RawSegment,
        error: CodecError,
    },
}

impl ParsedSegment {
    /// The reason a segment stayed raw; `None` for a typed segment.
    pub fn error(&self) -> Option<&CodecError> {
        match self {
            ParsedSegment::Typed { .. } => None,
            ParsedSegment::Raw { error, .. } => Some(error),
        }
    }

    /// The typed segment, if any.
    pub fn typed(&self) -> Option<&TypedSegment> {
        match self {
            ParsedSegment::Typed { segment, .. } => Some(segment),
            ParsedSegment::Raw { .. } => None,
        }
    }

    /// Segment code as received (empty if the header is unreadable).
    pub fn code(&self) -> &str {
        match self {
            ParsedSegment::Typed { header, .. } => &header.code,
            ParsedSegment::Raw { segment, .. } => segment.segment_type(),
        }
    }

    /// The segment in tokenized form: the original for a raw segment,
    /// re-encoded for a typed one.
    pub fn to_raw(&self) -> Result<RawSegment, CodecError> {
        match self {
            ParsedSegment::Typed { header, segment } => {
                let mut degs = vec![header.encode()];
                degs.extend(segment.encode_body()?);
                Ok(RawSegment { degs })
            }
            ParsedSegment::Raw { segment, .. } => Ok(segment.clone()),
        }
    }
}

/// Reads one tokenized segment with the registered definition for its code
/// and version, without taking it. The error says why it cannot be typed:
/// a malformed header, no definition ([`ErrorKind::UnknownSegment`]), or a
/// rule its definition found broken.
pub fn read_segment(raw: &RawSegment) -> Result<(SegmentHeader, TypedSegment), CodecError> {
    let header = match raw.degs.first().map(SegmentHeader::decode) {
        Some(h) => h?,
        None => return Err(CodecError::new(ErrorKind::InvalidHeader("empty".into()))),
    };
    let Some(def) = lookup(&header.code, header.version) else {
        return Err(CodecError::new(ErrorKind::UnknownSegment).in_segment(header.id()));
    };
    let body = raw.degs.get(1..).unwrap_or_default();
    match def.decode_body(body) {
        Ok(segment) => Ok((header, segment)),
        Err(e) => Err(e.in_segment(header.id())),
    }
}

/// Reads one tokenized segment with the registered definition for its code
/// and version; anything else becomes [`ParsedSegment::Raw`].
pub fn parse_segment(raw: RawSegment) -> ParsedSegment {
    match read_segment(&raw) {
        Ok((header, segment)) => ParsedSegment::Typed {
            header,
            segment: Box::new(segment),
        },
        Err(error) => ParsedSegment::Raw {
            segment: raw,
            error,
        },
    }
}

/// Reads tokenized segments; see [`parse_segment`].
pub fn parse_segments(raw: impl IntoIterator<Item = RawSegment>) -> Vec<ParsedSegment> {
    raw.into_iter().map(parse_segment).collect()
}

/// Tokenizes and reads a message. Only a syntax error that stops the
/// tokenizer fails the whole message; segment errors stay with their
/// segments.
pub fn parse_message(bytes: &[u8]) -> Result<Vec<ParsedSegment>, CodecError> {
    Ok(parse_segments(wire::tokenize(bytes)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MESSAGE: &str = "HNHBK:1:3+000000000300+300+DLG42+2+DLG42:1'\
        HIRMG:2:2+0010::Nachricht entgegengenommen.'\
        HITANS:3:6:4+1+1+1+J:N:0:921:2:TESTHHD:HHD:1.4:Testverfahren Alpha:6:1:TAN:6:N:1:N:0:0:N:N:00:0:N:0'\
        HITANS:4:6:4+1+1+1+J:N:0:921:2:TESTHHD:HHD:1.4:Ein viel zu langer Verfahrensname ohne Ende:6:1:TAN'\
        HKSAL:5:9+x'\
        HNHBS:6:1+2'";

    #[test]
    fn lookup_and_versions() {
        assert_eq!(
            lookup("HKSAL", 7).map(|d| (d.code, d.version)),
            Some(("HKSAL", 7))
        );
        assert_eq!(lookup("HKSAL", 6).map(|d| d.version), Some(6));
        assert_eq!(lookup("HKSAL", 5).map(|d| d.version), Some(5));
        assert_eq!(versions("HKSAL"), vec![5, 6, 7, 8]);
        assert_eq!(versions("HITANS"), vec![1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(versions("HKTAN"), vec![5, 6, 7]);
        assert_eq!(versions("HKXYZ"), Vec::<u16>::new());
        assert_eq!(definitions().len(), 88);
        // Every registered version is unique.
        for d in definitions() {
            assert_eq!(
                definitions()
                    .iter()
                    .filter(|o| o.code == d.code && o.version == d.version)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn broken_and_unknown_segments_stay_raw_with_their_error() {
        let parsed = parse_message(MESSAGE.as_bytes()).unwrap();
        let summary: Vec<_> = parsed
            .iter()
            .map(|p| (p.code(), p.error().map(|e| e.kind.clone())))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("HNHBK", None),
                ("HIRMG", None),
                ("HITANS", None),
                (
                    "HITANS",
                    Some(ErrorKind::TooLong {
                        max: 30,
                        actual: 43
                    })
                ),
                ("HKSAL", Some(ErrorKind::UnknownSegment)),
                ("HNHBS", None),
            ]
        );

        let broken = parsed[3].error().unwrap();
        assert_eq!(broken.field, "parameters.methods[0].name");
        assert_eq!(
            broken.to_string(),
            "HITANS v6 (segment 4), DE 5, element 9 (parameters.methods[0].name): \
             too long: 43 bytes, at most 30 allowed"
        );
        // The raw segment is the received one, unchanged.
        let ParsedSegment::Raw { segment, .. } = &parsed[3] else {
            panic!("expected raw")
        };
        assert_eq!(segment.segment_number(), 4);

        let Some(TypedSegment::HiTans6(t)) = parsed[2].typed() else {
            panic!("expected HITANS v6")
        };
        assert_eq!(t.parameters.methods[0].name.as_str(), "Testverfahren Alpha");
    }

    #[test]
    fn parsed_message_serialises_back_to_the_same_bytes() {
        let parsed = parse_message(MESSAGE.as_bytes()).unwrap();
        let raw: Vec<_> = parsed.iter().map(|p| p.to_raw().unwrap()).collect();
        assert_eq!(wire::serialize(&raw).unwrap(), MESSAGE.as_bytes());
    }

    #[test]
    fn malformed_header_stays_raw() {
        let parsed = parse_message(b"'HKSAL:x:7'").unwrap();
        assert!(parsed.iter().all(|p| matches!(
            p.error().map(|e| &e.kind),
            Some(ErrorKind::InvalidHeader(_))
        )));
    }

    /// Every definition's synthetic sample is read as typed and written
    /// back to the identical bytes through the registry.
    #[test]
    fn every_sample_round_trips_through_the_registry() {
        let samples = crate::codec::defs::testing::all_samples();
        for wire in &samples {
            let parsed = parse_message(wire.as_bytes()).unwrap();
            assert_eq!(parsed.len(), 1, "{wire}");
            assert!(
                parsed[0].typed().is_some(),
                "{wire}: {:?}",
                parsed[0].error()
            );
            let raw = parsed[0].to_raw().unwrap();
            assert_eq!(
                String::from_utf8_lossy(&wire::serialize(&[raw]).unwrap()),
                *wire
            );
        }
        // Every registered version has at least one sample.
        for d in definitions() {
            let prefix = format!("{}:", d.code);
            assert!(
                samples.iter().any(|s| s.starts_with(&prefix)
                    && parse_message(s.as_bytes()).unwrap()[0]
                        .typed()
                        .is_some_and(|t| t.version() == d.version)),
                "no sample for {} v{}",
                d.code,
                d.version
            );
        }
    }

    #[test]
    fn tokenizer_errors_fail_the_message() {
        let e = parse_message(b"HNHBS:6:1+2").unwrap_err();
        assert!(matches!(e.kind, ErrorKind::Wire(_)));
    }

    proptest! {
        #[test]
        fn parse_message_never_panics(input in proptest::collection::vec(any::<u8>(), 0..300)) {
            let _ = parse_message(&input);
        }

        #[test]
        fn mutated_samples_never_panic(
            pick in any::<prop::sample::Index>(),
            edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)
        ) {
            let samples = crate::codec::defs::testing::all_samples();
            let mut bytes = samples[pick.index(samples.len())].as_bytes().to_vec();
            for (at, byte) in edits {
                let i = at.index(bytes.len());
                bytes[i] = byte;
            }
            if let Ok(parsed) = parse_message(&bytes) {
                for p in parsed {
                    let _ = p.to_raw();
                }
            }
        }

        #[test]
        fn truncated_samples_never_panic(
            pick in any::<prop::sample::Index>(),
            cut in any::<prop::sample::Index>()
        ) {
            let samples = crate::codec::defs::testing::all_samples();
            let sample = samples[pick.index(samples.len())].as_bytes();
            let _ = parse_message(&sample[..cut.index(sample.len() + 1)]);
        }

        #[test]
        fn mutated_messages_never_panic(
            edits in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..10)
        ) {
            let mut bytes = MESSAGE.as_bytes().to_vec();
            for (at, byte) in edits {
                let i = at.index(bytes.len());
                bytes[i] = byte;
            }
            if let Ok(parsed) = parse_message(&bytes) {
                for p in parsed {
                    let _ = p.to_raw();
                }
            }
        }
    }
}
