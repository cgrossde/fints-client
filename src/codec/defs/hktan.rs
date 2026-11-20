//! HKTAN, Zwei-Schritt-TAN-Einreichung (two-step TAN submission). PIN/TAN
//! 2020, B.5.1 (version 6) and B.5.2 (version 7, adds decoupled). The
//! processes are described in B.4.

use super::common::Kti1;
use crate::codec::{An, Bin, Code, DataElementGroup, Jn, Num, Segment};

/// Parameter Challenge-Klasse, version 1. PIN/TAN 2020, D. Order-specific
/// data for the challenge class (up to 9 values, assigned by the method's
/// own specification).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct ChallengeClassParameters1 {
    /// 1 Challenge-Klasse Parameter, `an ..999`, O, up to 9.
    #[de(max = 999, max_count = 9)]
    pub parameters: Vec<An>,
}

/// Antwort HHD_UC, version 1. PIN/TAN 2020, D. Answer data of a
/// bidirectional chipTAN method (Secoder command `SECODER TRANSMIT HHDUC`).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct HhdUcResponse1 {
    /// 1 ATC, `an ..5`, M.
    #[de(max = 5)]
    pub atc: An,
    /// 2 Application Cryptogram AC, `bin ..256`, M.
    #[de(max = 256)]
    pub application_cryptogram: Bin,
    /// 3 EF_ID Data, `bin ..256`, M.
    #[de(max = 256)]
    pub ef_id_data: Bin,
    /// 4 CVR, `bin ..256`, M.
    #[de(max = 256)]
    pub cvr: Bin,
    /// 5 Versionsinfo der chipTAN-Applikation, `bin ..256`, M.
    #[de(max = 256)]
    pub chiptan_version_info: Bin,
}

/// HKTAN version 5, customer order. PIN/TAN 2020, E.1.5 a) (archive of
/// older segment versions).
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Segmentkopf                         DEG               M     1
///  2  TAN-Prozess                         DE   code   1     M     1   1, 2, 3, 4
///  3  Segmentkennung                      DE   an     ..6   C     1   M: process 1; N: else
///  4  Kontoverbindung international Auftraggeber DEG kti #  C     1   M: process 1, "Auftraggeberkonto erforderlich"=2 and account in the order
///  5  Auftrags-Hashwert                   DE   bin    ..256 C     1   M: hash method <> 0 and process 1
///  6  Auftragsreferenz                    DE   an     ..35  C     1   M: process 2, 3; O: process 1, 4
///  7  TAN-Listennummer                    DE   an     ..20  C     1   M: process 1, 3, 4, active TAN lists > 1 and "TAN-Listennummer erforderlich"=2; O: else
///  8  Weitere TAN folgt                   DE   jn     1     C     1   M: process 1, 2; N: process 3, 4
///  9  Auftrag stornieren                  DE   jn     1     C     1   O: process 2 and "Auftragsstorno erlaubt"=J
/// 10  SMS-Abbuchungskonto                 DEG  kti    #     C     1   M: process 1, 3, 4 and "SMS-Abbuchungskonto erforderlich"=2; O: else
/// 11  Challenge-Klasse                    DE   num    ..2   C     1   M: process 1 and "Challenge-Klasse erforderlich"=J
/// 12  Parameter Challenge-Klasse          DEG               C     1   O: process 1 and "Challenge-Klasse erforderlich"=J
/// 13  Bezeichnung des TAN-Mediums         DE   an     ..32  C     1   M: process 1, 3, 4, active TAN media > 1 and "Bezeichnung ... erforderlich"=2; O: else
/// ```
///
/// Differences from [`HkTan6`]: element 7 (TAN-Listennummer) exists, and
/// there is no Antwort HHD_UC. The conditional rules are not checked by the
/// codec. The table lists processes 1-4 only; the protocol polls a decoupled
/// confirmation with process `2` in this version (docs/architecture.md, section 2.2).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKTAN", version = 5)]
pub struct HkTan5 {
    /// 2 TAN-Prozess: 1, 2, 3 or 4.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Segmentkennung.
    #[de(max = 6)]
    pub segment_code: Option<An>,
    /// 4 Kontoverbindung international Auftraggeber.
    pub account: Option<Kti1>,
    /// 5 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 6 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 7 TAN-Listennummer.
    #[de(max = 20)]
    pub tan_list_number: Option<An>,
    /// 8 Weitere TAN folgt.
    pub further_tan_follows: Option<Jn>,
    /// 9 Auftrag stornieren.
    pub cancel_order: Option<Jn>,
    /// 10 SMS-Abbuchungskonto.
    pub sms_charge_account: Option<Kti1>,
    /// 11 Challenge-Klasse.
    #[de(max = 2)]
    pub challenge_class: Option<Num>,
    /// 12 Parameter Challenge-Klasse.
    pub challenge_class_parameters: Option<ChallengeClassParameters1>,
    /// 13 Bezeichnung des TAN-Mediums.
    #[de(max = 32)]
    pub tan_medium_name: Option<An>,
}

