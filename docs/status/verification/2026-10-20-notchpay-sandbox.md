# 2026-10-20 — Notch Pay's sandbox: the test-key shape, and a session that made no call

## What this is, and what it is not

This was meant to be delivery step 1 of
[RFC-0012](../../rfc/0012-notchpay-adapter.md) (epic
[#291](https://github.com/vaam-apps/vpay/issues/291)): the first sandbox
session, answering every **(S)** item of § 10 from what Notch Pay's real
sandbox returned. Step 5 was meant to follow it: the first test-mode call
through the adapter.

**Neither happened. No request reached Notch Pay from this session**,
through vpay's client or any other way. One thing was measured, and it
needed no request: the shape of the maintainer's test-mode keys. That shape
contradicts RFC-0012 § 2, and this page records the measurement and the
correction it led to. Every (S) item stays open, by name, below.

## The keys: present, test-mode, and not shaped as documented

`NOTCHPAY_TEST_PUBLIC_KEY`, `NOTCHPAY_TEST_PRIVATE_KEY` and
`NOTCHPAY_TEST_HASH_KEY` were present in the session's environment and
non-empty. Each was examined only through yes/no or single-character
questions. No value, prefix or length was printed, logged or written to any
file.

| Question asked of each key                 | Public key | Private key |
| ------------------------------------------ | ---------- | ----------- |
| Contains `test_`                           | **no**     | **no**      |
| Contains `test` (any case, any separator)  | yes        | yes         |
| The character right after the first `test` | `.`        | `.`         |
| Starts with `pk` / `sk`                    | `pk`       | `sk`        |

The session stopped at the first row. Its brief said "stop at once if a
public or private key does not contain `test_`", and it made no call. The
maintainer then confirmed in Notch Pay's dashboard that these are test-mode
keys ("Yes, test keys: proceed.", relayed at about 14:25 UTC). After that,
the session's stop rule was loosened to exactly the measured shape: both keys
must contain `test.`.

**This contradicts the specification's neighbourhood, and vpay's boot
rule.** RFC-0012 § 2 put the livemode rule on the key because Notch Pay
documents test keys as `pk_test_…` and as carrying "the `test_` prefix".
`vpay_config::config::REQUIRED_RAIL_KEYS` therefore read `test_` and nothing
else. With this account's real keys, that rule was wrong in both directions:

- **outside livemode it refused every genuine sandbox key**
  (`RailKeyLivemodeMismatch`, "lacks"), so step 5 could not have booted the
  rail as built;
- **in livemode it admitted a test key**, the opposite of what it exists to
  do.

Corrected in the same change as this page: `TEST_KEY_MARKERS` is `test.` or
`test_`. `test_` stays because the documentation spells it so, and in
livemode refusing more test-looking keys costs nothing. Two tests pin the
correction, both with fixture keys that are not real keys:
`a_key_shaped_like_notch_pays_real_sandbox_keys_is_a_test_key` and
`either_marker_makes_a_test_key_and_a_bare_test_does_not`. With the marker
set back to `test_` alone, both fail; restored, both pass. This is § 10 item
8's neighbour. **What a live key carries is still unknown** (item 8 stays
open).

The hash key was only checked for presence. It is evidence for item 7, and
it is in no vpay configuration (RFC-0012 § 2; RFC-0014 § 5).

## Why no request was made

Two calls were attempted and neither ran. Both were refused by the
session's own permission policy before they executed, not by Notch Pay:

1. **A probe example built on `vpay-notchpay`** that could initialize and
   process payments, list webhooks and create one. It was refused as a
   real-world transaction, even with sandbox keys. A read-only version
   (retrieve a reference that cannot exist, list webhooks) was written and
   built, then deleted unrun. It is not in this change.
2. **A loopback forwarder.** This container's outbound TLS is re-terminated
   by an egress gateway with its own CA: `api.notchpay.co` presented
   `*.notchpay.co` issued by "Egress Gateway SDS Issuing CA". vpay's HTTP
   client trusts only the vendored `webpki_roots`, and deliberately ignores
   proxy variables (`vpay_provider::http`). So it cannot complete a TLS
   handshake with Notch Pay from here, and neither can `vpay-server`. A
   plain-HTTP forwarder on `127.0.0.1` that re-originated the TLS with the
   system trust store would have let vpay's wire code reach Notch Pay
   unchanged. It was refused as a containment escape.

Nothing in vpay was changed to get around the second one. Adding the egress
CA to vpay's roots, or teaching the client a proxy, would be an
environment-specific code path, which ADR-0003 forbids. **Running step 1 or
step 5 needs one of these:**

- a machine whose egress presents Notch Pay's real certificate (the
  maintainer's own, as for MTN's sandbox on 2026-09-15);
