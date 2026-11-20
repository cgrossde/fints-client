use super::*;
use proptest::prelude::*;

const IBAN: &str = "DE89370400440532013000";
const USER: &str = "kunde4711";
const SYS: &str = "SYSID99887766";
const NUMBER: &str = "0532013000";

/// Wraps clear-text segments in an HNVSD like a real PIN/TAN message.
fn envelope(inner: &str) -> String {
    format!(
        "HNHBK:1:3+000000000123+300+0+1'HNVSK:998:3+PIN:2+998+1+1::{SYS}+1:20260101:120000+2:2:13:@8@00000000:5:1+280:37040044:{USER}:V:0:0+0'HNVSD:999:1+@{}@{inner}'HNHBS:5:1+1'",
        inner.len()
    )
}

fn masked(msg: &str) -> String {
    mask_for_log(msg.as_bytes())
}

fn assert_hidden(out: &str, secrets: &[&str]) {
    for s in secrets {
        assert!(!out.contains(s), "{s} visible in: {out}");
    }
}

#[test]
fn masked_text_keeps_only_its_length() {
    let out = masked("HKIDN:3:2+280:37040044+kunde4711+SYSID99887766+1'");
    assert_eq!(out, "HKIDN:3:2+280:37040044+*********+*************+1'");
}

#[test]
fn hnsha_hides_pin_and_tan() {
    let out = masked("HNSHA:7:2+4711++geheim?+pin:123456'");
    // Eight stars each, whatever the length of the PIN and the TAN.
    assert_eq!(out, "HNSHA:7:2+4711++********:********'");
    assert_hidden(&out, &["geheim", "123456"]);
}

#[test]
fn envelope_segments_hide_user_and_system_id() {
    let out = masked(&envelope(&format!(
        "HNSHK:2:4+PIN:2+921+4711+1+1+1::{SYS}+1+1:20260101:120000+1:999:1+6:10:16+280:37040044:{USER}:S:0:0'HNSHA:4:2+4711++pin'"
    )));
    assert_hidden(&out, &[USER, SYS, "pin'"]);
    assert!(out.contains("HNSHK:2:4"), "{out}");
    assert!(out.contains("280:37040044"), "bank code is kept: {out}");
    assert!(out.contains("HNVSD:999:1+@"), "{out}");
}

#[test]
fn hiupa_hiupd_hisyn_hide_identifiers() {
    let out = masked(&envelope(&format!(
        "HIUPA:4:4:3+{USER}+3+0+Erika Muster'\
         HIUPD:5:6:3+{NUMBER}::280:37040044+{IBAN}+{USER}+1+EUR+Erika Muster+Max Muster+Girokonto+HKSAK:1'\
         HISYN:6:4:3+{SYS}'"
    )));
    assert_hidden(
        &out,
        &[USER, SYS, IBAN, NUMBER, "Muster", "Erika", "Girokonto"],
    );
    assert!(out.contains("HKSAK:1"), "allowed orders are kept: {out}");
    assert!(out.contains("::280:37040044"), "bank code is kept: {out}");
}

#[test]
fn hiupd_of_another_version_is_masked_completely() {
    let out = masked(&format!(
        "HIUPD:5:4:3+{NUMBER}:280:37040044+{USER}+1+EUR+Muster'"
    ));
    assert_hidden(&out, &[NUMBER, USER, "Muster", "37040044"]);
}

#[test]
fn balance_orders_hide_the_account_number_and_iban() {
    let out = masked(&format!(
        "HKSAL:3:5+{NUMBER}::280:37040044+N'\
         HKSAL:4:7+{IBAN}:COBADEFFXXX:::+N'\
         HISAL:5:6:3+{NUMBER}:01:280:37040044+Giro+EUR+C:1,23:EUR:20260101'\
         HISAL:6:7:3+{IBAN}:COBADEFFXXX:{NUMBER}::280:37040044+Giro+EUR+C:1,23:EUR:20260101'"
    ));
    assert_hidden(&out, &[NUMBER, IBAN]);
    assert!(out.contains("COBADEFFXXX"), "BIC is kept: {out}");
    assert!(
        out.contains("+########+EUR+C:#,##:EUR:20260101'"),
        "product name and balance are fixed shapes: {out}"
    );
}

#[test]
fn transaction_orders_hide_account_and_statement() {
    let out = masked(&format!(
        "HKKAZ:3:6+{NUMBER}::280:37040044+N+20260101'\
         HKKAZ:4:7+{IBAN}:COBADEFFXXX:{NUMBER}::280:37040044+N+20260101'\
         HKCAZ:5:1+{IBAN}:COBADEFFXXX:::+urn?:iso?:std?:iso?:20022?:tech?:xsd?:camt.052.001.02+N'\
         HIKAZ:6:7:3+@40@:20:STARTUMS\r\n:25:37040044/{NUMBER}\r\n-'"
    ));
    assert_hidden(&out, &[NUMBER, IBAN, "STARTUMS"]);
    assert!(out.contains("HIKAZ:6:7:3+@40@<masked>'"), "{out}");
    assert!(out.contains("camt.052"), "report format is kept: {out}");
}

