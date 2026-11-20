use super::*;

fn text(b: &[u8]) -> String {
    b.iter().copied().map(char::from).collect()
}

fn assert_hidden(out: &str, secrets: &[&str]) {
    for s in secrets {
        assert!(!out.contains(s), "{s:?} survives in:\n{out}");
    }
}

/// Synthetic MT940 (ISO 8859-1: ü and ß are single bytes).
const MT940: &[u8] = b"\r\n:20:STARTUMS\r\n\
:25:12345678/0001234567\r\n\
:28C:00001/001\r\n\
:60F:C260101EUR1234,56\r\n\
:61:2601020102DR12,50NDDTKD-4711//BANKREF-1\r\n\
/OCMT/EUR12,50/\r\n\
:86:105?00SEPA-BASISLASTSCHRIFT?10931?20EREF+E2E-1?21MREF+M-42 SVWZ+Beitrag M\xfcller\r\n\
?22Gro\xdf?30TESTDEFFXXX?31DE00999999990000000001?32Versicherung AG?34992\r\n\
:61:260103C1000,NTRFNONREF\r\n\
:86:166?00GUTSCHRIFT?20SVWZ+Gehalt Januar?32Arbeitgeber GmbH\r\n\
:62F:C260103EUR2222,06\r\n\
-";

#[test]
fn mt940_keeps_markup_and_codes_and_shapes_values() {
    let out = text(&shape(Format::Mt940, MT940, None));
    // An amount is `#,##`, a name (`?32`) and every run of text eight
    // characters; the `:86:` layout stays.
    assert_eq!(
        out,
        "\r\n:20:########\r\n\
:25:########/########\r\n\
:28C:00001/001\r\n\
:60F:C260101EUR#,##\r\n\
:61:2601020102DR#,##NDDT########//########\r\n\
########\r\n\
:86:105?00########?10########?20EREF+########?21MREF+########SVWZ+########\r\n\
?22########?30########?31########?32########?34992\r\n\
:61:260103C#,##NTRFNONREF\r\n\
:86:166?00########?20SVWZ+########?32########\r\n\
:62F:C260103EUR#,##\r\n\
-"
    );
    assert_hidden(
        &out,
        &[
            "STARTUMS",
            "1234567",
            "1234,56",
            "12,50",
            "4711",
            "BANKREF",
            "Beitrag",
            "ller",
            "Gro",
            "TESTDEFF",
            "99999",
            "Versicherung",
            "Gehalt",
            "Arbeitgeber",
            "LASTSCHRIFT",
        ],
    );
}

#[test]
fn mt940_rebuilt_from_the_shape_parses_into_the_same_bookings() {
    let parse = |d: &[u8]| {
        crate::formats::mt940::parse_mt940(d, crate::types::TransactionStatus::Booked)
            .expect("parses")
    };
    let original = parse(MT940);
    let masked = shape(Format::Mt940, MT940, None);
    let rebuilt = unshape(Format::Mt940, &masked);
    assert!(!rebuilt.contains(&b'#'), "{}", text(&rebuilt));
    let replayed = parse(&rebuilt);
    assert_eq!(replayed.len(), original.len());
    assert_eq!(original.len(), 2);
    assert_eq!(replayed[0].date, original[0].date);
    assert_eq!(replayed[0].amount.to_string(), "-1.11");
    assert_eq!(replayed[1].amount.to_string(), "1.11");
    assert_eq!(replayed[0].gvc.as_deref(), Some("105"));
}

/// Synthetic MT942 with both floors, a reversal and the totals.
const MT942: &[u8] = b":20:STARTDISPE\r\n\
:25:12345678/0001234567\r\n\
:28C:0\r\n\
:34F:EURD5,25\r\n\
:34F:EURC17,\r\n\
:13D:2601021230+0100\r\n\
:61:2601020102D5,25NDDTNONREF\r\n\
:86:105?00LASTSCHRIFT?20SVWZ+Karte\r\n\
:61:2601020102D987,60NMSCNONREF\r\n\
:61:2601020102C17,NTRFNONREF\r\n\
:61:2601020102RD3,40NDDTNONREF\r\n\
:90D:2EUR992,85\r\n\
:90C:2EUR20,40\r\n\
-";

