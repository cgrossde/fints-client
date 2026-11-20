//! Byte-level helpers that break a bank answer on purpose, one per tolerant
//! quirk of the client (`fints_client::quirks`). Use them in
//! [`TestBank::tamper_raw`](crate::TestBank::tamper_raw) (whole messages)
//! and [`TestBank::tamper_clear`](crate::TestBank::tamper_clear) (inner
//! segments, each with its terminator).
//!
//! They work on the header group of a segment only, so binary data in the
//! segment body is never touched. The HNHBK size is not recomputed.

/// Position of the end of the HNHBK (its `'`) at the start of `msg`.
fn hnhbk_end(msg: &[u8]) -> Option<usize> {
    msg.starts_with(b"HNHBK")
        .then(|| msg.iter().position(|&b| b == b'\''))
        .flatten()
}

/// Rewrites the data element groups of the leading HNHBK.
fn edit_hnhbk(msg: Vec<u8>, f: impl FnOnce(&mut Vec<Vec<u8>>)) -> Vec<u8> {
    let Some(end) = hnhbk_end(&msg) else {
        return msg;
    };
    let mut degs: Vec<Vec<u8>> = msg[..end]
        .split(|&b| b == b'+')
        .map(<[u8]>::to_vec)
        .collect();
    f(&mut degs);
    let mut out = degs.join(&b'+');
    out.extend_from_slice(&msg[end..]);
    out
}

/// Drops the reference to the customer message (the sixth group of HNHBK,
/// `dialog:number`).
pub fn drop_message_reference(msg: Vec<u8>) -> Vec<u8> {
    edit_hnhbk(msg, |degs| {
        if degs.len() == 6 {
            degs.pop();
        }
    })
}

/// Sets the HBCI version of HNHBK (e.g. `""` or `"0"` instead of `300`).
pub fn header_version(msg: Vec<u8>, version: &str) -> Vec<u8> {
    edit_hnhbk(msg, |degs| {
        if let Some(v) = degs.get_mut(2) {
            *v = version.as_bytes().to_vec();
        }
    })
}

fn code_of(segment: &[u8]) -> &[u8] {
    let end = segment
        .iter()
        .position(|&b| b == b':')
        .unwrap_or(segment.len());
    &segment[..end]
}

/// Moves HIRMG and HIRMS behind the data segments (numbers unchanged).
pub fn status_after_data(segments: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let (status, data): (Vec<_>, Vec<_>) = segments
        .into_iter()
        .partition(|s| matches!(code_of(s), b"HIRMG" | b"HIRMS"));
    data.into_iter().chain(status).collect()
}

/// Numbers the segments consecutively from `start`.
pub fn renumber(segments: Vec<Vec<u8>>, start: u32) -> Vec<Vec<u8>> {
    segments
        .into_iter()
        .zip(start..)
        .map(|(s, n)| {
            let head_end = s
                .iter()
                .position(|&b| b == b'+' || b == b'\'')
                .unwrap_or(s.len());
            let mut head: Vec<Vec<u8>> = s[..head_end]
                .split(|&b| b == b':')
                .map(<[u8]>::to_vec)
                .collect();
            if let Some(number) = head.get_mut(1) {
                *number = n.to_string().into_bytes();
            }
            let mut out = head.join(&b':');
            out.extend_from_slice(&s[head_end..]);
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MSG: &[u8] = b"HNHBK:1:3+000000000042+300+DLG1+2+DLG1:2'HNHBS:2:1+2'";

    #[test]
    fn header_edits() {
        assert_eq!(
            drop_message_reference(MSG.to_vec()),
            b"HNHBK:1:3+000000000042+300+DLG1+2'HNHBS:2:1+2'"
        );
        assert_eq!(
            header_version(MSG.to_vec(), ""),
            b"HNHBK:1:3+000000000042++DLG1+2+DLG1:2'HNHBS:2:1+2'"
        );
    }

    #[test]
    fn segment_edits() {
        let segs = vec![
            b"HIRMG:2:2+0010::ok'".to_vec(),
            b"HIRMS:3:2:3+0020::ok'".to_vec(),
            b"HISAL:4:7:3+x'".to_vec(),
        ];
        let moved = renumber(status_after_data(segs), 7);
        assert_eq!(
            moved,
            vec![
                b"HISAL:7:7:3+x'".to_vec(),
                b"HIRMG:8:2+0010::ok'".to_vec(),
                b"HIRMS:9:2:3+0020::ok'".to_vec(),
            ]
        );
    }
}
