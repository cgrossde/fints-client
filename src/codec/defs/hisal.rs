//! HISAL, Saldo (bank response to HKSAL). HBCI 2.2, VII.2.2 (v5);
//! Messages 2022, C.2.1.2.1 (v6), C.2.1.2.2 (v7) and C.2.1.2.3 (v8).

use super::common::{Kti1, Ktv3, Sdo2, Tsp1};
use crate::codec::{An, Btg, Cur, Dat, Segment, Tim};

/// HISAL version 5. HBCI 2.2, VII.2.2.
///
/// No Überziehung (added in v6). Booking date and time are separate DEs,
/// not a `tsp` group.
///
/// ```text
/// Nr. Name                            Typ  Format Länge Status Anzahl
///  1  Segmentkopf                     DEG               M     1
///  2  Kontoverbindung Auftraggeber    DEG  ktv     #    M     1
///  3  Kontoproduktbezeichnung         DE   an     ..30  M     1
///  4  Kontowährung                    DE   cur     #    M     1
///  5  Gebuchter Saldo                 DEG  sdo     #    M     1
///  6  Saldo der vorgemerkten Umsätze  DEG  sdo     #    K     1
///  7  Kreditlinie                     DEG  btg     #    K     1
///  8  Verfügbarer Betrag              DEG  btg     #    K     1
///  9  Bereits verfügter Betrag        DEG  btg     #    K     1
/// 10  Buchungsdatum des Saldos        DE   dat     #    K     1
/// 11  Buchungsuhrzeit des Saldos      DE   tim     #    K     1
/// 12  Fälligkeit                      DE   dat     #    K     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISAL", version = 5)]
pub struct HiSal5 {
    /// 2 Kontoverbindung Auftraggeber.
    pub account: Ktv3,
    /// 3 Kontoproduktbezeichnung.
    #[de(max = 30)]
    pub product_name: An,
    /// 4 Kontowährung.
    pub currency: Cur,
    /// 5 Gebuchter Saldo.
    pub booked: Sdo2,
    /// 6 Saldo der vorgemerkten Umsätze.
    pub pending: Option<Sdo2>,
    /// 7 Kreditlinie.
    pub credit_line: Option<Btg>,
    /// 8 Verfügbarer Betrag.
    pub available: Option<Btg>,
    /// 9 Bereits verfügter Betrag.
    pub used: Option<Btg>,
    /// 10 Buchungsdatum des Saldos.
    pub booking_date: Option<Dat>,
    /// 11 Buchungsuhrzeit des Saldos.
    pub booking_time: Option<Tim>,
    /// 12 Fälligkeit: only for credit cards.
    pub due_date: Option<Dat>,
}

/// HISAL version 6.
///
/// ```text
/// Nr. Name                            Typ  Format Länge Status Anzahl
///  1  Segmentkopf                     DEG               M     1
///  2  Kontoverbindung Auftraggeber    DEG  ktv     #    M     1
///  3  Kontoproduktbezeichnung         DE   an     ..30  M     1
///  4  Kontowährung                    DE   cur     #    M     1
///  5  Gebuchter Saldo                 DEG  sdo     #    M     1
///  6  Saldo der vorgemerkten Umsätze  DEG  sdo     #    O     1
///  7  Kreditlinie                     DEG  btg     #    O     1
///  8  Verfügbarer Betrag              DEG  btg     #    O     1
///  9  Bereits verfügter Betrag        DEG  btg     #    O     1
/// 10  Überziehung                     DEG  btg     #    O     1
/// 11  Buchungszeitpunkt               DEG  tsp     #    O     1
/// 12  Fälligkeit                      DE   dat     #    C     1   O for credit cards
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISAL", version = 6)]
pub struct HiSal6 {
    /// 2 Kontoverbindung Auftraggeber.
    pub account: Ktv3,
    /// 3 Kontoproduktbezeichnung.
    #[de(max = 30)]
    pub product_name: An,
    /// 4 Kontowährung.
    pub currency: Cur,
    /// 5 Gebuchter Saldo.
    pub booked: Sdo2,
    /// 6 Saldo der vorgemerkten Umsätze.
    pub pending: Option<Sdo2>,
    /// 7 Kreditlinie.
    pub credit_line: Option<Btg>,
    /// 8 Verfügbarer Betrag.
    pub available: Option<Btg>,
    /// 9 Bereits verfügter Betrag.
    pub used: Option<Btg>,
    /// 10 Überziehung.
    pub overdraft: Option<Btg>,
    /// 11 Buchungszeitpunkt.
    pub booking_timestamp: Option<Tsp1>,
    /// 12 Fälligkeit: C (O for credit cards).
    pub due_date: Option<Dat>,
}