#[test]
fn camt_and_depot_statements_are_replaced_by_a_length_marker() {
    let camt = format!("<xml>{NUMBER}</xml>");
    let out = masked(&format!(
        "HICAZ:3:1:3+{IBAN}:COBADEFFXXX:::+urn?:x+@{}@{camt}'\
         HIWPD:4:6:3+@10@:16R:GENL-'\
         HIWDU:5:5:3+@8@MT536-1x'",
        camt.len()
    ));
    assert_hidden(&out, &[NUMBER, IBAN, "GENL", "MT536"]);
    assert!(out.contains("@21@<masked>"), "{out}");
    assert!(out.contains("@10@<masked>"), "{out}");
}

#[test]
fn depot_and_sepa_account_lists_are_masked() {
    let out = masked(&format!(
        "HKWPD:3:6+{NUMBER}::280:37040044+EUR'\
         HKWDU:4:5+{NUMBER}:01:280:37040044+EUR'\
         HKSPA:5:1+{NUMBER}::280:37040044+{NUMBER}:02:280:37040044'\
         HISPA:6:1:3+J:{IBAN}:COBADEFFXXX:{NUMBER}::280:37040044+N::::{NUMBER}:03:280:37040044'"
    ));
    assert_hidden(&out, &[NUMBER, IBAN]);
    assert!(out.contains("COBADEFFXXX"), "{out}");
}

#[test]
fn tan_segments_hide_the_account_and_the_challenge() {
    let out = masked(&format!(
        "HKTAN:3:6+4+HKSAL+{IBAN}:COBADEFFXXX:::+REF1'\
         HITAN:4:6:3+4++REF1+Bitte Auftrag an {IBAN} bestaetigen'"
    ));
    assert_hidden(&out, &[IBAN, "bestaetigen"]);
    assert!(out.contains("HITAN:4:6:3+4++REF1+"), "{out}");
}

#[test]
fn feedback_texts_lose_iban_like_words_only() {
    let out = masked(&format!(
        "HIRMS:3:2:3+9210::Konto {IBAN} gesperrt+0020::Auftrag ausgefuehrt'"
    ));
    assert_hidden(&out, &[IBAN]);
    assert!(
        out.contains("Konto ********************** gesperrt"),
        "{out}"
    );
    assert!(out.contains("0020::Auftrag ausgefuehrt"), "{out}");
}

#[test]
fn parameter_segments_and_envelope_are_kept() {
    let src = "HIBPA:3:3:3+1+280:37040044+Testbank+1+1+300'HITANS:4:7:3+1+1+1+N:N:1:HHD'HISALS:5:7:3+1+1+1+N'";
    assert_eq!(masked(src), src);
    // The general parameter segments (Formals D.3 to D.5) as well; none
    // of them is an unknown segment.
    let src = "HIKOM:6:4:4+280:12345678+1+3:https?://x'HISHV:7:3:4+N+PIN:1'HIKPV:8:1:4+0'";
    assert_eq!(masked(src), src);
    // Job parameters of orders outside the specification (live 2026-11-20,
    // comdirect and Consorsbank).
    let src = "DIKKSS:16:3:4+999+1+0'DIKKUS:17:2:4+999+1+0+9999:J:J'DIKAUS:18:2:4+999+1+0+J'\
               XIADAS:17:1:4+1+1'XIADSS:26:1:4+1+1'";
    assert_eq!(masked(src), src);
    for code in [
        "HIKOM", "HISHV", "HIKPV", "DIKKUS", "DIKKSS", "DIKAUS", "XIADAS", "XIADSS",
    ] {
        assert!(!super::is_unknown(code, 1), "{code}");
    }
    // An order or an answer in that namespace is still unknown.
    for code in ["XKADA", "XIADA", "HKXYZ", "XKADAS"] {
        assert!(super::is_unknown(code, 1), "{code}");
    }
}

#[test]
fn unknown_segments_are_masked_after_the_header() {
    let out = masked(&format!("HKXYZ:3:1+{NUMBER}+{IBAN}+Muster'"));
    assert_eq!(out, "HKXYZ:3:1+**********+**********************+******'");
}

#[test]
fn escapes_do_not_leak_or_break_masking() {
    // The user ID contains an escaped plus and an escaped colon.
    let out = masked("HKIDN:3:2+280:37040044+ab?+cd?:ef+SYS+1'HNHBS:4:1+1'");
    assert_eq!(out, "HKIDN:3:2+280:37040044+********+***+1'HNHBS:4:1+1'");
    // Escaped text that stays visible is escaped again.
    assert_eq!(masked("HIRMG:3:2+0010::a?+b'"), "HIRMG:3:2+0010::a?+b'");
}

#[test]
fn binary_data_with_separators_is_not_misread() {
    // The binary payload contains `'` and `+`; masking must neither stop
    // there nor show what follows.
    let out = masked(&format!("HIKAZ:3:7:3+@9@a'b+c:d@e'HKXYZ:4:1+{USER}'"));
    assert_eq!(out, "HIKAZ:3:7:3+@9@<masked>'HKXYZ:4:1+*********'");
}