#[test]
fn mt942_keeps_counts_currencies_and_time_and_shapes_amounts() {
    let out = text(&shape(Format::Mt940, MT942, None));
    for kept in [
        ":34F:EURD#,##\r\n",
        ":34F:EURC#,##\r\n",
        ":13D:2601021230+0100\r\n",
        ":90D:2EUR#,##\r\n",
        ":90C:2EUR#,##\r\n",
    ] {
        assert!(out.contains(kept), "{kept:?} in\n{out}");
    }
    assert_hidden(&out, &["992,85", "20,40", "5,25", "987,60", "Karte"]);
}

#[test]
fn mt942_rebuilt_from_the_shape_adds_up_again() {
    use crate::formats::mt940::parse_mt942;
    let original = parse_mt942(MT942).expect("parses");
    let rebuilt = unshape(Format::Mt940, &shape(Format::Mt940, MT942, None));
    let replayed = parse_mt942(&rebuilt).unwrap_or_else(|e| panic!("{e}\n{}", text(&rebuilt)));
    assert_eq!(replayed.transactions.len(), original.transactions.len());
    assert_eq!(replayed.created, original.created);
    let rebuilt = text(&rebuilt);
    // D 1,11 + D 1,11; C 1,11 + RD 1,11.
    assert!(rebuilt.contains(":90D:2EUR2,22\r\n"), "{rebuilt}");
    assert!(rebuilt.contains(":90C:2EUR2,22\r\n"), "{rebuilt}");
    assert!(rebuilt.contains(":34F:EURD1,11\r\n"), "{rebuilt}");

    // A total whose :20: is on another page is left alone.
    let tail = b":61:2601020102D1,11NMSCNONREF\r\n:90D:1EUR1,11\r\n-";
    assert_eq!(crate::formats::mt940::recompute_totals(tail), tail);
}

/// Synthetic camt.052 in UTF-8 (ü is two bytes).
const CAMT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.052.001.02">
 <BkToCstmrAcctRpt>
  <Rpt>
   <Id>RPT-0001</Id>
   <Acct><Id><IBAN>DE00120300009900012345</IBAN></Id><Ccy>EUR</Ccy></Acct>
   <Ntry>
    <Amt Ccy="EUR">1234.56</Amt>
    <CdtDbtInd>DBIT</CdtDbtInd>
    <Sts>BOOK</Sts>
    <BookgDt><Dt>2026-01-02</Dt></BookgDt>
    <ValDt><Dt>2026-01-02</Dt></ValDt>
    <BkTxCd><Prtry><Cd>NDDT+105+931</Cd><Issr>DK</Issr></Prtry></BkTxCd>
    <AddtlNtryInf>Lastschrift</AddtlNtryInf>
    <NtryDtls><TxDtls>
     <Refs><EndToEndId>E2E-1</EndToEndId></Refs>
     <RltdPties><Cdtr><Nm>Müller &amp; S&#246;hne</Nm></Cdtr>
      <CdtrAcct><Id><IBAN>DE00500000000000000001</IBAN></Id></CdtrAcct></RltdPties>
     <RmtInf><Ustrd>Beitrag Januar</Ustrd></RmtInf>
    </TxDtls></NtryDtls>
   </Ntry>
  </Rpt>
 </BkToCstmrAcctRpt>
</Document>
"#;

#[test]
fn camt_keeps_markup_and_codes_and_shapes_values() {
    let out = text(&shape(Format::Camt, CAMT.as_bytes(), None));
    for kept in [
        "<Amt Ccy=\"EUR\">#.##</Amt>",
        "<CdtDbtInd>DBIT</CdtDbtInd>",
        "<Sts>BOOK</Sts>",
        "<Dt>2026-01-02</Dt>",
        "<Cd>NDDT+105+931</Cd><Issr>########</Issr>",
        "<Ccy>EUR</Ccy>",
        "<Id>########</Id>",
        "<IBAN>########</IBAN>",
        "<Nm>########</Nm>",
        "<Ustrd>########</Ustrd>",
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Document xmlns=",
    ] {
        assert!(out.contains(kept), "{kept:?} not in:\n{out}");
    }
    assert_hidden(
        &out,
        &[
            "1234",
            "Lastschrift",
            "E2E",
            "Müller",
            "Beitrag",
            "Januar",
            "DE00",
            "&#246;",
            "&amp;",
        ],
    );
}

