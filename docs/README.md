# Documentation

This folder describes what fints-client does today and why it is built this way; what changed over time is in the [changelog](../CHANGELOG.md). Read from the top: each level assumes the one before it.

## 1. Use the library

| Document | What it holds |
|---|---|
| [../README.md](../README.md) | What fints-client does, a quick start, the tested banks |
| API docs (`cargo doc --open`) | Every public type; start at the module `jobs` |
| [../examples/sync_accounts.rs](../examples/sync_accounts.rs) | A complete run: login, TAN, accounts, stored session state, a paused dialog |
| [banks.md](banks.md) | The tested banks, with a page per bank on what it does that the specification does not say |

## 2. Understand the design

| Document | What it holds |
|---|---|
| [architecture.md](architecture.md) | The layers, the rules each one follows, how the library is tested |
| [architecture.html](architecture.html) | The same structure as an interactive page, published at <https://cgrossde.github.io/fints-client/> |
| [jobs-step-design.md](jobs-step-design.md) | `Step<T>`, resume handles, TAN and wrong-TAN handling, paging |
| [session-state-design.md](session-state-design.md) | `SessionState` between dialogs, `PausedDialog` inside one |
| [payments-design.md](payments-design.md) | Transfers, instant transfers and the payee check |
| [return-codes-design.md](return-codes-design.md) | From bank return codes to typed errors and notices |

## 3. Work on a layer

| Document | What it holds |
|---|---|
| [codec.md](codec.md) | How to add a segment version; the derive attributes |
| [bpd-upd-design.md](bpd-upd-design.md) | The typed model of bank and user parameter data |
| [version-negotiation-design.md](version-negotiation-design.md) | Which segment version is sent, and the error when none fits |
| [formats-design.md](formats-design.md) | MT940, MT942 and the SEPA fields of `:86:` |
| [bank-profiles-design.md](bank-profiles-design.md) | Bank profiles of the library and of the test bank |
| [spec-sources.md](spec-sources.md) | Which specification documents the code is written from, and where to get them |

## 4. Test with banks

| Document | What it holds |
|---|---|
| [adding-a-bank.md](adding-a-bank.md) | Try a new bank with one script, and what to send us |
| [trace-tooling.md](trace-tooling.md) | Traces: format, masking, replay, the threat model |
| [../testbank/README.md](../testbank/README.md) | The synthetic test bank: standalone server, in-process use, replay |

## Project

| Document | What it holds |
|---|---|
| [../CHANGELOG.md](../CHANGELOG.md) | What changed for users, by release |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | How to report a bank and how to change code |
| [../SECURITY.md](../SECURITY.md) | How to report a security problem, and what the library protects |
| [../AGENTS.md](../AGENTS.md) | Working rules for coding agents and people |