/// HISAL version 7: [`HiSal6`] with row 2 as `kti` instead of `ktv`.
///
/// ```text
/// Nr. Name                            Typ  Format Länge Status Anzahl
///  1  Segmentkopf                     DEG               M     1
///  2  Kontoverbindung international   DEG  kti     #    M     1
///  3  Kontoproduktbezeichnung         DE   an     ..30  M     1
///  4  Kontowährung                    DE   cur     #    M     1
///  5  Gebuchter Saldo                 DEG  sdo     #    M     1
///  6  Saldo der vorgemerkten Umsätze  DEG  sdo     #    O     1
///  7  Kreditlinie                     DEG  btg     #    O     1
///  8  Verfügbarer Betrag              DEG  btg     #    O     1
///  9  Bereits verfügter Betrag        DEG  btg     #    O     1
/// 10  Überziehung                     DEG  btg     #    O     1
/// 11  Buchungszeitpunkt               DEG  tsp     #    O     1
/// 12  Fälligkeit                      DE   dat     #    C     1   O for credit cards
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISAL", version = 7)]
pub struct HiSal7 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 Kontoproduktbezeichnung.
    #[de(max = 30)]
    pub product_name: An,
    /// 4 Kontowährung.
    pub currency: Cur,
    /// 5 Gebuchter Saldo.
    pub booked: Sdo2,
    /// 6 Saldo der vorgemerkten Umsätze.
    pub pending: Option<Sdo2>,
    /// 7 Kreditlinie.
    pub credit_line: Option<Btg>,
    /// 8 Verfügbarer Betrag.
    pub available: Option<Btg>,
    /// 9 Bereits verfügter Betrag.
    pub used: Option<Btg>,
    /// 10 Überziehung.
    pub overdraft: Option<Btg>,
    /// 11 Buchungszeitpunkt.
    pub booking_timestamp: Option<Tsp1>,
    /// 12 Fälligkeit: C (O for credit cards).
    pub due_date: Option<Dat>,
}

/// HISAL version 8. Messages 2022, C.2.1.2.3: [`HiSal7`] plus row 13.
///
/// ```text
/// Nr. Name                            Typ  Format Länge Status Anzahl
///  1  Segmentkopf                     DEG               M     1
///  2  Kontoverbindung international   DEG  kti     #    M     1
///  3  Kontoproduktbezeichnung         DE   an     ..30  M     1
///  4  Kontowährung                    DE   cur     #    M     1
///  5  Gebuchter Saldo                 DEG  sdo     #    M     1
///  6  Saldo der vorgemerkten Umsätze  DEG  sdo     #    O     1
///  7  Kreditlinie                     DEG  btg     #    O     1
///  8  Verfügbarer Betrag              DEG  btg     #    O     1
///  9  Bereits verfügter Betrag        DEG  btg     #    O     1
/// 10  Überziehung                     DEG  btg     #    O     1
/// 11  Buchungszeitpunkt               DEG  tsp     #    O     1
/// 12  Fälligkeit                      DE   dat     #    C     1   O for credit cards, N otherwise
/// 13  Ab Monatswechsel pfändbar       DEG  btg     #    O     1
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HISAL", version = 8)]
pub struct HiSal8 {
    /// 2 Kontoverbindung international.
    pub account: Kti1,
    /// 3 Kontoproduktbezeichnung.
    #[de(max = 30)]
    pub product_name: An,
    /// 4 Kontowährung.
    pub currency: Cur,
    /// 5 Gebuchter Saldo.
    pub booked: Sdo2,
    /// 6 Saldo der vorgemerkten Umsätze.
    pub pending: Option<Sdo2>,
    /// 7 Kreditlinie.
    pub credit_line: Option<Btg>,
    /// 8 Verfügbarer Betrag.
    pub available: Option<Btg>,
    /// 9 Bereits verfügter Betrag.
    pub used: Option<Btg>,
    /// 10 Überziehung.
    pub overdraft: Option<Btg>,
    /// 11 Buchungszeitpunkt.
    pub booking_timestamp: Option<Tsp1>,
    /// 12 Fälligkeit: C (O for credit cards).
    pub due_date: Option<Dat>,
    /// 13 Ab Monatswechsel pfändbar: the amount that becomes seizable
    /// at the turn of the month.
    pub seizable_from_month_change: Option<Btg>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HISAL:4:6:3+1234567890::280:12345678+Girokonto+EUR+C:1234,56:EUR:20260315'",
    "HISAL:4:6:3+1234567890::280:12345678+Girokonto+EUR+C:1234,56:EUR:20260315+D:10,5:EUR:20260315:120000++800,:EUR'",
    "HISAL:4:7:3+DE89370400440532013000:COBADEFFXXX+Girokonto+EUR+C:1234,56:EUR:20260315'",
    "HISAL:4:7:3+DE89370400440532013000:COBADEFFXXX+Girokonto+EUR+C:1234,56:EUR:20260315+D:10,5:EUR:20260315++800,:EUR+200,:EUR+50,:EUR+20260315:143000+20261231'",
    "HISAL:4:5:3+1234567890::280:12345678+Girokonto+EUR+C:1234,56:EUR:20260315'",
    "HISAL:4:5:3+1234567890::280:12345678+Girokonto+EUR+C:1234,56:EUR:20260315+D:10,5:EUR:20260315:120000++800,:EUR+200,:EUR+20260315+143000+20261231'",
    "HISAL:4:8:3+DE89370400440532013000:COBADEFFXXX+Girokonto+EUR+C:1234,56:EUR:20260315'",
    "HISAL:4:8:3+DE89370400440532013000:COBADEFFXXX+Girokonto+EUR+C:1234,56:EUR:20260315+D:10,5:EUR:20260315++800,:EUR+++20260315:143000++300,:EUR'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::{ErrorKind, Wrt, from_wire};
    use jiff::civil::{Date, Time};
    use rust_decimal::Decimal;

