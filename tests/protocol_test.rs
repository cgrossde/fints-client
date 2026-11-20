//! The protocol layer seen from outside: return codes and what an answer
//! means for the dialog (Formals C, return codes), the HKTAN decision per
//! order, the choice of TAN method, accounts and the public value types.
//! All data is synthetic.

use std::collections::BTreeSet;

use fints_client::FinTSError;
use fints_client::protocol::{Account, BankParams, Response, TanOverrides, TanReason, order_tan};
use fints_client::return_code::{Category, Class, Code, Credentials, Kind, ReturnCode, Scope};
use fints_client::types::{
    Currency, DialogId, Isin, SecurityFunction, SecurityHolding, SegmentType, SystemId, TanMethod,
    TanProcess, TouchdownPoint, Wkn,
};
use rust_decimal::Decimal;

fn code(number: &str, text: &str, parameters: &[&str]) -> ReturnCode {
    let parameters = parameters.iter().map(|p| p.to_string()).collect();
    ReturnCode::new(Code::new(number).unwrap(), text, Scope::Message).with_parameters(parameters)
}

/// An answer with `codes` as message codes (HIRMG) and `segment` as
/// segment codes (HIRMS).
fn answer(message: &[ReturnCode], segment: &[ReturnCode]) -> Response {
    Response {
        segments: Vec::new(),
        global_codes: message.to_vec(),
        segment_codes: segment.to_vec(),
    }
}

// ── Return codes ───────────────────────────────────────────────────────

/// The first digit gives the class; some codes have a kind of their own.
#[test]
fn class_and_kind_of_return_codes() {
    let cases = [
        ("0020", Class::Success, None),
        ("3920", Class::Warning, None),
        ("9942", Class::Error, Some(Kind::PinWrong)),
        ("9340", Class::Error, Some(Kind::SignatureInvalid)),
        ("9800", Class::Error, Some(Kind::DialogAborted)),
    ];
    for (number, class, kind) in cases {
        let rc = code(number, "Testtext", &[]);
        assert_eq!(rc.class(), class, "{number}");
        assert_eq!(rc.is_error(), class == Class::Error, "{number}");
        if let Some(kind) = kind {
            assert_eq!(rc.kind(), kind, "{number}");
        }
    }
    let allowed = code("3920", "Zugelassene Verfahren", &["922", "944"]);
    assert!(matches!(allowed.kind(), Kind::AllowedSecurityFunctions(_)));
}

// ── What an answer says ────────────────────────────────────────────────

#[test]
fn tan_and_decoupled_states() {
    // 0030: the order waits for a TAN.
    assert!(answer(&[code("0030", "TAN erforderlich", &[])], &[]).needs_tan());

    // 3955: approval in the app started.
    let started = answer(&[], &[code("3955", "Freigabe in der App", &[])]);
    assert!(started.needs_tan() && started.is_decoupled());

    // 3956, and 3957 without 0020: still waiting.
    for waiting in ["3956", "3957"] {
        let pending = answer(&[], &[code(waiting, "Noch nicht freigegeben", &[])]);
        assert!(pending.is_decoupled_pending(), "{waiting}");
    }
    // 3957 next to 0020 is a notice; the order is done.
    let done = answer(
        &[],
        &[
            code("0020", "Ausgefuehrt", &[]),
            code("3957", "Hinweis", &[]),
        ],
    );
    assert!(!done.is_decoupled_pending());

    // 3076: no strong authentication needed.
    let exempt = answer(&[], &[code("3076", "Keine SCA erforderlich", &[])]);
    assert!(exempt.has_sca_exemption() && !exempt.needs_tan());
}

#[test]
fn touchdown_point_and_allowed_methods() {
    let more = answer(&[], &[code("3040", "Weitere Daten", &["P-0002"])]);
    assert_eq!(more.touchdown(), Some(TouchdownPoint::new("P-0002")));
    assert_eq!(answer(&[code("0020", "ok", &[])], &[]).touchdown(), None);

    let allowed = answer(
        &[],
        &[code(
            "3920",
            "Zugelassene Verfahren",
            &["922", "944", "972"],
        )],
    );
    let functions: Vec<_> = allowed.allowed_security_functions();
    let names: Vec<&str> = functions.iter().map(SecurityFunction::as_str).collect();
    assert_eq!(names, ["922", "944", "972"]);
}

/// Login errors are credential categories; other 9xxx are bank errors
/// with their code and text.
#[test]
fn errors_from_an_answer() {
    for (number, expected) in [
        ("9942", Credentials::PinWrong),
        ("9931", Credentials::Locked),
    ] {
        let err = answer(&[code(number, "Anmeldung abgelehnt", &[])], &[])
            .check_errors()
            .unwrap_err();
        assert_eq!(
            err.kind(),
            Category::Credentials(expected),
            "{number}: {err}"
        );
    }

    let err = answer(&[code("9800", "Dialogabbruch", &[])], &[])
        .check_errors()
        .unwrap_err();
    let FinTSError::Bank(bank) = &err else {
        panic!("{err}");
    };
    assert_eq!(bank.code().kind(), Kind::DialogAborted);
    assert_eq!(bank.code().text, "Dialogabbruch");
    assert_eq!(bank.category(), Category::DialogEnded);
}

// ── HKTAN per order ────────────────────────────────────────────────────

