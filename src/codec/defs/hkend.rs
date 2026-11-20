//! HKEND, Dialogende (dialog end). Formals 2017, C.4.2.2.

use crate::codec::{Id, Segment};

/// HKEND version 1.
///
/// ```text
/// Nr. Name      Typ  Format Länge Status Anzahl
///  1  Segmentkopf DEG             M     1
///  2  Dialog-ID  DE   id     #     M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKEND", version = 1)]
pub struct HkEnd1 {
    /// 2 Dialog-ID: the dialog to end.
    pub dialog_id: Id,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HKEND:5:1+DLG1234567'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{from_wire, to_wire};

    #[test]
    fn wire_round_trips() {
        let v: HkEnd1 = round_trip(SAMPLES[0]);
        assert_eq!(v.dialog_id.as_str(), "DLG1234567");
    }

    #[test]
    fn builds_the_expected_wire() {
        let built = to_wire(
            &crate::segments::request::hkend("DLG1234567").unwrap(),
            5,
            None,
        )
        .unwrap();
        // The expected wire string for these values.
        assert_eq!(built, b"HKEND:5:1+DLG1234567'");
        let (_, derived) = from_wire::<HkEnd1>(SAMPLES[0].as_bytes()).unwrap();
        assert_eq!(to_wire(&derived, 5, None).unwrap(), built);
    }

    #[test]
    fn dialog_id_is_required() {
        assert_eq!(
            from_wire::<HkEnd1>(b"HKEND:5:1'").unwrap_err().field,
            "dialog_id"
        );
    }
}