#[test]
fn unparseable_input_shows_only_its_length() {
    let out = masked(&format!("HKIDN:3:2+280:37040044+{USER}+{SYS}"));
    assert!(out.starts_with('<') && out.contains("unparseable"), "{out}");
    assert_hidden(&out, &[USER, SYS]);
    assert!(masked("HKIDN:3:2+@99@short'").contains("unparseable"));
}

#[test]
fn parseable_form_reparses_and_hides() {
    let msg = envelope(&format!(
        "HKIDN:3:2+280:37040044+{USER}+{SYS}+1'HKKAZ:4:7+{IBAN}:COBADEFFXXX:{NUMBER}::280:37040044+N'"
    ));
    let out = redact_credentials(msg.as_bytes());
    let segments = wire::tokenize(&out).expect("masked message still tokenizes");
    assert_eq!(segments.len(), 4);
    let text = String::from_utf8_lossy(&out);
    assert_hidden(&text, &[USER, SYS, IBAN, NUMBER]);
    // Unparseable input keeps only separators.
    assert_eq!(redact_credentials(b"ab+c:d'e"), b"**+*:*'*".to_vec());
}

#[test]
fn nested_envelopes_stop_at_the_depth_limit() {
    let mut inner = format!("HKIDN:3:2+280:37040044+{USER}+{SYS}+1'");
    for _ in 0..4 {
        inner = format!("HNVSD:999:1+@{}@{inner}'", inner.len());
    }
    let out = masked(&inner);
    assert_hidden(&out, &[USER, SYS]);
}

const PAYEE: &str = "Erika Beispiel";
const PAYEE_IBAN: &str = "DE02120300000000202051";

fn pain001() -> String {
    format!(
        "<Document><CstmrCdtTrfInitn><GrpHdr><MsgId>MSG4711</MsgId><CtrlSum>1.00</CtrlSum></GrpHdr>\
         <PmtInf><PmtInfId>PMT4711</PmtInfId><PmtTpInf><SvcLvl><Cd>SEPA</Cd></SvcLvl></PmtTpInf>\
         <CdtTrfTxInf><PmtId><EndToEndId>E2E4711</EndToEndId></PmtId>\
         <Amt><InstdAmt Ccy=\"EUR\">1.00</InstdAmt></Amt>\
         <Cdtr><Nm>{PAYEE}</Nm></Cdtr><CdtrAcct><Id><IBAN>{PAYEE_IBAN}</IBAN></Id></CdtrAcct>\
         <RmtInf><Ustrd>Miete Oktober</Ustrd></RmtInf></CdtTrfTxInf></PmtInf></CstmrCdtTrfInitn></Document>"
    )
}

fn pain002() -> String {
    format!(
        "<Document><CstmrPmtStsRpt><OrgnlGrpInfAndSts><OrgnlMsgId>MSG4711</OrgnlMsgId>\
         <NbOfTxsPerSts><DtldNbOfTxs>1</DtldNbOfTxs><DtldSts>RVMC</DtldSts></NbOfTxsPerSts></OrgnlGrpInfAndSts>\
         <OrgnlPmtInfAndSts><TxInfAndSts><TxSts>RVMC</TxSts><StsRsnInf><AddtlInf>{PAYEE}</AddtlInf></StsRsnInf>\
         <OrgnlTxRef><Cdtr><Pty><Nm>{PAYEE}</Nm></Pty></Cdtr><CdtrAcct><Id><IBAN>{PAYEE_IBAN}</IBAN></Id></CdtrAcct></OrgnlTxRef>\
         </TxInfAndSts></OrgnlPmtInfAndSts></CstmrPmtStsRpt></Document>"
    )
}

fn binary(data: &str) -> String {
    format!("@{}@{data}", data.len())
}

const URN: &str = "urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.001.001.09";
const PAIN_SECRETS: &[&str] = &[
    PAYEE, "Beispiel", PAYEE_IBAN, "MSG4711", "PMT4711", "E2E4711", "Miete", ">1.00<",
];

#[test]
fn payment_orders_hide_account_and_pain_message() {
    let msg = format!(
        "HKCCS:3:1+{IBAN}:COBADEFFXXX:::+{URN}+{}'\
         HKIPZ:4:2+{IBAN}:COBADEFFXXX:::+{URN}+{}+J'",
        binary(&pain001()),
        binary(&pain001())
    );
    for out in [
        masked(&msg),
        String::from_utf8(redact_credentials(msg.as_bytes())).unwrap(),
    ] {
        assert_hidden(&out, PAIN_SECRETS);
        assert_hidden(&out, &[IBAN, "CstmrCdtTrfInitn"]);
        assert!(out.contains("COBADEFFXXX"), "BIC is kept: {out}");
        assert!(out.contains("pain.001.001.09"), "descriptor is kept: {out}");
        assert!(out.contains("+J'"), "conversion flag is kept: {out}");
    }
}

