//! Tolerant-parsing quirks: repairs for bank answers that break the message
//! layout of the FinTS 3.0 formals (message header HNHBK, status segment
//! order, continuous segment numbering).
//!
//! Each repair restores a rule of the Formals spec (message header, general
//! message layout, encrypted message) that a bank answer can break.
//!
//! # The four quirks
//!
//! | Quirk | Stage | Repairs |
//! |---|---|---|
//! | [`MissingMessageReference`] | raw | HNHBK without the reference to our message |
//! | [`HeaderVersion`] | raw | HNHBK with an empty or `0` HBCI version |
//! | [`StatusSegmentOrder`] | clear text | HIRMG / HIRMS / data segments in the wrong order |
//! | [`SegmentNumbering`] | clear text | segment numbers that are not 1, 2, 3, ... |
//!
//! # Order in the default chain
//!
//! `MissingMessageReference`, `HeaderVersion`, `StatusSegmentOrder`,
//! `SegmentNumbering`. The two header repairs touch only the first segment and
//! are independent. Reordering moves segments with their numbers unchanged, so
//! numbering must run after it. Numbering has to be last because every other
//! repair may disturb it.
//!
//! All four are no-ops (they return `Cow::Borrowed`) on well-formed messages.
//! A quirk that cannot understand a message (no HNHBK first, malformed
//! segment head, unterminated segment, envelope still present) leaves it
//! unchanged. The HNHBK size field is not recalculated by any of them: the
//! parser does not check it and the envelope unwrapping changes the length
//! anyway.
//!
//! Segments the parser does not know need no repair: they become
//! `RawSegment`s and are reported to the caller.

use std::borrow::Cow;
use std::ops::Range;

use super::{Quirk, QuirkContext, wire};

/// Segments of a clear-text answer, if the quirks below can safely work on it.
///
/// Requires: contiguous segments, every one complete, exactly one HNHBK which
/// comes first, no envelope segments left (HNVSK/HNVSD carry the numbers 998
/// and 999 and hide their content), well-formed segment codes.
fn clear_text_segments(msg: &[u8]) -> Option<Vec<Range<usize>>> {
    if !wire::is_terminated(msg) {
        return None;
    }
    let segs = wire::segments(msg);
    let mut pos = 0;
    let mut hnhbk = 0;
    for (i, r) in segs.iter().enumerate() {
        if r.start != pos {
            return None; // gap (e.g. line breaks) between segments
        }
        pos = r.end;
        let code = wire::segment_type(&msg[r.clone()]);
        if code.is_empty()
            || !code
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return None;
        }
        match code {
            b"HNHBK" if i == 0 => hnhbk += 1,
            b"HNHBK" | b"HNVSK" | b"HNVSD" => return None,
            _ => {}
        }
    }
    (pos == msg.len() && hnhbk == 1).then_some(segs)
}

/// Absolute byte range of the DEG with index `idx` in the HNHBK at the start
/// of `msg` (`HNHBK:1:3+size+version+dialog+msgnum[+reference]`), and the
/// total number of DEGs.
fn hnhbk_degs(msg: &[u8]) -> Option<(Vec<Range<usize>>, usize)> {
    let first = wire::segments(msg).into_iter().next()?;
    let seg = &msg[first.clone()];
    if wire::segment_type(seg) != b"HNHBK" || !wire::is_terminated(seg) {
        return None;
    }
    let degs = wire::degs(seg);
    let abs = degs
        .into_iter()
        .map(|r| first.start + r.start..first.start + r.end)
        .collect();
    Some((abs, first.end))
}

/// Escape the FinTS syntax characters in a value we insert.
fn escape(value: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len());
    for &b in value.as_bytes() {
        if matches!(b, b'+' | b':' | b'\'' | b'?' | b'@') {
            out.push(b'?');
        }
        out.push(b);
    }
    out
}

/// Adds the missing message reference to the bank's HNHBK.
///
/// Misbehaviour: some banks, typically in error answers, send
/// `HNHBK:1:3+size+version+dialog+msgnum` without the sixth DEG, the
/// reference `dialog:msgnum` to the client message being answered. The spec
/// makes it mandatory for bank messages.
///
/// Repair: when the head has exactly five DEGs, append `+dialog:msgnum` with
/// the dialog ID and message number of the request from the
/// [`QuirkContext`]. A present but empty reference is left alone. The head
/// is split into DEGs by the wire helpers, so an escaped `+` is not a
/// separator; a head with fewer than five DEGs is too broken to guess at and
/// stays as it is.
#[derive(Debug, Clone, Copy, Default)]
pub struct MissingMessageReference;

