//! Finding the structure of a FinTS message without parsing it.
//!
//! Quirks repair messages the parser would reject, so they cannot use it.
//! The functions here locate segments, groups and elements by byte offsets.
//! A separator counts only when it is not escaped by `?` and not inside a
//! binary element `@len@...` (Formals B.1). A `@` that does not start a
//! complete binary element is an ordinary character. Nothing here panics on
//! malformed input.

use std::ops::Range;

const SEGMENT_END: u8 = b'\'';
const GROUP_SEP: u8 = b'+';
const ELEMENT_SEP: u8 = b':';
const ESCAPE: u8 = b'?';
const BINARY_MARK: u8 = b'@';

/// Where the bytes of a binary element starting at `data[at]` lie, if a
/// complete one starts there: `@`, decimal length, `@`, that many bytes.
fn binary_content(data: &[u8], at: usize) -> Option<Range<usize>> {
    let after_mark = data.get(at + 1..)?;
    let digits = after_mark.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || after_mark.get(digits) != Some(&BINARY_MARK) {
        return None;
    }
    let len: usize = std::str::from_utf8(&after_mark[..digits])
        .ok()?
        .parse()
        .ok()?;
    let start = at + digits + 2;
    let end = start.checked_add(len)?;
    (end <= data.len()).then_some(start..end)
}

/// Offsets and bytes of the separators that structure `data`.
fn separators(data: &[u8]) -> impl Iterator<Item = (usize, u8)> + '_ {
    let mut pos = 0;
    std::iter::from_fn(move || {
        while let Some(&byte) = data.get(pos) {
            let at = pos;
            pos = match byte {
                ESCAPE => at + 2,
                BINARY_MARK => binary_content(data, at).map_or(at + 1, |c| c.end),
                SEGMENT_END | GROUP_SEP | ELEMENT_SEP => {
                    pos = at + 1;
                    return Some((at, byte));
                }
                _ => at + 1,
            };
        }
        None
    })
}

/// The pieces of `data` between `sep`s, separators left out. A separator
/// at the very end leaves an empty last piece; empty input has no pieces.
fn pieces(data: &[u8], sep: u8) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut from = 0;
    for (at, _) in separators(data).filter(|&(_, byte)| byte == sep) {
        out.push(from..at);
        from = at + 1;
    }
    if from < data.len() || from > 0 {
        out.push(from..data.len());
    }
    out
}

/// Whether `data` ends with a segment end mark that is a real separator,
/// so that no segment is cut off.
pub(crate) fn is_terminated(data: &[u8]) -> bool {
    separators(data)
        .last()
        .is_some_and(|(at, byte)| byte == SEGMENT_END && at + 1 == data.len())
}

/// The segments of a message, each with its closing `'`. Line breaks and
/// blanks some banks put between segments are not part of a segment. An
/// unterminated rest at the end is returned as a last segment.
pub(crate) fn segments(msg: &[u8]) -> Vec<Range<usize>> {
    let mut ends: Vec<usize> = separators(msg)
        .filter(|&(_, byte)| byte == SEGMENT_END)
        .map(|(at, _)| at + 1)
        .collect();
    if ends.last().copied().unwrap_or(0) < msg.len() {
        ends.push(msg.len());
    }
    let mut start = 0;
    let mut out = Vec::with_capacity(ends.len());
    for end in ends {
        let blank = msg[start..end]
            .iter()
            .take_while(|b| b" \r\n".contains(b))
            .count();
        if start + blank < end {
            out.push(start + blank..end);
        }
        start = end;
    }
    out
}

/// The data element groups of one segment, offsets into `seg`; the closing
/// `'` belongs to none of them.
pub(crate) fn degs(seg: &[u8]) -> Vec<Range<usize>> {
    let body = seg.strip_suffix(&[SEGMENT_END]).unwrap_or(seg);
    pieces(body, GROUP_SEP)
}

/// The data elements of one group, offsets into `deg`.
pub(crate) fn des(deg: &[u8]) -> Vec<Range<usize>> {
    pieces(deg, ELEMENT_SEP)
}

/// The segment code, the bytes before the first separator (`HNHBK`).
pub(crate) fn segment_type(seg: &[u8]) -> &[u8] {
    let is_sep = |b: &u8| matches!(*b, SEGMENT_END | GROUP_SEP | ELEMENT_SEP);
    seg.split(is_sep).next().unwrap_or_default()
}

