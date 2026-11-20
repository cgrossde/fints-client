//! HITAN, Zwei-Schritt-TAN-Einreichung Rückmeldung (bank response to
//! HKTAN). PIN/TAN 2020, B.5.1 (version 6), B.5.2 (version 7) and the
//! archive of older segment versions, E.1.1-E.1.5 b) (versions 1-5).
//!
//! The challenge grows with the versions: element version 1 `an ..256`
//! (HITAN 1), 2 `an ..999` (HITAN 2, 3), 3 `an ..2048` (HITAN 4-7).

use crate::codec::{An, Bin, Code, Dat, DataElementGroup, Segment, Tim};

/// Gültigkeitsdatum und -uhrzeit für Challenge, version 1. PIN/TAN 2020, D.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct ChallengeValidity1 {
    /// 1 Datum, `dat`, M.
    pub date: Dat,
    /// 2 Uhrzeit, `tim`, M.
    pub time: Tim,
}

/// HITAN version 1. PIN/TAN 2020, E.1.1 b).
///
/// ```text
/// Nr. Name                               Typ  Format Länge Status Anzahl
///  1  Segmentkopf                        DEG               M     1
///  2  TAN-Prozess                        DE   code   1     M     1   1, 2, 3, 4
///  3  Auftrags-Hashwert                  DE   bin    ..256 C     1   M: hash method <> 0 and process 1; N: else
///  4  Auftragsreferenz                   DE   an     ..35  C     1   M: process 2, 3, 4; O: process 1
///  5  Challenge                          DE   an     ..256 C     1   M: process 1, 3, 4; O: process 2
///  6  Gültigkeitsdatum und -uhrzeit für Challenge DEG    O     1
///  7  TAN-Listennummer                   DE   an     ..20  C     1   M: process 1, 3, 4 and "Anzahl unterstützter aktiver TAN-Listen" absent; O: else
///  8  TAN-Zusatzinformationen            DE   an     ..99  C     1   O: process 1; N: process 2, 3, 4
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITAN", version = 1)]
pub struct HiTan1 {
    /// 2 TAN-Prozess: 1, 2, 3 or 4.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 4 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 5 Challenge (element version 1).
    #[de(max = 256)]
    pub challenge: Option<An>,
    /// 6 Gültigkeitsdatum und -uhrzeit für Challenge.
    pub challenge_valid_until: Option<ChallengeValidity1>,
    /// 7 TAN-Listennummer.
    #[de(max = 20)]
    pub tan_list_number: Option<An>,
    /// 8 TAN-Zusatzinformationen.
    #[de(max = 99)]
    pub tan_additional_information: Option<An>,
}

/// HITAN version 2. PIN/TAN 2020, E.1.2 b): [`HiTan1`] with the challenge
/// in element version 2 (`an ..999`) and element 8 BEN instead of
/// TAN-Zusatzinformationen.
///
/// ```text
///  5  Challenge                          DE   an     ..999 C     1   M: process 1, 3, 4; O: process 2
///  8  BEN                                DE   an     ..99  C     1   O: process 2; N: else
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITAN", version = 2)]
pub struct HiTan2 {
    /// 2 TAN-Prozess: 1, 2, 3 or 4.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 4 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 5 Challenge (element version 2).
    #[de(max = 999)]
    pub challenge: Option<An>,
    /// 6 Gültigkeitsdatum und -uhrzeit für Challenge.
    pub challenge_valid_until: Option<ChallengeValidity1>,
    /// 7 TAN-Listennummer.
    #[de(max = 20)]
    pub tan_list_number: Option<An>,
    /// 8 BEN (Bestätigungsnummer, process 2).
    #[de(max = 99)]
    pub ben: Option<An>,
}

