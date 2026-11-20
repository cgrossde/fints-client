//! HKKAZ, Kontoumsätze anfordern (statement request, MT940). HBCI 2.2,
//! VII.2.1.1 (v5); Messages 2022, C.2.1.1.1.1 (v6, v7).

use super::common::{Kti1, Ktv3};
use crate::codec::{An, Dat, Jn, Num, Segment};

/// HKKAZ version 5, customer order. HBCI 2.2, VII.2.1.1. Account is national (`ktv`).
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung Auftraggeber  DEG  ktv     #    M     1
///  3  Alle Konten                   DE   jn      #    M     1
///  4  Von Datum                     DE   dat     #    K     1
///  5  Bis Datum                     DE   dat     #    K     1
///  6  Maximale Anzahl Einträge      DE   num    ..4   K     1   >0
///  7  Aufsetzpunkt                  DE   an     ..35  K     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKKAZ", version = 5)]
pub struct HkKaz5 {
    /// 2 Kontoverbindung Auftraggeber.
    pub account: Ktv3,
    /// 3 Alle Konten.
    pub all_accounts: Jn,
    /// 4 Von Datum.
    pub from_date: Option<Dat>,
    /// 5 Bis Datum.
    pub to_date: Option<Dat>,
    /// 6 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 7 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

/// HKKAZ version 6, customer order. Account is national (`ktv`).
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung Auftraggeber  DEG  ktv     #    M     1
///  3  Alle Konten                   DE   jn      #    M     1
///  4  Von Datum                     DE   dat     #    O     1
///  5  Bis Datum                     DE   dat     #    O     1
///  6  Maximale Anzahl Einträge      DE   num    ..4   C     1
///  7  Aufsetzpunkt                  DE   an     ..35  C     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKKAZ", version = 6)]
pub struct HkKaz6 {
    /// 2 Kontoverbindung Auftraggeber.
    pub account: Ktv3,
    /// 3 Alle Konten.
    pub all_accounts: Jn,
    /// 4 Von Datum.
    pub from_date: Option<Dat>,
    /// 5 Bis Datum.
    pub to_date: Option<Dat>,
    /// 6 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 7 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

/// HKKAZ version 7, customer order. Same body as version 6 with
/// international account (`kti`).
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung Auftraggeber  DEG  kti     #    M     1
///  3  Alle Konten                   DE   jn      #    M     1
///  4  Von Datum                     DE   dat     #    O     1
///  5  Bis Datum                     DE   dat     #    O     1
///  6  Maximale Anzahl Einträge      DE   num    ..4   C     1
///  7  Aufsetzpunkt                  DE   an     ..35  C     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKKAZ", version = 7)]
pub struct HkKaz7 {
    /// 2 Kontoverbindung Auftraggeber.
    pub account: Kti1,
    /// 3 Alle Konten.
    pub all_accounts: Jn,
    /// 4 Von Datum.
    pub from_date: Option<Dat>,
    /// 5 Bis Datum.
    pub to_date: Option<Dat>,
    /// 6 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 7 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKKAZ:3:6+1234567890::280:12345678+N'",
    "HKKAZ:3:6+1234567890::280:12345678+N+20250101+20250131'",
    "HKKAZ:3:6+1234567890::280:12345678+N++20250131'",
    "HKKAZ:3:6+1234567890::280:12345678+N+20250101+20250131+100+T1'",
    "HKKAZ:3:7+DE89370400440532013000:COBADEFFXXX+N+20250101+20250131'",
    "HKKAZ:3:7+DE89370400440532013000:COBADEFFXXX+N+20250101+20250131++T1'",
    "HKKAZ:3:5+1234567890::280:12345678+N'",
    "HKKAZ:3:5+1234567890::280:12345678+N+20250101+20250131+100+T1'",
];

#[cfg(test)]
mod tests {
    use super::super::common::Kik1;
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{Dig, ErrorKind, Id, from_wire, to_wire};
    use crate::protocol::Account;
    use jiff::civil::Date;

    const IBAN: &str = "DE00120300009900012345";
    const BIC: &str = "BYLADEM1001";

    fn date(y: i16, m: i8, d: i8) -> Date {
        Date::new(y, m, d).unwrap()
    }

    fn dat(y: i16, m: i8, d: i8) -> Dat {
        Dat::new(date(y, m, d)).unwrap()
    }

    fn v6(touchdown: Option<&str>) -> HkKaz6 {
        HkKaz6 {
            account: Ktv3 {
                account_number: Id::new("9900012345").unwrap(),
                subaccount: None,
                bank: Kik1 {
                    country: Dig::new("280").unwrap(),
                    bank_code: Some(An::new("12030000").unwrap()),
                },
            },
            all_accounts: Jn::NO,
            from_date: Some(dat(2025, 3, 29)),
            to_date: Some(dat(2026, 3, 29)),
            max_entries: None,
            touchdown: touchdown.map(|t| An::new(t).unwrap()),
        }
    }