#[test]
fn trace_shows_the_pain_message_as_its_shape() {
    let msg = format!(
        "HKCCS:3:1+{IBAN}:COBADEFFXXX:::+{URN}+{}'",
        binary(&pain001())
    );
    let out: String = mask_for_trace(msg.as_bytes(), &ids())
        .into_iter()
        .map(char::from)
        .collect();
    assert_hidden(&out, PAIN_SECRETS);
    assert_hidden(&out, &[IBAN]);
    // Markup, currency and service level stay; amounts keep their shape.
    assert!(out.contains("<Cd>SEPA</Cd>"), "{out}");
    assert!(
        out.contains("<InstdAmt Ccy=\"EUR\">#.##</InstdAmt>"),
        "{out}"
    );
    assert!(out.contains("<Nm>########</Nm>"), "{out}");
    // The length is the one of the shaped message.
    let shaped = shape::shape(shape::Format::Pain, pain001().as_bytes(), Some(&ids()));
    assert!(
        out.contains(&format!("@{}@<Document>", shaped.len())),
        "{out}"
    );
}

#[test]
fn payee_check_request_hides_ids() {
    let out = masked(
        "HKVPP:4:1+urn?:iso?:std?:iso?:20022?:tech?:xsd?:pain.002.001.10+@9@POLL-ID-1+20+TOUCH-77'\
         HKVPA:5:1+@9@VOP-ID-42'",
    );
    assert_hidden(&out, &["POLL-ID", "TOUCH-77", "VOP-ID"]);
    assert!(
        out.contains("pain.002.001.10+@9@<masked>+20+********'"),
        "{out}"
    );
    assert!(out.contains("HKVPA:5:1+@9@<masked>'"), "{out}");
}

#[test]
fn payee_check_result_hides_payee_and_keeps_the_outcome() {
    let src = format!(
        "HIVPP:5:1:4+@9@VOP-ID-42+20261117:101500+@9@POLL-ID-1+urn?:x?:pain.002.001.10+{}\
         ++Der Name {PAYEE} weicht ab+3'\
         HIVPP:6:1:4+++++\
         +{PAYEE_IBAN}:Zusatz Info:{PAYEE}:Andere Id:RVMC:Grund {PAYEE}+Bitte pruefen+3'",
        binary(&pain002())
    );
    for out in [
        masked(&src),
        String::from_utf8(redact_credentials(src.as_bytes())).unwrap(),
    ] {
        assert_hidden(&out, PAIN_SECRETS);
        assert_hidden(
            &out,
            &[
                "VOP-ID", "POLL-ID", "Zusatz", "Andere", "Grund", "weicht", "pruefen",
            ],
        );
        assert!(out.contains("20261117:101500"), "validity is kept: {out}");
        assert!(out.contains(":RVMC:"), "outcome is kept: {out}");
        assert!(out.ends_with("+3'"), "wait is kept: {out}");
    }
    // The explanation shows its shape only.
    let out = masked(&src);
    assert!(out.contains("+########+3'HIVPP:6"), "{out}");
}

#[test]
fn trace_shows_the_payee_report_as_its_shape_with_status_codes() {
    let src = format!(
        "HIVPP:5:1:4+@9@VOP-ID-42+20261117:101500+@9@POLL-ID-1+urn?:x?:pain.002.001.10+{}'",
        binary(&pain002())
    );
    let out: String = mask_for_trace(src.as_bytes(), &ids())
        .into_iter()
        .map(char::from)
        .collect();
    assert_hidden(&out, PAIN_SECRETS);
    assert_hidden(&out, &["VOP-ID", "POLL-ID"]);
    assert!(
        out.contains("<TxSts>RVMC</TxSts>") && out.contains("<DtldSts>RVMC</DtldSts>"),
        "status codes are kept: {out}"
    );
    assert!(
        out.contains("<AddtlInf>########</AddtlInf>"),
        "notes are shaped: {out}"
    );
}

#[test]
fn instant_order_ids_are_masked() {
    let out = masked(&format!(
        "HKIPS:3:1+{IBAN}:COBADEFFXXX:::+{URN}+ORDER-4711'\
         HIIPS:4:1:3+{IBAN}:COBADEFFXXX:::+{URN}+{}+ORDER-4711+4+5'\
         HIIPZ:5:2:3+ORDER-4711+4+2'",
        binary(&pain002())
    ));
    assert_hidden(&out, &["ORDER-4711", IBAN]);
    assert_hidden(&out, PAIN_SECRETS);
    assert!(out.contains("+**********+4+5'"), "{out}");
    assert!(out.contains("HIIPZ:5:2:3+**********+4+2'"), "{out}");
}

#[test]
fn payment_segments_of_an_unknown_version_are_masked_completely() {
    let out = masked(&format!(
        "HKCCS:3:9+{IBAN}:COBADEFFXXX+{URN}+{}'HIVPP:4:2:4+{PAYEE}'",
        binary(&pain001())
    ));
    assert_hidden(&out, PAIN_SECRETS);
    assert_hidden(&out, &[IBAN, "COBADEFFXXX", "pain.001"]);
}

// ── Trace: aliases and pseudonyms ───────────────────────────────────────────────────

fn ids() -> Identities {
    Identities::new(pseudonym::Salt::from_bytes([42; 32]))
}

fn traced(msg: &str, ids: &Identities) -> String {
    mask_for_trace(msg.as_bytes(), ids)
        .into_iter()
        .map(char::from)
        .collect()
}

