//! HKWPD, Depotaufstellung anfordern. HBCI 2.2, VII.4.3.1 (v5);
//! Messages 2022, C.4.3.1 (v6).

use super::common::Ktv3;
use crate::codec::{An, Code, Cur, Num, Segment};

/// HKWPD version 5, customer order. HBCI 2.2, VII.4.3.1.
///
/// Kursqualität is `num 1` (codes 1 Delayed, 2 Realtime). Version 6 uses `code 1`.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Depot                         DEG  ktv     #    M     1
///  3  Währung der Depotaufstellung  DE   cur     #    K     1
///  4  Kursqualität                  DE   num     1    K     1
///  5  Maximale Anzahl Einträge      DE   num    ..4   K     1   >0
///  6  Aufsetzpunkt                  DE   an     ..35  K     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKWPD", version = 5)]
pub struct HkWpd5 {
    /// 2 Depot.
    pub depot: Ktv3,
    /// 3 Währung der Depotaufstellung.
    pub currency: Option<Cur>,
    /// 4 Kursqualität: 1 Delayed, 2 Realtime.
    #[de(len = 1)]
    pub quality: Option<Num>,
    /// 5 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 6 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

/// HKWPD version 6, customer order.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Depot                         DEG  ktv v3  #    M     1
///  3  Währung der Depotaufstellung  DE   cur     #    C     1
///  4  Kursqualität                  DE   code    1    C     1   1, 2
///  5  Maximale Anzahl Einträge      DE   num    ..4   C     1
///  6  Aufsetzpunkt                  DE   an     ..35  C     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKWPD", version = 6)]
pub struct HkWpd6 {
    /// 2 Depot.
    pub depot: Ktv3,
    /// 3 Währung der Depotaufstellung.
    pub currency: Option<Cur>,
    /// 4 Kursqualität: 1, 2.
    #[de(len = 1)]
    pub quality: Option<Code>,
    /// 5 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 6 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKWPD:3:6+1234567890::280:12345678'",
    "HKWPD:3:6+1234567890::280:12345678+EUR+1+20+T1'",
    "HKWPD:3:6+1234567890::280:12345678++++T1'",
    "HKWPD:3:5+1234567890::280:12345678'",
    "HKWPD:3:5+1234567890::280:12345678+EUR+1+20+T1'",
    "HKWPD:3:5+1234567890::280:12345678++++T1'",
];

#[cfg(test)]
mod tests {
    use super::super::common::Kik1;
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{Dig, Id, from_wire, to_wire};
    use crate::protocol::Account;

    const IBAN: &str = "DE00120300009900012345";
    const BIC: &str = "BYLADEM1001";

    fn depot() -> Ktv3 {
        Ktv3 {
            account_number: Id::new("9900012345").unwrap(),
            subaccount: None,
            bank: Kik1 {
                country: Dig::new("280").unwrap(),
                bank_code: Some(An::new("12030000").unwrap()),
            },
        }
    }

    fn request(currency: Option<&str>, touchdown: Option<&str>) -> HkWpd6 {
        HkWpd6 {
            depot: depot(),
            currency: currency.map(|c| Cur::new(c).unwrap()),
            quality: None,
            max_entries: None,
            touchdown: touchdown.map(|s| An::new(s).unwrap()),
        }
    }

    fn request5(currency: Option<&str>, touchdown: Option<&str>) -> HkWpd5 {
        HkWpd5 {
            depot: depot(),
            currency: currency.map(|c| Cur::new(c).unwrap()),
            quality: None,
            max_entries: None,
            touchdown: touchdown.map(|s| An::new(s).unwrap()),
        }
    }

    #[test]
    fn wire_round_trips() {
        let v: HkWpd6 = round_trip(SAMPLES[0]);
        assert_eq!(v.depot.account_number.as_str(), "1234567890");
        assert!(v.currency.is_none());
        assert!(v.quality.is_none());
        assert!(v.max_entries.is_none());
        assert!(v.touchdown.is_none());
        let v: HkWpd6 = round_trip(SAMPLES[1]);
        assert_eq!(v.currency.as_ref().unwrap().as_str(), "EUR");
        assert_eq!(v.quality.as_ref().unwrap().as_str(), "1");
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");
        let v: HkWpd6 = round_trip(SAMPLES[2]);
        assert!(v.currency.is_none());
        assert!(v.quality.is_none());
        assert!(v.max_entries.is_none());
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");
        let v: HkWpd5 = round_trip(SAMPLES[3]);
        assert_eq!(v.depot.account_number.as_str(), "1234567890");
        assert!(v.currency.is_none());
        assert!(v.quality.is_none());
        let v: HkWpd5 = round_trip(SAMPLES[4]);
        assert_eq!(v.currency.as_ref().unwrap().as_str(), "EUR");
        assert_eq!(v.quality, Some(Num::new(1)));
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");
        let v: HkWpd5 = round_trip(SAMPLES[5]);
        assert!(v.quality.is_none());
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");
    }

    #[test]
    fn builds_the_expected_wire() {
        let account = Account::new(IBAN, BIC).unwrap();
        for (currency, touchdown) in [
            (None, None),
            (Some("EUR"), None),
            (None, Some("TOUCH123")),
            (Some("EUR"), Some("TOUCH123")),
        ] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hkwpd(3, 6, &account, currency, touchdown).unwrap(),
            )
            .unwrap();
            assert_eq!(
                to_wire(&request(currency, touchdown), 3, None).unwrap(),
                built
            );
        }
        for (currency, touchdown) in [
            (None, None),
            (Some("EUR"), None),
            (None, Some("TOUCH123")),
            (Some("EUR"), Some("TOUCH123")),
        ] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hkwpd(3, 5, &account, currency, touchdown).unwrap(),
            )
            .unwrap();
            assert_eq!(
                to_wire(&request5(currency, touchdown), 3, None).unwrap(),
                built
            );
        }
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HkWpd6>(b"HKWPD:3:6+1234567890::280:12345678+EURO'").unwrap_err();
        assert_eq!(e.field, "currency");
        let e = from_wire::<HkWpd6>(b"HKWPD:3:6+1234567890::280:12345678++12'").unwrap_err();
        assert_eq!(e.field, "quality");
        let e = from_wire::<HkWpd6>(
            b"HKWPD:3:6+1234567890::280:12345678++++AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'",
        )
        .unwrap_err();
        assert_eq!(e.field, "touchdown");
        assert_eq!(
            e.kind,
            crate::codec::ErrorKind::TooLong {
                max: 35,
                actual: 36
            }
        );
        let e = from_wire::<HkWpd5>(b"HKWPD:3:5+1234567890::280:12345678++12'").unwrap_err();
        assert_eq!(e.field, "quality");
    }
}
