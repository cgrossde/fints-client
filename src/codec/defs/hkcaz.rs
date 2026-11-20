//! HKCAZ, Kontoumsätze/Zeitraum camt (camt transaction request). Messages
//! 2022, C.2.3.1.1.1.

use super::common::Kti1;
use crate::codec::{An, Dat, DataElementGroup, Jn, Num, Segment};

/// HKCAZ version 1, customer order.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung international DEG  kti     #    M     1
///  3  Unterstützte camt-messages    DEG               M     1
///  4  Alle Konten                   DE   jn      #    M     1
///  5  Von Datum                     DE   dat     #    O     1
///  6  Bis Datum                     DE   dat     #    O     1
///  7  Maximale Anzahl Einträge      DE   num    ..4   C     1
///  8  Aufsetzpunkt                  DE   an     ..35  C     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKCAZ", version = 1)]
pub struct HkCaz1 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 Unterstützte camt-messages.
    pub camt_messages: SupportedCamtMessages1,
    /// 4 Alle Konten.
    pub all_accounts: Jn,
    /// 5 Von Datum.
    pub from_date: Option<Dat>,
    /// 6 Bis Datum.
    pub to_date: Option<Dat>,
    /// 7 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 8 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

/// Unterstützte camt-messages, version 1. Messages 2022, data dictionary:
/// `1 camt-Descriptor DE an ..256 M ..99`.
///
/// `max_count` is 99 because this group is not last in HKCAZ.
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SupportedCamtMessages1 {
    /// 1 camt-Descriptor.
    #[de(max = 256, min_count = 1, max_count = 99)]
    pub descriptors: Vec<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKCAZ:3:1+DE89370400440532013000:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+N'",
    "HKCAZ:3:1+DE89370400440532013000:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+N+20260101+20260331'",
    "HKCAZ:3:1+DE89370400440532013000:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+N+20260101+20260331++T1'",
    "HKCAZ:3:1+DE89370400440532013000:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02:urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.08+N+20260101+20260331+20'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, from_wire, to_wire};
    use crate::protocol::Account;
    use jiff::civil::Date;

    const IBAN: &str = "DE89370400440532013000";
    const CAMT: &str = "urn:iso:std:iso:20022:tech:xsd:camt.052.001.02";
    const CAMT08: &str = "urn:iso:std:iso:20022:tech:xsd:camt.052.001.08";

    fn an(s: &str) -> An {
        An::new(s).unwrap()
    }

    fn account(iban: &str, bic: &str) -> Kti1 {
        Kti1 {
            iban: Some(an(iban)),
            bic: Some(an(bic)),
            account_number: None,
            subaccount: None,
            bank: None,
        }
    }

    fn dat(y: i16, m: i8, d: i8) -> Dat {
        Dat::new(Date::new(y, m, d).unwrap()).unwrap()
    }

    #[test]
    fn wire_round_trips() {
        let v: HkCaz1 = round_trip(SAMPLES[0]);
        assert_eq!(v.account.iban.as_ref().map(An::as_str), Some(IBAN));
        assert_eq!(v.camt_messages.descriptors, vec![an(CAMT)]);
        assert_eq!(v.all_accounts, Jn::NO);
        assert_eq!(
            (v.from_date, v.to_date, v.max_entries, v.touchdown),
            (None, None, None, None)
        );

        let v: HkCaz1 = round_trip(SAMPLES[1]);
        assert_eq!(v.from_date, Some(dat(2026, 1, 1)));
        assert_eq!(v.to_date, Some(dat(2026, 3, 31)));
        assert_eq!((v.max_entries, v.touchdown), (None, None));

        let v: HkCaz1 = round_trip(SAMPLES[2]);
        assert_eq!(v.touchdown, Some(an("T1")));
        assert_eq!(v.max_entries, None);

        let v: HkCaz1 = round_trip(SAMPLES[3]);
        assert_eq!(v.camt_messages.descriptors, vec![an(CAMT), an(CAMT08)]);
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert_eq!(v.touchdown, None);
    }

    #[test]
    fn builds_the_expected_wire() {
        let iban = "DE00120300009900012345";
        let bic = "BYLADEM1001";
        let account_id = Account::new(iban, bic).unwrap();
        let start = Date::new(2026, 1, 1).unwrap();
        let end = Date::new(2026, 3, 31).unwrap();
        let format = crate::segments::request::CAMT052_DEFAULT;
        for touchdown in [None, Some("TD-1")] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hkcaz(3, 1, &account_id, format, start, end, touchdown)
                    .unwrap(),
            )
            .unwrap();
            let v = HkCaz1 {
                account: account(iban, bic),
                camt_messages: SupportedCamtMessages1 {
                    descriptors: vec![an(format)],
                },
                all_accounts: Jn::NO,
                from_date: Some(dat(2026, 1, 1)),
                to_date: Some(dat(2026, 3, 31)),
                max_entries: None,
                touchdown: touchdown.map(an),
            };
            assert_eq!(to_wire(&v, 3, None).unwrap(), built);
        }
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HkCaz1>(
            b"HKCAZ:3:1+DE89370400440532013000:COBADEFFXXX+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+X'",
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::InvalidYesNo);
        assert_eq!(e.field, "all_accounts");
    }
}