#[test]
fn trace_gives_identifiers_stand_ins_and_keeps_the_pin_starred() {
    let request = envelope(&format!(
        "HNSHK:2:4+PIN:2+921+4711+1+1+1::{SYS}+1+1:20260101:120000+1:999:1+6:10:16+280:37040044:{USER}:S:0:0'\
         HKIDN:3:2+280:37040044+{USER}+{SYS}+1'\
         HKKAZ:4:7+{IBAN}:COBADEFFXXX:{NUMBER}::280:37040044+N'\
         HNSHA:5:2+4711++geheim'"
    ));
    let answer = envelope(&format!(
        "HIUPA:4:4:3+{USER}+3+0+Erika Muster'\
         HIUPD:5:6:3+{NUMBER}::280:37040044+{IBAN}+{USER}+1+EUR+Erika Muster'\
         HISYN:6:4:3+{SYS}'\
         HIRMS:7:2:3+9210::Konto {IBAN} gesperrt'"
    ));
    let p = ids();
    let (out, back) = (traced(&request, &p), traced(&answer, &p));
    assert_hidden(&out, &[USER, SYS, IBAN, NUMBER, "geheim"]);
    assert_hidden(&back, &[USER, SYS, IBAN, NUMBER, "Erika"]);
    let user = p.text(IdClass::UserId, USER);
    let sys = p.text(IdClass::SystemId, SYS);
    let iban = p.text(IdClass::Iban, IBAN);
    let number = p.text(IdClass::Account, NUMBER);
    assert!(out.contains(&format!("+{user}+{sys}+1'")), "{out}");
    assert!(out.contains(&format!("1::{sys}+")), "HNSHK: {out}");
    assert!(out.contains(&format!(":{user}:S:0:0'")), "HNSHK: {out}");
    assert!(
        out.contains(&format!("+{iban}:COBADEFFXXX:{number}::280")),
        "{out}"
    );
    assert!(out.contains("HNSHA:5:2+4711++********'"), "{out}");
    assert!(back.contains(&format!("HIUPA:4:4:3+{user}+")), "{back}");
    assert!(
        back.contains(&format!("+{number}::280:37040044+{iban}+{user}+")),
        "{back}"
    );
    assert!(back.contains(&format!("HISYN:6:4:3+{sys}'")), "{back}");
    // An IBAN in a bank's text may be anyone's: the whole BBAN is aliased.
    let quoted = p.text(IdClass::OtherIban, IBAN);
    assert!(back.contains(&format!("Konto {quoted} gesperrt")), "{back}");
    assert!(crate::types::Iban::parse(&iban).is_ok(), "{iban}");
    assert!(
        iban.ends_with(&number),
        "KTV {number} and IBAN {iban} match"
    );
    // Another recorder with the same salt: the same system ID pseudonym,
    // other aliases for the account identifiers; another salt, another
    // system ID pseudonym.
    let again = ids();
    let second = traced(&request, &again);
    assert!(second.contains(&format!("1::{sys}+")), "{second}");
    for (class, value) in [
        (IdClass::UserId, USER),
        (IdClass::Iban, IBAN),
        (IdClass::Account, NUMBER),
    ] {
        let alias = again.text(class, value);
        assert!(second.contains(&alias), "{second}");
        assert!(!out.contains(&alias), "{value}: {alias} in {out}");
    }
    let other = Identities::new(pseudonym::Salt::from_bytes([7; 32]));
    assert!(!traced(&request, &other).contains(&sys));
    // The log does not change.
    assert!(masked(&request).contains("+*********+*************+1'"));
}

#[test]
fn trace_links_vop_and_polling_ids_between_answer_and_request() {
    let p = ids();
    let answer = traced("HIVPP:5:1:4+@9@VOP-ID-42+20261117:101500+@9@POLL-ID-1'", &p);
    let request = traced(
        "HKVPP:4:1+urn?:x+@9@POLL-ID-1+20+TOUCH-77'HKVPA:5:1+@9@VOP-ID-42'",
        &p,
    );
    let as_text = |b: Vec<u8>| b.into_iter().map(char::from).collect::<String>();
    let vop = as_text(p.bytes(IdClass::VopId, b"VOP-ID-42"));
    let poll = as_text(p.bytes(IdClass::PollingId, b"POLL-ID-1"));
    assert!(answer.contains(&format!("+@9@{vop}+")), "{answer}");
    assert!(
        request.contains(&format!("HKVPA:5:1+@9@{vop}'")),
        "{request}"
    );
    assert!(answer.contains(&format!("+@9@{poll}'")), "{answer}");
    assert!(
        request.contains(&format!("+@9@{poll}+20+********'")),
        "{request}"
    );
    assert_hidden(&answer, &["VOP-ID-42", "POLL-ID-1"]);
}

