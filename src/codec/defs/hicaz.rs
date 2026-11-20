//! HICAZ, Kontoumsätze/Zeitraum camt (camt transaction response). Messages
//! 2022, C.2.3.1.1.1.

use super::common::Kti1;
use crate::codec::{An, Bin, DataElementGroup, Segment};

/// HICAZ version 1, bank response.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung international DEG  kti     #    M     1
///  3  camt-Descriptor               DE   an     ..256 M     1
///  4  Gebuchte camt-Umsätze         DEG               M     1
///  5  Nicht gebuchte camt-Umsätze   DE   bin     ..   O     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HICAZ", version = 1)]
pub struct HiCaz1 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 camt-Descriptor.
    #[de(max = 256)]
    pub camt_descriptor: An,
    /// 4 Gebuchte camt-Umsätze.
    pub booked: BookedCamt1,
    /// 5 Nicht gebuchte camt-Umsätze.
    pub pending: Option<Bin>,
}

/// Gebuchte camt-Umsätze, version 1. Messages 2022, data dictionary:
/// `1 camt-Umsätze gebucht DE bin .. M n`.
///
/// The spec says unbounded `n` (one camt.052 per booking day). The codec
/// allows an open repetition only in the last element and pending follows
/// this group, so it is capped at 999 (about four years of booking days per
/// response; a common client choice).
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct BookedCamt1 {
    /// 1 camt-Umsätze gebucht. Spec says unbounded `n`; capped at 999.
    #[de(min_count = 1, max_count = 999)]
    pub documents: Vec<Bin>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HICAZ:4:1:3+DE89370400440532013000:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+@5@<xml>'",
    "HICAZ:4:1:3+DE89370400440532013000:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+@5@<xml>+@4@<p/>'",
    "HICAZ:4:1:3+DE89370400440532013000:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+@5@<xml>:@4@<p/>'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire};

    const IBAN: &str = "DE89370400440532013000";
    const CAMT: &str = "urn:iso:std:iso:20022:tech:xsd:camt.052.001.02";

    #[test]
    fn wire_round_trips() {
        let v: HiCaz1 = round_trip(SAMPLES[0]);
        assert_eq!(v.account.iban.as_ref().map(An::as_str), Some(IBAN));
        assert_eq!(v.camt_descriptor.as_str(), CAMT);
        assert_eq!(v.booked.documents, vec![Bin(b"<xml>".to_vec())]);
        assert_eq!(v.pending, None);

        let v: HiCaz1 = round_trip(SAMPLES[1]);
        assert_eq!(v.booked.documents, vec![Bin(b"<xml>".to_vec())]);
        assert_eq!(v.pending, Some(Bin(b"<p/>".to_vec())));

        let v: HiCaz1 = round_trip(SAMPLES[2]);
        assert_eq!(
            v.booked.documents,
            vec![Bin(b"<xml>".to_vec()), Bin(b"<p/>".to_vec())]
        );
        assert_eq!(v.pending, None);
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HiCaz1>(
            format!(
                "HICAZ:4:1:3+{IBAN}:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02'"
            )
            .as_bytes(),
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::TooFewRepetitions { min: 1, actual: 0 });
        assert_eq!(e.field, "booked.documents");
    }

    /// One document per booking day: 999 are read, followed by pending.
    #[test]
    fn reads_up_to_999_booked_documents() {
        let docs = vec!["@5@<xml>"; 999].join(":");
        let wire = format!(
            "HICAZ:4:1:3+{IBAN}:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+{docs}+@4@<p/>'"
        );
        let v: HiCaz1 = round_trip(&wire);
        assert_eq!(v.booked.documents.len(), 999);
        assert_eq!(v.pending, Some(Bin(b"<p/>".to_vec())));

        let docs = vec!["@5@<xml>"; 1000].join(":");
        let wire = format!(
            "HICAZ:4:1:3+{IBAN}:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+{docs}'"
        );
        assert!(from_wire::<HiCaz1>(wire.as_bytes()).is_err());
    }
}