#[test]
fn camt_rebuilt_from_the_shape_parses_into_the_same_bookings() {
    let original = crate::camt::parse_camt052(CAMT.as_bytes(), crate::banks::BookingIdSource::None)
        .expect("parses");
    let rebuilt = unshape(Format::Camt, &shape(Format::Camt, CAMT.as_bytes(), None));
    assert!(!rebuilt.contains(&b'#'));
    let replayed = crate::camt::parse_camt052(&rebuilt, crate::banks::BookingIdSource::None)
        .expect("rebuilt parses");
    assert_eq!(replayed.len(), original.len());
    assert_eq!(replayed[0].amount.to_string(), "-1.11");
    assert_eq!(replayed[0].date, original[0].date);
}

/// Synthetic MT535 with one holding.
const MT535: &[u8] = b":16R:GENL\r\n\
:28E:1/ONLY\r\n\
:98A::STAT//20260315\r\n\
:97A::SAFE//12345678/0001234567\r\n\
:16S:GENL\r\n\
:16R:FIN\r\n\
:35B:ISIN DE0001234567\r\n\
/DE/123456\r\n\
BEISPIEL AG NAMENSAKTIEN\r\n\
:90B::MRKT//ACTU/EUR42,5\r\n\
:94B::PRIC//LMAR/XETR\r\n\
:98A::PRIC//20260315\r\n\
:93B::AGGR//UNIT/100,\r\n\
:19A::HOLD//NEUR4250,\r\n\
:70E::HOLD//1STK++++20200101+\r\n\
238,5+EUR\r\n\
:16S:FIN\r\n\
-";

#[test]
fn mt535_keeps_codes_and_dates_and_shapes_holdings() {
    let out = text(&shape(Format::Mt535, MT535, None));
    assert_eq!(
        out,
        ":16R:GENL\r\n\
:28E:1/ONLY\r\n\
:98A::STAT//20260315\r\n\
:97A::SAFE//########/########\r\n\
:16S:GENL\r\n\
:16R:FIN\r\n\
:35B:ISIN ########\r\n\
/DE/########\r\n\
########\r\n\
:90B::MRKT//ACTU/EUR#,##\r\n\
:94B::PRIC//LMAR/XETR\r\n\
:98A::PRIC//20260315\r\n\
:93B::AGGR//UNIT/#,##\r\n\
:19A::HOLD//NEUR#,##\r\n\
:70E::HOLD//########++++20200101+\r\n\
#,##+EUR\r\n\
:16S:FIN\r\n\
-"
    );
    assert_hidden(
        &out,
        &[
            "DE0001", "123456", "BEISPIEL", "42,5", "100,", "4250", "238",
        ],
    );
    let original = crate::segments::mt535::parse_mt535(MT535);
    let rebuilt = unshape(Format::Mt535, out.as_bytes());
    let replayed = crate::segments::mt535::parse_mt535(&rebuilt);
    assert_eq!(original.len(), 1);
    assert_eq!(replayed.len(), original.len());
}