    fn v5(touchdown: Option<&str>) -> HkKaz5 {
        HkKaz5 {
            account: Ktv3 {
                account_number: Id::new("9900012345").unwrap(),
                subaccount: None,
                bank: Kik1 {
                    country: Dig::new("280").unwrap(),
                    bank_code: Some(An::new("12030000").unwrap()),
                },
            },
            all_accounts: Jn::NO,
            from_date: Some(dat(2025, 3, 29)),
            to_date: Some(dat(2026, 3, 29)),
            max_entries: None,
            touchdown: touchdown.map(|t| An::new(t).unwrap()),
        }
    }

    fn v7(touchdown: Option<&str>) -> HkKaz7 {
        HkKaz7 {
            account: Kti1 {
                iban: Some(An::new(IBAN).unwrap()),
                bic: Some(An::new(BIC).unwrap()),
                account_number: None,
                subaccount: None,
                bank: None,
            },
            all_accounts: Jn::NO,
            from_date: Some(dat(2025, 3, 29)),
            to_date: Some(dat(2026, 3, 29)),
            max_entries: None,
            touchdown: touchdown.map(|t| An::new(t).unwrap()),
        }
    }

    #[test]
    fn wire_round_trips() {
        let v: HkKaz6 = round_trip(SAMPLES[0]);
        assert_eq!(v.account.account_number.as_str(), "1234567890");
        assert!(v.from_date.is_none());
        assert!(v.to_date.is_none());
        assert!(v.max_entries.is_none());
        assert!(v.touchdown.is_none());

        let v: HkKaz6 = round_trip(SAMPLES[1]);
        assert_eq!(v.from_date.unwrap().date(), date(2025, 1, 1));
        assert_eq!(v.to_date.unwrap().date(), date(2025, 1, 31));
        assert!(v.max_entries.is_none());

        let v: HkKaz6 = round_trip(SAMPLES[2]);
        assert!(v.from_date.is_none());
        assert_eq!(v.to_date.unwrap().date(), date(2025, 1, 31));

        let v: HkKaz6 = round_trip(SAMPLES[3]);
        assert_eq!(v.max_entries, Some(Num::new(100)));
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");

        let v: HkKaz7 = round_trip(SAMPLES[4]);
        assert_eq!(
            v.account.iban.as_ref().unwrap().as_str(),
            "DE89370400440532013000"
        );
        assert_eq!(v.account.bic.as_ref().unwrap().as_str(), "COBADEFFXXX");
        assert!(v.touchdown.is_none());

        let v: HkKaz7 = round_trip(SAMPLES[5]);
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");
        assert!(v.max_entries.is_none());

        let v: HkKaz5 = round_trip(SAMPLES[6]);
        assert_eq!(v.account.account_number.as_str(), "1234567890");
        assert!(v.from_date.is_none());
        assert!(v.touchdown.is_none());
        let v: HkKaz5 = round_trip(SAMPLES[7]);
        assert_eq!(v.from_date.unwrap().date(), date(2025, 1, 1));
        assert_eq!(v.to_date.unwrap().date(), date(2025, 1, 31));
        assert_eq!(v.max_entries, Some(Num::new(100)));
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");
    }

    #[test]
    fn builds_the_expected_wire() {
        let account = Account::new(IBAN, BIC).unwrap();
        let start = date(2025, 3, 29);
        let end = date(2026, 3, 29);
        for touchdown in [None, Some("T1")] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hkkaz(3, 6, &account, start, end, touchdown).unwrap(),
            )
            .unwrap();
            assert_eq!(to_wire(&v6(touchdown), 3, None).unwrap(), built);
        }
        for touchdown in [None, Some("4711")] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hkkaz(3, 7, &account, start, end, touchdown).unwrap(),
            )
            .unwrap();
            assert_eq!(to_wire(&v7(touchdown), 3, None).unwrap(), built);
        }
        for touchdown in [None, Some("T1")] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hkkaz(3, 5, &account, start, end, touchdown).unwrap(),
            )
            .unwrap();
            assert_eq!(to_wire(&v5(touchdown), 3, None).unwrap(), built);
        }
    }

    #[test]
    fn rules_are_enforced() {
        let long = "x".repeat(36);
        let e = from_wire::<HkKaz7>(
            format!("HKKAZ:3:7+DE89370400440532013000:COBADEFFXXX+N++++{long}'").as_bytes(),
        )
        .unwrap_err();
        assert_eq!(e.field, "touchdown");
        assert_eq!(
            e.kind,
            ErrorKind::TooLong {
                max: 35,
                actual: 36
            }
        );
        let e = from_wire::<HkKaz5>(
            format!("HKKAZ:3:5+1234567890::280:12345678+N++++{long}'").as_bytes(),
        )
        .unwrap_err();
        assert_eq!(e.field, "touchdown");
        assert_eq!(
            e.kind,
            ErrorKind::TooLong {
                max: 35,
                actual: 36
            }
        );
    }
}