#[test]
fn trace_links_the_ids_of_pain001_and_pain002() {
    let report = "<Document><CstmrPmtStsRpt><OrgnlGrpInfAndSts><OrgnlMsgId>MSG4711</OrgnlMsgId>\
         </OrgnlGrpInfAndSts><OrgnlPmtInfAndSts><OrgnlPmtInfId>PMT4711</OrgnlPmtInfId>\
         <TxInfAndSts><OrgnlEndToEndId>E2E4711</OrgnlEndToEndId><TxSts>ACCP</TxSts>\
         </TxInfAndSts></OrgnlPmtInfAndSts></CstmrPmtStsRpt></Document>";
    let p = ids();
    let order = traced(
        &format!(
            "HKCCS:3:1+{IBAN}:COBADEFFXXX:::+{URN}+{}'",
            binary(&pain001())
        ),
        &p,
    );
    let answer = traced(
        &format!(
            "HIVPP:5:1:4+++++{}'HIIPS:6:1:3+{IBAN}:COBADEFFXXX:::+{URN}+{}+ORDER-1+4'",
            binary(report),
            binary(report)
        ),
        &p,
    );
    assert_hidden(&order, PAIN_SECRETS);
    assert_hidden(&answer, &["MSG4711", "PMT4711", "E2E4711", "ORDER-1"]);
    for (class, value) in [
        (IdClass::EndToEnd, "E2E4711"),
        (IdClass::MessageId, "MSG4711"),
        (IdClass::PaymentInfoId, "PMT4711"),
    ] {
        let pseudonym = p.text(class, value);
        assert!(order.contains(&pseudonym), "{value}: {order}");
        assert_eq!(answer.matches(&pseudonym).count(), 2, "{value}: {answer}");
    }
    let payee = p.text(IdClass::OtherIban, PAYEE_IBAN);
    assert!(order.contains(&format!("<IBAN>{payee}</IBAN>")), "{order}");
    assert!(order.contains("<Nm>########</Nm>"), "{order}");
    assert!(answer.contains("<TxSts>ACCP</TxSts>"), "{answer}");
}

/// Synthetic MT940: the end-to-end ID runs over two subfields.
const MT940_IDS: &str = "\r\n:20:STARTUMS\r\n:25:37040044/0532013000\r\n:28C:1/1\r\n\
:60F:C260101EUR1,00\r\n\
:61:2601020102DR12,50NDDTE2E-4711//BANKREF-1\r\n\
:86:105?00LASTSCHRIFT?20EREF+E2E-47?2111 MREF+M-42 CRED+DE98ZZZ0999\r\n\
?229999999 SVWZ+Beitrag?30TESTDEFFXXX?31DE02120300000000202051?32Versicherung\r\n\
:62F:C260102EUR1,00\r\n-";

#[test]
fn trace_pseudonymises_references_and_accounts_in_mt940() {
    let p = ids();
    let msg = format!("HIKAZ:3:7:3+{}'", binary(MT940_IDS));
    let out = traced(&msg, &p);
    assert_hidden(
        &out,
        &[
            "0532013000",
            "E2E-4711",
            "BANKREF",
            "M-42",
            "9999999",
            "DE0212",
            "Beitrag",
            "Versicherung",
        ],
    );
    let e2e = p.text(IdClass::EndToEnd, "E2E-4711");
    let own = p.text(IdClass::Account, "0532013000");
    let bank_ref = p.text(IdClass::BookingRef, "BANKREF-1");
    let mandate = p.text(IdClass::Mandate, "M-42");
    let creditor = p.text(IdClass::CreditorId, "DE98ZZZ09999999999");
    let payee = p.text(IdClass::OtherIban, "DE02120300000000202051");
    assert!(
        !payee.contains("12030000"),
        "a counterparty's bank code is hidden: {payee}"
    );
    assert!(out.contains(&format!(":25:########/{own}\r\n")), "{out}");
    assert!(
        out.contains(&format!("NDDT{e2e}//{bank_ref}\r\n")),
        "customer and bank reference: {out}"
    );
    // A value the bank split over two subfields is written whole where
    // it began; the space before a keyword stays a space.
    assert!(
        out.contains(&format!(
            "?20EREF+{e2e}?21 MREF+{mandate} CRED+{creditor}\r\n?22 SVWZ+########?30"
        )),
        "{out}"
    );
    assert!(out.contains(&format!("?31{payee}?32")), "{out}");
    assert!(crate::types::Iban::parse(&payee).is_ok());
    // The replay rebuilds it and the client reads the pseudonyms.
    let shaped = shape::shape(shape::Format::Mt940, MT940_IDS.as_bytes(), Some(&p));
    let rebuilt = shape::unshape(shape::Format::Mt940, &shaped);
    let txs = crate::formats::mt940::parse_mt940(&rebuilt, crate::types::TransactionStatus::Booked)
        .expect("rebuilt MT940 parses");
    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].bank_reference.as_deref(), Some(bank_ref.as_str()));
}

