//! HKWDU, Depotumsätze anfordern (depot transactions request). Messages
//! 2022, C.4.3.2.

use super::common::Ktv3;
use crate::codec::{An, Code, Dat, DataElementGroup, Jn, Num, Segment};

/// Wertpapierreferenz, version 2. Messages 2022, data dictionary.
///
/// ```text
/// Nr. Name            Typ  Format Länge Status Anzahl
///  1  Referenzart     DE   code    1    M     1    1, 2, 3, 4
///  2  Wertpapiercode  DE   an     ..30  M     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct SecurityReference2 {
    /// 1 Referenzart: `1`, `2`, `3` or `4`.
    #[de(len = 1)]
    pub kind: Code,
    /// 2 Wertpapiercode.
    #[de(max = 30)]
    pub security_code: An,
}

/// HKWDU version 5, customer order.
///
/// ```text
/// Nr. Name                     Typ  Format Länge Status Anzahl
///  1  Segmentkopf              DEG              M     1
///  2  Depot                    DEG  ktv v3 #    M     1
///  3  Alle Depots              DE   jn     #    M     1
///  4  Wertpapierreferenz       DEG  v2     #    O     1
///  5  Zeitraum von             DE   dat    #    O     1
///  6  Zeitraum bis             DE   dat    #    O     1
///  7  Maximale Anzahl Einträge DE   num   ..4   O     1
///  8  Aufsetzpunkt             DE   an    ..35  C     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKWDU", version = 5)]
pub struct HkWdu5 {
    /// 2 Depot.
    pub depot: Ktv3,
    /// 3 Alle Depots.
    pub all_depots: Jn,
    /// 4 Wertpapierreferenz.
    pub security_reference: Option<SecurityReference2>,
    /// 5 Zeitraum von.
    pub start_date: Option<Dat>,
    /// 6 Zeitraum bis.
    pub end_date: Option<Dat>,
    /// 7 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 8 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKWDU:3:5+0123456789::280:12030000+N'",
    "HKWDU:3:5+0123456789::280:12030000+N++20260101+20260331'",
    "HKWDU:3:5+0123456789::280:12030000+N+++++TD-9'",
    "HKWDU:3:5+0123456789::280:12030000+N+1:DE0001234567+20260101+20260331+10+TD-9'",
];

#[cfg(test)]
mod tests {
    use super::super::common::Kik1;
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{An, Dig, ErrorKind, Id, from_wire, to_wire};
    use crate::protocol::Account;
    use crate::types::Blz;
    use jiff::civil::Date;

    fn depot() -> Ktv3 {
        Ktv3 {
            account_number: Id::new("0123456789").unwrap(),
            subaccount: None,
            bank: Kik1 {
                country: Dig::new("280").unwrap(),
                bank_code: Some(An::new("12030000").unwrap()),
            },
        }
    }

    fn account() -> Account {
        Account::national("0123456789", None, &Blz::new("12030000"), "BYLADEM1001").unwrap()
    }

    fn date(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).unwrap()
    }

    fn request(
        security_reference: Option<SecurityReference2>,
        start_date: Option<Date>,
        end_date: Option<Date>,
        max_entries: Option<u64>,
        touchdown: Option<&str>,
    ) -> HkWdu5 {
        HkWdu5 {
            depot: depot(),
            all_depots: Jn::NO,
            security_reference,
            start_date: start_date.map(|d| Dat::new(d).unwrap()),
            end_date: end_date.map(|d| Dat::new(d).unwrap()),
            max_entries: max_entries.map(Num::new),
            touchdown: touchdown.map(|s| An::new(s).unwrap()),
        }
    }

    #[test]
    fn wire_round_trips() {
        let v: HkWdu5 = round_trip(SAMPLES[0]);
        assert_eq!(v.depot.account_number.as_str(), "0123456789");
        assert_eq!(v.all_depots, Jn::NO);
        assert!(v.security_reference.is_none());
        assert!(v.start_date.is_none());
        assert!(v.end_date.is_none());
        assert!(v.max_entries.is_none());
        assert!(v.touchdown.is_none());

        let v: HkWdu5 = round_trip(SAMPLES[1]);
        assert_eq!(v.start_date.unwrap().date(), date(2026, 1, 1));
        assert_eq!(v.end_date.unwrap().date(), date(2026, 3, 31));
        assert!(v.security_reference.is_none());

        let v: HkWdu5 = round_trip(SAMPLES[2]);
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "TD-9");
        assert!(v.start_date.is_none());

        let v: HkWdu5 = round_trip(SAMPLES[3]);
        let r = v.security_reference.unwrap();
        assert_eq!(r.kind.as_str(), "1");
        assert_eq!(r.security_code.as_str(), "DE0001234567");
        assert_eq!(v.max_entries, Some(Num::new(10)));
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "TD-9");
    }

    #[test]
    fn builds_the_expected_wire() {
        let start = date(2026, 1, 1);
        let end = date(2026, 3, 31);
        let cases = [
            (Some(start), Some(end), None),
            (None, None, None),
            (None, None, Some("TD-9")),
        ];
        for (start_date, end_date, touchdown) in cases {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hkwdu(3, 5, &account(), start_date, end_date, touchdown)
                    .unwrap(),
            )
            .unwrap();
            let derived = request(None, start_date, end_date, None, touchdown);
            assert_eq!(to_wire(&derived, 3, None).unwrap(), built);
        }
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HkWdu5>(b"HKWDU:3:5+0123456789::280:12030000+N+12:DE0001234567'")
            .unwrap_err();
        assert_eq!(e.field, "security_reference.kind");
        assert_eq!(
            e.kind,
            ErrorKind::WrongLength {
                expected: 1,
                actual: 2
            }
        );
    }
}
