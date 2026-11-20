//! HIUPD, Kontoinformation (account information of the user parameters).
//! HBCI 2.2, V.3 (version 4); Formals 2017, E.3 (version 6).
//!
//! Version 5 is not defined: no published document available to us has its
//! table (Formals 2017 shows only examples, HBCI 2.2 ends at version 4). It
//! stays on the hand-written path (`segments::response::parse_hiupd`).

use super::common::Ktv3;
use crate::codec::{An, Btg, Code, Cur, DataElementGroup, Id, Num, Segment};

/// HIUPD version 4. HBCI 2.2, V.3.
///
/// ```text
/// Nr. Name                       Typ  Format Länge  Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Kontoverbindung            DEG  ktv    #      M     1
///  3  Kunden-ID                  DE   id     #      M     1
///  4  Kontowährung               DE   cur    #      K     1
///  5  Name 1                     DE   an     ..27   M     1
///  6  Name 2                     DE   an     ..27   K     1
///  7  Kontoproduktbezeichnung    DE   an     ..30   K     1
///  8  Kontolimit                 DEG               K     1
///  9  Erlaubte Geschäftsvorfälle DEG               K     999
/// ```
///
/// HBCI 2.2's `ktv` is printed as four DEs (number, subaccount, country,
/// bank code), the flattened form of [`Ktv3`] (as in HKSAL 5). The two
/// groups print the rows of [`AccountLimit2`] and [`AllowedOrder2`]; in
/// HBCI 2.2 the Kontolimit's Limitbetrag is M (the FinTS 3.0 version makes
/// it conditional), which this definition does not enforce.
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIUPD", version = 4)]
pub struct HiUpd4 {
    /// 2 Kontoverbindung.
    pub account: Ktv3,
    /// 3 Kunden-ID.
    pub customer_id: Id,
    /// 4 Kontowährung (may be left out for depots).
    pub currency: Option<Cur>,
    /// 5 Name 1.
    #[de(max = 27)]
    pub holder_name_1: An,
    /// 6 Name 2.
    #[de(max = 27)]
    pub holder_name_2: Option<An>,
    /// 7 Kontoproduktbezeichnung.
    #[de(max = 30)]
    pub product_name: Option<An>,
    /// 8 Kontolimit.
    pub limit: Option<AccountLimit2>,
    /// 9 Erlaubte Geschäftsvorfälle, up to 999.
    #[de(max_count = 999)]
    pub allowed_orders: Vec<AllowedOrder2>,
}

/// HIUPD version 6: one segment per account the user may use.
///
/// ```text
/// Nr. Name                       Typ  Format Länge  Status Anzahl
///  1  Segmentkopf                DEG               M     1
///  2  Kontoverbindung            DEG  ktv    #      C     1    N: orders without account, M: else
///  3  IBAN                       DE   an     ..34   O     1
///  4  Kunden-ID                  DE   id     #      M     1
///  5  Kontoart                   DE   num    ..2    O     1
///  6  Kontowährung               DE   cur    #      O     1
///  7  Name des Kontoinhabers 1   DE   an     ..27*  M     1
///  8  Name des Kontoinhabers 2   DE   an     ..27*  O     1
///
/// * The table prints ..27, but the Belegungsrichtlinie "Name Kontoinhaber 1
///   und 2" in the same section (Formals E.3) allows up to 35; we accept 35.
///  9  Kontoproduktbezeichnung    DE   an     ..30   O     1
/// 10  Kontolimit                 DEG               O     1
/// 11  Erlaubte Geschäftsvorfälle DEG               O     999
/// 12  Erweiterung, kontobezogen  DE   an     ..2048 O     1
/// ```
///
/// Where the spec allows more than the table (Belegungsrichtlinien): the
/// IBAN may arrive with 35 characters (the 35th may be cut) and the two
/// names with up to 35 (the excess may be cut). The codec cannot cut, so
/// such a segment stays raw with a `TooLong` error.
///
/// Position of element 12: element 11 is a repetition with a bound of 999,
/// so by Formals H.1.2 element 12 sits after 999 positions of element 11.
/// A bank that sends the extension directly after its last allowed
/// business transaction is not read by this definition (it stays raw with
/// an error at element 11).
#[derive(Debug, Clone, PartialEq, Eq, Segment)]
#[segment(code = "HIUPD", version = 6)]
pub struct HiUpd6 {
    /// 2 Kontoverbindung.
    pub account: Option<Ktv3>,
    /// 3 IBAN.
    #[de(max = 34)]
    pub iban: Option<An>,
    /// 4 Kunden-ID.
    pub customer_id: Id,
    /// 5 Kontoart: the account type code (the document lists no values here).
    #[de(max = 2)]
    pub account_type: Option<Num>,
    /// 6 Kontowährung.
    pub currency: Option<Cur>,
    /// 7 Name des Kontoinhabers 1: up to 35 per the Belegungsrichtlinie.
    #[de(max = 35)]
    pub holder_name_1: An,
    /// 8 Name des Kontoinhabers 2: up to 35 per the Belegungsrichtlinie.
    #[de(max = 35)]
    pub holder_name_2: Option<An>,
    /// 9 Kontoproduktbezeichnung.
    #[de(max = 30)]
    pub product_name: Option<An>,
    /// 10 Kontolimit: not together with order-specific limits.
    pub limit: Option<AccountLimit2>,
    /// 11 Erlaubte Geschäftsvorfälle, up to 999.
    #[de(max_count = 999)]
    pub allowed_orders: Vec<AllowedOrder2>,
    /// 12 Erweiterung, kontobezogen: JSON (Formals E.3.1).
    #[de(max = 2048)]
    pub extension: Option<An>,
}

