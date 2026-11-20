# fints-client-testbank

Offline synthetic FinTS 3.0 PIN/TAN bank for fints-client's tests (see `src/lib.rs`). Synthetic data only; `publish = false`. It is an unpublished workspace member and a path-only dev-dependency of `fints-client` (no version, so `cargo publish` drops it). Use it from `tests/` only: unit tests in `src/` compile a second copy of the library whose types do not match.

## Standalone test bank

Run it as a local server (127.0.0.1 only) to try the library, examples and apps by hand without a real bank:

```text
cargo run -p fints-client-testbank --bin fints-testbank -- --tan decoupled
```

Or install it once and run `fints-testbank` from anywhere:

```text
cargo install --path testbank --locked
fints-testbank --tan decoupled
```

Options (all optional):

- `--port N`: port, default 47331, 0 picks a free one.
- `--tan decoupled|typed|none`: login behaviour. `none` (default) is exempt from SCA, `typed` asks for the TAN, `decoupled` waits for an approval.
- `--polls N`: with `decoupled`, the number of polls before the bank approves (default 2).

On start it prints the URL, bank code, user ID, PIN, the TAN (typed) and the accounts. Protocol errors of the client are printed to stderr as `VIOLATION ...` while it runs. Ctrl-C ends it.

Pointing the client at it: build fints-client with the `testbank` feature and use bank code `12345678` like any known bank (`fints_client::bank_by_blz`); it resolves to `http://127.0.0.1:47331/fints`. Without the feature, or for another port, build a `BankConfig` with the printed URL:

```rust
let client = fints_client::jobs::BankClient::new(
    fints_client::BankConfig::new(
        "Testbank", "12345678", "TESTDEFFXXX", "http://127.0.0.1:47331/fints",
    ),
    fints_client::ProductId::new("TEST-123"),
);
```

An app turns the feature on in development builds only, e.g. `fints-client = { version = "…", features = ["testbank"] }` under a dev profile or its own `dev` feature.

## Profiles

Each profile is a `BankModel` built from a live run, with the real bank code (public) and synthetic customer, accounts and bookings:

- `profiles::consorsbank()` (76030080): its SCA rules, national KTI, a depot without IBAN, a dialog-only UPD (`upd_version` 0 sends the UPD in every dialog init).
- `profiles::dkb()` (12030000): decoupled login, HKSAL 8 and HKCAZ 1 (camt) with HKTAN answered 3076, a credit card (Kontoart 50) without IBAN.
- `profiles::comdirect()` (20041155, one of eight codes of the host): no Kontoart, a depot recognised by its allowed jobs, HKSAL 5, HKCAZ 1, HKWPD 5, and `foreign_blz_9210` for a bank code that does not match the user.

The evidence and behaviour tests are in `tests/bank_profiles_test.rs`. Profiles are for in-process tests and never reach the network. The binary serves one under the test bank code: `fints-testbank --profile consorsbank` listens as 12345678.

## Model features

`BankModel` answers init and sync, system IDs, BPD/UPD by version, SCA exemption, decoupled and typed TANs for login and orders, 9075 when HKTAN is missing (`TanRule::RequiredExempt` for the retry), HKSAL 5-7, HKKAZ 5-7 with touchdown, HKCAZ, HKWPD 5-6, HKSPA, HKWDU, payments, the payee check and instant orders, wrong typed TANs (`BankModel::wrong_tan: WrongTanModel`: `kept` wrong TANs answered 9941 with the order kept, optionally with 9360 and the attempts left, then `WrongTanEnd::OrderDeleted` (9941 + 9210, dialog open) or `WrongTanEnd::DialogEnded` (9941 + 9800); default 2 kept, then deleted), and the error codes 9942, 9391, 3072 and 9210. Every request is checked strictly: a broken message rule is a recorded violation (`assert_clean`). Tamper helpers exercise the four tolerant quirks, and `Script` gives exact answer sequences.