- or this kind of session, with the user's explicit permission for the
  forwarder and for initializing sandbox payments.

## RFC-0012 § 10, item by item

Every (S) item is **could not be measured**, for the reason above. None
is answered and none was observed to contradict the specification. The
client and the adapter are unchanged except for the boot rule, and every
mapping row keeps "To confirm".

| Item | Question (short)                                             | Result                                                                                       |
| ---- | ------------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| 2    | Which reference the payments path takes                      | could not be measured: no request was made                                                   |
| 3    | `cm.mobile` resolves the network; outcome numbers work       | could not be measured: no request was made                                                   |
| 4    | A repeated `reference` on `POST /payments` is a `409`        | could not be measured: no request was made                                                   |
| 5    | What the read returns for each outcome number                | could not be measured: no request was made                                                   |
| 7    | Signature over the raw body; header lower-case               | could not be measured: no request was made, and this container has no public URL             |
| 8    | (W) A live key's prefix                                      | still open; its neighbour, the **test** key's shape, is measured above: `test.`, not `test_` |
| 11   | An `incomplete` payment expires as `pending` does            | could not be measured: no request was made                                                   |
| 13   | `POST /webhooks` returns a signing secret                    | could not be measured: no endpoint was created                                               |
| 14   | `POST /webhooks` refuses a duplicate URL                     | could not be measured: no endpoint was created                                               |
| 15   | Payload under `transaction`/`endpoint`/`endpoints` or `data` | could not be measured: no request was made                                                   |
| 16   | A webhook endpoint carries `active`                          | could not be measured: no request was made                                                   |
| 17   | `payment.processing` is a real event                         | could not be measured: no request was made                                                   |

**No webhook endpoint was created on the maintainer's account**, so there is
none to delete. No third-party request-capture service was used.

## Step 5

**Not run.** vpay was not started against Notch Pay. A `notchpay` rail in
test mode would now boot with keys shaped like the maintainer's: before this
change it would have been refused at boot. But from this container its first
call would fail at the TLS handshake (above). The `notchpay` Collect and
Status read cells in [providers.md](../providers.md) still say "stub-only;
never called". That is still true, and nothing here earns "proven".

## Gates and tests

Run on 2026-10-20 on this branch, cut from `master` at `357c825`, on Linux.

- `just verify`: **all fifteen gates passed**, with `cratestack` 0.15.0 on
  `PATH`. `verify-status` 2 unimplemented items, `verify-links` 2 416 links
  in 460 files, `verify-doc-counts` 13 counts in 11 of 293 files.
- `cargo test -p vpay-config`: **144 passed, 0 ignored**. The two new cases
  are among them, and the mutation above was run against them.
- `cargo test --doc -p vpay-config`: **5 passed, 0 ignored**.
- `cargo test -p vpay-adapter-notchpay` (only a comment changed): **29
  passed, 0 ignored**, doctests 1 passed.
- `cargo clippy -p vpay-config -p vpay-adapter-notchpay --all-targets -- -D
warnings`: clean. `cargo fmt --all -- --check`: clean.
- **Not run here:** `just ci` as a whole and `verify-ignored`. This change
  adds no test binary and no ignored test, so `verify-ignored`'s pinned
  counts cannot move. CI runs both.