/// HKTAN version 6, customer order.
///
/// ```text
/// Nr. Name                                Typ  Format Länge Status Anzahl
///  1  Segmentkopf                         DEG               M     1
///  2  TAN-Prozess                         DE   code   1     M     1   1, 2, 3, 4
///  3  Segmentkennung                      DE   an     ..6   C     1   M: process 1; M: process 4 with strong authentication; O: else
///  4  Kontoverbindung international Auftraggeber DEG kti    C     1   M: process 1, "Auftraggeberkonto erforderlich"=2 and account in the order
///  5  Auftrags-Hashwert                   DE   bin    ..256 C     1   M: hash method <> 0 and process 1
///  6  Auftragsreferenz                    DE   an     ..35  C     1   M: process 2, 3; O: process 1, 4
///  7  Weitere TAN folgt                   DE   jn     1     C     1   M: process 1, 2; N: process 3, 4
///  8  Auftrag stornieren                  DE   jn     1     C     1   O: process 2 and "Auftragsstorno erlaubt"=J
///  9  SMS-Abbuchungskonto                 DEG  kti    #     C     1   M: process 1, 3, 4 and "SMS-Abbuchungskonto erforderlich"=2
/// 10  Challenge-Klasse                    DE   num    ..2   C     1   M: process 1 and "Challenge-Klasse erforderlich"=J
/// 11  Parameter Challenge-Klasse          DEG               C     1   O: process 1 and "Challenge-Klasse erforderlich"=J
/// 12  Bezeichnung des TAN-Mediums         DE   an     ..32  C     1   M: process 1, 3, 4 and media count > 1 and "Bezeichnung ... erforderlich"=2
/// 13  Antwort HHD_UC                      DEG               C     1   M: process 2 and "Antwort HHD_UC erforderlich"=J
/// ```
///
/// The conditional rules above are not checked by the codec. Process `S`
/// (decoupled status poll) does not exist before version 7.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKTAN", version = 6)]
pub struct HkTan6 {
    /// 2 TAN-Prozess: 1, 2, 3 or 4.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Segmentkennung: the segment the challenge refers to (`HKIDN` in the
    /// dialog initialisation).
    #[de(max = 6)]
    pub segment_code: Option<An>,
    /// 4 Kontoverbindung international Auftraggeber.
    pub account: Option<Kti1>,
    /// 5 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 6 Auftragsreferenz: the value the bank gave when it accepted the order.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 7 Weitere TAN folgt.
    pub further_tan_follows: Option<Jn>,
    /// 8 Auftrag stornieren.
    pub cancel_order: Option<Jn>,
    /// 9 SMS-Abbuchungskonto.
    pub sms_charge_account: Option<Kti1>,
    /// 10 Challenge-Klasse.
    #[de(max = 2)]
    pub challenge_class: Option<Num>,
    /// 11 Parameter Challenge-Klasse.
    pub challenge_class_parameters: Option<ChallengeClassParameters1>,
    /// 12 Bezeichnung des TAN-Mediums.
    #[de(max = 32)]
    pub tan_medium_name: Option<An>,
    /// 13 Antwort HHD_UC.
    pub hhd_uc_response: Option<HhdUcResponse1>,
}