impl Quirk for MissingMessageReference {
    fn name(&self) -> &'static str {
        "MissingMessageReference"
    }

    fn incoming_raw<'a>(&self, msg: &'a [u8], ctx: &QuirkContext<'_>) -> Cow<'a, [u8]> {
        let Some((degs, seg_end)) = hnhbk_degs(msg) else {
            return Cow::Borrowed(msg);
        };
        if degs.len() != 5 {
            return Cow::Borrowed(msg);
        }
        let mut reference = b"+".to_vec();
        reference.extend(escape(ctx.dialog_id));
        reference.push(b':');
        reference.extend_from_slice(ctx.message_number.to_string().as_bytes());
        // `seg_end - 1` is the terminating `'` (checked by `hnhbk_degs`).
        let at = seg_end - 1;
        Cow::Owned(wire::splice(msg, at..at, &reference))
    }
}

/// Replaces an empty or `0` HBCI version in the bank's HNHBK.
///
/// Misbehaviour: in answers that report a global error about our message,
/// some banks put an empty value or `0` where the HBCI version belongs.
///
/// Repair: that element becomes the version we sent
/// ([`QuirkContext::hbci_version`]). Any other value, including a wrong
/// non-zero one such as `220`, is left alone: the caller decides what to do
/// with a real version mismatch.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeaderVersion;

impl Quirk for HeaderVersion {
    fn name(&self) -> &'static str {
        "HeaderVersion"
    }

    fn incoming_raw<'a>(&self, msg: &'a [u8], ctx: &QuirkContext<'_>) -> Cow<'a, [u8]> {
        let Some((degs, _)) = hnhbk_degs(msg) else {
            return Cow::Borrowed(msg);
        };
        let Some(version) = degs.get(2) else {
            return Cow::Borrowed(msg);
        };
        let current = &msg[version.clone()];
        if ctx.hbci_version.is_empty() || !(current.is_empty() || current == b"0") {
            return Cow::Borrowed(msg);
        }
        Cow::Owned(wire::splice(
            msg,
            version.clone(),
            &escape(ctx.hbci_version),
        ))
    }
}

/// Puts status segments in the order the spec prescribes.
///
/// Misbehaviour: the spec orders a bank answer as message header (HNHBK, then
/// the signature head HNSHK if present), exactly one global status HIRMG, the
/// segment status segments HIRMS, then all data segments, then the signature
/// trailer HNSHA and the message trailer HNHBS. Some banks send HIRMS before
/// HIRMG or a data segment before the status segments.
///
/// Repair: if the segments are not already in that order, they are stably
/// sorted by those five groups, so the relative order inside a group stays.
/// Segment bytes, and therefore segment numbers, are copied verbatim;
/// [`SegmentNumbering`] runs afterwards.
///
/// A message without HIRMG is reordered only if something else is out of
/// order. HNSHA and HNHBS form their own trailer group, so they stay at the
/// end. A message with an unterminated trailing segment is left untouched.
#[derive(Debug, Clone, Copy, Default)]
pub struct StatusSegmentOrder;

fn status_rank(code: &[u8]) -> u8 {
    match code {
        b"HNHBK" | b"HNSHK" => 0,
        b"HIRMG" => 1,
        b"HIRMS" => 2,
        b"HNSHA" | b"HNHBS" => 4,
        _ => 3,
    }
}

impl Quirk for StatusSegmentOrder {
    fn name(&self) -> &'static str {
        "StatusSegmentOrder"
    }

    fn incoming<'a>(&self, msg: &'a [u8], _ctx: &QuirkContext<'_>) -> Cow<'a, [u8]> {
        let Some(segs) = clear_text_segments(msg) else {
            return Cow::Borrowed(msg);
        };
        let mut ranked: Vec<(u8, &Range<usize>)> = segs
            .iter()
            .map(|r| (status_rank(wire::segment_type(&msg[r.clone()])), r))
            .collect();
        if ranked.windows(2).all(|w| w[0].0 <= w[1].0) {
            return Cow::Borrowed(msg);
        }
        ranked.sort_by_key(|(rank, _)| *rank); // stable
        let mut out = Vec::with_capacity(msg.len());
        for (_, r) in ranked {
            out.extend_from_slice(&msg[r.clone()]);
        }
        Cow::Owned(out)
    }
}

