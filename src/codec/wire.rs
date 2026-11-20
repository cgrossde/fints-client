//! FinTS wire syntax (Formals H.1): tokenizer and serializer.
//!
//! - Segments end with `'`, data elements are separated by `+`, group data
//!   elements by `:` (H.1.1, H.1.2). Nested groups are flattened: the wire
//!   knows only two levels.
//! - `?` releases the next character (H.1.3).
//! - `@<len>@<bytes>` carries binary data; its bytes are not interpreted
//!   (H.1.4).
//! - Text is ISO 8859-1 (B.1, "Codeset 1 (Latin 1)"): every byte is one
//!   character, U+0000 to U+00FF, both ways.
//! - On output, empty elements at the end of a group and empty data elements
//!   at the end of a segment are left out (H.1.5); empty positions in the
//!   middle stay as bare separators.
//!
//! The tokenizer produces the crate's existing [`RawSegment`] /
//! [`DEG`] / [`DataElement`] representation: `degs[0]` is the segment
//! header, every further entry one data element (a `DEG` of one or more
//! group elements). It never panics; malformed input gives a [`WireError`]
//! with the byte offset.
//!
//! Difference to the older `crate::parser`: that one decodes text as
//! Windows-1252, which maps bytes 0x80–0x9F to other characters (e.g.
//! 0x80 to '€') and so cannot give back the original bytes. This one decodes
//! ISO 8859-1 exactly, so `serialize(tokenize(x)) == x` for canonical input.

use crate::codec::error::{CodecError, ErrorKind, WireError, WireErrorKind};
use crate::parser::{DEG, DataElement, RawSegment};

/// Tokenizes a whole message (or the clear text of an HNVSD) into segments.
///
/// Carriage returns, line feeds and spaces between segments are skipped.
pub fn tokenize(input: &[u8]) -> Result<Vec<RawSegment>, WireError> {
    let mut segments = Vec::new();
    let mut pos = 0;
    while pos < input.len() {
        match input.get(pos) {
            Some(b'\r' | b'\n' | b' ') => pos += 1,
            _ => {
                let (seg, next) = tokenize_segment(input, pos)?;
                segments.push(seg);
                pos = next;
            }
        }
    }
    Ok(segments)
}

/// Tokenizes one segment starting at `start`; returns it and the offset
/// after its `'`.
pub fn tokenize_segment(input: &[u8], start: usize) -> Result<(RawSegment, usize), WireError> {
    let err = |offset: usize, kind: WireErrorKind| WireError { offset, kind };

    let mut degs: Vec<DEG> = Vec::new();
    let mut group: Vec<DataElement> = Vec::new();
    // The element being read: None = nothing yet (empty element).
    let mut current: Option<DataElement> = None;
    let mut pos = start;

    loop {
        let Some(&byte) = input.get(pos) else {
            return Err(err(pos, WireErrorKind::UnterminatedSegment));
        };
        match byte {
            b':' | b'+' | b'\'' => {
                group.push(current.take().unwrap_or(DataElement::Empty));
                pos += 1;
                if byte != b':' {
                    degs.push(DEG(std::mem::take(&mut group)));
                }
                if byte == b'\'' {
                    return Ok((RawSegment { degs }, pos));
                }
            }
            b'?' => {
                let Some(&escaped) = input.get(pos + 1) else {
                    return Err(err(pos, WireErrorKind::DanglingEscape));
                };
                push_char(&mut current, escaped).map_err(|k| err(pos, k))?;
                pos += 2;
            }
            b'@' => {
                if current.is_some() {
                    return Err(err(pos, WireErrorKind::BinaryInsideText));
                }
                let (data, next) = read_binary(input, pos)?;
                current = Some(DataElement::Binary(data));
                pos = next;
                match input.get(pos) {
                    Some(b':' | b'+' | b'\'') => {}
                    Some(_) => return Err(err(pos, WireErrorKind::DataAfterBinary)),
                    None => return Err(err(pos, WireErrorKind::UnterminatedSegment)),
                }
            }
            other => {
                push_char(&mut current, other).map_err(|k| err(pos, k))?;
                pos += 1;
            }
        }
    }
}

