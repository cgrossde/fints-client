//! MT940 and MT942 statements from [`Booking`]s (SWIFT MT940 as used in FinTS,
//! Messages B.8, with the German `:86:` structure: `?00` posting text,
//! `?20`–`?29` purpose, `?32`/`?33` other party).

use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::model::{Booking, Mt942Model};

/// Longest subfield of `:86:`.
const SUBFIELD: usize = 27;

fn amount(value: Decimal) -> String {
    // MT940 amounts: no sign, comma as decimal separator, always a comma.
    let s = format!("{:.2}", value.abs());
    s.replace('.', ",")
}

fn mark(value: Decimal) -> char {
    if value.is_sign_negative() && !value.is_zero() {
        'D'
    } else {
        'C'
    }
}

fn yymmdd(d: Date) -> String {
    d.strftime("%y%m%d").to_string()
}

fn chunks(text: &str) -> Vec<String> {
    text.chars()
        .collect::<Vec<_>>()
        .chunks(SUBFIELD)
        .map(|c| c.iter().collect())
        .collect()
}

/// Geschäftsvorfallcode: 166 for a credit, 177 for a debit (SEPA transfer).
fn gvc(b: &Booking) -> &'static str {
    if b.amount.is_sign_negative() {
        "177"
    } else {
        "166"
    }
}

fn details(b: &Booking) -> String {
    let mut out = format!("{}?00{}", gvc(b), b.posting_text);
    for (i, part) in chunks(&b.purpose).iter().take(10).enumerate() {
        out.push_str(&format!("?{}{part}", 20 + i));
    }
    if let Some(name) = &b.counterparty {
        for (i, part) in chunks(name).iter().take(2).enumerate() {
            out.push_str(&format!("?{}{part}", 32 + i));
        }
    }
    out
}

/// One statement: `account` is the `:25:` value (`BLZ/number`), `number`
/// the statement number, `opening` the balance before the first booking.
pub fn mt940(
    account: &str,
    number: usize,
    opening: Decimal,
    opening_date: Date,
    bookings: &[Booking],
    currency: &str,
) -> String {
    let mut lines = vec![
        ":20:STARTUMS".to_string(),
        format!(":25:{account}"),
        format!(":28C:{number}"),
        format!(
            ":60F:{}{}{currency}{}",
            mark(opening),
            yymmdd(opening_date),
            amount(opening)
        ),
    ];
    let mut closing = opening;
    let mut closing_date = opening_date;
    for b in bookings {
        closing += b.amount;
        closing_date = b.booking_date;
        lines.push(format!(
            ":61:{}{}{}{}NMSCNONREF",
            yymmdd(b.value_date),
            b.booking_date.strftime("%m%d"),
            mark(b.amount),
            amount(b.amount)
        ));
        lines.push(format!(":86:{}", details(b)));
    }
    lines.push(format!(
        ":62F:{}{}{currency}{}",
        mark(closing),
        yymmdd(closing_date),
        amount(closing)
    ));
    lines.push("-".into());
    lines.join("\r\n")
}

/// One MT942 statement (HBCI 2.2 IX.2.9) with `bookings` as pending
/// entries: one `:34F:` floor of zero for both sides, the creation time
/// (`:13D:` with the model's offset, `:13:` without), the bookings as in
/// [`mt940`], and `:90D:`/`:90C:` with their counts and sums (the debit
/// count one too high when [`Mt942Model::miscount`] is set, the debit sum
/// one cent too high when [`Mt942Model::missum`] is).
pub fn mt942(
    account: &str,
    number: usize,
    bookings: &[Booking],
    currency: &str,
    model: &Mt942Model,
) -> String {
    let stamp = model.created.strftime("%y%m%d%H%M");
    let mut lines = vec![
        ":20:STARTDISPE".to_string(),
        format!(":25:{account}"),
        format!(":28C:{number}"),
        format!(":34F:{currency}0,"),
        match model.offset {
            Some(offset) => format!(":13D:{stamp}{offset}"),
            None => format!(":13:{stamp}"),
        },
    ];
    let (mut debits, mut credits) = ((0usize, Decimal::ZERO), (0usize, Decimal::ZERO));
    for b in bookings {
        let side = if mark(b.amount) == 'D' {
            &mut debits
        } else {
            &mut credits
        };
        side.0 += 1;
        side.1 += b.amount.abs();
        lines.push(format!(
            ":61:{}{}{}{}NMSCNONREF",
            yymmdd(b.value_date),
            b.booking_date.strftime("%m%d"),
            mark(b.amount),
            amount(b.amount)
        ));
        lines.push(format!(":86:{}", details(b)));
    }
    if model.miscount {
        debits.0 += 1;
    }
    if model.missum {
        debits.1 += Decimal::new(1, 2);
    }
    lines.push(format!(":90D:{}{currency}{}", debits.0, amount(debits.1)));
    lines.push(format!(":90C:{}{currency}{}", credits.0, amount(credits.1)));
    lines.push("-".into());
    lines.join("\r\n")
}

