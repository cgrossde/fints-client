# Changelog

What changed for users of the library, newest first. New entries go under `Unreleased`; they move under a version and date when that version is released. How the library works today is in [docs/](docs/README.md).

## Unreleased

### Fixed

- The warning "HNHBK states N bytes, the answer has M bytes" no longer fires on every encrypted answer. The size in HNHBK counts the message with its PIN/TAN envelope, but the check counted the answer after the envelope was opened, so every answer looked 100 to 150 bytes short. The check now runs on the answer as it arrived (in `transport`), and the warning's log target moves from `fints_client::protocol` to `fints_client::transport`.

## 0.1.0 — 2026-11-23

Initial release. Validated live at DKB, comdirect and Consorsbank: login, balances and transactions of every account, a SEPA transfer, and the payee check where the bank offers it (DKB, comdirect); holdings at comdirect and Consorsbank.