/// Appends one ISO 8859-1 byte to the text element being read.
fn push_char(current: &mut Option<DataElement>, byte: u8) -> Result<(), WireErrorKind> {
    let ch = char::from(byte);
    match current {
        None => {
            *current = Some(DataElement::Text(ch.to_string()));
            Ok(())
        }
        Some(DataElement::Text(s)) => {
            s.push(ch);
            Ok(())
        }
        // The tokenizer checks for a separator right after binary data, and
        // never stores Empty in `current`; kept total for safety.
        Some(_) => Err(WireErrorKind::DataAfterBinary),
    }
}

/// Reads `@<len>@<bytes>` at `at` (which holds the first `@`).
fn read_binary(input: &[u8], at: usize) -> Result<(Vec<u8>, usize), WireError> {
    let bad = WireError {
        offset: at,
        kind: WireErrorKind::BadBinaryLength,
    };
    let mut pos = at + 1;
    let mut len: usize = 0;
    let mut digits = 0;
    loop {
        match input.get(pos) {
            Some(&b) if b.is_ascii_digit() => {
                len = len
                    .checked_mul(10)
                    .and_then(|l| l.checked_add(usize::from(b - b'0')))
                    .ok_or_else(|| bad.clone())?;
                digits += 1;
                pos += 1;
            }
            Some(b'@') if digits > 0 => {
                pos += 1;
                break;
            }
            _ => return Err(bad),
        }
    }
    let end = pos.checked_add(len).filter(|&end| end <= input.len());
    match end.and_then(|end| input.get(pos..end).map(|d| (d, end))) {
        Some((data, end)) => Ok((data.to_vec(), end)),
        None => Err(WireError {
            offset: at,
            kind: WireErrorKind::BinaryOverrun { declared: len },
        }),
    }
}

/// True for an element that serialises to nothing.
fn is_blank(de: &DataElement) -> bool {
    match de {
        DataElement::Empty => true,
        DataElement::Text(s) => s.is_empty(),
        DataElement::Binary(_) => false,
    }
}

/// True for a data element (group) that serialises to nothing.
pub(crate) fn is_blank_group(deg: &DEG) -> bool {
    deg.0.iter().all(is_blank)
}

/// Serialises one segment, header included (`degs[0]`), with the trailing
/// empty elements left out (H.1.5).
pub fn serialize_segment(seg: &RawSegment) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    write_segment(&seg.degs, &mut out)?;
    Ok(out)
}

/// Serialises several segments into one message body.
pub fn serialize(segments: &[RawSegment]) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    for seg in segments {
        write_segment(&seg.degs, &mut out)?;
    }
    Ok(out)
}

fn write_segment(degs: &[DEG], out: &mut Vec<u8>) -> Result<(), CodecError> {
    // The header (index 0) is always written, even if blank.
    let used = degs
        .iter()
        .rposition(|d| !is_blank_group(d))
        .map_or(1, |i| i + 1)
        .min(degs.len());
    for (i, deg) in degs.iter().take(used).enumerate() {
        if i > 0 {
            out.push(b'+');
        }
        write_group(deg, out).map_err(|e| e.at_de(i + 1))?;
    }
    out.push(b'\'');
    Ok(())
}

fn write_group(deg: &DEG, out: &mut Vec<u8>) -> Result<(), CodecError> {
    let used = deg
        .0
        .iter()
        .rposition(|d| !is_blank(d))
        .map_or(0, |i| i + 1);
    for (i, de) in deg.0.iter().take(used).enumerate() {
        if i > 0 {
            out.push(b':');
        }
        write_element(de, out).map_err(|e| e.at_element(i + 1))?;
    }
    Ok(())
}

