//! Segments: the header framework and the [`Segment`] trait.
//!
//! Every segment starts with the group "Segmentkopf" (Formals B.5.1; data
//! dictionary entry "Segmentkopf"): `code:number:version[:reference]`,
//! code `an ..6`, number and version `num ..3`, reference `num ..3`
//! (only in bank messages). The codec reads and writes it; a segment
//! definition holds only the data elements after it.

use crate::codec::element::{Cursor, Element, Rules};
use crate::codec::error::{CodecError, ErrorKind, SegmentId};
use crate::codec::schema::FieldSchema;
use crate::codec::types::{An, Num};
use crate::codec::wire;
use crate::parser::{DEG, DataElement, RawSegment};

/// One segment version, declared with `#[derive(Segment)]`.
pub trait Segment: Sized {
    /// Segment code ("Segmentkennung"), e.g. `HKSAL`.
    const CODE: &'static str;
    /// Segment version.
    const VERSION: u16;
    /// The data elements after the header, in wire order.
    const SCHEMA: &'static [FieldSchema];

    /// Reads the data elements after the header.
    fn decode_body(body: &[DEG]) -> Result<Self, CodecError>;

    /// Writes the data elements after the header.
    fn encode_body(&self) -> Result<Vec<DEG>, CodecError>;
}

/// The segment header ("Segmentkopf", Formals B.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentHeader {
    /// Segment code, `an ..6`.
    pub code: String,
    /// Position of the segment in its message, `num ..3`, at least 1.
    pub number: u16,
    /// Segment version, `num ..3`.
    pub version: u16,
    /// Number of the customer segment this bank segment refers to
    /// ("Bezugssegment"); `None` in customer messages.
    pub reference: Option<u16>,
}

const HEADER_CODE: Rules = Rules {
    max: Some(6),
    len: None,
};
const HEADER_NUM: Rules = Rules {
    max: Some(3),
    len: None,
};

fn header_num(value: Num, what: &str) -> Result<u16, CodecError> {
    u16::try_from(value.value())
        .map_err(|_| CodecError::new(ErrorKind::InvalidHeader(format!("{what} out of range"))))
}

impl SegmentHeader {
    /// Reads a header group.
    pub fn decode(deg: &DEG) -> Result<Self, CodecError> {
        let mut cur = Cursor::new(&deg.0);
        let fail = |what: &'static str| {
            move |e: CodecError| {
                CodecError::new(ErrorKind::InvalidHeader(format!("{what}: {}", e.kind)))
                    .at_element(e.element.unwrap_or(1))
                    .at_de(1)
            }
        };
        let code = An::decode(&mut cur, &HEADER_CODE).map_err(fail("code"))?;
        let number = Num::decode(&mut cur, &HEADER_NUM).map_err(fail("number"))?;
        let version = Num::decode(&mut cur, &HEADER_NUM).map_err(fail("version"))?;
        let reference = Option::<Num>::decode(&mut cur, &HEADER_NUM).map_err(fail("reference"))?;
        if !cur.rest_empty() {
            return Err(CodecError::new(ErrorKind::InvalidHeader(
                "more than four elements".into(),
            ))
            .at_de(1));
        }
        Ok(SegmentHeader {
            code: code.into_string(),
            number: header_num(number, "number").map_err(|e| e.at_de(1))?,
            version: header_num(version, "version").map_err(|e| e.at_de(1))?,
            reference: reference
                .map(|r| header_num(r, "reference"))
                .transpose()
                .map_err(|e| e.at_de(1))?,
        })
    }

    /// Writes the header group.
    pub fn encode(&self) -> DEG {
        let mut elements = vec![
            DataElement::Text(self.code.clone()),
            DataElement::Text(self.number.to_string()),
            DataElement::Text(self.version.to_string()),
        ];
        if let Some(r) = self.reference {
            elements.push(DataElement::Text(r.to_string()));
        }
        DEG(elements)
    }

    /// The identification used in errors.
    pub fn id(&self) -> SegmentId {
        SegmentId {
            code: self.code.clone(),
            version: self.version,
            number: self.number,
        }
    }
}

/// Reads a tokenized segment as `T`: checks code and version, then reads
/// the body. Errors carry the segment's code, version and number.
pub fn decode_segment<T: Segment>(raw: &RawSegment) -> Result<(SegmentHeader, T), CodecError> {
    let header = raw
        .degs
        .first()
        .map(SegmentHeader::decode)
        .unwrap_or_else(|| Err(CodecError::new(ErrorKind::InvalidHeader("empty".into()))))?;
    if header.code != T::CODE || header.version != T::VERSION {
        return Err(CodecError::new(ErrorKind::WrongSegment {
            expected_code: T::CODE,
            expected_version: T::VERSION,
        })
        .at_de(1)
        .in_segment(header.id()));
    }
    let body = raw.degs.get(1..).unwrap_or_default();
    let value = T::decode_body(body).map_err(|e| e.in_segment(header.id()))?;
    Ok((header, value))
}

/// Writes `value` as a tokenized segment with the given segment number and
/// optional reference.
pub fn encode_segment<T: Segment>(
    value: &T,
    number: u16,
    reference: Option<u16>,
) -> Result<RawSegment, CodecError> {
    let header = SegmentHeader {
        code: T::CODE.to_string(),
        number,
        version: T::VERSION,
        reference,
    };
    let mut degs = vec![header.encode()];
    degs.extend(value.encode_body().map_err(|e| e.in_segment(header.id()))?);
    Ok(RawSegment { degs })
}

/// Writes `value` as wire bytes (ISO 8859-1, trailing empties cut).
pub fn to_wire<T: Segment>(
    value: &T,
    number: u16,
    reference: Option<u16>,
) -> Result<Vec<u8>, CodecError> {
    let raw = encode_segment(value, number, reference)?;
    wire::serialize_segment(&raw).map_err(|e| {
        e.in_segment(SegmentId {
            code: T::CODE.to_string(),
            version: T::VERSION,
            number,
        })
    })
}

/// Reads one segment from wire bytes as `T`.
pub fn from_wire<T: Segment>(bytes: &[u8]) -> Result<(SegmentHeader, T), CodecError> {
    let (raw, end) = wire::tokenize_segment(bytes, 0)?;
    if end != bytes.len() {
        return Err(CodecError::new(ErrorKind::UnexpectedData));
    }
    decode_segment(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(s: &[u8]) -> Result<SegmentHeader, CodecError> {
        let (raw, _) = wire::tokenize_segment(s, 0).unwrap();
        SegmentHeader::decode(&raw.degs[0])
    }

    #[test]
    fn header_with_and_without_reference() {
        let h = header(b"HIRMS:4:2:3'").unwrap();
        assert_eq!(
            h,
            SegmentHeader {
                code: "HIRMS".into(),
                number: 4,
                version: 2,
                reference: Some(3)
            }
        );
        assert_eq!(header(b"HNVSK:998:3'").unwrap().reference, None);
    }

    #[test]
    fn malformed_headers_are_typed_errors() {
        for bad in [
            &b"'"[..],
            b"HKSAL'",
            b"HKSAL:x:7'",
            b"HKSAL:3:07'",
            b"HKSAL:3:1000'",
            b"TOOLONG:1:1'",
            b"HKSAL:1:1:2:3'",
        ] {
            let e = header(bad).unwrap_err();
            assert!(matches!(e.kind, ErrorKind::InvalidHeader(_)), "{e}");
            assert_eq!(e.de, Some(1));
        }
    }
}
