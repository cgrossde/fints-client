//! # fints-client-testbank — an offline synthetic FinTS bank
//!
//! A FinTS 3.0 PIN/TAN bank for fints-client's end-to-end tests. It runs in
//! the test process, over loopback HTTP, and holds only synthetic data. No
//! test using it contacts a real bank.
//!
//! What it simulates:
//!
//! - dialog initialisation and synchronisation (HKIDN, HKVVB, HKSYN):
//!   system IDs it issues (`TESTSYS0001`, …), dialog IDs (`TESTDLG0001`,
//!   …), message numbering, BPD and UPD sent when the client's versions are
//!   older, allowed TAN methods (3920);
//! - the TAN step of the login and of business orders (HKTAN process 4,
//!   then 2 or S): SCA exemption (3076 with the dummy HITAN), decoupled
//!   approval after a number of polls (3955, 3956), typed TANs (0030),
//!   wrong TANs kept for another try (9941, 9360) or ending the order
//!   (9210) or the dialog (9800) ([`WrongTanModel`]), and a missing HKTAN
//!   (9075);
//! - balance (HKSAL 5–7), transactions (HKKAZ 5–7, MT940, touchdown 3040)
//!   and depot holdings (HKWPD 5–6, MT535 as given);
//! - payments (HKCCS, HKIPZ, HKIPS) with the payee check (HKVPP, HKVPA):
//!   see [`PaymentsModel`] and [`VopModel`], [`BankHandle::payments`];
//! - wrong PIN (9942), unknown system ID (9391), changed user ID (3072),
//!   account not allowed (9210), dialog end (0100).
//!
//! Everything the bank does is a field of [`BankModel`]; [`TestBank::on`]
//! replaces the answer to one segment and the [`tamper`] helpers break an
//! answer on purpose. A request that breaks a message rule (wrong numbering,
//! unreadable segment, a job version the BPD does not offer, …) is a
//! [`Violation`]: it is recorded and answered with HTTP 500.
//! [`Script`] serves canned answers in a fixed order instead, and
//! [`replay::Replay`] plays back a recorded (masked) dialog and compares the
//! client's requests with the recorded ones.
//!
//! Clean room: written from the FinTS 3.0 specification (Formals, PIN/TAN,
//! Messages) and our own tests. Answers are built with the segment
//! definitions of `fints_client::codec`; the self-tests compare them with
//! wire strings written by hand from the specification, so a codec error
//! does not hide on both sides.

pub mod analysis;
mod engine;
mod fault;
mod http;
pub mod model;
pub mod profiles;
pub mod replay;
mod request;
mod response;
mod script;
pub mod statement;
pub mod tamper;

pub use engine::{BankHandle, TestBank};

/// "Now" of the clients this crate builds (`BankClient::with_now`): the
/// synthetic bookings are dated in 2026, and no test may depend on the real
/// clock.
pub const TESTBANK_NOW: jiff::civil::DateTime = jiff::civil::datetime(2026, 6, 15, 12, 0, 0, 0);
/// The date of [`TESTBANK_NOW`].
pub const TESTBANK_TODAY: jiff::civil::Date = jiff::civil::date(2026, 6, 15);
pub use fault::Fault;
pub use http::HttpBank;
pub use model::{
    Account, BankModel, Booking, CheckOrder, Customer, Delivery, InstantStatusModel, JobSpec,
    LimitAnswer, LoginSca, Mt942Model, OnMatch, Payee, Payment, PaymentsModel, ResultAs, TanKind,
    TanMethod, TanRule, VopModel, WrongTanEnd, WrongTanModel, date, iban_de,
};
pub use request::{Request, Violation, latin1};
pub use response::{Answer, Feedback, Reply};
pub use script::{HttpScript, Script};
