# Dashboard auth — the rate-limit budget, and which address it counts

_Split out of [docs/flows/dashboard-auth.md](../dashboard-auth.md) on 2026-09-11 by exp57, which broke a 732-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

### The budget is the deployment's

~~**The limits are per replica.** Three replicas admit three times the attempts
one does.~~ **Corrected 2026-09-10 (issue #79 item 2).** They were, and ADR-0017
called it "the first thing to revisit if a deployment runs many replicas". It
was revisited: the counters are rows in `rate_limit_windows` (migration
`0038`), so **ten attempts means ten attempts** whatever a deployment's replica
count is and however a load balancer spreads a burst across it.

The objection ADR-0017 raised against a durable counter — "a write on the
unauthenticated path … a denial-of-service amplifier of a different kind" — is
answered by three properties of one statement rather than waved away:

| Property                                                                                                                         | What it stops                                                                                                                                                                                                                                                                |
| -------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The row key is `SHA-256("<action>:<dimension>:<value>")`                                                                         | The caller chooses the _value_ — for the interesting attempts, an address with no account. A column holding it verbatim is a column an attacker sizes, and a copy of somebody's email address written down by the act of guessing it                                         |
| Up to **32 elapsed rows** are deleted in the same statement — `FOR UPDATE SKIP LOCKED`, and never the row the statement is about | Sixteen times the two rows one attempt adds, so a caller spending fresh keys drains the table faster than they fill it. `SKIP LOCKED` is what makes two concurrent sign-ins unable to deadlock on the sweep; without it a deadlock is a `500` for somebody typing a password |
| One `INSERT … ON CONFLICT (id) DO UPDATE … RETURNING attempts`                                                                   | The row lock Postgres takes before evaluating `DO UPDATE` serialises two replicas on one key. A read-then-write would have kept the over-admission and added a race: two replicas reading 9 both write 10 and both admit                                                     |

**The numbers are configuration** — `staff_auth.rate_limits.sign_in` and
`staff_auth.rate_limits.change_password`, each `{ attempts, window_seconds }` —
defaulting to 10/300 and 5/300. Configurable because ten was chosen for a
limiter three replicas multiplied by three, and it is not obvious that it is
still the right number now that they do not. **Whether the shared default
should now be below ten is a maintainer decision and has not been taken.**

_Reviewed 2026-09-10 (exp36 review): ten stays._ The case for lowering is that
the budget got stricter by becoming shared, so there is headroom. The half
that would be spent is the per-**address** one, and with `trusted_proxies`
empty — the default — that half is shared by everybody behind the proxy, so
lowering it makes a proxy-fronted deployment lock its whole staff out faster.
The reason for ten was never replicas: it is that a person who mistypes a
printed one-time password twice must not be locked out of their first login,
and that has not changed. The number is configuration, so a deployment that
measures its own traffic still moves it without a release; the review takes
the default, not the choice.

**Failing closed is part of the contract.** Every count returns a `Result` and
a database failure is a refusal, never an allowance: a limiter that answered
"allowed" when it could not count would have been removed by the very attack
it exists to bound.

**The window is fixed, not sliding**, and the cost is the classic one, stated
rather than hidden: an attacker who straddles a window boundary gets twice the
limit in one instant. A sliding window needs a row per attempt, which is the
unbounded table the digest key and the sweep exist to avoid.

Proof: `two_replicas_share_one_sign_in_budget` — two vpay servers on two ports
over one Postgres, six wrong passwords for one account alternating between
them against a configured budget of five, and the sixth is `429`. With the
counters per limiter instance again it reads `[401 × 6]`.

### Which address the limiter counts

**The transport peer, unless the peer is a machine the operator named.**
`staff_auth.trusted_proxies` is a list of addresses and CIDR blocks
(`10.0.0.0/8`, `198.51.100.7`, `fd00::/8`), **empty by default**, and empty
means the peer — which is exactly what ADR-0017 shipped, and is why a
deployment that does not set it behaves as it always did.

When the peer _is_ in the list, the client address is the **first untrusted
hop of `X-Forwarded-For`, walking from the right**: the rightmost entries are
the ones this deployment's own infrastructure appended, so the first one that
is not ours is the last value a trusted machine vouched for.

> Taking the **leftmost** entry — the shape most "get the real IP" snippets
> have — is whatever the caller wrote, and a caller writes a new one per
> request. That is not a smaller version of the same feature; it is the hole
> the feature exists to not open, because it hands every caller a fresh
> rate-limit bucket while the limiter goes on reporting that it limits.

Four things end the walk at the peer, and each is a hole if it is dropped:

- the peer is **not** in the allow-list — the header is not read at all;
- a hop does not parse as an address, so nothing to its left has been vouched
  for by anything checkable;
- every hop is one of ours, which names no client;
- there is no header.

A **repeated** `X-Forwarded-For` is one chain. RFC 9110 §5.2 makes repeated
field lines one comma-separated list in the order received, so the walk runs
right to left _across_ the lines: the nearest hop is the last hop of the last
line, because that is what the most downstream proxy appended.

> **Corrected 2026-09-10 by the exp36 review (finding F2).** This read
> `HeaderMap::get` — the _first_ line only — as first delivered. Behind a
> proxy that appends its hop as a new line rather than rewriting the caller's,
> the whole chain being walked was then the one the caller wrote, and the
> allow-list handed out a fresh bucket per request rather than closing one.
> `a_repeated_forwarded_for_is_one_chain_and_buys_no_fresh_budget` sends two
> field lines over a real socket and reads `[401 × 6]` against a budget of
> five with the first-line-only reading restored.

A field line that is not readable as text ends the walk at the peer rather
than being skipped: skipping it means walking _past_ an unreadable hop into a
line further from the peer, which is the one direction this walk never goes.

`Forwarded` (RFC 7239) is deliberately **not** read. Two parsers over one
caller-supplied string means the more permissive answer wins, and the more
permissive answer is the one that buys a fresh bucket.

An entry that does not parse takes the **login** down: `vpay-server` logs the
entry by index and mounts the read surface with no staff login, rather than
narrowing the list silently — a typo'd allow-list is a per-address budget that
has quietly become the whole deployment's again, which is the failure this
feature exists to fix and the one nobody would notice.

**With the list empty and a reverse proxy in front, every staff member shares
one per-address budget**, because the peer is the proxy. The per-email budget
still binds per account, and `vpay-server` says so at `info` on every boot.

Proof, and it is a pair rather than one case, because either alone would pass
with the trusted-proxy check deleted:
`a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget` (six
attempts, six different claimed client addresses, one peer, `429` on the
sixth) and `a_forwarded_for_header_from_a_trusted_peer_is_the_client_address`
(one claimed client exhausted, a second one's first attempt still served).

_Corrected 2026-09-07 (exp24 review, finding F2)._ Until that review the
per-IP half **did not exist**: axum supplies the peer address through
`ConnectInfo`, `ConnectInfo` is present only when the service is built with
`into_make_service_with_connect_info`, and neither `vpay-server` nor the test
harness did that. Every attempt in the process was counted under the
limiter's one `ip:unknown` key, so ten requests from anywhere locked every
staff member out of the dashboard for five minutes — the deployment-wide
version of exactly the lockout the design refuses to build. Both call sites
now build the service with connect info, and
`the_sign_in_rate_limit_is_per_source_address` burns one loopback source's
budget and asserts a second source's first attempt is still served.
