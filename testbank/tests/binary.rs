//! Runs the `fints-testbank` binary and syncs against it with the client.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use fints_client::jobs::{self, BankClient, Credentials, Step};
use fints_client::{BankConfig, Blz, Pin, ProductId, SessionState, UserId};

struct Kill(std::process::Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn binary_serves_a_sync() {
    let child = Command::new(env!("CARGO_BIN_EXE_fints-testbank"))
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("start fints-testbank");
    let mut child = Kill(child);
    let mut lines = BufReader::new(child.0.stdout.take().unwrap()).lines();
    let mut url = None;
    let mut printed = String::new();
    for line in lines.by_ref() {
        let line = line.expect("stdout");
        if let Some(u) = line.strip_prefix("URL:") {
            url = Some(u.trim().to_string());
        }
        printed.push_str(&line);
        if line.starts_with("Ctrl-C") {
            break;
        }
    }
    let url = url.expect("printed URL");
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(printed.contains("testuser1") && printed.contains("test-pin-1"));

    let client = BankClient::new(
        BankConfig::new("Testbank", "12345678", "TESTDEFFXXX", url),
        ProductId::new("TestProdukt"),
    );
    let user = UserId::new("testuser1");
    let creds = Credentials::new(user.clone(), Pin::new("test-pin-1"));
    let state = SessionState::new(&Blz::new("12345678"), &user);
    let (dialog, step) = jobs::open(&client, &creds, &state).await.expect("open");
    let Step::Done(done) = step else {
        panic!("the login needs no TAN");
    };
    assert_eq!(done.value.accounts.len(), 1);
    dialog.end().await.expect("end");
}

#[tokio::test]
async fn binary_serves_a_profile_under_the_synthetic_code() {
    let child = Command::new(env!("CARGO_BIN_EXE_fints-testbank"))
        .args(["--port", "0", "--profile", "consorsbank"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("start fints-testbank");
    let mut child = Kill(child);
    let mut lines = BufReader::new(child.0.stdout.take().unwrap()).lines();
    let mut url = None;
    let mut printed = String::new();
    for line in lines.by_ref() {
        let line = line.expect("stdout");
        if let Some(u) = line.strip_prefix("URL:") {
            url = Some(u.trim().to_string());
        }
        printed.push_str(&line);
        printed.push('\n');
        if line.starts_with("Ctrl-C") {
            break;
        }
    }
    assert!(printed.contains("consorsbank") && printed.contains("Bank code: 12345678"));
    assert!(
        !printed.contains("76030080"),
        "the real bank code is not served"
    );

    let client = BankClient::new(
        BankConfig::new(
            "Consors",
            "12345678",
            "CSDBDE71XXX",
            url.expect("printed URL"),
        ),
        ProductId::new("TestProdukt"),
    );
    let user = UserId::new("testuser001");
    let creds = Credentials::new(user.clone(), Pin::new("test-pin-1"));
    let state = SessionState::new(&Blz::new("12345678"), &user);
    let (dialog, step) = jobs::open(&client, &creds, &state).await.expect("open");
    let Step::Done(done) = step else {
        panic!("the login needs no TAN");
    };
    assert_eq!(done.value.accounts.len(), 4);
    dialog.end().await.expect("end");
}
