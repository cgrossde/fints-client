# Contributing

## Report a bank

The most useful contribution needs no code. Run the read-only check at your bank and open a [bank report](https://github.com/cgrossde/fints-client/issues/new?template=bank-report.yml) with the summary table it prints, whether the run worked or failed:

```sh
scripts/try-bank.sh --dry-run     # try the flow against the local test bank first
scripts/try-bank.sh <BLZ>         # the BLZ is characters 5 to 12 of your IBAN
```

[docs/adding-a-bank.md](docs/adding-a-bank.md) walks through it. Please attach the trace file as well: with it your bank becomes a test that runs on every change. The trace holds the messages of the run with the bank's parameters and return codes. It holds no PIN, TAN, user ID, name, IBAN, account number, counterparty, text or amount. It does show how many bookings you have, how many days apart and of which kind; the dates themselves are moved by a few days ([docs/trace-tooling.md](docs/trace-tooling.md)).

## Change code

Start with [docs/README.md](docs/README.md), which lists the documents from overview to detail, and [docs/architecture.md](docs/architecture.md). The working rules are in [AGENTS.md](AGENTS.md); they apply to people as well as to coding agents. The ones that matter most:

- **MIT only.** Write every segment definition, flow and fix from the FinTS specification ([docs/spec-sources.md](docs/spec-sources.md)) or from bank behaviour you observed. Do not copy or port code, tests or data from another FinTS client, and add no LGPL or GPL dependency.
- **No bank contact in tests.** Tests, examples and CI run offline, against fixtures or the synthetic bank in `testbank/`. Only a person runs a command that contacts a bank.
- **No personal data.** Fixtures are synthetic: no real name, IBAN, account number, user ID, purpose or amount.
- **Bank differences are data.** A bank that behaves differently gets a profile in `src/banks/` and a page under `docs/banks/` (index: [docs/banks.md](docs/banks.md)), not a branch on the bank code.
- **A new segment version** is a `#[derive(Segment)]` struct plus a wire-string test ([docs/codec.md](docs/codec.md)).

**Sign off your commits.** Every commit of a pull request carries a `Signed-off-by` line (`git commit -s`). With it you certify the [Developer Certificate of Origin](https://developercertificate.org): you wrote the change, or have the right to submit it, under the project's licence. CI rejects a pull request with an unsigned commit. There is no contributor agreement to sign.

Before you open a pull request, these must pass:

```sh
cargo test --workspace
cargo test --workspace --all-features
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p fints-client -- -D warnings
cargo clippy -p fints-client --no-default-features --features trace -- -D warnings
cargo fmt --all --check
cargo deny check
```

Update the documents your change makes wrong, and add a line to [CHANGELOG.md](CHANGELOG.md) under `Unreleased` when a user of the library would notice the change.

## Releases

A maintainer cuts a release with the `/release` skill ([.claude/skills/release/SKILL.md](.claude/skills/release/SKILL.md)). It makes the changelog match the commits, sets the version, and writes the release description.

## Security problems

Report them privately as [SECURITY.md](SECURITY.md) describes, not in a public issue.

## Licence

Contributions are licensed under the [MIT licence](LICENSE) of the project: what comes in has the licence of what goes out. The sign-off on each commit records that.