#[test]
fn trace_pseudonymises_references_and_accounts_in_camt() {
    let camt = format!(
        "<?xml version=\"1.0\"?><Document><BkToCstmrAcctRpt><GrpHdr><MsgId>CAMT-1</MsgId>\
         <CreDtTm>2026-01-02T10:00:00</CreDtTm></GrpHdr><Rpt><Id>R1</Id>\
         <Acct><Id><IBAN>{IBAN}</IBAN></Id></Acct>\
         <Ntry><Amt Ccy=\"EUR\">12.50</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts>\
         <BookgDt><Dt>2026-01-02</Dt></BookgDt><ValDt><Dt>2026-01-02</Dt></ValDt>\
         <AcctSvcrRef>2026-01-02-10.11.12.123456</AcctSvcrRef>\
         <BkTxCd/><NtryDtls><TxDtls><Refs><EndToEndId>E2E4711</EndToEndId>\
         <MndtId>M-42</MndtId></Refs>\
         <RltdPties><Cdtr><Nm>Versicherung AG</Nm></Cdtr>\
         <CdtrAcct><Id><IBAN>{PAYEE_IBAN}</IBAN></Id></CdtrAcct>\
         <CdtrSchmeId><Id><PrvtId><Othr><Id>DE98ZZZ09999999999</Id></Othr></PrvtId></Id></CdtrSchmeId>\
         </RltdPties></TxDtls></NtryDtls></Ntry></Rpt></BkToCstmrAcctRpt></Document>"
    );
    let p = ids();
    let out = traced(
        &format!(
            "HICAZ:3:1:3+{IBAN}:COBADEFFXXX:::+urn?:x+{}'",
            binary(&camt)
        ),
        &p,
    );
    assert_hidden(
        &out,
        &[
            IBAN,
            PAYEE_IBAN,
            "123456",
            "E2E4711",
            "M-42",
            "9999999",
            "Versicherung",
        ],
    );
    for (class, value) in [
        (IdClass::Iban, IBAN),
        (IdClass::OtherIban, PAYEE_IBAN),
        (IdClass::BookingRef, "2026-01-02-10.11.12.123456"),
        (IdClass::EndToEnd, "E2E4711"),
        (IdClass::Mandate, "M-42"),
        (IdClass::CreditorId, "DE98ZZZ09999999999"),
        (IdClass::MessageId, "CAMT-1"),
    ] {
        let pseudonym = p.text(class, value);
        assert!(out.contains(&format!(">{pseudonym}<")), "{value}: {out}");
    }
    assert!(
        out.contains("<Id>########</Id>"),
        "the report ID is text: {out}"
    );
    assert!(out.contains("<Nm>########</Nm>"), "{out}");
    // The replay rebuilds it; the client reads the pseudonymised reference.
    let shaped = shape::shape(shape::Format::Camt, camt.as_bytes(), Some(&p));
    let rebuilt = shape::unshape(shape::Format::Camt, &shaped);
    let txs = crate::camt::parse_camt052(
        &rebuilt,
        crate::banks::BookingIdSource::CamtEntryAcctSvcrRef,
    )
    .expect("rebuilt camt parses");
    assert_eq!(
        txs[0].booking_id.as_deref(),
        Some(
            p.text(IdClass::BookingRef, "2026-01-02-10.11.12.123456")
                .as_str()
        )
    );
    let payee = p.text(IdClass::OtherIban, PAYEE_IBAN);
    assert_eq!(
        txs[0].applicant_iban.as_ref().map(|i| i.as_str()),
        Some(payee.as_str())
    );
}

proptest! {
    #[test]
    fn never_panics_on_arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..300)) {
        let _ = mask_for_log(&data);
        let _ = redact_credentials(&data);
    }

    #[test]
    fn never_panics_on_wire_like_input(
        data in proptest::string::string_regex("[A-Z]{0,5}[:+'@?0-9a-z\r\n]{0,200}").unwrap()
    ) {
        let _ = mask_for_log(data.as_bytes());
        let _ = redact_credentials(data.as_bytes());
    }

    #[test]
    fn identifiers_never_survive(
        user in "user-[A-Za-z0-9]{8,20}",
        sys in "sys-[A-Za-z0-9]{8,20}",
        num in "77[0-9]{8}",
    ) {
        let msg = envelope(&format!(
            "HKIDN:3:2+280:37040044+{user}+{sys}+1'HKSAL:4:5+{num}::280:37040044+N'HNSHA:5:2+1++{user}:{sys}'"
        ));
        let outs = [
            mask_for_log(msg.as_bytes()),
            String::from_utf8_lossy(&redact_credentials(msg.as_bytes())).into_owned(),
            traced(&msg, &ids()),
        ];
        for out in outs {
            for secret in [&user, &sys, &num] {
                prop_assert!(!out.contains(secret.as_str()), "{} visible in {}", secret, out);
            }
        }
    }
}

#[test]
fn the_product_registration_is_starred_in_the_log() {
    let log = mask_for_log(b"HNHBK:1:3+000000000010+300+0+1'HKVVB:2:3+0+0+0+REALPRODUCT+1.0'");
    assert!(!log.contains("REALPRODUCT"), "{log}");
    assert!(log.contains("HKVVB:2:3+0+0+0+***********+1.0'"), "{log}");
}

