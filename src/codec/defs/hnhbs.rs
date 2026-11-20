//! HNHBS, Nachrichtenabschluss (message trailer). Formals 2017, B.5.3.

use crate::codec::{Num, Segment};

/// HNHBS version 1: the segment that ends every message.
///
/// ```text
/// Nr. Name               Typ  Format Länge Status Anzahl
///  1  Segmentkopf        DEG              M     1
///  2  Nachrichtennummer  DE   num   ..4   M     1    >0
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HNHBS", version = 1)]
pub struct HnHbs1 {
    /// 2 Nachrichtennummer: the number in the message header (HNHBK DE 5).
    #[de(max = 4)]
    pub message_number: Num,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HNHBS:6:1+2'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    #[test]
    fn wire_round_trips() {
        let v: HnHbs1 = round_trip("HNHBS:6:1+2'");
        assert_eq!(v.message_number, Num::new(2));
    }

    /// The expected wire string for these values.
    #[test]
    fn builds_the_expected_wire() {
        let built = b"HNHBS:5:1+1'".to_vec();
        let derived = HnHbs1 {
            message_number: Num::new(1),
        };
        assert_eq!(crate::codec::to_wire(&derived, 5, None).unwrap(), built);
    }

    #[test]
    fn number_is_required_and_bounded() {
        let e = from_wire::<HnHbs1>(b"HNHBS:6:1'").unwrap_err();
        assert_eq!(e.field, "message_number");
        let e = from_wire::<HnHbs1>(b"HNHBS:6:1+12345'").unwrap_err();
        assert!(matches!(e.kind, ErrorKind::TooLong { max: 4, .. }));
    }
}