/// Kontolimit, version 2. Formals 2017, data dictionary "Kontolimit".
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct AccountLimit2 {
    /// 1 Limitart, `code 1`, M: E single order, T day, W week, M month,
    /// Z time limit.
    #[de(len = 1)]
    pub kind: Code,
    /// 2 Limitbetrag, `btg`, C: O unless the kind is `Z`, N for `Z`.
    pub amount: Option<Btg>,
    /// 3 Limit-Tage, `num ..3`, C: O for kind `Z`, N otherwise; > 0.
    #[de(max = 3)]
    pub days: Option<Num>,
}

/// Erlaubte Geschäftsvorfälle, version 2. Formals 2017, data dictionary
/// "Erlaubte Geschäftsvorfälle".
#[derive(Debug, Clone, PartialEq, Eq, DataElementGroup)]
pub struct AllowedOrder2 {
    /// 1 Geschäftsvorfall, `an ..6`, M: the order's segment code.
    #[de(max = 6)]
    pub segment_code: An,
    /// 2 Anzahl benötigter Signaturen, `num ..2`, M: 0, 1, 2, 3.
    #[de(max = 2)]
    pub signatures_required: Num,
    /// 3 Limitart, `code 1`, O: E, T, W, M, Z.
    #[de(len = 1)]
    pub limit_kind: Option<Code>,
    /// 4 Limitbetrag, `btg`, C: O unless the limit kind is `Z`, N otherwise.
    pub limit_amount: Option<Btg>,
    /// 5 Limit-Tage, `num ..3`, C: O for limit kind `Z`, N otherwise; > 0.
    #[de(max = 3)]
    pub limit_days: Option<Num>,
}

