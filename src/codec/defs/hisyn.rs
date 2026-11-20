//! HISYN, Synchronisierungsantwort (synchronisation response). Formals 2017,
//! C.8.2.2.

use crate::codec::{Id, Num, Segment};

/// HISYN version 4, bank response to HKSYN. Which element is present depends
/// on the mode the customer asked for.
///
/// ```text
/// Nr. Name                                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                                DEG              M     1
///  2  Kundensystem-ID                            DE   id     #     C     1    M: mode 0, N: else
///  3  Nachrichtennummer                          DE   num    ..4   C     1    >0; M: mode 1, N: else
///  4  Sicherheitsreferenznummer für Signierschl. DE   num    ..16  C     1    M: mode 2, N: else
///  5  Sicherheitsreferenznummer für Digitale Sig. DE  num    ..16  C     1    M: mode 2 and RAH-7, RDH-3, -6, -7, N: else
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISYN", version = 4)]
pub struct HiSyn4 {
    /// 2 Kundensystem-ID (mode 0).
    pub system_id: Option<Id>,
    /// 3 Nachrichtennummer (mode 1): the last message number the bank
    /// processed.
    #[de(max = 4)]
    pub message_number: Option<Num>,
    /// 4 Sicherheitsreferenznummer für Signierschlüssel (mode 2).
    #[de(max = 16)]
    pub signature_id_key: Option<Num>,
    /// 5 Sicherheitsreferenznummer für Digitale Signatur (mode 2 with RAH-7,
    /// RDH-3, RDH-6 or RDH-7): the signature ID of the D key.
    #[de(max = 16)]
    pub signature_id_ds: Option<Num>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &["HISYN:6:4:5+SYS12345678'", "HISYN:6:4:5+++7'"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiSyn4 = round_trip(SAMPLES[0]);
        assert_eq!(v.system_id.unwrap().as_str(), "SYS12345678");
        assert!(v.message_number.is_none());
        let v: HiSyn4 = round_trip(SAMPLES[1]);
        assert_eq!(v.signature_id_key, Some(Num::new(7)));
        round_trip::<HiSyn4>("HISYN:6:4:5++12'");
    }

    /// Same string the hand-written parser's test reads.
    #[test]
    fn reads_the_hand_written_parsers_fixture() {
        let (h, v) = from_wire::<HiSyn4>(b"HISYN:173:4:6+MYSYSID123'").unwrap();
        assert_eq!((h.number, h.reference), (173, Some(6)));
        assert_eq!(v.system_id.unwrap().as_str(), "MYSYSID123");
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiSyn4>(b"HISYN:6:4:5++12345'").unwrap_err();
        assert_eq!(e.field, "message_number");
    }
}
