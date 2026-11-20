//! Bank data formats.
//!
//! - Statement text: SEPA keywords in MT940/MT942 field `:86:`
//!   (DFÜ-Abkommen Anlage 3, Teil 5, §5.1.4) and the placeholders that mean
//!   "no value" (`docs/formats-design.md`).
//! - Payments: IBAN, BIC and SEPA character set checks ([`sepa`], public),
//!   the pain.001 builder and the VoP status report (pain.002) reader
//!   (internal to the transfer job; `docs/payments-design.md` section 5).

mod absent;
pub(crate) mod mt940;
pub(crate) mod pain001;
pub(crate) mod pain002;
pub mod sepa;
pub(crate) mod tag86;

pub(crate) use absent::absent;
