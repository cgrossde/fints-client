//! Standalone synthetic FinTS bank on 127.0.0.1, for trying the library by
//! hand. Synthetic data only; Ctrl-C ends it.
//!
//! ```text
//! fints-testbank [--port N] [--tan decoupled|typed|none] [--polls N] [--profile NAME]
//! ```

use std::time::Duration;

use fints_client_testbank::{BankModel, LoginSca, TanKind, TanMethod, TestBank, profiles};

const USAGE: &str = "usage: fints-testbank [--port N] [--tan decoupled|typed|none] [--polls N] [--profile NAME]\n  \
    --port N   port on 127.0.0.1 (default 47331, 0 = any free port)\n  \
    --tan T    login TAN: decoupled, typed (TAN asked at login) or none (SCA exempt, default)\n  \
    --polls N  decoupled: polls before the approval (default 2)\n  \
    --profile NAME  serve a bank profile (consorsbank) under the synthetic bank code 12345678; --tan is ignored";

type Args = (u16, String, u32, Option<String>);

fn parse() -> Result<Args, String> {
    let (mut port, mut tan, mut polls, mut profile) = (
        fints_client::banks::TESTBANK_PORT,
        "none".to_string(),
        2u32,
        None,
    );
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--port" => port = value("--port")?.parse().map_err(|_| "bad --port")?,
            "--tan" => tan = value("--tan")?,
            "--polls" => polls = value("--polls")?.parse().map_err(|_| "bad --polls")?,
            "--profile" => profile = Some(value("--profile")?),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if !["decoupled", "typed", "none"].contains(&tan.as_str()) {
        return Err(format!("--tan must be decoupled, typed or none, not {tan}"));
    }
    if let Some(p) = &profile
        && !profiles::all().iter().any(|(n, _)| n == p)
    {
        let names: Vec<_> = profiles::all().iter().map(|(n, _)| *n).collect();
        return Err(format!("unknown profile {p} (known: {})", names.join(", ")));
    }
    Ok((port, tan, polls, profile))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (port, tan, polls, profile) = match parse() {
        Ok(v) => v,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("{USAGE}");
            std::process::exit(if e.is_empty() { 0 } else { 2 });
        }
    };
    let mut model = BankModel::default();
    if let Some(name) = &profile {
        let build = profiles::all()
            .iter()
            .find(|(n, _)| n == name)
            .expect("checked")
            .1;
        model = build();
        // The real bank code never goes on the wire, not even to loopback.
        model.blz = "12345678".into();
        for a in &mut model.accounts {
            if a.iban.is_some() {
                a.iban = Some(fints_client_testbank::iban_de("12345678", &a.number));
            }
        }
        println!("Profile:   {name} (served under the synthetic bank code 12345678)");
    }
    match tan.as_str() {
        "decoupled" => {
            model.tan_methods = vec![TanMethod::decoupled(942, "Test-Decoupled", polls)];
            model.login_sca = LoginSca::Tan;
        }
        "typed" => model.login_sca = LoginSca::Tan,
        _ => {}
    }

    let bank = TestBank::new(model.clone())
        .serve_on(([127, 0, 0, 1], port).into())
        .await;
    println!("URL:       {}", bank.url());
    println!("Bank code: {}  (BIC {})", model.blz, model.bic);
    println!("User ID:   {}", model.customer.user_id);
    println!("PIN:       {}", model.customer.pin);
    for m in &model.tan_methods {
        match &m.kind {
            TanKind::Typed { tan, .. } => {
                println!("TAN:       {tan}  (method {} {})", m.function, m.name)
            }
            TanKind::Decoupled { .. } => println!(
                "TAN:       approval by the bank after {polls} polls  (method {} {})",
                m.function, m.name
            ),
        }
    }
    println!(
        "Login:     {}",
        match model.login_sca {
            LoginSca::Exempt => "no TAN (SCA exempt)",
            LoginSca::Tan => "TAN required",
        }
    );
    for a in &model.accounts {
        println!(
            "Account:   {}  number {}  {}  balance {} {}",
            a.iban.as_deref().unwrap_or("-"),
            a.number,
            a.holder,
            a.balance,
            a.currency
        );
    }
    println!("Ctrl-C to stop.");

    let mut seen = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let violations = bank.violations();
        for v in &violations[seen..] {
            eprintln!("VIOLATION {v}");
        }
        seen = violations.len();
    }
}