/// HKTAN version 7, customer order: version 6 plus the decoupled process
/// `S` (PIN/TAN B.5.2, B.4.2.2, B.4.3.4).
///
/// The layout is that of [`HkTan6`]; differences:
///
/// - 2 TAN-Prozess is version 2 of the element: 1, 2, 3, 4 or `S` (status
///   poll of a decoupled order).
/// - 6 Auftragsreferenz is M for process 2, 3 and S.
/// - 7 Weitere TAN folgt is M for process 1, 2 and S.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKTAN", version = 7)]
pub struct HkTan7 {
    /// 2 TAN-Prozess: 1, 2, 3, 4 or S.
    #[de(len = 1)]
    pub process: Code,
    /// 3 Segmentkennung.
    #[de(max = 6)]
    pub segment_code: Option<An>,
    /// 4 Kontoverbindung international Auftraggeber.
    pub account: Option<Kti1>,
    /// 5 Auftrags-Hashwert.
    #[de(max = 256)]
    pub order_hash: Option<Bin>,
    /// 6 Auftragsreferenz.
    #[de(max = 35)]
    pub order_reference: Option<An>,
    /// 7 Weitere TAN folgt.
    pub further_tan_follows: Option<Jn>,
    /// 8 Auftrag stornieren.
    pub cancel_order: Option<Jn>,
    /// 9 SMS-Abbuchungskonto.
    pub sms_charge_account: Option<Kti1>,
    /// 10 Challenge-Klasse.
    #[de(max = 2)]
    pub challenge_class: Option<Num>,
    /// 11 Parameter Challenge-Klasse.
    pub challenge_class_parameters: Option<ChallengeClassParameters1>,
    /// 12 Bezeichnung des TAN-Mediums.
    #[de(max = 32)]
    pub tan_medium_name: Option<An>,
    /// 13 Antwort HHD_UC.
    pub hhd_uc_response: Option<HhdUcResponse1>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKTAN:4:6+4+HKIDN'",
    "HKTAN:5:6+2++++REF-0001+N'",
    "HKTAN:4:7+4+HKKAZ+++++++++Mein Handy'",
    "HKTAN:3:7+S++++REF-0001+N'",
    "HKTAN:3:5+2++++REF-0001++N'",
    "HKTAN:3:5+4+HKIDN++++++++++Mein Handy'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{from_wire, to_wire};

    fn build(
        version: u16,
        process: &str,
        segment: Option<&str>,
        reference: Option<&str>,
        medium: Option<&str>,
    ) -> Vec<u8> {
        use crate::segments::request::{TanRequest, hktan};
        let request = match (process, segment, reference) {
            ("4", Some(segment), _) => TanRequest::Start { segment },
            ("2", _, Some(task_reference)) => TanRequest::Submit { task_reference },
            ("S", _, Some(task_reference)) => TanRequest::DecoupledStatus { task_reference },
            other => panic!("no such request: {other:?}"),
        };
        crate::codec::wire::serialize_segment(&hktan(version, request, medium, 3).unwrap()).unwrap()
    }

    #[test]
    fn wire_round_trips() {
        let v: HkTan6 = round_trip(SAMPLES[0]);
        assert_eq!(v.process.as_str(), "4");
        assert_eq!(v.segment_code.unwrap().as_str(), "HKIDN");
        let v: HkTan6 = round_trip(SAMPLES[1]);
        assert_eq!(v.order_reference.unwrap().as_str(), "REF-0001");
        assert_eq!(v.further_tan_follows, Some(Jn::NO));
        let v: HkTan7 = round_trip(SAMPLES[2]);
        assert_eq!(v.tan_medium_name.unwrap().as_str(), "Mein Handy");
        let v: HkTan7 = round_trip(SAMPLES[3]);
        assert_eq!(v.process.as_str(), "S");
    }

    #[test]
    fn version_5_round_trips() {
        let v: HkTan5 = round_trip(SAMPLES[4]);
        assert_eq!(v.order_reference.unwrap().as_str(), "REF-0001");
        assert_eq!(v.tan_list_number, None);
        assert_eq!(v.further_tan_follows, Some(Jn::NO));
        let v: HkTan5 = round_trip(SAMPLES[5]);
        assert_eq!(v.segment_code.unwrap().as_str(), "HKIDN");
        assert_eq!(v.tan_medium_name.unwrap().as_str(), "Mein Handy");
        let v: HkTan5 = round_trip(
            "HKTAN:3:5+1+HKCCS+DE89370400440532013000:COBADEFFXXX+@4@HASH++LISTE-1+N++DE89370400440532013000:COBADEFFXXX+3+P1+Mein Handy'",
        );
        assert_eq!(v.tan_list_number.unwrap().as_str(), "LISTE-1");
        // No element 14 (Antwort HHD_UC) in this version.
        let e = from_wire::<HkTan5>(b"HKTAN:3:5+2++++R++N+++++M+X'").unwrap_err();
        assert_eq!(e.kind, crate::codec::ErrorKind::UnexpectedData);
    }