- **Faults.** `Fault` (`Status`, `Silence`, `CutWire`, `CutBase64`, `Utf8Text`, `InvalidBase64`) is armed once with `TestBank::fault` or `BankHandle::inject`. `abort_dialog_on` answers the nth segment with 9800. The client must fail with a typed error and leave resumable state. An answer without HIRMG or with an unreadable HNVSD is an error.
- **MT942.** `BankModel::mt942: Option<Mt942Model>` (default `None`: no MT942 part) answers HKKAZ with `Account::pending` as an MT942 part. Page `n` carries the `n`-th slice of the booked and of the pending list, each pending slice as its own MT942 statement; paging goes on while either list has more. `Mt942Model` sets the creation time (`:13D:` with offset or `:13:` without), `miscount` (`:90D:` one too high) and `missum` (a wrong sum with right counts).

## In-process use

No server is needed in tests: `TestBank::new(model).handle()` gives the `BankHandle`, and `handle.client(&product)` a `jobs::BankClient` wired to it (`BankHandle` implements `fints_client::transport::Transport`, the URL is ignored). HTTP-level faults do not apply in-process; a protocol violation of the client is answered as HTTP 500, like the server does.

```rust
let bank = TestBank::new(BankModel::default()).handle();
let client = bank.client(&ProductId::new("TEST-123"));
// ... jobs::open(&client, ...), jobs::balance, ... or replay::standard_run ...
bank.assert_clean();
```

## Replay of recorded dialogs

`replay` (`src/replay.rs`) turns one real dialog into an offline regression test. A human records a dialog at a bank once; a masked trace is replayed from `fixtures/replay/` (live recordings stay in the gitignored `fixtures/replay/local/` until a human has reviewed them, see `docs/trace-tooling.md`); from then on `tests/replay_test.rs` plays the bank's side back to the client and checks that the client still sends structurally the same requests.

### Recording (a human runs this; it contacts the bank)

The library writes no files. `fints_client::trace::Recorder` (feature `trace`) is a `Transport` decorator: `bank.with_transport(recorder.wrap(Arc::new(HttpTransport::new()?)))` records every exchange that goes through it, whatever thread it runs on, with the wire message (PIN/TAN envelope included, after outgoing quirks) masked as it is recorded (see "What a trace keeps" below); a failed send is recorded with a short `error` (the replay answers it with HTTP 500). The transport's `TRACE` event (`fints_client::mask::EXCHANGE_TARGET`) is separate and for log consumers. `examples/record_trace.rs` wraps the transport like that, runs `replay::standard_run` (first login with synchronisation and TAN, balance and transactions for every account, holdings for a depot, dialog end) and writes the trace as JSON (`Trace::to_json`).

Try it first against the local test bank (no bank contact): start `cargo run -p fints-client-testbank --bin fints-testbank -- --tan decoupled`, then

```text
FINTS_URL=http://127.0.0.1:47331/fints FINTS_BLZ=12345678 FINTS_USER=testuser1 FINTS_PIN=test-pin-1 \
  FINTS_PRODUCT_ID=TEST-123 cargo run --features trace --example record_trace -- target/dry.trace.json
```

Then, once per bank, run `scripts/try-bank.sh <BLZ>` (it asks for user ID, PIN and product ID with hidden input, so they stay out of the command line and shell history; the product ID prompt is skipped when `FINTS_PRODUCT_ID` is set or a `FINTS_PRODUCT_ID=` line is in the git-ignored `.env`; copy `.env.example` to start one). Set `FINTS_TRACE_SALT` there too (`openssl rand -hex 32`) to compare traces recorded on different days: their references then share pseudonyms. The BLZ is the one from your IBAN (characters 5–12); a bank can have several (comdirect has 20041111 to 20041199). Traces go to `testbank/fixtures/replay/local/<bank>-<BLZ>.trace.json`, which git ignores. A failed run never replaces that file: it is kept as `<bank>-<BLZ>.failed-<time>.json` beside it (not replayed). Confirm the login in the app when asked:

```text
scripts/try-bank.sh 12030000   # DKB, the last 30 days of transactions
scripts/try-bank.sh 20041111   # comdirect
scripts/try-bank.sh 76030080   # Consorsbank
scripts/try-bank.sh sparkasse     # search the directory for a BLZ
```

