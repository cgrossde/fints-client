//! HIVPP, Namensabgleich Prüfergebnis (Verification of Payee, result). VoP
//! 1.02, C.10.7.1.1 and the data dictionary.

use super::common::Tsp1;
use crate::codec::{An, Bin, Code, DataElementGroup, Num, Segment};

/// Ergebnis VOP-Prüfung Einzeltransaktion, version 1. VoP 1.02, D. The
/// result for one payee, an alternative to the pain.002 in single orders.
///
/// ```text
/// Nr. Name                      Typ  Format Länge Status Anzahl
///  1  IBAN Empfänger            DE   an    ..34   M     1
///  2  IBAN-Zusatzinformationen  DE   an    ..140  O     1
///  3  Abweichender Empfängername DE  an    ..140  C     1
///  4  Anderes Identifikationsmerkmal DE an ..256  C     1
///  5  VOP-Prüfergebnis          DE   code    4    M     1
///  6  Grund RVNA                DE   an    ..256  C     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct PayeeCheckResult1 {
    /// 1 IBAN Empfänger.
    #[de(max = 34)]
    pub iban: An,
    /// 2 IBAN-Zusatzinformationen.
    #[de(max = 140)]
    pub iban_info: Option<An>,
    /// 3 Abweichender Empfängername. M when "Anderes Identifikationsmerkmal"
    /// is absent and the result is RVMC; N otherwise.
    #[de(max = 140)]
    pub registered_name: Option<An>,
    /// 4 Anderes Identifikationsmerkmal (name of the checked ID, e.g. LEI).
    /// O when "Abweichender Empfängername" is absent; N otherwise.
    #[de(max = 256)]
    pub other_id: Option<An>,
    /// 5 VOP-Prüfergebnis: RVMC, RCVC, RVNM, RVNA or PDNG.
    #[de(len = 4)]
    pub outcome: Code,
    /// 6 Grund RVNA. O when the result is RVNA; N otherwise.
    #[de(max = 256)]
    pub rvna_reason: Option<An>,
}

/// HIVPP version 1, bank answer to HKVPP 1. VoP 1.02, C.10.7.1.1.
///
/// ```text
/// Nr. Name                       Typ  Format Länge Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  VOP-ID                     DE   bin    ..    C     1
///  3  VOP-ID gültig bis          DEG  tsp     #    C     1
///  4  Polling-ID                 DE   bin    ..    O     1
///  5  Payment Status Report Descriptor DE an ..256 C    1
///  6  Payment Status Report      DE   bin    ..    C     1
///  7  Ergebnis VOP-Prüfung Einzeltransaktion DEG   C     1
///  8  Aufklärungstext Autorisierung trotz Abweichung DE an ..65535 O 1
///  9  Wartezeit vor nächster Abfrage DE num   1    O     1
/// ```
///
/// The VOP-ID and its validity come with the final answer only (touchdown
/// handling). The result is in the pain.002 (5, 6) or in the group (7),
/// the bank's choice.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIVPP", version = 1)]
pub struct HiVpp1 {
    /// 2 VOP-ID. M when the Payment Status Report or the single-transaction
    /// result is delivered completely; N otherwise.
    pub vop_id: Option<Bin>,
    /// 3 VOP-ID gültig bis. O when the VOP-ID is set.
    pub vop_id_valid_until: Option<Tsp1>,
    /// 4 Polling-ID.
    pub polling_id: Option<Bin>,
    /// 5 Payment Status Report Descriptor. M when the report is set.
    #[de(max = 256)]
    pub report_descriptor: Option<An>,
    /// 6 Payment Status Report (the pain.002). O when the single-transaction
    /// result is absent.
    pub report: Option<Bin>,
    /// 7 Ergebnis VOP-Prüfung Einzeltransaktion. O when the report is absent.
    pub result: Option<PayeeCheckResult1>,
    /// 8 Aufklärungstext Autorisierung trotz Abweichung. Must be shown to the
    /// user.
    #[de(max = 65535)]
    pub explanation: Option<An>,
    /// 9 Wartezeit vor nächster Abfrage (seconds).
    #[de(len = 1)]
    pub wait: Option<Num>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIVPP:5:1:4+@5@vop-1+20261117:101500++urn?:x?:pain.002.001.10+@6@<a/>  '",
    "HIVPP:5:1:4+@5@vop-1+20261117+@4@poll+++DE00120300009900012345::Example Payee GmbH::RVMC+Check the name+3'",
    "HIVPP:5:1:4+++@4@poll+++DE00120300009900012345:::LEI:RVNA:Not supported++5'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiVpp1 = round_trip(SAMPLES[0]);
        assert_eq!(
            v.vop_id.as_ref().map(|b| b.0.as_slice()),
            Some(&b"vop-1"[..])
        );
        assert!(v.vop_id_valid_until.as_ref().unwrap().time.is_some());
        assert_eq!(v.report.as_ref().map(|b| b.0.len()), Some(6));
        assert!(v.result.is_none() && v.explanation.is_none() && v.wait.is_none());

        let v: HiVpp1 = round_trip(SAMPLES[1]);
        let r = v.result.as_ref().unwrap();
        assert_eq!(r.outcome.as_str(), "RVMC");
        assert_eq!(
            r.registered_name.as_ref().map(An::as_str),
            Some("Example Payee GmbH")
        );
        assert!(r.iban_info.is_none() && r.other_id.is_none());
        assert_eq!(
            v.explanation.as_ref().map(An::as_str),
            Some("Check the name")
        );
        assert_eq!(v.wait, Some(Num::new(3)));
        assert!(v.vop_id_valid_until.as_ref().unwrap().time.is_none());
    }

    #[test]
    fn a_pending_answer_has_no_vop_id() {
        let v: HiVpp1 = round_trip(SAMPLES[2]);
        assert!(v.vop_id.is_none());
        assert_eq!(v.polling_id.as_ref().map(|b| b.0.len()), Some(4));
        let r = v.result.unwrap();
        assert_eq!(r.outcome.as_str(), "RVNA");
        assert_eq!(r.other_id.as_ref().map(An::as_str), Some("LEI"));
        assert_eq!(
            r.rvna_reason.as_ref().map(An::as_str),
            Some("Not supported")
        );
    }

    #[test]
    fn the_outcome_has_four_characters() {
        let e = from_wire::<HiVpp1>(b"HIVPP:5:1:4+++++++DE00120300009900012345::::RC'");
        assert!(e.is_err());
    }
}