/// HITAN version 3. PIN/TAN 2020, E.1.3 b): [`HiTan2`] plus element 9.
///
/// ```text
///  9  Bezeichnung des TAN-Mediums        DE   an     ..32  C     1   M: process 1, 3, 4 and "Anzahl unterstützter aktiver TAN-Medien" absent; O: else
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITAN", version = 3)]
pub struct HiTan3 {
    /// 2 TAN-Prozess: 1, 2, 3 or 4.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 4 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 5 Challenge (element version 2).
    #[de(max = 999)]
    pub challenge: Option<An>,
    /// 6 Gültigkeitsdatum und -uhrzeit für Challenge.
    pub challenge_valid_until: Option<ChallengeValidity1>,
    /// 7 TAN-Listennummer.
    #[de(max = 20)]
    pub tan_list_number: Option<An>,
    /// 8 BEN.
    #[de(max = 99)]
    pub ben: Option<An>,
    /// 9 Bezeichnung des TAN-Mediums.
    #[de(max = 32)]
    pub tan_medium_name: Option<An>,
}

/// HITAN version 4. PIN/TAN 2020, E.1.4 b).
///
/// ```text
/// Nr. Name                               Typ  Format Länge Status Anzahl
///  1  Segmentkopf                        DEG               M     1
///  2  TAN-Prozess                        DE   code   1     M     1   1, 2, 3, 4
///  3  Auftrags-Hashwert                  DE   bin    ..256 C     1   M: hash method <> 0 and process 1; N: else
///  4  Auftragsreferenz                   DE   an     ..35  C     1   M: process 2, 3, 4; O: process 1
///  5  Challenge                          DE   an     ..2048 C    1   M: process 1, 3, 4; O: process 2
///  6  Challenge HHD_UC                   DE   bin    ..    O     1
///  7  Gültigkeitsdatum und -uhrzeit für Challenge DEG    O     1
///  8  TAN-Listennummer                   DE   an     ..20  C     1   M: process 1, 3, 4 and "Anzahl unterstützter aktiver TAN-Listen" absent; O: else
///  9  BEN                                DE   an     ..99  C     1   O: process 2; N: else
/// 10  Bezeichnung des TAN-Mediums        DE   an     ..32  C     1   M: process 1, 3, 4 and "Anzahl unterstützter aktiver TAN-Medien" absent; O: else
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITAN", version = 4)]
pub struct HiTan4 {
    /// 2 TAN-Prozess: 1, 2, 3 or 4.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 4 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 5 Challenge (element version 3).
    #[de(max = 2048)]
    pub challenge: Option<An>,
    /// 6 Challenge HHD_UC.
    pub challenge_hhd_uc: Option<Bin>,
    /// 7 Gültigkeitsdatum und -uhrzeit für Challenge.
    pub challenge_valid_until: Option<ChallengeValidity1>,
    /// 8 TAN-Listennummer.
    #[de(max = 20)]
    pub tan_list_number: Option<An>,
    /// 9 BEN.
    #[de(max = 99)]
    pub ben: Option<An>,
    /// 10 Bezeichnung des TAN-Mediums.
    #[de(max = 32)]
    pub tan_medium_name: Option<An>,
}

/// HITAN version 5. PIN/TAN 2020, E.1.5 b): the rows of [`HiTan4`]
/// unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITAN", version = 5)]
pub struct HiTan5 {
    /// 2 TAN-Prozess: 1, 2, 3 or 4.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 4 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 5 Challenge (element version 3).
    #[de(max = 2048)]
    pub challenge: Option<An>,
    /// 6 Challenge HHD_UC.
    pub challenge_hhd_uc: Option<Bin>,
    /// 7 Gültigkeitsdatum und -uhrzeit für Challenge.
    pub challenge_valid_until: Option<ChallengeValidity1>,
    /// 8 TAN-Listennummer.
    #[de(max = 20)]
    pub tan_list_number: Option<An>,
    /// 9 BEN.
    #[de(max = 99)]
    pub ben: Option<An>,
    /// 10 Bezeichnung des TAN-Mediums.
    #[de(max = 32)]
    pub tan_medium_name: Option<An>,
}

