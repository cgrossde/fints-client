//! HKSAL, Saldenabfrage (balance request). HBCI 2.2, VII.2.2 (v5);
//! Messages 2022, C.2.1.2.1 (v6), C.2.1.2.2 (v7) and C.2.1.2.3 (v8).

use super::common::{Kti1, Ktv3};
use crate::codec::{An, Jn, Num, Segment};

/// HKSAL version 5, customer order. HBCI 2.2, VII.2.2.
///
/// Account is national `ktv`. HBCI 2.2 II.5.3.3 prints four DEs (number,
/// subaccount, country, bank code); that is the flattened form of [`Ktv3`].
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung Auftraggeber  DEG  ktv     #    M     1
///  3  Alle Konten                   DE   jn      #    M     1
///  4  Maximale Anzahl Einträge      DE   num    ..4   K     1   >0
///  5  Aufsetzpunkt                  DE   an     ..35  K     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKSAL", version = 5)]
pub struct HkSal5 {
    /// 2 Kontoverbindung Auftraggeber.
    pub account: Ktv3,
    /// 3 Alle Konten.
    pub all_accounts: Jn,
    /// 4 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 5 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

/// HKSAL version 6, customer order.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung Auftraggeber  DEG  ktv     #    M     1
///  3  Alle Konten                   DE   jn      #    M     1
///  4  Maximale Anzahl Einträge      DE   num    ..4   O     1   >0
///  5  Aufsetzpunkt                  DE   an     ..35  C     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKSAL", version = 6)]
pub struct HkSal6 {
    /// 2 Kontoverbindung Auftraggeber.
    pub account: Ktv3,
    /// 3 Alle Konten.
    pub all_accounts: Jn,
    /// 4 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 5 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

/// HKSAL version 7, customer order.
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung international DEG  kti     #    M     1
///  3  Alle Konten                   DE   jn      #    M     1
///  4  Maximale Anzahl Einträge      DE   num    ..4   C     1   >0; O if BPD allows, N otherwise
///  5  Aufsetzpunkt                  DE   an     ..35  C     1   M after a touchdown, N otherwise
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKSAL", version = 7)]
pub struct HkSal7 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 Alle Konten.
    pub all_accounts: Jn,
    /// 4 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 5 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

/// HKSAL version 8, customer order. Messages 2022, C.2.1.2.3: the same
/// rows as [`HkSal7`].
///
/// ```text
/// Nr. Name                          Typ  Format Länge Status Anzahl
///  1  Segmentkopf                   DEG               M     1
///  2  Kontoverbindung international DEG  kti     #    M     1
///  3  Alle Konten                   DE   jn      #    M     1
///  4  Maximale Anzahl Einträge      DE   num    ..4   C     1   >0; O if BPD allows, N otherwise
///  5  Aufsetzpunkt                  DE   an     ..35  C     1   M after a touchdown, N otherwise
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HKSAL", version = 8)]
pub struct HkSal8 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 Alle Konten.
    pub all_accounts: Jn,
    /// 4 Maximale Anzahl Einträge.
    #[de(max = 4)]
    pub max_entries: Option<Num>,
    /// 5 Aufsetzpunkt.
    #[de(max = 35)]
    pub touchdown: Option<An>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HKSAL:3:6+1234567890::280:12345678+N'",
    "HKSAL:3:6+1234567890::280:12345678+N+20+T1'",
    "HKSAL:3:7+DE89370400440532013000:COBADEFFXXX+N'",
    "HKSAL:3:7+DE89370400440532013000:COBADEFFXXX+N++T1'",
    "HKSAL:3:5+1234567890::280:12345678+N'",
    "HKSAL:3:5+1234567890::280:12345678+N+20+T1'",
    "HKSAL:3:8+DE89370400440532013000:COBADEFFXXX+N'",
    "HKSAL:3:8+DE89370400440532013000:COBADEFFXXX+N+20+T1'",
];

#[cfg(test)]
mod tests {
    use super::super::common::{Kik1, Ktv3};
    use super::*;
    use crate::codec::{Dig, Id, from_wire, to_wire};
    use crate::protocol::Account;
    use proptest::prelude::*;

    // Synthetic; the same values as the builder's wire tests.
    const IBAN: &str = "DE00120300009900012345";
    const BIC: &str = "BYLADEM1001";

    fn an(s: &str) -> An {
        An::new(s).unwrap()
    }

    fn by_iban(touchdown: Option<&str>) -> HkSal7 {
        HkSal7 {
            account: Kti1 {
                iban: Some(an(IBAN)),
                bic: Some(an(BIC)),
                account_number: None,
                subaccount: None,
                bank: None,
            },
            all_accounts: Jn::NO,
            max_entries: None,
            touchdown: touchdown.map(an),
        }
    }

