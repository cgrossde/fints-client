# Security

fints-client handles online-banking credentials and can send payments, so we treat security reports with priority.

## Reporting a problem

Report it privately: on the repository's GitHub page open **Security**, then **Report a vulnerability**. Do not open a public issue, and do not attach a trace of a real account, a PIN, a TAN or a product ID.

Tell us what you observed, the version or commit, and how to reproduce it against the bundled test bank if you can ([testbank/README.md](testbank/README.md)).

Only the latest release gets fixes.

## What counts

- A PIN, TAN, user ID, product ID, account identifier, name or amount that reaches a log line, an error message or a trace file unmasked.
- A way to recover personal data from a masked trace ([docs/trace-tooling.md](docs/trace-tooling.md) describes what the masking promises and what it does not).
- A transfer that is sent with other data than the caller gave, sent twice, or sent without the caller confirming a payee-check warning.
- A bank answer that makes the library panic, loop or treat a failed order as done.
- A weakness in how the TLS connection to the bank is made.

## What the library does and does not protect

- **Logs and traces.** PIN, TAN, user ID, system ID and account identifiers are masked at the transport layer before a log line or trace is written.
- **Stored state.** `SessionState` and a `PausedDialog` are plain values the caller stores. They hold bank parameters, the system ID and account data, and a paused dialog can hold an open order. The library does not encrypt them: the application must.
- **Credentials.** The PIN lives in a `Pin` value that is never printed or serialised. The library does not store it; where it is kept between runs is the application's decision.
- **Transport.** HTTPS with rustls and the Mozilla root certificates. A caller that supplies its own transport takes over this responsibility.