#[test]
fn text_and_amount_shapes() {
    assert_eq!(shape_text("Spar-Giro Plus"), "########");
    assert_eq!(shape_text("Müßig\r\nA & B"), "########\r\n########");
    assert_eq!(shape_text("a\n\nb"), "########\n\n########");
    // A round sum and a large one look the same.
    assert_eq!(amount_shape("1234,56"), "#,##");
    assert_eq!(amount_shape("100,"), "#,##");
    assert_eq!(amount_shape("0,5"), "#,##");
    assert_eq!(amount_shape("#,"), "#,##");
    assert_eq!(amount_shape("1234.56"), "#.##");
    assert_eq!(amount_shape("-12.5"), "-#.##");
    assert_eq!(amount_shape("12"), "#");
    assert_eq!(name_shape(0, 27), "");
    assert_eq!(name_shape(3, 27), "########");
    assert_eq!(name_shape(27, 27), "########");
    assert_eq!(name_shape(30, 27), "#".repeat(28));
    assert_eq!(unshape_text("####,##"), "1111,11");
    assert_eq!(unshape_text("####,"), "1111,");
    assert_eq!(unshape_text("####-##"), "XXXX-XX");
    assert!(is_amount("1234,56") && !is_amount("1.234,56") && !is_amount(",5"));
    assert!(is_amount("#,##") && is_amount("#,"));
}

#[test]
fn never_panics_on_odd_input() {
    for format in [Format::Mt940, Format::Camt, Format::Mt535] {
        for data in [
            &b""[..],
            b":",
            b":61:",
            b":61:2601",
            b":61:260101X",
            b":86:123",
            b":86:123?",
            b":86:123?2",
            b"<",
            b"<a",
            b"<![CDATA[x",
            b"<!--",
            b"&#",
            b"<a>&#12",
            b":93B::",
            b":93B::A/",
            b"\xff\xfe:20:\xfc",
        ] {
            let masked = shape(format, data, None);
            let _ = unshape(format, &masked);
        }
    }
}

// ── No value shows ──────────────────────────────────────────────────────

fn mt940_with(name: &str, purpose: &str, amount: &str, balance: &str) -> Vec<u8> {
    format!(
        "\r\n:20:STARTUMS\r\n:25:12345678/0001234567\r\n:28C:1/1\r\n\
         :60F:C260101EUR{balance},56\r\n\
         :61:2601020102D{amount},50NDDTNONREF\r\n\
         :86:105?00LASTSCHRIFT?20SVWZ+{purpose}?32{name}\r\n\
         :62F:C260102EUR{balance},06\r\n-"
    )
    .into_bytes()
}

fn camt_with(name: &str, purpose: &str, amount: &str) -> Vec<u8> {
    format!(
        "<Document><BkToCstmrAcctRpt><Rpt><Ntry><Amt Ccy=\"EUR\">{amount}.50</Amt>\
         <CdtDbtInd>DBIT</CdtDbtInd><NtryDtls><TxDtls><RltdPties><Cdtr><Nm>{name}</Nm></Cdtr>\
         </RltdPties><RmtInf><Ustrd>{purpose}</Ustrd></RmtInf></TxDtls></NtryDtls></Ntry>\
         </Rpt></BkToCstmrAcctRpt></Document>"
    )
    .into_bytes()
}

fn mt535_with(name: &str, quantity: &str, value: &str) -> Vec<u8> {
    format!(
        ":16R:FIN\r\n:35B:ISIN DE0001234567\r\n{name}\r\n\
         :93B::AGGR//UNIT/{quantity},\r\n:19A::HOLD//EUR{value},25\r\n:16S:FIN\r\n-"
    )
    .into_bytes()
}

proptest::proptest! {
    /// Two statements of the same layout that differ in a name (its
    /// length and punctuation too), in the size of the amounts and in the
    /// characters and the length of a purpose have the same shape: none of
    /// these shows.
    #[test]
    fn statements_differing_only_in_values_have_the_same_shape(
        names in proptest::array::uniform2("[A-Za-z][A-Za-z .,()-]{0,25}[A-Za-z]"),
        purposes in proptest::array::uniform2("[A-Za-z0-9][A-Za-z0-9 .,/-]{0,26}"),
        amounts in proptest::array::uniform2("[1-9][0-9]{0,7}"),
        balances in proptest::array::uniform2("[1-9][0-9]{0,7}"),
    ) {
        let shaped = |n: usize| {
            (
                text(&shape(Format::Mt940, &mt940_with(&names[n], &purposes[n], &amounts[n], &balances[n]), None)),
                text(&shape(Format::Camt, &camt_with(&names[n], &purposes[n], &amounts[n]), None)),
                text(&shape(Format::Mt535, &mt535_with(&names[n], &amounts[n], &balances[n]), None)),
            )
        };
        proptest::prop_assert_eq!(shaped(0), shaped(1));
    }
}