/// One camt.052.001.02 document (ISO 20022 account report) with `bookings`
/// as booked entries: amount, sign, booking and value date, the bank's
/// reference at entry level (as DKB sends it), posting text, the other
/// party's name and the purpose as unstructured remittance.
pub fn camt052(iban: &str, bookings: &[Booking], currency: &str) -> String {
    fn xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:camt.052.001.02\">\
         <BkToCstmrAcctRpt><Rpt><Id>TESTRPT</Id>",
    );
    out.push_str(&format!(
        "<Acct><Id><IBAN>{}</IBAN></Id><Ccy>{currency}</Ccy></Acct>",
        xml(iban)
    ));
    for b in bookings {
        let (indicator, party) = if b.amount.is_sign_negative() {
            ("DBIT", "Cdtr")
        } else {
            ("CRDT", "Dbtr")
        };
        out.push_str(&format!(
            "<Ntry><Amt Ccy=\"{currency}\">{:.2}</Amt><CdtDbtInd>{indicator}</CdtDbtInd>\
             <Sts>BOOK</Sts><BookgDt><Dt>{}</Dt></BookgDt><ValDt><Dt>{}</Dt></ValDt>",
            b.amount.abs(),
            b.booking_date.strftime("%Y-%m-%d"),
            b.value_date.strftime("%Y-%m-%d"),
        ));
        if let Some(reference) = &b.reference {
            out.push_str(&format!("<AcctSvcrRef>{}</AcctSvcrRef>", xml(reference)));
        }
        out.push_str(&format!(
            "<AddtlNtryInf>{}</AddtlNtryInf><NtryDtls><TxDtls>",
            xml(&b.posting_text),
        ));
        if let Some(name) = &b.counterparty {
            out.push_str(&format!(
                "<RltdPties><{party}><Nm>{}</Nm></{party}></RltdPties>",
                xml(name)
            ));
        }
        out.push_str(&format!(
            "<RmtInf><Ustrd>{}</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>",
            xml(&b.purpose)
        ));
    }
    out.push_str("</Rpt></BkToCstmrAcctRpt></Document>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::date;

    #[test]
    fn writes_the_fields_by_hand() {
        let bookings = [
            Booking::new(date(2026, 1, 2), "-12.34", "SVWZ+Testrechnung 1")
                .posting_text("SEPA-UEBERWEISUNG")
                .counterparty("Test Empfaenger"),
            Booking::new(date(2026, 1, 5), "246.9", "SVWZ+Testgutschrift"),
        ];
        let text = mt940(
            "12345678/0001234567",
            1,
            "1000".parse().unwrap(),
            date(2026, 1, 1),
            &bookings,
            "EUR",
        );
        assert_eq!(
            text,
            ":20:STARTUMS\r\n\
             :25:12345678/0001234567\r\n\
             :28C:1\r\n\
             :60F:C260101EUR1000,00\r\n\
             :61:2601020102D12,34NMSCNONREF\r\n\
             :86:177?00SEPA-UEBERWEISUNG?20SVWZ+Testrechnung 1?32Test Empfaenger\r\n\
             :61:2601050105C246,90NMSCNONREF\r\n\
             :86:166?00GUTSCHRIFT?20SVWZ+Testgutschrift\r\n\
             :62F:C260105EUR1234,56\r\n\
             -"
        );
    }

    #[test]
    fn mt942_writes_the_fields_by_hand() {
        let bookings = [
            Booking::new(date(2026, 6, 15), "-7.5", "SVWZ+Karte"),
            Booking::new(date(2026, 6, 16), "20", "SVWZ+Eingang"),
        ];
        let model = Mt942Model::default();
        let text = mt942("12345678/0001234567", 2, &bookings, "EUR", &model);
        assert_eq!(
            text,
            ":20:STARTDISPE\r\n\
             :25:12345678/0001234567\r\n\
             :28C:2\r\n\
             :34F:EUR0,\r\n\
             :13D:2606151200+0200\r\n\
             :61:2606150615D7,50NMSCNONREF\r\n\
             :86:177?00UEBERWEISUNG?20SVWZ+Karte\r\n\
             :61:2606160616C20,00NMSCNONREF\r\n\
             :86:166?00GUTSCHRIFT?20SVWZ+Eingang\r\n\
             :90D:1EUR7,50\r\n\
             :90C:1EUR20,00\r\n\
             -"
        );
        let model = Mt942Model {
            offset: None,
            miscount: true,
            ..model
        };
        let text = mt942("12345678/0001234567", 1, &[], "EUR", &model);
        assert!(text.contains(":13:2606151200\r\n"), "{text}");
        assert!(text.contains(":90D:1EUR0,00\r\n:90C:0EUR0,00"), "{text}");
    }

    #[test]
    fn camt052_has_the_entries_by_hand() {
        let bookings = [Booking::new(date(2026, 1, 2), "-12.34", "Testrechnung 1")
            .posting_text("SEPA-UEBERWEISUNG")
            .counterparty("Test & Empfaenger")];
        assert_eq!(
            camt052("DE00123456780000000001", &bookings, "EUR"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <Document xmlns=\"urn:iso:std:iso:20022:tech:xsd:camt.052.001.02\">\
             <BkToCstmrAcctRpt><Rpt><Id>TESTRPT</Id>\
             <Acct><Id><IBAN>DE00123456780000000001</IBAN></Id><Ccy>EUR</Ccy></Acct>\
             <Ntry><Amt Ccy=\"EUR\">12.34</Amt><CdtDbtInd>DBIT</CdtDbtInd>\
             <Sts>BOOK</Sts><BookgDt><Dt>2026-01-02</Dt></BookgDt><ValDt><Dt>2026-01-02</Dt></ValDt>\
             <AddtlNtryInf>SEPA-UEBERWEISUNG</AddtlNtryInf><NtryDtls><TxDtls>\
             <RltdPties><Cdtr><Nm>Test &amp; Empfaenger</Nm></Cdtr></RltdPties>\
             <RmtInf><Ustrd>Testrechnung 1</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>\
             </Rpt></BkToCstmrAcctRpt></Document>"
        );
    }

    #[test]
    fn long_purpose_is_split_into_subfields() {
        let long = "A".repeat(30);
        let b = Booking::new(date(2026, 1, 2), "1", &long);
        assert!(details(&b).ends_with(&format!("?20{}?21AAA", "A".repeat(27))));
    }
}
