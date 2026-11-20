# Adding a bank

fints-client follows the FinTS specification and each bank's own parameter data, so most banks work without any bank-specific code. This guide checks one bank and records the result. You need an online-banking access at the bank, a FinTS product ID ([registration](https://www.fints.org/de/hersteller/produktregistrierung)) and a Rust toolchain.

## 1. Try the flow without a bank

```sh
scripts/try-bank.sh --dry-run
```

This starts the synthetic test bank on 127.0.0.1, records a dialog with it and prints the summary table. No bank is contacted.

## 2. Run it at your bank

```sh
scripts/try-bank.sh sparkasse köln   # search the bank directory by name or city
scripts/try-bank.sh 37050198         # run: the BLZ is characters 5 to 12 of your IBAN
```

The script asks for the user ID and PIN with hidden input. Put the product ID into `.env` (copy `.env.example`) or type it when asked. The run logs in, reads balance and transactions of every account and the holdings of a depot, and ends the dialog. It sends no order. Confirm the login in your banking app, or type the TAN, when the bank asks.

Several failed logins can lock an online-banking access. If a run fails on the login, check the user ID and PIN before you repeat it.

## 3. Read the result

Every run prints a summary table: one line per message, with the orders sent, their versions and the bank's return codes.

```text
#00  HKIDN:2 HKVVB:3 HKSYN:3  | 3060 | HKVVB 0020, HKVVB 3920, HKSYN 0020
#02  HKIDN:2 HKVVB:3 +HKTAN(4,HKIDN)  | 3060 | HKVVB 0020, HKVVB 3920, HKTAN 3076
#03  HKSAL:7  | 0010 | HKSAL 0020  account #1
#04  HKKAZ:7  | 0010 | HKKAZ 0020  account #1
```

The table holds no personal data: no name, account number, amount or booking text. The full recording is `testbank/fixtures/replay/local/<bank>-<BLZ>.trace.json`, masked before it is written (step 4 says what it holds). Git ignores that folder, so the file goes nowhere unless you send it. Print the table again at any time:

```sh
cargo run -q --features trace --example trace_summary -- testbank/fixtures/replay/local/<bank>-<BLZ>.trace.json
```

## 4. Tell us

Open a [bank report](https://github.com/cgrossde/fints-client/issues/new?template=bank-report.yml) with:

- the bank's name and BLZ, and the date of the run;
- the summary table;
- what worked and what did not: login, TAN method (app approval or typed), balances, transactions, holdings;
- the trace file, if you like. Please do: it is what turns your bank into a test that runs on every change.

**What the trace holds.** The messages of the run: which orders the client sent in which version, the bank's parameters, and its return codes and their texts. Everything that identifies you is replaced before the file is written, and the script searches the finished file for your user ID, PIN, product ID, IBANs, account numbers and name, and writes nothing if it finds one.

| In the trace | Not in the trace |
|---|---|
| the bank's parameters, segment versions and return codes | PIN, TAN and user ID |
| how many bookings you have, how many days apart, and the kind of each (card payment, direct debit, ...) | your name, IBANs and account numbers |
| how many lines each purpose text has | counterparties: their names, IBANs and banks |
| how many accounts of which kind you have | purpose and booking texts |
| | amounts and balances: every amount reads `#,##`, and no balance shows as negative |
| | the real dates: all dates are moved by the same few days |

The file is plain JSON, so you can read it before you attach it. [trace-tooling.md](trace-tooling.md) has every masking rule.

**Unknown segments.** If your bank sends a segment fints-client does not know, the script shows it to you as it was sent and asks whether the trace may keep it readable. Say yes if you see no personal data in it; such a segment is usually what we need to support the bank. If you say no, the trace keeps its header and field lengths.

A bank that works gets a line in the README table and a page in `banks/`, linked from [banks.md](banks.md).

## When the run fails

A failed run is the most useful report. The recording is kept as `<bank>-<BLZ>.failed-<time>.json` beside the trace, and the summary table marks the failing message with `FAILED` and the bank's 9xxx code and text. Send the table as above, and attach the recording if you can: it is masked like any trace. From it a maintainer can usually tell which of these applies:

| What the table shows | What it means | Where the fix goes |
|---|---|---|
| `9075` on an order, then a retry with HKTAN that works | the bank's parameter data understates its TAN rule; the client learns it | nothing to fix |
| `9010`, `9210` or `9110` on one order | the bank rejects a field or a segment version | a rule in `jobs`, or a profile field ([bank-profiles-design.md](bank-profiles-design.md)) |
| the login fails with `9942`, `9210` or `9340` | the bank does not accept the PIN, the user ID form or the signature | check the credentials first; else often a user ID rule in the bank's profile (`src/banks/table.rs`) |
| a segment is reported as unreadable | the bank sends a version or layout the codec does not define | a segment definition ([codec.md](codec.md)) or a quirk |

## Making the bank a regression test

For maintainers, once a trace may be committed:

1. Review the trace ([trace-tooling.md](trace-tooling.md), "Before committing a trace") and move it to `testbank/fixtures/replay/`. `cargo test --test replay_test` then replays it on every run: a change in what the client sends to this bank fails the test.
2. Where the bank deviates from the generic behaviour, add a profile in `src/banks/table.rs` with the observation from the bank's page it rests on.
3. Optionally model the bank in `testbank/src/profiles.rs`. `tests/bank_profiles_test.rs` checks the model's parameter data against the trace.