/// Renumbers the segments of the clear-text answer 1, 2, 3, ...
///
/// Misbehaviour: the second DE of every segment head is the segment number,
/// which the spec wants continuous within a message. Some banks send gaps,
/// duplicates or a restart.
///
/// Where numbering starts: after the PIN/TAN envelope
/// has been unwrapped the spec says the message "wieder eine kontinuierliche
/// Segmentnumerierung aufweist" (Formals, encrypted message), so the clear
/// text starts with HNHBK as number 1 and the inner segments continue from 2.
/// The quirk only works on a message that starts with HNHBK and no longer
/// contains HNVSK/HNVSD (whose numbers 998 and 999 are exempt from the rule).
///
/// Repair: each segment whose number differs from its position gets the
/// position. Only those digits change; the segment reference (fourth head
/// element), the size field and everything else stay.
///
/// Left unchanged: a message with a segment head whose number is not
/// numeric (the parser reports it), and one with more than 999 segments
/// (the number has at most three digits).
#[derive(Debug, Clone, Copy, Default)]
pub struct SegmentNumbering;

impl Quirk for SegmentNumbering {
    fn name(&self) -> &'static str {
        "SegmentNumbering"
    }

    fn incoming<'a>(&self, msg: &'a [u8], _ctx: &QuirkContext<'_>) -> Cow<'a, [u8]> {
        let Some(segs) = clear_text_segments(msg) else {
            return Cow::Borrowed(msg);
        };
        if segs.len() > 999 {
            return Cow::Borrowed(msg);
        }
        // (range of the number digits in `msg`, wanted number)
        let mut fixes: Vec<(Range<usize>, usize)> = Vec::new();
        for (i, r) in segs.iter().enumerate() {
            let seg = &msg[r.clone()];
            let Some(head) = wire::degs(seg).into_iter().next() else {
                return Cow::Borrowed(msg);
            };
            let Some(number) = wire::des(&seg[head.clone()]).into_iter().nth(1) else {
                return Cow::Borrowed(msg);
            };
            let digits = &seg[head.start + number.start..head.start + number.end];
            if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
                return Cow::Borrowed(msg);
            }
            let want = i + 1;
            if digits != want.to_string().as_bytes() {
                fixes.push((
                    r.start + head.start + number.start..r.start + head.start + number.end,
                    want,
                ));
            }
        }
        if fixes.is_empty() {
            return Cow::Borrowed(msg);
        }
        let mut out = Vec::with_capacity(msg.len() + fixes.len());
        let mut pos = 0;
        for (range, want) in fixes {
            out.extend_from_slice(&msg[pos..range.start]);
            out.extend_from_slice(want.to_string().as_bytes());
            pos = range.end;
        }
        out.extend_from_slice(&msg[pos..]);
        Cow::Owned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quirks::{QuirkChain, unwrap_envelope};
    use proptest::prelude::*;

    const CTX: QuirkContext<'static> = QuirkContext {
        hbci_version: "300",
        dialog_id: "DLG9",
        message_number: 2,
    };

    fn raw(q: &dyn Quirk, msg: &str) -> String {
        String::from_utf8(q.incoming_raw(msg.as_bytes(), &CTX).into_owned()).unwrap()
    }

    fn clear(q: &dyn Quirk, msg: &str) -> String {
        String::from_utf8(q.incoming(msg.as_bytes(), &CTX).into_owned()).unwrap()
    }

    // A data segment with an escaped quote and plus, and a binary element
    // containing a quote and a plus; every quirk must carry it through intact.
    const TRICKY: &str = "HIUPD:4:6+a?'b?+c?:d??+@6@x'y+z!'";

    // --- MissingMessageReference ---

    #[test]
    fn reference_is_added_when_missing() {
        let before = "HNHBK:1:3+000000000100+300+DLG9+2'HIRMG:2:2+9800::Fehler'";
        let after = "HNHBK:1:3+000000000100+300+DLG9+2+DLG9:2'HIRMG:2:2+9800::Fehler'";
        assert_eq!(raw(&MissingMessageReference, before), after);
    }

    #[test]
    fn reference_repair_leaves_following_segments_byte_identical() {
        let before = format!("HNHBK:1:3+000000000100+300+DLG9+2'HIRMG:2:2+9800::a?'b'{TRICKY}");
        let after =
            format!("HNHBK:1:3+000000000100+300+DLG9+2+DLG9:2'HIRMG:2:2+9800::a?'b'{TRICKY}");
        assert_eq!(raw(&MissingMessageReference, &before), after);
    }

    #[test]
    fn reference_escapes_inserted_dialog_id() {
        let ctx = QuirkContext {
            dialog_id: "A+B:C?",
            ..CTX
        };
        let out = MissingMessageReference
            .incoming_raw(b"HNHBK:1:3+1+300+X+2'HIRMG:2:2+0010::ok'", &ctx)
            .into_owned();
        assert_eq!(
            out,
            b"HNHBK:1:3+1+300+X+2+A?+B?:C??:2'HIRMG:2:2+0010::ok'".to_vec()
        );
    }

    #[test]
    fn reference_untouched_when_present_empty_or_head_short() {
        for msg in [
            "HNHBK:1:3+000000000100+300+DLG9+2+DLG9:2'HIRMG:2:2+0010::ok'",
            "HNHBK:1:3+000000000100+300+DLG9+2+'HIRMG:2:2+0010::ok'",
            "HNHBK:1:3+000000000100+300'HIRMG:2:2+0010::ok'",
            "HNHBK:1:3+000000000100+300+DLG9+2+DLG9:2",
            "HNHBK:1:3+000000000100+300+DLG9+2",
            "HIRMG:2:2+0010::ok'",
            "",
        ] {
            assert_eq!(raw(&MissingMessageReference, msg), msg);
        }
    }

    #[test]
    fn reference_escaped_plus_in_head_is_not_a_separator() {
        // The dialog ID contains an escaped plus: still five DEGs, so repaired once.
        let before = "HNHBK:1:3+1+300+D?+1+2'HIRMG:2:2+0010::ok'";
        assert_eq!(
            raw(&MissingMessageReference, before),
            "HNHBK:1:3+1+300+D?+1+2+DLG9:2'HIRMG:2:2+0010::ok'"
        );
    }

    // --- HeaderVersion ---

    #[test]
    fn version_zero_and_empty_are_replaced() {
        let ok = "HNHBK:1:3+000000000090+300+DLG9+1+DLG9:1'HIRMG:2:2+9050::Nachricht fehlerhaft'";
        for bad in [
            "HNHBK:1:3+000000000090+0+DLG9+1+DLG9:1'HIRMG:2:2+9050::Nachricht fehlerhaft'",
            "HNHBK:1:3+000000000090++DLG9+1+DLG9:1'HIRMG:2:2+9050::Nachricht fehlerhaft'",
        ] {
            assert_eq!(raw(&HeaderVersion, bad), ok);
        }
    }

    #[test]
    fn version_repair_keeps_tricky_segments() {
        let before = format!("HNHBK:1:3+1+0+D+1+D:1'HIRMG:2:2+0010::a?'b'{TRICKY}");
        let after = format!("HNHBK:1:3+1+300+D+1+D:1'HIRMG:2:2+0010::a?'b'{TRICKY}");
        assert_eq!(raw(&HeaderVersion, &before), after);
    }

    #[test]
    fn version_other_values_and_odd_heads_untouched() {
        for msg in [
            "HNHBK:1:3+1+300+D+1+D:1'HIRMG:2:2+0010::ok'",
            "HNHBK:1:3+1+220+D+1+D:1'HIRMG:2:2+0010::ok'",
            "HNHBK:1:3+1+00+D+1'",
            "HNHBK:1:3+1'",
            "HNHBK:1:3",
            "HIRMG:2:2+1+0+3'",
            "",
        ] {
            assert_eq!(raw(&HeaderVersion, msg), msg);
        }
    }

    #[test]
    fn version_search_does_not_leave_first_segment() {
        // Version is fine; a later segment has an empty DEG in the same place.
        let msg = "HNHBK:1:3+1+300+D+1+D:1'HIRMG:2:2++0+x'";
        assert_eq!(raw(&HeaderVersion, msg), msg);
    }

    // --- StatusSegmentOrder ---

    #[test]
    fn hirms_before_hirmg_is_swapped() {
        let before = "HNHBK:1:3+000000000123+300+DLG1+1+DLG1:1'HIRMS:3:2:3+0020::ok'HIRMG:2:2+0010::ok'HIUPD:4:6+x'";
        let after = "HNHBK:1:3+000000000123+300+DLG1+1+DLG1:1'HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::ok'HIUPD:4:6+x'";
        assert_eq!(clear(&StatusSegmentOrder, before), after);
    }

    #[test]
    fn data_before_status_and_trailer_stay_in_place() {
        let before = format!(
            "HNHBK:1:3+1+300+D+1'{TRICKY}HIRMS:3:2:3+0020::a?'b'HIRMG:2:2+0010::ok?+'HIRMS:5:2:4+0021::z'HISAL:6:7+@3@'+'HNHBS:7:1+1'"
        );
        let after = format!(
            "HNHBK:1:3+1+300+D+1'HIRMG:2:2+0010::ok?+'HIRMS:3:2:3+0020::a?'b'HIRMS:5:2:4+0021::z'{TRICKY}HISAL:6:7+@3@'+'HNHBS:7:1+1'"
        );
        assert_eq!(clear(&StatusSegmentOrder, &before), after);
    }

    #[test]
    fn signature_head_stays_with_header_and_trailer_last() {
        let before = "HNHBK:1:3+1+300+D+1'HNSHK:2:4+PIN:1'HIRMS:4:2+0020::a'HIRMG:3:2+0010::b'HIX:5:1+q'HNSHA:6:2+1'HNHBS:7:1+1'";
        let after = "HNHBK:1:3+1+300+D+1'HNSHK:2:4+PIN:1'HIRMG:3:2+0010::b'HIRMS:4:2+0020::a'HIX:5:1+q'HNSHA:6:2+1'HNHBS:7:1+1'";
        assert_eq!(clear(&StatusSegmentOrder, before), after);
    }

    #[test]
    fn correct_or_not_understood_order_is_untouched() {
        for msg in [
            "HNHBK:1:3+1+300+D+1'HIRMG:2:2+0010::ok'HIRMS:3:2+0020::ok'HIX:4:1+q'HNHBS:5:1+1'",
            // no HIRMG at all but in a consistent order
            "HNHBK:1:3+1+300+D+1'HIRMS:2:2+0020::ok'HIX:3:1+q'HNHBS:4:1+1'",
            // unterminated trailing segment: not understood
            "HNHBK:1:3+1+300+D+1'HIRMS:2:2+0020::ok'HIRMG:3:2+0010::ok'HIX:4:1+q",
            // does not start with HNHBK
            "HIRMS:2:2+0020::ok'HIRMG:3:2+0010::ok'",
            // envelope still present
            "HNHBK:1:3+1+300+D+1'HNVSK:998:3+PIN:1'HIRMS:3:2+1::a'HIRMG:2:2+1::b'",
            // line breaks between segments
            "HNHBK:1:3+1+300+D+1'\r\nHIRMS:2:2+0020::ok'\r\nHIRMG:3:2+0010::ok'",
            "",
        ] {
            assert_eq!(clear(&StatusSegmentOrder, msg), msg, "{msg:?}");
        }
    }

    // --- SegmentNumbering ---

    #[test]
    fn wrong_numbers_are_fixed() {
        let before = "HNHBK:1:3+1+300+D+1+D:1'HIRMG:5:2+0010::ok'HIRMS:5:2:3+0020::ok'HIUPD:9:6+x'";
        let after = "HNHBK:1:3+1+300+D+1+D:1'HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::ok'HIUPD:4:6+x'";
        assert_eq!(clear(&SegmentNumbering, before), after);
    }

    #[test]
    fn numbering_changes_width_and_keeps_references_and_tricky_data() {
        let before = format!(
            "HNHBK:7:3+1+300+D+1'HIRMG:12:2+0010::a?'b'HIRMS:2:2:3+0020::ok'{TRICKY}HNHBS:100:1+1'"
        );
        let after = "HNHBK:1:3+1+300+D+1'HIRMG:2:2+0010::a?'b'HIRMS:3:2:3+0020::ok'HIUPD:4:6+a?'b?+c?:d??+@6@x'y+z!'HNHBS:5:1+1'".to_string();
        assert_eq!(clear(&SegmentNumbering, &before), after);
    }

    #[test]
    fn numbering_does_not_touch_segment_reference() {
        let before = "HNHBK:1:3+1+300+D+1'HIRMS:9:2:3+0020::ok'";
        assert_eq!(
            clear(&SegmentNumbering, before),
            "HNHBK:1:3+1+300+D+1'HIRMS:2:2:3+0020::ok'"
        );
    }

    #[test]
    fn numbering_untouched_when_not_understood() {
        for msg in [
            // correct
            "HNHBK:1:3+1+300+D+1'HIRMG:2:2+0010::ok'HNHBS:3:1+1'",
            // non-numeric number
            "HNHBK:1:3+1+300+D+1'HIRMG:x:2+0010::ok'",
            // head without a number
            "HNHBK:1:3+1+300+D+1'HIRMG+0010::ok'",
            // empty number
            "HNHBK:1:3+1+300+D+1'HIRMG::2+0010::ok'",
            // envelope present (998/999 are legitimate numbers there)
            "HNHBK:1:3+1+300+D+1'HNVSK:998:3+PIN:1'HNVSD:999:1+@6@abc'de'HNHBS:6:1+1'",
            // unterminated
            "HNHBK:1:3+1+300+D+1'HIRMG:7:2+0010::ok",
            // does not start with HNHBK
            "HIRMG:7:2+0010::ok'",
            "",
        ] {
            assert_eq!(clear(&SegmentNumbering, msg), msg, "{msg:?}");
        }
    }

    // --- chain ---

    #[test]
    fn default_chain_order() {
        assert_eq!(
            QuirkChain::default().names(),
            [
                "MissingMessageReference",
                "HeaderVersion",
                "StatusSegmentOrder",
                "SegmentNumbering"
            ]
        );
    }

    #[test]
    fn chain_repairs_a_message_broken_in_every_way() {
        let inner = "HIRMS:9:2:3+0020::ok'HIRMG:9:2+0010::ok'";
        let mut msg = b"HNHBK:1:3+000000000100+0+DLG9+2'HNVSK:998:3+PIN:1'HNVSD:999:1+@".to_vec();
        msg.extend_from_slice(inner.len().to_string().as_bytes());
        msg.push(b'@');
        msg.extend_from_slice(inner.as_bytes());
        msg.extend_from_slice(b"'HNHBS:3:1+2'");
        assert_eq!(
            QuirkChain::default().incoming(&msg, &CTX),
            b"HNHBK:1:3+000000000100+300+DLG9+2+DLG9:2'HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::ok'HNHBS:4:1+2'"
                .to_vec()
        );
    }

    // --- no-op on well-formed messages ---

    fn all_quirks() -> Vec<Box<dyn Quirk>> {
        vec![
            Box::new(MissingMessageReference),
            Box::new(HeaderVersion),
            Box::new(StatusSegmentOrder),
            Box::new(SegmentNumbering),
        ]
    }

    /// Extract simple `"HNHBK:..."` string literals from Rust source.
    /// `{body}` placeholders (format strings) get a small valid body.
    fn literals(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = src;
        while let Some(at) = rest.find("\"HNHBK:") {
            let tail = &rest[at + 1..];
            let mut lit = String::new();
            let mut chars = tail.chars().peekable();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some('\n') => {
                            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                                chars.next();
                            }
                        }
                        Some('"') => lit.push('"'),
                        Some('\\') => lit.push('\\'),
                        Some('\'') => lit.push('\''),
                        _ => break,
                    },
                    c => lit.push(c),
                }
            }
            rest = &rest[at + 1..];
            if closed {
                out.push(lit.replace("{body}", "HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::ok'"));
            }
        }
        out
    }

    #[test]
    #[cfg(repo_checkout)]
    fn no_quirk_changes_existing_well_formed_fixtures() {
        let sources = [
            include_str!("../../tests/decoupled_test.rs"),
            include_str!("../../tests/open_dialog_test.rs"),
        ];
        let mut checked = 0;
        for lit in sources.iter().flat_map(|s| literals(s)) {
            // Deliberately malformed fixtures of other tests: a client-style
            // header-only message, HNHBS before HIRMG, and a message that
            // does not start with HNHBK.
            if lit.contains("HNHBS:3:1+1'HIRMG:4:2") || lit.ends_with("(header with total size)") {
                continue;
            }
            // The mock responses built by the integration tests omit the
            // message reference and number HNHBS 9; those two repairs rightly
            // fire on them, so only the other two quirks must be no-ops.
            let nonconforming = lit.contains("+DLG-1+1'");
            let msg = lit.as_bytes();
            let unwrapped = unwrap_envelope(msg);
            for q in all_quirks() {
                if nonconforming
                    && matches!(q.name(), "MissingMessageReference" | "SegmentNumbering")
                {
                    continue;
                }
                assert!(
                    matches!(q.incoming_raw(msg, &CTX), Cow::Borrowed(_)),
                    "{} changed raw fixture {lit:?}",
                    q.name()
                );
                assert!(
                    matches!(q.incoming(&unwrapped, &CTX), Cow::Borrowed(_)),
                    "{} changed clear fixture {lit:?}",
                    q.name()
                );
            }
            checked += 1;
        }
        assert!(checked >= 2, "only {checked} fixtures reached");
    }

    #[test]
    fn well_formed_enveloped_message_is_unchanged_by_every_quirk() {
        let inner = "HIRMG:2:2+0010::ok'HIRMS:3:2:3+0020::fine'HISAL:4:7:3+X'";
        let mut msg =
            b"HNHBK:1:3+000000000100+300+DLG9+2+DLG9:2'HNVSK:998:3+PIN:1'HNVSD:999:1+@".to_vec();
        msg.extend_from_slice(format!("{}@{inner}'HNHBS:5:1+2'", inner.len()).as_bytes());
        let clear_text = unwrap_envelope(&msg);
        for q in all_quirks() {
            assert!(matches!(q.incoming_raw(&msg, &CTX), Cow::Borrowed(_)));
            assert!(matches!(q.incoming(&clear_text, &CTX), Cow::Borrowed(_)));
        }
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(msg in proptest::collection::vec(any::<u8>(), 0..300)) {
            for q in all_quirks() {
                let _ = q.incoming_raw(&msg, &CTX);
                let _ = q.incoming(&msg, &CTX);
            }
            let _ = QuirkChain::default().incoming(&msg, &CTX);
        }

        #[test]
        fn wire_like_text_never_panics(msg in "[HNIRMSBK0-9:+'?@a-z ]{0,200}") {
            for q in all_quirks() {
                let _ = q.incoming_raw(msg.as_bytes(), &CTX);
                let _ = q.incoming(msg.as_bytes(), &CTX);
            }
            let _ = QuirkChain::default().incoming(msg.as_bytes(), &CTX);
        }

        #[test]
        fn generated_well_formed_messages_are_unchanged(
            data in proptest::collection::vec("[a-z0-9?+:']{0,12}", 0..6),
            blobs in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..10), 0..3),
            statuses in 0usize..4,
        ) {
            let mut msg = b"HNHBK:1:3+000000000100+300+DLG9+2+DLG9:2'HIRMG:2:2+0010::ok'".to_vec();
            let mut n = 3;
            for _ in 0..statuses {
                msg.extend_from_slice(format!("HIRMS:{n}:2:3+0020::ok'").as_bytes());
                n += 1;
            }
            for d in &data {
                msg.extend_from_slice(format!("HIX:{n}:1+").as_bytes());
                for &b in d.as_bytes() {
                    if matches!(b, b'+' | b':' | b'\'' | b'?' | b'@') {
                        msg.push(b'?');
                    }
                    msg.push(b);
                }
                msg.extend_from_slice(b"'");
                n += 1;
            }
            for blob in &blobs {
                msg.extend_from_slice(format!("HIY:{n}:1+@{}@", blob.len()).as_bytes());
                msg.extend_from_slice(blob);
                msg.extend_from_slice(b"'");
                n += 1;
            }
            msg.extend_from_slice(format!("HNHBS:{n}:1+2'").as_bytes());
            for q in all_quirks() {
                prop_assert!(matches!(q.incoming_raw(&msg, &CTX), Cow::Borrowed(_)));
                prop_assert!(matches!(q.incoming(&msg, &CTX), Cow::Borrowed(_)));
            }
        }
    }
}
