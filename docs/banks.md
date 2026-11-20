# Banks

Which banks fints-client is tested with. Each bank has a page with what its FinTS server does that the specification does not say, and what that means for the bank's profile ([architecture.md, section 2.5](architecture.md#25-banks-quirks-as-data-not-code)).

| Bank | Login | Accounts | Depot | Transfer | Payee check |
|---|---|---|---|---|---|
| [DKB](banks/dkb.md) | ✅ app approval | ✅ | ? | ✅ | ✅ |
| [comdirect](banks/comdirect.md) | ✅ | ✅ | ✅ | ✅ | ✅ |
| [Consorsbank](banks/consorsbank.md) | ✅ | ✅ | ✅ | ✅ | — |
| [ING](banks/ing.md) | ? | ? | ? | ? | ? |

✅ ran live in November 2026, ? not yet tested, — the bank does not offer it over FinTS. Accounts means balance and transactions of every account the bank lists with an IBAN; a transfer means one 1.00 EUR SEPA transfer. Bank codes, the server address, the statement format and what else each bank does (credit cards, pending bookings, instant transfers) are on the bank's page. Each bank with a profile of the test bank (`testbank/src/profiles.rs`) also has behaviour tests (`tests/bank_profiles_test.rs`).

## Your bank is not listed

It will probably work: fints-client follows the specification and the bank's own parameters, and the bank directory knows the address of about 1,700 bank codes. Find out with one read-only run:

```sh
scripts/try-bank.sh sparkasse köln   # find the BLZ
scripts/try-bank.sh 37050198         # log in, read every account, record, replay
```

[adding-a-bank.md](adding-a-bank.md) explains the run and what to send us. A bank that needs more than the generic flow gets a profile in `src/banks/table.rs` and a page in `banks/`, in the shape of the existing ones: one heading per topic, each entry saying what was seen, when, and what it means for the profile. Mark anything no live run confirmed as **unverified**.

## More

- [Across banks](banks/across-banks.md): patterns several banks share, and what is reported for the savings banks.
- [Not yet seen at a bank](banks/unverified.md): what is built and tested offline only.