#[cfg(test)]
pub(crate) const SAMPLES: &[&str] = &[
    "HIUPD:58:6:4+1234567890::280:12345678+DE89370400440532013000+CUST0001+1+EUR+Erika Beispiel++Girokonto++HKSAL:1+HKKAZ:1+HKCCS:1:T:2500,:EUR'",
    "HIUPD:59:6:4+9876543210::280:12345678++CUST0001+30+EUR+Erika Beispiel++Depot+Z:500,:EUR:7+HKWPD:1'",
    "HIUPD:15:4:7+1234567::280:10020030+12345+EUR+Erika Beispiel++Giro Spezial+T:2000,:EUR+HKPRO:1+HKSAL:1'",
    "HIUPD:16:4:7+1234568::280:10020030+12345+EUR+Erika Beispiel++Sparkonto++HKUEB:2:Z:1000,:EUR:7+HKSAL:2'",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::defs::testing::round_trip;
    use crate::codec::from_wire;

    #[test]
    fn wire_round_trips() {
        let v: HiUpd6 = round_trip(SAMPLES[0]);
        assert_eq!(
            v.account.as_ref().unwrap().account_number.as_str(),
            "1234567890"
        );
        assert_eq!(v.iban.as_ref().unwrap().as_str(), "DE89370400440532013000");
        assert_eq!(v.currency.as_ref().unwrap().as_str(), "EUR");
        assert!(v.limit.is_none());
        assert_eq!(v.allowed_orders.len(), 3);
        let ccs = &v.allowed_orders[2];
        assert_eq!(ccs.limit_kind.as_ref().unwrap().as_str(), "T");
        assert_eq!(ccs.limit_amount.as_ref().unwrap().currency.as_str(), "EUR");
        let v: HiUpd6 = round_trip(SAMPLES[1]);
        assert_eq!(v.limit.as_ref().unwrap().kind.as_str(), "Z");
        assert_eq!(v.account_type, Some(Num::new(30)));
    }

    #[test]
    fn segment_without_account_round_trips() {
        let v: HiUpd6 = round_trip("HIUPD:60:6:4+++CUST0001+++Erika Beispiel'");
        assert!(v.account.is_none());
        assert!(v.allowed_orders.is_empty());
    }

    /// The strings of the hand-written parser's tests read with the
    /// definition (the one with a non-numeric account type does not).
    #[test]
    fn reads_the_hand_written_parsers_fixtures() {
        for wire in [
            "HIUPD:58:6:4+1::280:12345678+DE111234567800000001+test1++EUR+Fullname++Girokonto++HKSAL:1+HKKAZ:1'",
            "HIUPD:59:6:4+9876543::280:12345678++test1+30+EUR+Fullname++Depot++HKWPD:1'",
            "HIUPD:58:6:4+1::280:12345678+DE111234567800000001+test1+1+EUR+Fullname++Girokonto++HKSAL:1'",
            "HIUPD:60:6:4+4111::280:12345678++test1+50+EUR+Fullname++Kreditkarte++HKSAL:1'",
        ] {
            let v: HiUpd6 = round_trip(wire);
            assert_eq!(v.customer_id.as_str(), "test1");
        }
        let e = from_wire::<HiUpd6>(b"HIUPD:61:6:4+2::280:12345678++test1+abc+EUR'").unwrap_err();
        assert_eq!(e.field, "account_type");
    }

    /// Formals E.3, Belegungsrichtlinie "Name Kontoinhaber 1 und 2": the
    /// table prints ..27, the rule allows 35.
    #[test]
    fn holder_names_up_to_35_characters_are_accepted() {
        let name35 = "Erika Beispiel-Musterfrau Testkonto";
        assert_eq!(name35.len(), 35);
        let wire = format!("HIUPD:58:6:4+1::280:12345678++C+1+EUR+{name35}+{name35}'");
        let v: HiUpd6 = round_trip(&wire);
        assert_eq!(v.holder_name_1.as_str(), name35);
        assert_eq!(v.holder_name_2.as_ref().unwrap().as_str(), name35);

        let wire = format!("HIUPD:58:6:4+1::280:12345678++C+1+EUR+{name35}X'");
        let e = from_wire::<HiUpd6>(wire.as_bytes()).unwrap_err();
        assert_eq!(e.field, "holder_name_1");
    }

    #[test]
    fn spec_leniencies_the_codec_cannot_express_stay_raw() {
        // Belegungsrichtlinie: an IBAN of 35 characters may be cut to 34.
        let iban35 = format!("{}0", "DE89370400440532013000");
        let long = iban35.repeat(2)[..35].to_string();
        let wire = format!("HIUPD:58:6:4+1::280:12345678+{long}+C+1+EUR+Name'");
        let e = from_wire::<HiUpd6>(wire.as_bytes()).unwrap_err();
        assert_eq!(e.field, "iban");
    }

    #[test]
    fn version_4_round_trips() {
        let v: HiUpd4 = round_trip(SAMPLES[2]);
        assert_eq!(v.account.account_number.as_str(), "1234567");
        assert_eq!(v.account.bank.bank_code.unwrap().as_str(), "10020030");
        assert_eq!(v.customer_id.as_str(), "12345");
        assert_eq!(v.holder_name_1.as_str(), "Erika Beispiel");
        assert_eq!(v.product_name.unwrap().as_str(), "Giro Spezial");
        assert_eq!(v.limit.unwrap().kind.as_str(), "T");
        assert_eq!(v.allowed_orders.len(), 2);
        let v: HiUpd4 = round_trip(SAMPLES[3]);
        assert!(v.limit.is_none());
        assert_eq!(v.allowed_orders[0].limit_days, Some(Num::new(7)));
        // Name 1 is at most 27 in this version.
        let long = SAMPLES[3].replace("Erika Beispiel", &"x".repeat(28));
        assert_eq!(
            from_wire::<HiUpd4>(long.as_bytes()).unwrap_err().field,
            "holder_name_1"
        );
    }
}
