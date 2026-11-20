---
name: update-docs
description: Bring fints-client's docs and CHANGELOG.md in line with changed code. Use after a notable change to behaviour, API, cargo features, scripts, trace or state format, bank profiles or architecture.
---

# Update the fints-client docs

`docs/` describes what the code does today; `CHANGELOG.md` records what changed for users. This skill finds the documents a change made wrong, proposes the edits, and writes them after one approval. It reads source and docs only: it runs no build, test, formatter or generator.

## 1. Pick the scope

- **A prompt was given** (`/update-docs the freeze format`): that is the scope. Inspect the named source and docs plus the indexes and links they need. Report unrelated drift you notice; leave it unedited.
- **No prompt, working tree has changes:** the scope is every staged, unstaged, untracked and deleted file (`git status --porcelain`, `git diff HEAD`). Sort them into source, tests, docs, guidance and unrelated.
- **No prompt, clean tree:** ask the human to choose between a **full sweep** of the repository and the **last five commits**, then inspect that.

Done when you can list the changed behaviours in one line each.

## 2. Map each change to its documents

| Changed | Check |
|---|---|
| `src/jobs.rs`, `src/jobs/` | `docs/jobs-step-design.md`, `docs/payments-design.md` (transfers, payee check), `docs/session-state-design.md` (`paused.rs`), `docs/architecture.md` section 2.6, the operations table in `README.md` |
| `src/banks/` | the bank's page in `docs/banks/`, the table in `docs/banks.md`, `docs/bank-profiles-design.md`, the tested-banks table in `README.md` |
| `src/codec/`, `derive/` | `docs/codec.md`, `docs/spec-sources.md`, the segment count in `README.md` |
| `src/protocol.rs`, `src/protocol/` | `docs/bpd-upd-design.md`, `docs/version-negotiation-design.md`, `docs/return-codes-design.md` |
| `src/session.rs` | `docs/session-state-design.md` |
| `src/return_code.rs`, `src/error.rs` | `docs/return-codes-design.md` |
| `src/formats/`, `src/camt.rs`, `src/segments/mt535.rs`, `mt536.rs` | `docs/formats-design.md`, `docs/payments-design.md` (pain.001, pain.002) |
| `src/quirks/` | the module docs in `src/quirks/tolerant.rs`, `docs/architecture.md` |
| `src/trace.rs`, `src/mask.rs`, `src/mask/` | `docs/trace-tooling.md`, `SECURITY.md` ("What the library does and does not protect") |
| `testbank/` | `testbank/README.md`, `docs/bank-profiles-design.md` (test bank profiles), `docs/trace-tooling.md` (replay) |
| `scripts/`, `.env.example`, `examples/` | `docs/adding-a-bank.md`, `testbank/README.md`, the "Adding a bank" section of `README.md` |
| Cargo features, public re-exports in `src/lib.rs` | the crate docs at the top of `src/lib.rs`, the features line and quick start in `README.md`, `docs/architecture.md` section 2.6 |
| A layer added, removed or re-wired | `docs/architecture.md`, `docs/architecture.html` (the boxes in its markup, the component list and the three flows in its script), `docs/architecture.svg` (the same boxes), the mermaid diagram in `README.md` |
| A document added, removed or renamed | `docs/README.md` lists every page and no absent one |
| The checks a change must pass | the "Done" section of `AGENTS.md` and the command list in `CONTRIBUTING.md` |
| This table went stale | this file |

Every change a user of the library would notice also gets an entry in `CHANGELOG.md` under `Unreleased`, in its category (Added, Changed, Fixed, Removed, Security), saying what changed and what to do about it. Refactors, tests and wording get none.

Done when every changed behaviour has its documents named, or a stated reason why none applies.

## 3. Rules for the edits

- Describe the code as it is, not a plan or wish.
- Name no other FinTS client: state the bank or protocol fact and its source (spec section, live run with date). Only the "Inspired by" section of `README.md`, `NOTICE` and `LICENSE` carry such names.
- Examples are synthetic: no real name, IBAN, account number, user ID, purpose or amount. Use the test bank's values (BLZ `12345678`, `testuser1`, `DE00…` test IBANs).
- Mark what no live run confirmed as unverified, as the pages in `docs/banks/` do.
- Keep `README.md` and `AGENTS.md` short; detail belongs in `docs/`.
- When source, tests and docs contradict each other and the source does not settle which is intended, show the human the contradiction and ask. If settling it needs a command that builds, tests or contacts a bank, name the command and what it would show, and ask first. A command that contacts a bank is always run by the human.

## 4. Propose, then write

Show a table of every file to change with the claim that changes in each, and wait for one explicit approval. Then make exactly those edits.

Done when the human approved the table as shown.

## 5. Report

Re-read each changed document and report: the files changed, the claims changed in each, drift you saw outside the scope, and questions still open. Leave committing to the human or the calling task.
