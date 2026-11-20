# Not yet seen at a bank

Part of the [bank list](../banks.md).

Everything below is implemented and tested offline only; no live run has confirmed it.

- CAMT (HKCAZ): the split of the DK `BkTxCd/Prtry/Cd` into GVC and primanota. (Pending documents in HICAZ are seen: DKB and comdirect.)
- HKWDU v5 and MT536 contents as German banks fill them (layout from the specification text only).
- HKTAN on every touchdown page: needed at one savings bank; DKB, comdirect and Consorsbank behaviour unknown (Consorsbank HIPINS says no TAN for HKKAZ/HKWPD, so nothing changes there).
- Decoupled poll with HKTAN 6 (process 2, PIN/TAN B.5.2; process `S` only from HKTAN 7): not tried at any bank; comdirect never asks for approval.
- Per-order decoupled confirmation: that the confirming HKTAN:S response carries the order data, and the re-send fallback when it does not.
- Parameter probe with a stored system ID (HKIDN+HKVVB, function 999, no HKTAN): that banks answer with BPD and 3920 and grant SCA exemptions to the kept ID.
- Quirks (`tolerant` chain): none has seen a real malformed bank message. Unclear: where the segment counter starts (taken from the spec: HNHBK is 1, inner segments continue from 2), which version strings a bank puts in the header (only empty and `0` are repaired).
- The test bank (`testbank/`) is written from the specification: answer layout, return-code texts, the touchdown format (`TD-n`), the dummy HITAN of the exemption and when a bank resends BPD/UPD are plausible, not observed. A passing test says the client follows the spec as we read it, not that a bank accepts it.
- Decoupled poll with HKTAN 5 (process 2, as for HKTAN 6): not tried at any bank.
- Wrong typed TAN: no bank has been seen answering one. The client asks again when the answer carries 9941 alone (with 9050, and 9360 with attempts left) and no 9800 ([jobs-step-design.md](../jobs-step-design.md) §2, "Wrong TAN"); that rule is from the spec, which does not say whether a bank keeps the order. Unknown for DKB, comdirect, Consorsbank and ING: whether 9941 keeps the order, after how many wrong TANs the bank deletes it, locks or ends the dialog, whether it sends 9360 or a 3xxx text with the remaining attempts, and whether a transfer's second TAN message must carry HKVPA again (the client sends it with the first only). A bank that deviates gets a profile field.
- MT942 (pending bookings with HKKAZ): no bank has sent one. The reading is built and tested offline with the test bank ([formats-design.md](../formats-design.md)); see [Consorsbank](consorsbank.md), Transaction format.