    fn round_trip(wire: &str) -> HkSal7 {
        let (header, value) = from_wire::<HkSal7>(wire.as_bytes()).unwrap();
        let back = to_wire(&value, header.number, header.reference).unwrap();
        assert_eq!(String::from_utf8(back).unwrap(), wire);
        value
    }

    #[test]
    fn wire_round_trips() {
        assert_eq!(
            round_trip(&format!("HKSAL:3:7+{IBAN}:{BIC}+N'")),
            by_iban(None)
        );
        assert_eq!(
            round_trip(&format!("HKSAL:3:7+{IBAN}:{BIC}+N++T1'")),
            by_iban(Some("T1"))
        );
        let v = round_trip("HKSAL:3:7+:BYLADEM1001:9900012345::280:12030000+J+20+T?:2'");
        assert_eq!(
            v.account.bank,
            Some(Kik1 {
                country: Dig::new("280").unwrap(),
                bank_code: Some(an("12030000"))
            })
        );
        assert_eq!(
            v.account.account_number,
            Some(Id::new("9900012345").unwrap())
        );
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert_eq!(v.touchdown, Some(an("T:2")));
    }

    #[test]
    fn builds_the_expected_wire() {
        let account = Account::new(IBAN, BIC).unwrap();
        for touchdown in [None, Some("T1")] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hksal(3, 7, &account, touchdown).unwrap(),
            )
            .unwrap();
            assert_eq!(to_wire(&by_iban(touchdown), 3, None).unwrap(), built);
        }
    }

    #[test]
    fn rules_are_enforced() {
        let e = from_wire::<HkSal7>(format!("HKSAL:3:7+{IBAN}:{BIC}+X'").as_bytes()).unwrap_err();
        assert_eq!((e.de, e.field.as_str()), (Some(3), "all_accounts"));
        let e = from_wire::<HkSal7>(b"HKSAL:3:7+:::::2800+N'").unwrap_err();
        assert_eq!((e.de, e.element), (Some(2), Some(5)));
        assert_eq!(e.field, "account.bank.country");
        let e = from_wire::<HkSal7>(b"HKSAL:3:7+DE00:ABCDEFGHIJKL+N'").unwrap_err();
        assert_eq!(e.field, "account.bic");
        assert_eq!(
            e.kind,
            crate::codec::ErrorKind::TooLong {
                max: 11,
                actual: 12
            }
        );
        assert_eq!(
            from_wire::<HkSal7>(b"HKSAL:3:7'").unwrap_err().field,
            "all_accounts"
        );
        let e = from_wire::<HkSal7>(b"HKSAL:3:6+x+N'").unwrap_err();
        assert!(matches!(
            e.kind,
            crate::codec::ErrorKind::WrongSegment { .. }
        ));
    }

    proptest! {
        #[test]
        fn arbitrary_requests_round_trip(
            iban in proptest::option::of("[A-Z]{2}[0-9]{2}[A-Z0-9]{1,30}"),
            bic in proptest::option::of("[A-Z0-9]{8,11}"),
            number in proptest::option::of("[0-9]{1,30}"),
            blz in proptest::option::of("[0-9]{8}"),
            all in any::<bool>(),
            max_entries in proptest::option::of(1u64..10_000),
            touchdown in proptest::option::of("[ -~]{1,35}"),
        ) {
            let touchdown = touchdown.and_then(|t| An::new(t).ok()).filter(|t| !t.as_str().is_empty());
            let v = HkSal7 {
                account: Kti1 {
                    iban: iban.map(|s| an(&s)),
                    bic: bic.map(|s| an(&s)),
                    account_number: number.map(|s| Id::new(s).unwrap()),
                    subaccount: None,
                    bank: blz.map(|b| Kik1 { country: Dig::new("280").unwrap(), bank_code: Some(an(&b)) }),
                },
                all_accounts: Jn(all),
                max_entries: max_entries.map(Num::new),
                touchdown,
            };
            let bytes = to_wire(&v, 3, None).unwrap();
            prop_assert_eq!(from_wire::<HkSal7>(&bytes).unwrap().1, v);
        }
    }

    fn by_national(touchdown: Option<&str>) -> HkSal6 {
        HkSal6 {
            account: Ktv3 {
                account_number: Id::new("9900012345").unwrap(),
                subaccount: None,
                bank: Kik1 {
                    country: Dig::new("280").unwrap(),
                    bank_code: Some(an("12030000")),
                },
            },
            all_accounts: Jn::NO,
            max_entries: None,
            touchdown: touchdown.map(an),
        }
    }

    #[test]
    fn v6_wire_round_trips() {
        use crate::codec::defs::testing::round_trip;
        let v: HkSal6 = round_trip(SAMPLES[0]);
        assert_eq!(v.account.account_number.as_str(), "1234567890");
        assert_eq!(v.all_accounts, Jn::NO);
        assert!(v.max_entries.is_none());
        assert!(v.touchdown.is_none());
        let v: HkSal6 = round_trip(SAMPLES[1]);
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");
        let v: HkSal6 = round_trip("HKSAL:3:6+1234567890::280:12345678+J+20'");
        assert_eq!(v.all_accounts, Jn::YES);
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert!(v.touchdown.is_none());
    }

    #[test]
    fn builds_the_expected_wire_v6() {
        let account = Account::new(IBAN, BIC).unwrap();
        for touchdown in [None, Some("T1")] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hksal(3, 6, &account, touchdown).unwrap(),
            )
            .unwrap();
            assert_eq!(to_wire(&by_national(touchdown), 3, None).unwrap(), built);
        }
    }

    #[test]
    fn v6_rules_are_enforced() {
        let e = from_wire::<HkSal6>(b"HKSAL:3:6+1234567890::280:12345678+N+12345'").unwrap_err();
        assert_eq!(e.field, "max_entries");
        assert_eq!(
            e.kind,
            crate::codec::ErrorKind::TooLong { max: 4, actual: 5 }
        );
    }

    fn by_national_v5(touchdown: Option<&str>) -> HkSal5 {
        HkSal5 {
            account: Ktv3 {
                account_number: Id::new("9900012345").unwrap(),
                subaccount: None,
                bank: Kik1 {
                    country: Dig::new("280").unwrap(),
                    bank_code: Some(an("12030000")),
                },
            },
            all_accounts: Jn::NO,
            max_entries: None,
            touchdown: touchdown.map(an),
        }
    }

    #[test]
    fn v5_wire_round_trips() {
        use crate::codec::defs::testing::round_trip;
        let v: HkSal5 = round_trip(SAMPLES[4]);
        assert_eq!(v.account.account_number.as_str(), "1234567890");
        assert_eq!(v.all_accounts, Jn::NO);
        assert!(v.max_entries.is_none());
        assert!(v.touchdown.is_none());
        let v: HkSal5 = round_trip(SAMPLES[5]);
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert_eq!(v.touchdown.as_ref().unwrap().as_str(), "T1");
        let v: HkSal5 = round_trip("HKSAL:3:5+1234567890::280:12345678+J+20'");
        assert_eq!(v.all_accounts, Jn::YES);
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert!(v.touchdown.is_none());
    }

    #[test]
    fn builds_the_expected_wire_v5() {
        let account = Account::new(IBAN, BIC).unwrap();
        for touchdown in [None, Some("T1")] {
            let built = crate::codec::wire::serialize_segment(
                &crate::segments::request::hksal(3, 5, &account, touchdown).unwrap(),
            )
            .unwrap();
            assert_eq!(to_wire(&by_national_v5(touchdown), 3, None).unwrap(), built);
        }
    }

    #[test]
    fn v5_rules_are_enforced() {
        let e = from_wire::<HkSal5>(b"HKSAL:3:5+1234567890::280:12345678+N+12345'").unwrap_err();
        assert_eq!(e.field, "max_entries");
        assert_eq!(
            e.kind,
            crate::codec::ErrorKind::TooLong { max: 4, actual: 5 }
        );
    }

    #[test]
    fn v8_wire_round_trips() {
        use crate::codec::defs::testing::round_trip;
        let v: HkSal8 = round_trip(SAMPLES[6]);
        assert_eq!(v.account.iban.unwrap().as_str(), "DE89370400440532013000");
        assert!(v.max_entries.is_none() && v.touchdown.is_none());
        let v: HkSal8 = round_trip(SAMPLES[7]);
        assert_eq!(v.max_entries, Some(Num::new(20)));
        assert_eq!(v.touchdown.unwrap().as_str(), "T1");
    }

    /// The protocol's HKSAL 8 and its expected wire string.
    #[test]
    fn builds_the_expected_wire_v8() {
        let account = Account::new(IBAN, BIC).unwrap();
        let wire = |td| {
            String::from_utf8(
                crate::codec::wire::serialize_segment(
                    &crate::segments::request::hksal(3, 8, &account, td).unwrap(),
                )
                .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(wire(None), format!("HKSAL:3:8+{IBAN}:{BIC}+N'"));
        assert_eq!(wire(Some("T1")), format!("HKSAL:3:8+{IBAN}:{BIC}+N++T1'"));
        let e = from_wire::<HkSal8>(b"HKSAL:3:8+DE89370400440532013000:COBADEFFXXX+N+12345'")
            .unwrap_err();
        assert_eq!(e.field, "max_entries");
    }
}
