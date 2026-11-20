//! HIRMG, Rückmeldungen zur Gesamtnachricht (return codes for the whole
//! message). Formals 2017, B.7.2.

use super::feedback::Feedback1;
use crate::codec::Segment;

/// HIRMG version 2.
///
/// ```text
/// Nr. Name           Typ  Format Länge Status Anzahl
///  1  Segmentkopf    DEG             M     1
///  2  Rückmeldung    DEG             M     1..99
/// ```
///
/// A success code (class 0) may only be sent when no segment of the message
/// failed (not checked here). The reference data element of each feedback
/// must stay empty.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIRMG", version = 2)]
pub struct HiRmg2 {
    /// 2 Rückmeldung, 1 to 99.
    #[de(min_count = 1, max_count = 99)]
    pub feedback: Vec<Feedback1>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIRMG:2:2+0010::Nachricht entgegengenommen.'",
    "HIRMG:2:2+9050::Die Nachricht enthaelt Fehler.+9800::Dialog abgebrochen.:DLG1'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiRmg2 = round_trip(SAMPLES[0]);
        assert_eq!(v.feedback.len(), 1);
        assert_eq!(v.feedback[0].code.as_str(), "0010");
        assert!(v.feedback[0].reference_element.is_none());
        let v: HiRmg2 = round_trip(SAMPLES[1]);
        assert_eq!(v.feedback.len(), 2);
        assert_eq!(v.feedback[1].parameters[0].as_str(), "DLG1");
    }

    /// Strings the protocol tests feed to the hand-written parser.
    #[test]
    fn reads_the_protocol_tests_fixtures() {
        for wire in [
            "HIRMG:2:2+0010::ok'",
            "HIRMG:2:2+9050::Die Nachricht enthaelt Fehler'",
            "HIRMG:2:2+9999::Interner Fehler beim Institut'",
        ] {
            round_trip::<HiRmg2>(wire);
        }
    }

    #[test]
    fn rules_are_enforced() {
        // The code is four digits.
        let e = from_wire::<HiRmg2>(b"HIRMG:2:2+XYZ::Bad code.'").unwrap_err();
        assert_eq!(e.field, "feedback[0].code");
        // At least one feedback.
        assert!(from_wire::<HiRmg2>(b"HIRMG:2:2'").is_err());
        // The text is at most 80 bytes.
        let long = "x".repeat(81);
        let e = from_wire::<HiRmg2>(format!("HIRMG:2:2+0010::{long}'").as_bytes()).unwrap_err();
        assert_eq!(e.field, "feedback[0].text");
    }
}