// ── Balance marks, dates, stand-ins ─────────────────────────────────────

fn ids(days: i32) -> Identities {
    Identities::new(pseudonym::Salt::from_bytes([9; 32])).with_days(days)
}

#[test]
fn an_overdrawn_balance_reads_like_any_other() {
    let mt940 =
        b":20:S\r\n:60F:D260101EUR12,50\r\n:61:260102D5,NMSCNONREF\r\n:62F:D260102EUR17,50\r\n-";
    let out = text(&shape(Format::Mt940, mt940, None));
    assert!(out.contains(":60F:C260101EUR#,##"), "{out}");
    assert!(out.contains(":62F:C260102EUR#,##"), "{out}");
    // The mark of a booking stays.
    assert!(out.contains(":61:260102D#,##NMSC"), "{out}");

    let camt = b"<Rpt><Bal><Amt Ccy=\"EUR\">5.00</Amt><CdtDbtInd>DBIT</CdtDbtInd></Bal>\
<Ntry><Amt Ccy=\"EUR\">5</Amt><CdtDbtInd>DBIT</CdtDbtInd></Ntry></Rpt>";
    let out = text(&shape(Format::Camt, camt, None));
    assert!(
        out.contains("<Bal><Amt Ccy=\"EUR\">#.##</Amt><CdtDbtInd>CRDT</CdtDbtInd></Bal>"),
        "{out}"
    );
    // An amount without decimals has the same shape; the entry keeps its mark.
    assert!(
        out.contains("<Ntry><Amt Ccy=\"EUR\">#.##</Amt><CdtDbtInd>DBIT</CdtDbtInd></Ntry>"),
        "{out}"
    );
}

#[test]
fn every_date_of_a_statement_moves_by_the_days_of_the_trace() {
    let ids = ids(-5);
    let out = text(&shape(Format::Mt940, MT942, Some(&ids)));
    // The time of day and the UTC offset do not show.
    assert!(out.contains(":13D:2512281200+0000\r\n"), "{out}");
    // Value date 2 January 2026, entry date 0102: both five days earlier.
    assert!(out.contains(":61:2512281228D#,##NDDTNONREF"), "{out}");
    let out = text(&shape(Format::Mt940, MT940, Some(&ids)));
    assert!(out.contains(":60F:C251227EUR"), "{out}");
    assert!(out.contains(":62F:C251229EUR"), "{out}");

    let out = text(&shape(Format::Camt, CAMT.as_bytes(), Some(&ids)));
    assert_eq!(out.matches("<Dt>2025-12-28</Dt>").count(), 2, "{out}");
    assert!(!out.contains("2026-01-02"), "{out}");

    let out = text(&shape(Format::Mt535, MT535, Some(&ids)));
    assert!(out.contains(":98A::STAT//20260310"), "{out}");
    assert!(out.contains(":98A::PRIC//20260310"), "{out}");
    assert!(out.contains("++++20191227+"), "{out}");

    // Without a shift nothing moves.
    let out = text(&shape(
        Format::Camt,
        CAMT.as_bytes(),
        Some(&super::tests::ids(0)),
    ));
    assert!(out.contains("<Dt>2026-01-02</Dt>"), "{out}");
    // A time of day is noon, and a UTC offset zero, with or without a shift.
    let stamp = b"<GrpHdr><CreDtTm>2026-01-02T10:15:30.123+01:00</CreDtTm></GrpHdr>";
    let out = text(&shape(Format::Camt, stamp, Some(&super::tests::ids(0))));
    assert!(
        out.contains("<CreDtTm>2026-01-02T12:00:00.000+00:00</CreDtTm>"),
        "{out}"
    );
    let out = text(&shape(
        Format::Mt535,
        b":98C::PREP//20260315101530\r\n",
        Some(&super::tests::ids(-1)),
    ));
    assert_eq!(out, ":98C::PREP//20260314120000\r\n");
}