    #[test]
    fn wire_round_trips() {
        let v: HiSal6 = round_trip(SAMPLES[0]);
        assert_eq!(v.account.account_number.as_str(), "1234567890");
        assert_eq!(
            v.account.bank.bank_code.as_ref().unwrap().as_str(),
            "12345678"
        );
        assert_eq!(v.product_name.as_str(), "Girokonto");
        assert_eq!(v.currency.as_str(), "EUR");
        assert_eq!(v.booked.debit_credit.as_str(), "C");
        assert_eq!(
            v.booked.amount.value,
            Wrt::new(Decimal::new(123456, 2)).unwrap()
        );
        assert_eq!(
            v.booked.date,
            Some(Dat::new(Date::new(2026, 3, 15).unwrap()).unwrap())
        );
        assert!(v.booked.time.is_none());
        assert!(v.pending.is_none());
        assert!(v.available.is_none());
        assert!(v.due_date.is_none());

        let v: HiSal6 = round_trip(SAMPLES[1]);
        let pending = v.pending.as_ref().unwrap();
        assert_eq!(pending.debit_credit.as_str(), "D");
        assert_eq!(
            pending.amount.value,
            Wrt::new(Decimal::new(105, 1)).unwrap()
        );
        assert!(pending.time.is_some());
        assert!(v.credit_line.is_none());
        assert_eq!(
            v.available.as_ref().unwrap().value,
            Wrt::new(Decimal::new(800, 0)).unwrap()
        );
        assert!(v.used.is_none());

        let v: HiSal7 = round_trip(SAMPLES[2]);
        assert_eq!(
            v.account.iban.as_ref().unwrap().as_str(),
            "DE89370400440532013000"
        );
        assert_eq!(v.account.bic.as_ref().unwrap().as_str(), "COBADEFFXXX");
        assert!(v.pending.is_none());
        assert!(v.available.is_none());

        let v: HiSal7 = round_trip(SAMPLES[3]);
        assert!(v.pending.is_some());
        assert!(v.credit_line.is_none());
        assert_eq!(
            v.available.as_ref().unwrap().value,
            Wrt::new(Decimal::new(800, 0)).unwrap()
        );
        assert_eq!(
            v.used.as_ref().unwrap().value,
            Wrt::new(Decimal::new(200, 0)).unwrap()
        );
        assert_eq!(
            v.overdraft.as_ref().unwrap().value,
            Wrt::new(Decimal::new(50, 0)).unwrap()
        );
        assert!(v.booking_timestamp.is_some());
        assert_eq!(
            v.due_date.unwrap(),
            Dat::new(Date::new(2026, 12, 31).unwrap()).unwrap()
        );

        let v: HiSal5 = round_trip(SAMPLES[4]);
        assert_eq!(v.account.account_number.as_str(), "1234567890");
        assert_eq!(v.product_name.as_str(), "Girokonto");
        assert!(v.pending.is_none());
        assert!(v.booking_date.is_none());
        assert!(v.booking_time.is_none());
        assert!(v.due_date.is_none());

        let v: HiSal5 = round_trip(SAMPLES[5]);
        assert!(v.pending.is_some());
        assert!(v.credit_line.is_none());
        assert_eq!(
            v.available.as_ref().unwrap().value,
            Wrt::new(Decimal::new(800, 0)).unwrap()
        );
        assert_eq!(
            v.used.as_ref().unwrap().value,
            Wrt::new(Decimal::new(200, 0)).unwrap()
        );
        assert_eq!(
            v.booking_date.unwrap(),
            Dat::new(Date::new(2026, 3, 15).unwrap()).unwrap()
        );
        assert_eq!(
            v.booking_time.unwrap(),
            Tim::new(Time::constant(14, 30, 0, 0)).unwrap()
        );
        assert_eq!(
            v.due_date.unwrap(),
            Dat::new(Date::new(2026, 12, 31).unwrap()).unwrap()
        );
    }