Before writing, the example searches the trace for the user ID, PIN, product ID and every IBAN, account number and holder name of the run, and writes nothing if one is found. Read a recorded trace as a table for humans (offline): `cargo run -q --features trace --example trace_summary -- testbank/fixtures/replay/local/12030000.trace.json` prints a legend line, the accounts, then one line per exchange (jobs, HKTAN, return codes, `FAILED` on any 9xxx code or transport error, `retry of #NN`, the account alias). `record_trace` prints the same table after every run, successful or not. Replay the local traces with `REPLAY_LOCAL=1 cargo test --test replay_test` (the default test replays only `fixtures/replay/*.trace.json`; files with another extension, such as `.trace` text files, are ignored).

#### Before committing a trace

- Traces stay in `fixtures/replay/local/` until a human has reviewed them (`docs/trace-tooling.md`); then move the file to `fixtures/replay/`.
- Read `cargo run -q --features trace --example trace_summary -- <file.trace.json>` first: it shows the story (which jobs, which codes) without the raw messages.
- Grep the `.trace.json` file for your name, IBAN and account number fragments, and amounts.
- Read every HIRMG/HIRMS text: they are kept as the bank sent them, and a bank may put a name or amount there.
- Check that unknown segments are starred (everything after the header).
- Run `REPLAY_LOCAL=1 cargo test --test replay_test`.

### Live transfer trace (a human runs this; it moves real money)