/// Under a two-step method an order gets HKTAN when HIPINS says so; one
/// HIPINS does not list gets none (a 9075 answer retries with HKTAN).
#[test]
fn hktan_follows_hipins() {
    let decide = |order: &str, hipins: Option<bool>| {
        order_tan(
            &SegmentType::new(order),
            true,
            hipins,
            &TanOverrides::default(),
            &BTreeSet::new(),
        )
    };
    assert!(!decide("HKSAL", Some(false)).hktan);
    assert!(decide("HKKAZ", Some(true)).hktan);
    let unlisted = decide("HKXYZ", None);
    assert_eq!(
        (unlisted.hktan, unlisted.reason),
        (false, TanReason::Unlisted)
    );

    // HIPINS as stored in the BPD: an empty BankParams has none.
    let params = BankParams::new();
    let code = SegmentType::new("HKABC");
    let from_bpd = params.bpd().and_then(|bpd| bpd.tan_required(code.as_str()));
    assert!(
        !order_tan(
            &code,
            true,
            from_bpd,
            &TanOverrides::default(),
            &params.tan_required_learned
        )
        .hktan
    );
}

// ── Choosing the TAN method ────────────────────────────────────────────

fn method(function: &str, name: &str, decoupled: bool) -> TanMethod {
    TanMethod {
        security_function: SecurityFunction::new(function),
        tan_process: TanProcess::TwoStep,
        name: name.into(),
        needs_tan_medium: decoupled,
        decoupled_max_polls: if decoupled { 30 } else { -1 },
        wait_before_first_poll: if decoupled { 4 } else { 0 },
        wait_before_next_poll: if decoupled { 2 } else { 0 },
        is_decoupled: decoupled,
        hktan_version: 7,
        zka_name: None,
        decoupled_auto_polling: None,
        challenge_structured: false,
    }
}

/// A reader method (922) and an app method (944), both allowed.
fn two_methods() -> BankParams {
    let mut params = BankParams::new();
    params.allowed_security_functions =
        vec![SecurityFunction::new("922"), SecurityFunction::new("944")];
    params.tan_methods = vec![
        method("922", "Kartenleser", false),
        method("944", "Test-App", true),
    ];
    params
}

#[test]
fn without_a_preference_the_app_method_is_chosen() {
    let mut params = two_methods();
    params.select_security_function();
    assert_eq!(params.selected_security_function.as_str(), "944");
    assert!(params.is_decoupled());
}

#[test]
fn an_allowed_preference_wins() {
    let mut params = two_methods();
    params.preferred_security_function = Some(SecurityFunction::new("922"));
    params.select_security_function();
    assert_eq!(params.selected_security_function.as_str(), "922");
    assert!(!params.is_decoupled());
}

/// Without any TAN method no polling rule is set; the caller's timing
/// applies.
#[test]
fn no_tan_method_sets_no_polling_rule() {
    let rules = BankParams::new().decoupled_params();
    assert_eq!(
        (rules.wait_before_first_poll, rules.wait_before_next_poll),
        (None, None)
    );
    assert_eq!((rules.max_polls, rules.auto_polling_allowed), (None, None));
}

// ── Accounts and value types ───────────────────────────────────────────

/// IBAN and BIC are both needed: a missing BIC makes banks answer 9160.
#[test]
fn an_account_needs_iban_and_bic() {
    let (iban, bic) = ("DE00123456780000004711", "TESTDEFFXXX");
    let account = Account::new(iban, bic).unwrap();
    assert_eq!((account.iban(), account.bic()), (iban, bic));
    for (iban, bic) in [("", bic), (iban, ""), ("", "")] {
        assert!(Account::new(iban, bic).is_err(), "{iban:?} {bic:?}");
    }
}

/// "0" stands for an ID the bank has not handed out yet.
#[test]
fn unassigned_ids() {
    assert!(!SystemId::unassigned().is_assigned());
    assert!(!SystemId::new("0").is_assigned());
    assert!(SystemId::new("SYS-0042").is_assigned());
    assert!(!DialogId::unassigned().is_assigned());
    assert!(!DialogId::new("0").is_assigned());
    assert!(DialogId::new("D-7").is_assigned());
}

#[test]
fn a_holding_survives_json() {
    let holding = SecurityHolding {
        isin: Some(Isin::new("DE0000000001")),
        wkn: Some(Wkn::new("A0TEST")),
        name: "TESTAKTIE AG".into(),
        quantity: Decimal::from(40),
        price: Some(Decimal::new(125, 1)),
        price_currency: Some(Currency::new("EUR")),
        price_date: Some(jiff::civil::date(2026, 1, 30)),
        market_value: Some(Decimal::from(500)),
        market_value_currency: Some(Currency::new("EUR")),
        acquisition_value: Some(Decimal::from(450)),
        acquisition_date: None,
        acquisition_price: None,
        acquisition_price_currency: None,
        profit_loss: Some(Decimal::from(50)),
        exchange: Some("XFRA".into()),
        depot_id: Some("0000004711".into()),
        statement_date: None,
        raw: serde_json::json!({"source": "MT535"}),
    };
    let json = serde_json::to_string(&holding).unwrap();
    let back: SecurityHolding = serde_json::from_str(&json).unwrap();
    assert_eq!(back.isin.as_ref().map(Isin::as_str), Some("DE0000000001"));
    assert_eq!(
        back.wkn.as_ref().map(ToString::to_string).as_deref(),
        Some("A0TEST")
    );
    assert_eq!(
        (back.name.as_str(), back.quantity),
        ("TESTAKTIE AG", Decimal::from(40))
    );
    assert_eq!(
        (back.price, back.profit_loss),
        (holding.price, holding.profit_loss)
    );
    assert_eq!(back.raw, holding.raw);
}