/// A counterparty IBAN the bank wrapped over two lines of `?31` is one
/// value: it gets one alias, written where the value began, and no part of
/// it stays.
#[test]
fn trace_aliases_a_counterparty_iban_split_by_a_line_break() {
    let p = ids();
    let mt940 = "\r\n:20:S\r\n:25:37040044/0532013000\r\n:28C:1/1\r\n:60F:C260101EUR1,00\r\n\
:61:2601020102D1,00NTRFNONREF\r\n\
:86:166?00X?30TESTDEFFXXX?31DE021203000000\r\n00202051?32Versicherung\r\n\
:62F:C260102EUR1,00\r\n-";
    let out = traced(&format!("HIKAZ:3:7:3+{}'", binary(mt940)), &p);
    let alias = p.text(IdClass::OtherIban, "DE02120300000000202051");
    assert!(out.contains(&format!("?31{alias}\r\n?32")), "{out}");
    assert_hidden(&out, &["DE0212", "202051", "12030000"]);
}

proptest::proptest! {
    /// Segments that differ only in the holder's name, the product name,
    /// the size of a balance and the PIN are masked to the same bytes.
    #[test]
    fn segment_values_do_not_show(
        names in proptest::array::uniform2("[A-Za-z][A-Za-z .-]{0,25}"),
        products in proptest::array::uniform2("[A-Za-z][A-Za-z -]{0,28}"),
        amounts in proptest::array::uniform2("[1-9][0-9]{0,8}"),
        pins in proptest::array::uniform2("[A-Za-z0-9]{5,20}"),
    ) {
        let message = |n: usize| masked(&format!(
            "HIUPA:4:4:3+testuser1+3+0+{name}'\
             HIUPD:5:6:3+0001234567::280:12345678+DE00123456780001234567+testuser1+1+EUR+{name}++{product}'\
             HISAL:6:7:3+DE00123456780001234567:TESTDEFFXXX:0001234567::280:12345678+{product}+EUR+C:{amount},56:EUR:20260101'\
             HNSHA:7:2+4711++{pin}'",
            name = names[n], product = products[n], amount = amounts[n], pin = pins[n],
        ));
        proptest::prop_assert_eq!(message(0), message(1));
    }
}

#[test]
fn unknown_segments_stay_for_review_and_are_hidden_unless_kept() {
    let p = ids();
    let message = envelope("HIXYZ:4:1:3+Erika Muster+@3@abc'HISYN:5:4:3+SYS1'");
    let held = mask_for_recorder(message.as_bytes(), &p);
    let unknown = unknown_segments(&held);
    assert_eq!(unknown, [b"HIXYZ:4:1:3+Erika Muster+@3@abc'".to_vec()]);
    let text = |b: Vec<u8>| b.into_iter().map(char::from).collect::<String>();
    let hidden = text(hide_unknown(&held, &|_| false));
    assert!(
        hidden.contains("HIXYZ:4:1:3+************+@3@***'"),
        "{hidden}"
    );
    let kept = text(hide_unknown(&held, &|code| code == "HIXYZ"));
    assert!(kept.contains("HIXYZ:4:1:3+Erika Muster+@3@abc'"), "{kept}");
    // A HIUPD without a field list is never offered.
    assert!(!is_unknown("HIUPD", 5) && is_unknown("HIXYZ", 1) && !is_unknown("HISAL", 7));
}

/// The masking table is complete for what the library can read: a segment
/// with a definition in the codec is never an unknown one. A new definition
/// fails here until `Policy::of` says which of its fields are hidden.
/// HIUPD in a version other than 6 is hidden whole on purpose.
#[test]
fn every_defined_segment_has_a_masking_rule() {
    let missing: Vec<String> = crate::codec::registry::definitions()
        .iter()
        .filter(|def| def.code != "HIUPD" && Policy::of(def.code, def.version) == Policy::All)
        .map(|def| format!("{}:{}", def.code, def.version))
        .collect();
    assert!(missing.is_empty(), "no masking rule for {missing:?}");
}

/// In a trace the dates of a segment move with the days of the trace and
/// times of day are noon; the log shows both as sent. The debit mark of a
/// balance is written as credit.
#[test]
fn trace_moves_segment_dates_and_hides_times_and_balance_marks() {
    let p = Identities::new(pseudonym::Salt::from_bytes([3; 32])).with_days(-10);
    let src = "HNSHK:2:4+PIN:1+999+1+1+1+1::0+1+1:20261120:114135+1:999:1+6:10:16+280:12345678:u:S:0:0'\
               HKKAZ:3:7+DE02120300000000202051:TESTDEFFXXX+N+20261021+20261120'\
               HKCAZ:4:1+DE02120300000000202051:TESTDEFFXXX+urn?:x+N+20261021+20261120'\
               HISAL:5:6:3+1234567::280:12345678+Giro+EUR+D:12,50:EUR:20261120:101530++0,:EUR'";
    let out = traced(src, &p);
    assert!(out.contains("+1:20261110:120000+"), "{out}");
    assert_eq!(out.matches("+N+20261011+20261110'").count(), 2, "{out}");
    assert!(
        out.contains("+EUR+C:#,##:EUR:20261110:120000++#,##:EUR'"),
        "{out}"
    );
    let log = masked(src);
    assert!(
        log.contains("+1:20261120:114135+") && log.contains("+N+20261021+20261120'"),
        "{log}"
    );
}
