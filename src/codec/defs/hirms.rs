//! HIRMS, Rückmeldungen zu Segmenten (return codes for segments). Formals
//! 2017, B.7.3.

use super::feedback::Feedback1;
use crate::codec::Segment;

/// HIRMS version 2: the feedback for one customer segment; the header's
/// reference is that segment's number.
///
/// ```text
/// Nr. Name           Typ  Format Länge Status Anzahl
///  1  Segmentkopf    DEG             M     1
///  2  Rückmeldung    DEG             M     1..99
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIRMS", version = 2)]
pub struct HiRms2 {
    /// 2 Rückmeldung, 1 to 99: the codes for the segment and its data
    /// elements.
    #[de(min_count = 1, max_count = 99)]
    pub feedback: Vec<Feedback1>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIRMS:3:2:3+0020::Auftrag ausgefuehrt.'",
    "HIRMS:4:2:4+9010::Verarbeitung nicht moeglich.+9210:HKSAL:Konto unbekannt.:A1:B2'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiRms2 = round_trip(SAMPLES[0]);
        assert_eq!(v.feedback[0].code.as_str(), "0020");
        let v: HiRms2 = round_trip(SAMPLES[1]);
        assert_eq!(
            v.feedback[1].reference_element.as_ref().unwrap().as_str(),
            "HKSAL"
        );
        assert_eq!(v.feedback[1].parameters.len(), 2);
    }

    /// Strings the protocol tests feed to the hand-written parser.
    #[test]
    fn reads_the_protocol_tests_fixtures() {
        for wire in [
            "HIRMS:3:2:3+3072::Zugangsdaten geaendert:new-user:new-customer'",
            "HIRMS:4:2:4+9010::Verarbeitung nicht moeglich+9210::Keine gueltige Kontoverbindung'",
            "HIRMS:4:2:1+9942::PIN falsch'",
        ] {
            let v: HiRms2 = round_trip(wire);
            assert!(!v.feedback.is_empty());
        }
    }

    #[test]
    fn the_reference_is_the_customer_segment() {
        let (h, _) = from_wire::<HiRms2>(SAMPLES[0].as_bytes()).unwrap();
        assert_eq!(h.reference, Some(3));
    }

    #[test]
    fn at_most_ten_parameters() {
        let params = (0..11)
            .map(|i| format!("P{i}"))
            .collect::<Vec<_>>()
            .join(":");
        let wire = format!("HIRMS:3:2:3+3920::Zugelassene Verfahren:{params}'");
        let e = from_wire::<HiRms2>(wire.as_bytes()).unwrap_err();
        assert!(e.field.starts_with("feedback[0]"), "{}", e.field);
    }
}