    /// A booked balance without date, without currency, or without both
    /// (specification: mandatory) is read; the missing parts are `None`.
    #[test]
    fn booked_balance_may_lack_date_and_currency() {
        for (wire, currency, date) in [
            ("C:1234,56:EUR", Some("EUR"), false),
            ("C:1234,56::20260315", None, true),
            ("C:1234,56", None, false),
        ] {
            let full = format!("HISAL:4:6:3+1234567890::280:12345678+Girokonto+EUR+{wire}'");
            let v: HiSal6 = round_trip(&full);
            assert_eq!(v.currency.as_str(), "EUR");
            assert_eq!(
                v.booked.amount.value,
                Wrt::new(Decimal::new(123456, 2)).unwrap()
            );
            assert_eq!(
                v.booked.amount.currency.as_ref().map(|c| c.as_str()),
                currency
            );
            assert_eq!(v.booked.date.is_some(), date, "{wire}");
        }
        let v: HiSal5 = round_trip("HISAL:4:5:3+1234567890::280:12345678+Girokonto+EUR+D:5,:EUR'");
        assert!(v.booked.date.is_none());
    }

    #[test]
    fn rules_are_enforced() {
        let name = "X".repeat(31);
        let wire =
            format!("HISAL:4:6:3+1234567890::280:12345678+{name}+EUR+C:1234,56:EUR:20260315'");
        let e = from_wire::<HiSal6>(wire.as_bytes()).unwrap_err();
        assert_eq!(e.field, "product_name");
        assert_eq!(
            e.kind,
            ErrorKind::TooLong {
                max: 30,
                actual: 31
            }
        );
        let wire =
            format!("HISAL:4:5:3+1234567890::280:12345678+{name}+EUR+C:1234,56:EUR:20260315'");
        let e = from_wire::<HiSal5>(wire.as_bytes()).unwrap_err();
        assert_eq!(e.field, "product_name");
        assert_eq!(
            e.kind,
            ErrorKind::TooLong {
                max: 30,
                actual: 31
            }
        );
    }

    #[test]
    fn v8_wire_round_trips() {
        let v: HiSal8 = round_trip(SAMPLES[6]);
        assert_eq!(v.currency.as_str(), "EUR");
        assert!(v.seizable_from_month_change.is_none());
        let v: HiSal8 = round_trip(SAMPLES[7]);
        assert!(v.pending.is_some() && v.credit_line.is_none());
        assert!(v.booking_timestamp.is_some() && v.due_date.is_none());
        assert_eq!(
            v.seizable_from_month_change.unwrap().value,
            Wrt::new(Decimal::new(300, 0)).unwrap()
        );
        let e =
            from_wire::<HiSal8>(b"HISAL:4:8:3+DE89370400440532013000:COBADEFFXXX+Girokonto+EUR'")
                .unwrap_err();
        assert_eq!(
            (e.field.as_str(), e.kind),
            ("booked.debit_credit", ErrorKind::Missing)
        );
    }
}
