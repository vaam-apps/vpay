# exp36 review — the staff sign-in follow-ups, attacked

**Date:** 2026-09-10 · **Branch:** `claude/exp36-staff-followups` ·
**Rebased onto** `f55e002` (PR #94, the dashboard's port as a demo variable),
which is what made the one gate the implementer could not run runnable.

Read [opus.md](opus.md) first — it is the implementer's own account, and this
document only records what the attack found on top of it.

## The headline

The implementation is sound where it is proven, and **the one thing that was
not proven was wrong**. `just test-e2e` had never been run — the notes say so
plainly rather than hiding it, which is why it was the first thing this review
did. It fails: **dashboard.cy.ts, 6 of 8 failing**, and the first failure is
`leg 4`, the current-password field this change exists for. It is written up as **F1** in this review's next commit, once the mutation that proves the fix has been run in a browser rather than reasoned about.

**This document is written as the review proceeds**, one section per finding,
each landing in the commit that fixes it. A finding is not written down here
until the mutation that proves the fix has actually been run.

## Findings

### F2 — a repeated `X-Forwarded-For` was read as its first line only · **correctness**

`client_address_from` read `HeaderMap::get`, which answers the **first** field
line. A proxy may append its hop as a **new** `X-Forwarded-For` line rather
than rewriting the caller's — a per-proxy configuration difference, not a
rarity — and RFC 9110 §5.2 makes repeated field lines one comma-separated list
in the order received.

On such a deployment the entire chain this module walked was therefore the one
the **caller** wrote. The "first untrusted hop from the right" was whatever
they put at the end of their own line, and the allow-list handed out **a fresh
rate-limit bucket per request** instead of closing one — the same hole
`a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget` refuses
from an untrusted peer, arriving instead from a trusted one, which is the
deployment the feature exists for.

**Fix:** `get_all`, walked right to left across every line. A line that is not
readable as text ends the walk at the peer rather than being stepped over —
skipping it would mean trusting what lies on the far side of a hop this
deployment could not check.

**The mutation:** restore `headers.get(..)`.

| Level | As delivered | Fixed |
|---|---|---|
| unit — `a_repeated_header_is_one_chain_and_the_callers_line_is_not_the_end` | `Some(198.51.100.99)`, the address the caller chose | `Some(203.0.113.7)`, the one the proxy vouched for |
| socket — `a_repeated_forwarded_for_is_one_chain_and_buys_no_fresh_budget` | `[401, 401, 401, 401, 401, 401]` — no `429` at all | `[401 × 5, 429]` |