/// The content of `de` if the whole element is one binary element.
pub(crate) fn binary_payload(de: &[u8]) -> Option<&[u8]> {
    let content = binary_content(de, 0).filter(|c| c.end == de.len())?;
    (de.first() == Some(&BINARY_MARK)).then(|| &de[content])
}

/// `data` with the bytes in `range` replaced by `with`.
pub(crate) fn splice(data: &[u8], range: Range<usize>, with: &[u8]) -> Vec<u8> {
    [&data[..range.start], with, &data[range.end..]].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(data: &[u8], ranges: Vec<Range<usize>>) -> Vec<&[u8]> {
        ranges.into_iter().map(|r| &data[r]).collect()
    }

    #[test]
    fn segment_ends_inside_escapes_and_binary_data_do_not_count() {
        let msg = b"AAA:1:1+x?'y'BBB:2:1+@3@'+''\r\nCCC:3:1'";
        let expected: [&[u8]; 3] = [b"AAA:1:1+x?'y'", b"BBB:2:1+@3@'+''", b"CCC:3:1'"];
        assert_eq!(cut(msg, segments(msg)), expected);
    }

    #[test]
    fn unterminated_rest_is_a_last_segment() {
        let msg = b"AAA:1:1'BBB:2";
        let expected: [&[u8]; 2] = [b"AAA:1:1'", b"BBB:2"];
        assert_eq!(cut(msg, segments(msg)), expected);
        assert!(segments(b"").is_empty());
        assert!(segments(b"\r\n").is_empty());
    }

    #[test]
    fn groups_and_elements() {
        let seg = b"HIRMS:4:2:3+3040::Weitere?: Daten:TP-1+0020::ok+'";
        let groups = degs(seg);
        let expected: [&[u8]; 4] = [
            b"HIRMS:4:2:3",
            b"3040::Weitere?: Daten:TP-1",
            b"0020::ok",
            b"",
        ];
        assert_eq!(cut(seg, groups.clone()), expected);

        let group = &seg[groups[1].clone()];
        let expected: [&[u8]; 4] = [b"3040", b"", b"Weitere?: Daten", b"TP-1"];
        assert_eq!(cut(group, des(group)), expected);
        assert!(des(b"").is_empty());
    }

    #[test]
    fn termination() {
        assert!(is_terminated(b"AAA:1:1'"));
        assert!(is_terminated(b"AAA:1:1+@1@''"));
        for open in [&b""[..], b"AAA", b"AAA?'", b"AAA+@2@x'", b"AAA'B", b"A?"] {
            assert!(!is_terminated(open), "{:?}", String::from_utf8_lossy(open));
        }
    }

    #[test]
    fn segment_codes() {
        assert_eq!(segment_type(b"HNHBK:1:3+x'"), b"HNHBK");
        assert_eq!(segment_type(b"HNHBS'"), b"HNHBS");
        assert_eq!(segment_type(b""), b"");
    }

    #[test]
    fn binary_payloads() {
        assert_eq!(binary_payload(b"@4@'+:?"), Some(&b"'+:?"[..]));
        assert_eq!(binary_payload(b"@0@"), Some(&b""[..]));
        assert_eq!(binary_payload(b"@4@abc"), None);
        assert_eq!(binary_payload(b"@2@abc"), None);
        assert_eq!(binary_payload(b"x@1@a"), None);
    }

    #[test]
    fn splicing() {
        assert_eq!(splice(b"HNHBK:1:3", 6..7, b"9"), b"HNHBK:9:3");
        assert_eq!(splice(b"ab", 1..1, b"--"), b"a--b");
    }

    #[test]
    fn broken_binary_marks_are_plain_bytes() {
        let huge = format!("A+@{}@'", usize::MAX);
        let broken = [
            &b"@"[..],
            b"?",
            b"A+@",
            b"A+@7@xy'",
            b"A+@@'",
            huge.as_bytes(),
        ];
        for msg in broken {
            segments(msg);
            degs(msg);
            des(msg);
            binary_payload(msg);
            is_terminated(msg);
        }
        assert_eq!(segments(b"A+@7@xy'B'").len(), 2);
    }
}