    /// The protocol's HKTAN 5 and its expected wire string, with the list
    /// index element. The protocol's decoupled
    /// poll does not send `S` with HKTAN 5
    /// (`segments::request::tests::hktan_v5_decoupled_poll_uses_process_2_wire_format`);
    /// the codec builds whatever process it is given.
    #[test]
    fn version_5_builds_the_expected_wire() {
        assert_eq!(
            build(5, "2", None, Some("REF-1"), None),
            b"HKTAN:3:5+2++++REF-1++N'"
        );
        assert_eq!(
            build(5, "4", Some("HKIDN"), None, Some("Handy")),
            b"HKTAN:3:5+4+HKIDN++++++++++Handy'"
        );
        assert_eq!(
            build(5, "S", None, Some("REF-1"), None),
            b"HKTAN:3:5+2++++REF-1++N'"
        );
    }

    #[test]
    fn optional_groups_round_trip() {
        let wire = "HKTAN:3:7+1+HKCCS+DE89370400440532013000:COBADEFFXXX+@4@HASH++J+J+DE89370400440532013000:COBADEFFXXX+3+P1:P2+Mein Handy+ATC1:@2@AC:@2@EF:@2@CV:@2@VI'";
        let v: HkTan7 = round_trip(wire);
        assert_eq!(v.account.unwrap().bic.unwrap().as_str(), "COBADEFFXXX");
        assert_eq!(v.order_hash.unwrap().as_bytes(), b"HASH");
        assert_eq!(v.challenge_class, Some(Num::new(3)));
        assert_eq!(v.challenge_class_parameters.unwrap().parameters.len(), 2);
        assert_eq!(v.hhd_uc_response.unwrap().atc.as_str(), "ATC1");
    }

    /// The protocol's HKTAN and its expected wire strings.
    #[test]
    fn builds_the_expected_wire() {
        let cases = [
            (7, "4", Some("HKIDN"), None, None, "HKTAN:3:7+4+HKIDN'"),
            (
                7,
                "4",
                Some("HKKAZ"),
                None,
                Some("Mein Handy"),
                "HKTAN:3:7+4+HKKAZ+++++++++Mein Handy'",
            ),
            (
                7,
                "2",
                None,
                Some("REF-0001"),
                None,
                "HKTAN:3:7+2++++REF-0001+N'",
            ),
        ];
        for (version, process, segment, reference, medium, old) in cases {
            let built = build(version, process, segment, reference, medium);
            assert_eq!(built, old.as_bytes());
            let (h, v) = from_wire::<HkTan7>(&built).unwrap();
            assert_eq!(to_wire(&v, h.number, None).unwrap(), built);
            assert_eq!(v.process.as_str(), process);
        }
    }

    /// PIN/TAN B.5.1 lists only processes 1-4 for version 6 (`S` is
    /// version 7, B.5.2), so the protocol polls a decoupled confirmation
    /// with process 2 in version 6 (see
    /// `segments::request::tests::hktan_v6_decoupled_poll_uses_process_2_wire_format`).
    /// The codec does not check value lists, so `S` in version 6 still reads.
    #[test]
    fn version_6_status_poll_is_process_2() {
        let built = build(6, "S", None, Some("REF-1"), None);
        assert_eq!(built, b"HKTAN:3:6+2++++REF-1+N'");
        let v = from_wire::<HkTan6>(b"HKTAN:3:6+S++++REF-1+N'").unwrap().1;
        assert_eq!(v.process.as_str(), "S");
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HkTan7>(b"HKTAN:3:7'").unwrap_err();
        assert_eq!(e.field, "process");
        let e = from_wire::<HkTan7>(b"HKTAN:3:7+44'").unwrap_err();
        assert_eq!(e.field, "process");
        let e = from_wire::<HkTan7>(b"HKTAN:3:7+4+HKIDN123'").unwrap_err();
        assert_eq!(e.field, "segment_code");
        let e = from_wire::<HkTan7>(b"HKTAN:3:7+2++++REF+X'").unwrap_err();
        assert_eq!((e.de, e.field.as_str()), (Some(7), "further_tan_follows"));
    }
}