/// HITAN version 6.
///
/// ```text
/// Nr. Name                               Typ  Format Länge Status Anzahl
///  1  Segmentkopf                        DEG               M     1
///  2  TAN-Prozess                        DE   code   1     M     1   1, 2, 3, 4
///  3  Auftrags-Hashwert                  DE   bin    ..256 C     1   M: hash method <> 0 and process 1; N: else
///  4  Auftragsreferenz                   DE   an     ..35  C     1   M: process 2, 3, 4; O: process 1 ("noref" for a dummy HITAN)
///  5  Challenge                          DE   an     ..2048 C    1   M: process 1, 3, 4; O: process 2 ("nochallenge" for a dummy HITAN)
///  6  Challenge HHD_UC                   DE   bin    ..    O     1
///  7  Gültigkeitsdatum und -uhrzeit für Challenge DEG    O     1
///  8  Bezeichnung des TAN-Mediums        DE   an     ..32  C     1   M: process 1, 3, 4 and "Anzahl unterstützter aktiver TAN-Medien" absent; O: else
/// ```
///
/// The conditional rules are not checked by the codec. Challenge is element
/// version 3 (`an ..2048`).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITAN", version = 6)]
pub struct HiTan6 {
    /// 2 TAN-Prozess: 1, 2, 3 or 4.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Auftrags-Hashwert: the value the customer sent in HKTAN, unchanged.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 4 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 5 Challenge: shown to the customer unchanged; may carry format codes
    /// when the BPD says "Challenge strukturiert".
    #[de(max = 2048)]
    pub challenge: Option<An>,
    /// 6 Challenge HHD_UC: structure of the HHD_UC specification, binary.
    pub challenge_hhd_uc: Option<Bin>,
    /// 7 Gültigkeitsdatum und -uhrzeit für Challenge.
    pub challenge_valid_until: Option<ChallengeValidity1>,
    /// 8 Bezeichnung des TAN-Mediums.
    #[de(max = 32)]
    pub tan_medium_name: Option<An>,
}

