# fints-client

A FinTS 3.0 PIN/TAN client library in Rust. The design is in [docs/architecture.md](docs/architecture.md); [docs/README.md](docs/README.md) lists every document.

## Clean room

fints-client is MIT and stays MIT. Write every segment definition, flow and fix from the FinTS specification and from observed bank behaviour. Other FinTS clients under LGPL may be read to learn *behaviour*; never copy or port their code, tests or data, and add no LGPL or GPL dependency (`deny.toml` denies both). Third-party reference files are read from a local path at test time and never stored here: no element lists, snapshots or generated code.

Docs and code name no other FinTS client: state the bank or protocol fact and its source (spec section, live run). Two exceptions: `README.md` names the libraries that inspired fints-client and `NOTICE` and `LICENSE` carry the credits the MIT licence requires (fints-rs, the fork fints-client began as).

## Banks

A human runs every command that contacts a bank. Tests, examples and CI run offline against fixtures. End-to-end tests run against the synthetic bank in `testbank/` (`fints-client-testbank`). When a task needs a live result, stop and write down the exact command for the human to run.

Fixtures are synthetic or anonymised: no real names, IBANs, account numbers, user IDs, purposes or amounts. Logs mask PIN, TAN, user ID, system ID and account identifiers at the transport layer.

Bank-specific behaviour goes into a profile in `banks` and the bank's page under `docs/banks/` (index: [docs/banks.md](docs/banks.md)), never into a branch on the bank code inside `protocol` or `jobs`.

## Design rules

- **Steps.** Operations return `Step<T>` (`Done`, `NeedTan`, `NeedVopConfirmation`, `VopPending`) with a resume handle. The library never calls back into the caller.
- **Caller-owned state.** Session state and paused dialogs are versioned, serialisable values; the library writes no files.
- **Declarative segments.** A new segment version is a `#[derive(Segment)]` struct in `codec` plus a wire-string test.
- **Typed errors.** Bank return codes map to error and status variants.
- **No compatibility before 0.1.0.** Change APIs, trace and state formats freely; write no shims, serde defaults or parsers for old data.
- **Raw fallback.** An unparseable segment becomes `RawSegment` with its error, reported to the caller.

## Docs

`docs/` is the maintained description of what the code does; [docs/README.md](docs/README.md) is its index. After a notable change to behaviour, API, cargo features, scripts, trace or state format, bank profiles or architecture, run `/update-docs` ([.claude/skills/update-docs/SKILL.md](.claude/skills/update-docs/SKILL.md)): it maps the change to the documents it makes wrong. Keep `README.md` and this file short and put detail in `docs/`.

`docs/architecture.html` (published as the project page) and `docs/architecture.svg` show the components and how they call each other. They change only when a component is added, removed, renamed or wired differently; then update both, together with `docs/architecture.md`.

In the same change, add an entry to [CHANGELOG.md](CHANGELOG.md) under `Unreleased`, in its category (Added, Changed, Fixed, Removed, Security), for anything a user of the library would notice; say what changed and what to do about it. Leave released sections as they are: a version and date are set only by `/release`.

## Done

A change is done when `cargo test --workspace` (and with `--all-features`), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo clippy -p fints-client -- -D warnings` (default features, as a consumer builds it), `cargo clippy -p fints-client --no-default-features --features trace -- -D warnings`, `cargo fmt --all --check` and `cargo deny check` pass, and the docs it touches (banks, architecture) say what the code now does.