#[test]
fn stand_ins_of_other_people_have_one_form_whatever_the_value() {
    let ids = ids(0);
    let camt = |iban: &str, e2e: &str, mandate: &str, creditor: &str| {
        format!(
            "<Ntry><NtryDtls><TxDtls><Refs><EndToEndId>{e2e}</EndToEndId><MndtId>{mandate}</MndtId></Refs>\
<RltdPties><CdtrAcct><Id><IBAN>{iban}</IBAN></Id></CdtrAcct>\
<CdtrSchmeId><Id><PrvtId><Othr><Id>{creditor}</Id></Othr></PrvtId></Id></CdtrSchmeId></RltdPties>\
</TxDtls></NtryDtls></Ntry>"
        )
    };
    let shaped = |text_in: String| text(&shape(Format::Camt, text_in.as_bytes(), Some(&ids)));
    let field = |out: &str, tag: &str| {
        let open = format!("<{tag}>");
        let from = out.find(&open).unwrap() + open.len();
        out[from..from + out[from..].find('<').unwrap()].to_string()
    };
    // A German and a Luxembourg IBAN, a short and a long reference, two
    // creditor IDs of different countries and lengths.
    let a = shaped(camt(
        "DE00500000000000000001",
        "E2E-1",
        "M-1",
        "DE98ZZZ09999999999",
    ));
    let b = shaped(camt(
        "LU120010001234567891",
        "1234567890123 LONG.REF/2026",
        "MANDATE-0000000000042",
        "LU96ZZZ0000000000123456789",
    ));
    for out in [&a, &b] {
        let iban = field(out, "IBAN");
        assert!(
            iban.starts_with("DE") && iban.len() == 22 && &iban[4..12] == "00000000",
            "{iban}"
        );
        assert!(pseudonym::is_iban(iban.as_bytes()));
        for tag in ["EndToEndId", "MndtId"] {
            let v = field(out, tag);
            assert!(
                v.len() == 12
                    && v.bytes()
                        .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()),
                "{v}"
            );
        }
        let creditor = out[out.find("<Othr><Id>").unwrap() + 10..]
            .split('<')
            .next()
            .unwrap()
            .to_string();
        assert!(
            creditor.starts_with("DE") && creditor.len() == 18 && &creditor[4..7] == "ZZZ",
            "{creditor}"
        );
    }
    assert_ne!(field(&a, "IBAN"), field(&b, "IBAN"));
    assert_ne!(field(&a, "EndToEndId"), field(&b, "EndToEndId"));
    // The same value gets the same stand-in again.
    assert_eq!(
        a,
        shaped(camt(
            "DE00500000000000000001",
            "E2E-1",
            "M-1",
            "DE98ZZZ09999999999"
        ))
    );
    // "No reference" stays what it is.
    let none = shaped(camt(
        "DE00500000000000000001",
        "NOTPROVIDED",
        "M-1",
        "DE98ZZZ09999999999",
    ));
    assert_eq!(field(&none, "EndToEndId"), "NOTPROVIDED");
}

#[test]
fn a_shaped_statement_shaped_again_keeps_its_markup() {
    // What `Trace::remask` relies on: the shape of a shape has the same
    // fields, and a keyword after hidden text is still a keyword.
    for (format, data) in [
        (Format::Mt940, MT940),
        (Format::Mt940, MT942),
        (Format::Camt, CAMT.as_bytes()),
        (Format::Mt535, MT535),
    ] {
        let once = shape(format, data, None);
        let twice = shape(format, &once, None);
        assert_eq!(text(&twice), text(&once));
    }
    // A trace of the earlier rules: decimals as sent, text by length.
    let old = b":60F:D260101EUR#,\r\n:61:260102D#,#NMSCNONREF\r\n:86:166?00#####?20EREF+ABC#SVWZ+###########\r\n-";
    let new = text(&shape(Format::Mt940, old, None));
    assert_eq!(
        new,
        ":60F:C260101EUR#,##\r\n:61:260102D#,##NMSCNONREF\r\n:86:166?00########?20EREF+########SVWZ+########\r\n-"
    );
}