/// HITAN version 7: version 6 with the decoupled process `S`. Differences
/// from [`HiTan6`]: element 2 (TAN-Prozess) is version 2 and allows `S`;
/// 4 Auftragsreferenz is M also for process S; 5 Challenge is O also for
/// process S (in the decoupled process it can carry instructions for the
/// customer).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HITAN", version = 7)]
pub struct HiTan7 {
    /// 2 TAN-Prozess: 1, 2, 3, 4 or S.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 4 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 5 Challenge.
    #[de(max = 2048)]
    pub challenge: Option<An>,
    /// 6 Challenge HHD_UC.
    pub challenge_hhd_uc: Option<Bin>,
    /// 7 Gültigkeitsdatum und -uhrzeit für Challenge.
    pub challenge_valid_until: Option<ChallengeValidity1>,
    /// 8 Bezeichnung des TAN-Mediums.
    #[de(max = 32)]
    pub tan_medium_name: Option<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HITAN:5:6:4+4++REF-0001+Bitte Freigabe erteilen.'",
    "HITAN:5:7:4+4++REF-0001+Bitte in der App bestaetigen.+@3@HHD+20260115:101500+Mein Handy'",
    "HITAN:5:7:4+S++REF-0001+Freigabe steht noch aus.'",
    "HITAN:5:1:4+4++REF-0001+Bitte TAN eingeben.+20260115:101500+LISTE-1+Hinweis'",
    "HITAN:5:2:4+2++REF-0001++++BEN-1'",
    "HITAN:5:3:4+4++REF-0001+Bitte TAN eingeben.++++Mein Handy'",
    "HITAN:5:4:4+4++REF-0001+Bitte TAN eingeben.+@3@HHD+20260115:101500'",
    "HITAN:5:5:4+4++REF-0001+Bitte TAN eingeben.+@3@HHD++LISTE-1++Mein Handy'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiTan6 = round_trip(SAMPLES[0]);
        assert_eq!(v.order_reference.as_ref().unwrap().as_str(), "REF-0001");
        assert!(v.order_hash.is_none());
        let v: HiTan7 = round_trip(SAMPLES[1]);
        assert_eq!(v.challenge_hhd_uc.unwrap().as_bytes(), b"HHD");
        assert_eq!(
            v.challenge_valid_until.unwrap().date.date().to_string(),
            "2026-01-15"
        );
        assert_eq!(v.tan_medium_name.unwrap().as_str(), "Mein Handy");
        let v: HiTan7 = round_trip(SAMPLES[2]);
        assert_eq!(v.process.as_str(), "S");
    }

    /// Strings the hand-written parser's tests read. The version 6 string
    /// with process `S` does not exist in the spec (see `hktan.rs`) but
    /// reads, as value lists are not checked.
    #[test]
    fn reads_the_hand_written_parsers_fixtures() {
        let v: HiTan7 = round_trip("HITAN:5:7:4+4++4711-REF+Bitte bestaetigen Sie in der App'");
        assert_eq!(v.order_reference.unwrap().as_str(), "4711-REF");
        assert_eq!(
            v.challenge.unwrap().as_str(),
            "Bitte bestaetigen Sie in der App"
        );
        let v: HiTan6 = round_trip("HITAN:5:6:4+S+@4@HASH+REF-9+nochallenge'");
        assert_eq!(v.order_hash.unwrap().as_bytes(), b"HASH");
    }

    #[test]
    fn rules_are_enforced() {
        // The challenge is at most 2048 bytes in this version.
        let long = "x".repeat(2049);
        let e = from_wire::<HiTan7>(format!("HITAN:5:7:4+4++R+{long}'").as_bytes()).unwrap_err();
        assert_eq!(e.field, "challenge");
        // The validity needs both date and time.
        let e = from_wire::<HiTan7>(b"HITAN:5:7:4+4++R+C++20260115'").unwrap_err();
        assert_eq!(e.field, "challenge_valid_until.time");
    }

    #[test]
    fn older_versions_round_trip() {
        let v: HiTan1 = round_trip(SAMPLES[3]);
        assert_eq!(v.challenge.unwrap().as_str(), "Bitte TAN eingeben.");
        assert!(v.challenge_valid_until.is_some());
        assert_eq!(v.tan_list_number.unwrap().as_str(), "LISTE-1");
        assert_eq!(v.tan_additional_information.unwrap().as_str(), "Hinweis");
        let v: HiTan2 = round_trip(SAMPLES[4]);
        assert_eq!(v.ben.unwrap().as_str(), "BEN-1");
        let v: HiTan3 = round_trip(SAMPLES[5]);
        assert_eq!(v.tan_medium_name.unwrap().as_str(), "Mein Handy");
        let v: HiTan4 = round_trip(SAMPLES[6]);
        assert_eq!(v.challenge_hhd_uc.unwrap().as_bytes(), b"HHD");
        assert!(v.challenge_valid_until.is_some());
        let v: HiTan5 = round_trip(SAMPLES[7]);
        assert_eq!(v.tan_list_number.unwrap().as_str(), "LISTE-1");
        assert_eq!(v.tan_medium_name.unwrap().as_str(), "Mein Handy");
    }

    #[test]
    fn older_versions_challenge_lengths() {
        let at = |n: usize, version: u16| {
            format!("HITAN:5:{version}:4+4++R+{}'", "x".repeat(n)).into_bytes()
        };
        assert!(from_wire::<HiTan1>(&at(256, 1)).is_ok());
        assert_eq!(
            from_wire::<HiTan1>(&at(257, 1)).unwrap_err().field,
            "challenge"
        );
        assert!(from_wire::<HiTan2>(&at(999, 2)).is_ok());
        assert_eq!(
            from_wire::<HiTan3>(&at(1000, 3)).unwrap_err().field,
            "challenge"
        );
        assert!(from_wire::<HiTan5>(&at(2048, 5)).is_ok());
        assert_eq!(
            from_wire::<HiTan4>(&at(2049, 4)).unwrap_err().field,
            "challenge"
        );
    }
}