fn write_element(de: &DataElement, out: &mut Vec<u8>) -> Result<(), CodecError> {
    match de {
        DataElement::Empty => {}
        DataElement::Text(s) => {
            for ch in s.chars() {
                let byte = u8::try_from(u32::from(ch))
                    .map_err(|_| CodecError::new(ErrorKind::NotLatin1 { found: ch }))?;
                if matches!(byte, b'+' | b':' | b'\'' | b'?' | b'@') {
                    out.push(b'?');
                }
                out.push(byte);
            }
        }
        DataElement::Binary(data) => {
            out.push(b'@');
            out.extend_from_slice(data.len().to_string().as_bytes());
            out.push(b'@');
            out.extend_from_slice(data);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn text(s: &str) -> DataElement {
        DataElement::Text(s.to_string())
    }

    fn one(input: &[u8]) -> RawSegment {
        let mut segs = tokenize(input).unwrap();
        assert_eq!(segs.len(), 1);
        segs.remove(0)
    }

    #[test]
    fn separators_build_two_levels() {
        let seg = one(b"HKSAL:3:7+DE00120300009900012345:BYLADEM1001+N++T1'");
        assert_eq!(seg.degs.len(), 5);
        assert_eq!(seg.degs[0].0, vec![text("HKSAL"), text("3"), text("7")]);
        assert_eq!(
            seg.degs[1].0,
            vec![text("DE00120300009900012345"), text("BYLADEM1001")]
        );
        assert_eq!(seg.degs[3].0, vec![DataElement::Empty]);
        assert_eq!(seg.degs[4].0, vec![text("T1")]);
    }

    #[test]
    fn empty_group_elements_keep_their_positions() {
        let seg = one(b"X:1:1+a::c:+'");
        assert_eq!(
            seg.degs[1].0,
            vec![text("a"), DataElement::Empty, text("c"), DataElement::Empty]
        );
        assert_eq!(seg.degs[2].0, vec![DataElement::Empty]);
    }

    #[test]
    fn release_character_formals_h13_examples() {
        // Formals H.1.3, Beispiel 1 and 2.
        let seg = one(b"X:1:1+Taschengeld f\xfcr Hans ?+ Franz+Ist das so richtig????'");
        assert_eq!(seg.degs[1].0, vec![text("Taschengeld für Hans + Franz")]);
        assert_eq!(seg.degs[2].0, vec![text("Ist das so richtig??")]);
        assert_eq!(
            serialize_segment(&seg).unwrap(),
            b"X:1:1+Taschengeld f\xfcr Hans ?+ Franz+Ist das so richtig????'"
        );
    }

    #[test]
    fn binary_data_is_not_interpreted() {
        let seg = one(b"HNVSD:999:1+@7@a+b:c'd'");
        assert_eq!(
            seg.degs[1].0,
            vec![DataElement::Binary(b"a+b:c'd".to_vec())]
        );
        assert_eq!(serialize_segment(&seg).unwrap(), b"HNVSD:999:1+@7@a+b:c'd'");
    }

    #[test]
    fn empty_binary_differs_from_empty_element() {
        let seg = one(b"X:1:1+@0@+x'");
        assert_eq!(seg.degs[1].0, vec![DataElement::Binary(vec![])]);
        assert_eq!(serialize_segment(&seg).unwrap(), b"X:1:1+@0@+x'");
    }

    #[test]
    fn latin1_round_trips_every_byte() {
        // 0x80-0x9F are C1 controls in ISO 8859-1, not Windows-1252 letters.
        let seg = one(b"X:1:1+\x80\x9f\xa4\xff'");
        assert_eq!(seg.degs[1].0, vec![text("\u{80}\u{9f}\u{a4}\u{ff}")]);
        assert_eq!(serialize_segment(&seg).unwrap(), b"X:1:1+\x80\x9f\xa4\xff'");
    }

    #[test]
    fn serializer_leaves_out_trailing_empties_formals_h15() {
        let seg = RawSegment {
            degs: vec![
                DEG(vec![text("X"), text("1"), text("1")]),
                DEG(vec![text("a"), DataElement::Empty, DataElement::Empty]),
                DEG(vec![DataElement::Empty]),
                DEG(vec![DataElement::Empty, text("d")]),
                DEG(vec![]),
                DEG(vec![DataElement::Empty, DataElement::Empty]),
            ],
        };
        assert_eq!(serialize_segment(&seg).unwrap(), b"X:1:1+a++:d'");
    }

    #[test]
    fn serializer_escapes_syntax_characters_and_rejects_non_latin1() {
        let seg = RawSegment {
            degs: vec![
                DEG(vec![text("X"), text("1"), text("1")]),
                DEG(vec![text("a+b:c'd?e@f")]),
            ],
        };
        assert_eq!(serialize_segment(&seg).unwrap(), b"X:1:1+a?+b?:c?'d??e?@f'");
        let bad = RawSegment {
            degs: vec![DEG(vec![text("X")]), DEG(vec![text("ok"), text("€")])],
        };
        let e = serialize_segment(&bad).unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotLatin1 { found: '€' });
        assert_eq!((e.de, e.element), (Some(2), Some(2)));
    }

    #[test]
    fn several_segments_with_line_breaks_between() {
        let segs = tokenize(b"A:1:1+x'\r\nB:2:1'").unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[1].segment_type(), "B");
    }

    #[test]
    fn malformed_input_gives_typed_errors() {
        let kind = |input: &[u8]| tokenize(input).unwrap_err().kind;
        assert_eq!(kind(b"A:1:1+x"), WireErrorKind::UnterminatedSegment);
        assert_eq!(kind(b"A:1:1+x?"), WireErrorKind::DanglingEscape);
        assert_eq!(kind(b"A:1:1+@x@'"), WireErrorKind::BadBinaryLength);
        assert_eq!(kind(b"A:1:1+@@'"), WireErrorKind::BadBinaryLength);
        assert_eq!(
            kind(b"A:1:1+@9@abc'"),
            WireErrorKind::BinaryOverrun { declared: 9 }
        );
        assert_eq!(kind(b"A:1:1+ab@1@c'"), WireErrorKind::BinaryInsideText);
        assert_eq!(kind(b"A:1:1+@1@cd'"), WireErrorKind::DataAfterBinary);
        assert_eq!(
            kind(b"A:1:1+@99999999999999999999999@'"),
            WireErrorKind::BadBinaryLength
        );
        assert_eq!(tokenize(b"A:1:1+x?").unwrap_err().offset, 7);
    }

    fn arb_element() -> impl Strategy<Value = DataElement> {
        prop_oneof![
            Just(DataElement::Empty),
            "[\\x00-\\xff]{1,12}".prop_map(DataElement::Text),
            proptest::collection::vec(any::<u8>(), 0..12).prop_map(DataElement::Binary),
        ]
    }

    fn arb_segment() -> impl Strategy<Value = RawSegment> {
        proptest::collection::vec(proptest::collection::vec(arb_element(), 1..5), 0..6).prop_map(
            |body| {
                let mut degs = vec![DEG(vec![text("ABCDE"), text("1"), text("1")])];
                degs.extend(body.into_iter().map(DEG));
                RawSegment { degs }
            },
        )
    }

    proptest! {
        #[test]
        fn tokenize_never_panics(input in proptest::collection::vec(any::<u8>(), 0..256)) {
            let _ = tokenize(&input);
        }

        #[test]
        fn tokenize_never_panics_on_syntax_heavy_input(
            input in proptest::collection::vec(
                prop_oneof![Just(b'+'), Just(b':'), Just(b'\''), Just(b'?'), Just(b'@'),
                            Just(b'1'), Just(b'9'), Just(b'a')],
                0..64)
        ) {
            let _ = tokenize(&input);
        }

        /// serialize ∘ tokenize ∘ serialize = serialize: the serializer's output
        /// is canonical and tokenizes back to the same bytes.
        #[test]
        fn serialized_segments_round_trip(seg in arb_segment()) {
            let bytes = serialize_segment(&seg).unwrap();
            let back = tokenize(&bytes).unwrap();
            prop_assert_eq!(back.len(), 1);
            prop_assert_eq!(serialize_segment(&back[0]).unwrap(), bytes);
        }
    }
}