`scripts/try-bank.sh <BLZ> --transfer` (`examples/transfer.rs`, feature `trace`) sends one SEPA transfer at a real bank and records the dialog. It asks for user ID and PIN (hidden; product ID as in `try-bank.sh`), then, visibly, for the payee name, IBAN, BIC (optional), amount, purpose and instant choice (`no`, `prefer`, `require`). The example logs in, lists your accounts with an IBAN to pick the debtor from, and runs `jobs::transfer`: a payee check that is not a clean match is shown (result per payee, registered name, the bank's explanation) and asks `confirm? [y/N]`; a TAN is typed (hidden) or approved in the app and confirmed with Enter. It prints the receipt. **This sends the money.** The amount is capped at 1.00: the script and the example refuse more unless `FINTS_ALLOW_AMOUNT` is set; use your own account at another bank as the payee.

```text
scripts/try-bank.sh 12030000 --transfer   # the BLZ is characters 5 to 12 of the IBAN you send from
```

The trace goes to `fixtures/replay/local/<bank>-<BLZ>.transfer.trace.json` (git ignores it); a failed run is kept as `<bank>-<BLZ>.transfer.failed-<time>.json`, a transfer cancelled at the login or the payee check as `<bank>-<BLZ>.transfer.cancelled-<time>.json`. As in `try-bank.sh`, a segment fints-client does not know is shown and asked about before the trace is written, and the trace records how the run ended (`client.outcome`: `completed`, the kind of error, `cancelled` or `stopped`). `tests/replay_test.rs` skips `*.transfer.trace.json`: the replay runs the standard run, which sends no payment. Masking of payment segments: the debtor account, the instant order ID, VOP-ID, polling ID and the payee IBAN in HIVPP get pseudonyms; the touchdown point and the payee's names, reasons and explanation text are starred or shown as their shape; the pain.001 and pain.002 are shown as their shape (markup, status and service level codes and currency stay; names, BICs, dates, purpose and amounts do not), with pseudonyms for IBANs, account numbers, `MsgId`, `PmtInfId`, end-to-end IDs, mandate references and creditor IDs. Before writing, the example searches the trace for the credentials, your account identifiers, the payee name, IBAN, purpose and any registered name; if one is found the trace is not written there but kept as `<name>.leak` beside it (not replayed) for masking by hand. Account numbers count only where no digit touches them, so one is not found inside a longer number. Read the HIRMS texts of the trace by hand all the same: a bank may put a name there.

### `.expect` and bless

When the client's requests change on purpose (a segment version, a KTI form) a recorded trace no longer matches them, and the recorded answer was given to the old request. The trace stays as it is; a sidecar `<name>.expect` next to `<name>.trace.json` lists the differences that are expected. `#` lines are comments (the first one names the reason); every other line is a `Violation` as it prints: `<exchange> <message>`, e.g. `7 HKKAZ:3 data element 1: 2 group elements recorded, 6 sent`. `every_trace_in_the_fixture_directory_replays_as_expected` compares the actual violations with the lines as an exact multiset, for `fixtures/replay/` and, with `REPLAY_LOCAL=1`, for `fixtures/replay/local/`. A new difference fails with an expected-vs-actual list (`-` expected but gone, `+` new, each with the summary line of the exchange); an expected difference that went away fails too, so a sidecar never goes stale. The exchange count and order and the success of the run are always checked.

`REPLAY_BLESS=1 cargo test --test replay_test` rewrites the sidecars from the actual violations (comment lines kept; a sidecar of a clean run is deleted). It refuses, and the test fails, when the run went out of step (`the client stopped after`, `the client sent one more`, or more `segments differ` lines than the trace has job exchanges): move that trace to `fixtures/replay/stale/` (not replayed) and re-record it. A bless commit must explain each changed line of the `.expect` diff, because a real regression looks the same. To clear a sidecar, re-record to the same path (`record_trace` reminds you when `<out>.expect` exists) and delete it once the new trace replays clean.

### What a trace keeps, and how masking is handled

A trace (`fixtures/replay/*.trace.json`) is one pretty-printed JSON document (`Trace::to_json`, `Trace::from_json`):

```json
{
  "format": "fints-client-trace/4",
  "comment": ["Recorded with examples/record_trace.rs at BLZ 12345678, …"],
  "client": { "version": "0.1.0", "outcome": "completed" },
  "bank": { "blz": "12345678", "bic": "TESTDEFFXXX" },
  "accounts": [
    { "alias": "#1", "class": "checking", "kontoart": 1, "jobs": ["HKSAL", "HKKAZ"] }
  ],
  "exchanges": [
    {
      "index": 5,
      "account": "#1",
      "retry_of": null,
      "sent": { "jobs": ["HKSAL:7"], "tan": { "process": "4", "for": "HKSAL" } },
      "answer": {
        "message": [{ "code": "3060", "text": "Bitte beachten Sie …" }],
        "segments": [{ "job": "HKSAL", "code": "0020", "text": "Auftrag ausgeführt" }]
      },
      "failed": false,
      "error": null,
      "notes": [],
      "request": ["HNHBK:1:3+000000000389+300+DIALOG0002+4'", "…", "HKSAL:3:7+**********************:TESTDEFFXXX+N'", "…"],
      "response": ["HNHBK:1:3+000000000483+300+DIALOG0002+4+DIALOG0002:4'", "…"]
    }
  ]
}
```

- `request` and `response` are the raw messages, one string per segment: a segment ends at a `'` that is not escaped (`?'`) and not inside binary data (`@len@…`); the binary data of HNVSD is split too, so the HNVSD header up to its `@len@` is one string (`"HNVSD:999:1+@207@"`), each inner segment gets its own, and the closing `'` of HNVSD is a string of its own. Each ISO 8859-1 byte is the one Unicode character of the same value; joined, the strings are the bytes again. No base64: binary content is masked (statements by shape, everything else by stars). A failed send has `error` (HTTP status or transport error; the replay answers it with HTTP 500) and an empty `response`.
- The raw `request`/`response` are the only source of truth. Everything else in an exchange except `account`, `error` and `notes` is derived for readers: `index` (position from 0), `sent.jobs` (every non-envelope segment except HKTAN, as `CODE:version`), `sent.tan` (HKTAN's process and the segment it is for), `answer.message` (HIRMG codes and texts), `answer.segments` (HIRMS codes and texts with the job they refer to), `failed` (a transport error, an unreadable response or any 9xxx code), `retry_of` (the index of the exchange before, when that one was answered with 9075 and this one sends the same jobs again, with HKTAN), and `bank` (`blz` from the first HKIDN, `bic` from the first KTI of a job). `Trace::from_json` recomputes them and rejects a file whose derived fields differ, naming the exchange and the field (`exchange 5: derived field sent.jobs[0] is "HKSAL:6" in the file, but the recorded request and response give "HKSAL:7"`): edit the raw messages, not the derived fields. It also rejects unknown fields, another `format`, and an `account` that `accounts` does not list.

`accounts` lists every account the trace names, once; an exchange's `account` is the alias of the account its request is about (or null). The `Recorder` numbers the accounts in the order they first appear in a HIUPD (`#1`, `#2`, …) and gives each its class (from the Kontoart, or `depot` when there is none and the jobs allow HKWPD and none of HKSAL, HKKAZ, HKCAZ, else null), its Kontoart and the jobs the UPD allows; never the product name, number, IBAN or holder. It reads the unmasked HIUPD and the account element (data element 1, KTI or KTV) of HKSAL, HKKAZ, HKCAZ, HKWPD, HKWDU, HKSPA, HKCCS and HKIPZ as they pass, and keeps only keyed hashes of IBAN and of number (leading zeros dropped), subaccount and BLZ, with a random key per recorder that is never written. The summary table (`trace_summary`, `record_trace`) lists the accounts under the legend and ends each exchange line with the alias; `record_trace` prints the account next to each failed fetch.

`client` says which fints-client version recorded the trace and how the run ended: `completed`, or the kind of error that stopped it or a job (`Bank 9010`, `SegmentUnreadable InvalidDate at booked[0]`), never an error text. `notes` of an exchange say what the client made of the answer, without any value of it: a segment its definition could not read (code, version, kind of error, field) and statements with characters outside the SWIFT character set (a count). The recorder writes both from the answer before it is masked.

Masked by the library (`src/mask.rs`), with stars: PIN and TAN (always eight, whatever their length), and one `*` per character for subaccounts, the TAN challenge, every unknown segment after its header (unless the person recording kept it, see "Unknown segments" in [docs/trace-tooling.md](../docs/trace-tooling.md)), every binary element other than a statement, a pain message or an ID (keys, certificates).

**Aliases and pseudonyms** (`src/mask/pseudonym.rs`, trace only): identity values get a stand-in, equal values the same one within a trace (the user's own identifiers and dialog IDs in their own length and character classes; a counterparty's IBAN, a creditor ID and every reference in one fixed form each), so a trace shows that two values are equal without showing either. Account identifiers get an alias: the user's own IBANs (country and bank code kept, account part from the alias, check digits valid), IBANs of anyone else (always `DE`, valid check digits, bank code `00000000` and ten digits of the alias: a counterparty's country, bank and IBAN length would say who it is), account numbers (leading zeros kept; the same digits as the account part of the IBAN), user and customer ID, creditor IDs. An alias is a number given in order of first appearance in the trace, from a random start per recorder; it has no link to the value, and the table lives only in the recorder's memory. References and IDs get a keyed pseudonym: system ID, booking references (camt `AcctSvcrRef`, MT940 `:61:` after `//`), end-to-end IDs, pain `MsgId` and `PmtInfId`, mandate references, the instant order ID, VOP-ID and polling ID. `record_trace` and `transfer` use `Recorder::new()`: a random salt per trace that is never written, so their traces link to no other trace. A consumer that passes its own salt (`Recorder::with_salt`) links the references of its traces, never their accounts. The full list, the alias scheme, the derivation and the threat model are in `docs/trace-tooling.md` (section "Aliases and keyed pseudonyms"). `cargo run -q --features trace --example trace_identities -- fixtures/replay/local/*.trace.json` (from the repository root, with the `testbank/` prefix) reports repeated booking references, pending entries booked later (across traces of one salt) and echoed end-to-end IDs, as counts, aliases and pseudonyms. Anonymised on top by `Trace::from_exchanges`: dialog IDs and order references become `DIALOG0001`, `TASK0001`, …, the product registration in HKVVB becomes stars. Traces are anonymous: no account number, IBAN, name, amount, transaction text or product name survives. Kept: segment codes, versions, numbers and element structure, return codes and texts, BPD (public bank parameters, bank code, BIC), TAN process steps. Dates are moved by 3 to 30 days per trace.

**Shape masking** (`src/mask/shape.rs`): values lose their content and keep only what a parser needs to see.

- **Amounts** become `#,##` (`#.##` in XML), whatever their size and decimals: `1234,56`, `100,` and `0,5` look the same, so neither the size of an amount nor a round sum shows. The debit mark of a balance is written as credit.
- **Names** (holder, user, product, counterparty, security, address lines) become eight `#`, whatever their length. A name longer than its format allows becomes limit + 1 characters, so a bank that breaks the limit stays visible.
- **Other free text** (purposes, booking texts, references without a stand-in) becomes eight `#` for each run of it, up to the next subfield code, keyword, tag or line break. Its length does not show; the subfields and line breaks do, because how a bank splits a purpose over MT940 subfields is what a parser must cope with.
- **Dates** are moved by the same 3 to 30 days everywhere in a trace, and a time of day is noon (`docs/trace-tooling.md`, "Masking").
- The length in `@len@` is the length of the shaped data.

Shaped in segments: holder and user names (HIUPA 4, HIUPD 6 and 7), product names (HIUPD 8, HISAL 2), the extensions (HIUPA 5, HIUPD 12), every amount in HISAL and HIUPD. Statements are shaped field by field:

| Format | Kept (markup and codes) | Shape-masked |
|---|---|---|
| MT940/MT942 (HIKAZ) | tags; `:28C:`, `:13:`, `:13D:`; balances: mark, date, currency; MT942 `:34F:` currency and mark, `:90D:`/`:90C:` count and currency; `:61:` value and entry date, mark (`C`, `D`, `RC`, `RD`), funds code, transaction type (`NTRF`), `NONREF`, `//`; `:86:` GV code, `?nn` codes, `?34`, SEPA keywords (`EREF+`, `KREF+`, `MREF+`, `CRED+`, `DEBT+`, `COAM+`, `OAMT+`, `SVWZ+`, `ABWA+`, `ABWE+`, `IBAN+`, `BIC+`, `PURP+`) at a subfield start or after a space | amounts (balances, `:61:`, `:34F:`, `:90D:`/`:90C:`; the replay recomputes the `:90D:`/`:90C:` sums from the rebuilt `:61:` amounts of the same data element); text: `:20:`, `:21:`, `:25:` bank code, `:61:` supplementary details, every `:86:` subfield value (booking text, primanota, purpose, counterparty name, BLZ/BIC), unstructured `:86:`, unknown tags. Pseudonyms: `:25:` account or IBAN, `:61:` customer reference (end-to-end ID) and bank reference, `:86:` `?31` account/IBAN, the values of `EREF+`, `MREF+`, `CRED+`, `IBAN+` |
| camt.052 (HICAZ) | tags (element and attribute names, attribute values such as `Ccy="EUR"`), XML declaration, comments, whitespace between tags, text of `CdtDbtInd`, `Cd`, `Sts`, `Dt`, `DtTm`, `CreDtTm`, `FrDtTm`, `ToDtTm`, `RvslInd`, `BtchBookg`, `LastPgInd`, `Ccy` | amounts and counts (`…Amt`, `Sum`, `…Nb`, `NbOfNtries`, `NbOfTxs`); text: every other element (names, BICs, other references, `Ustrd`, `AddtlNtryInf`); entity references (`&amp;`, `&#246;`) are text like any other. Pseudonyms: `IBAN`, `…Acct/Id/Othr/Id`, `AcctSvcrRef`, `EndToEndId`, `MsgId`, `PmtInfId` (and their `Orgnl…` forms), `MndtId`, `CdtrSchmeId/…/Id` |
| MT535/536 (HIWPD, HIWDU) | tags, qualifiers (`:HOLD//`); `:16R:`, `:16S:`, `:23G:`, `:22F:`, `:22H:`, `:25D:`, `:17B:`, `:28E:`, `:13A:`, `:98A:`/`B`/`C`/`E`, `:69A:`/`B`, `:94B:`, `:11A:`, `:12A:`/`B`/`C`; `ISIN ` of `:35B:`; four-letter codes (`ACTU`, `UNIT`), sign `N` and currency; dates in `:70E:` | amounts (prices, quantities, holding values, acquisition price: numbers with a decimal comma); text: `:35B:` (ISIN, WKN, name), `:97A:` bank code and `:97B:`, `:95P:`/`Q`/`R`/`S`, `:20C:`, `:70C:`/`D`/`G`, any other part. Pseudonym: `:97A:` safekeeping account number |

Example (`:61:` and `:86:` of a recorded MT940):

```text
:61:2601020102DR#,##NDDTQM-4417//2093381-7
:86:105?00#####################?10###?20EREF+LTZ-3?21MREF+K-80#SVWZ+##############?32########
```

Only the trace recorder (`mask::mask_for_trace`) shape-masks statements. The log (`mask_for_log`, the `TRACE` event via `redacted_text`, `debug`) keeps them as stars or `<masked>`: a trace is a file a human reviews before committing, a log goes to any sink, so the log stays the more masked of the two. Names, product names and amounts in HIUPA, HIUPD and HISAL are shape-masked in the log too.

The trace keeps the structure and the replay fills the values in. A shape-masked value is made valid again (`trace::unshape_text`, `trace::unshape_statement`, with the same field rules as the mask): each `#` becomes `1` in an amount and `X` in text, so the client parses the bank's own statement layout, with as many bookings and holdings as were recorded. An alias or pseudonym is a valid value of its kind and is served as recorded; the replay needs no salt. Where a recorded response has a value that is all stars, the replay puts a synthetic one of valid syntax:

- account answers (HISAL, HIKAZ, HICAZ, HIWPD, HIWDU) and HNVSK take the value from the same position of the client's own request (HKSAL, HKKAZ, …, HNVSK), so the answer names the account the client asked for;
- system ID (HISYN) `REPLAYSYS0001`; user ID (HIUPA, HIUPD) the one the client sent in HKIDN; the HIUPD segments get account numbers `0001234567`, `0001234568`, … and their IBANs; the HITAN challenge a fixed text;
- every other starred value is left out (empty). A mandatory field this hits makes the client read the segment as raw; the replay then shows it as a different request or a failed run, and the table above gets one more line.

The envelope of each answer (HNHBK size, message number and reference, HNHBS) is computed from the request just received.

What is lost: the values of the statements (texts, names, the size of amounts) and the identity values, of which only equality survives. What stays: layout, markup, codes, dates, the number of entries and the length of free text.

### What the replay compares

For each request, in order, with the recorded one: the segment codes in order (outer and inside HNVSD), each segment's version and number, the number of data elements and of group elements in each, and for HKTAN the TAN process and the segment it refers to. Values are not compared: most are stars or shapes, the rest (dates, times, control references, dialog IDs, message numbers, sizes, order references) change from run to run. A difference is a `Violation`; so are a request beyond the end of the trace (answered with HTTP 500) and a client that stops early. `HttpReplay::assert_clean` fails on any of them. The recorded answer is sent even after a difference, so one run shows every difference.

The replay drives the client through `replay::standard_run` over a `jobs::BankClient` (`HttpReplay::client`: the recorded bank code and the replay's URL): `jobs::open`, then per account `Dialog::capabilities` and the jobs the bank offers and the UPD allows (balance, transactions, holdings for a depot), each answering its TAN in a caller-side loop (`RunOptions` holds the waits; `bank_waits: false`, set by the replay tests, ignores the bank's HITANS waits so a recorded decoupled login replays without sleeping), then `Dialog::end`. `record_trace` runs the same function, so recording and replay take the same path.

For an MT942 the replay recomputes the `:90D:`/`:90C:` sums from the rebuilt `:61:` amounts of the same data element, so a recorded MT942 replays as `Reported`. A statement whose `:20:` is on an earlier page keeps the rebuilt sum and replays as `Unreadable`. `record_trace` prints the pending state per account: `pending 2 reported (made …)`, `not sent`, or `UNREADABLE (…)`.
