# Emergency changes

The auditable exception to the normal change path in
[docs/flows/sdlc.md](sdlc.md). An emergency change is a change that must reach
production faster than the normal `just ci`-gated path allows, or that cannot
complete every normal gate before it is needed. This page is the policy for
when that is and is not allowed, and the record that makes an emergency change
auditable after the fact.

Scope: this is the repository's procedure. It complements — it does not replace
— the normal path. A change that can wait for the normal path must use it.

## What counts as an emergency

A change is emergency only if waiting for the normal path would materially
increase risk to live data, money, availability or the organisation's security
posture. Recognised triggers:

- a live security vulnerability being exploited, or a high/critical advisory
  on a dependency a running deployment ships;
- a production incident (an outage, a settlement failure, a data-integrity
  problem) whose fix cannot wait for a scheduled release;
- an urgent legal or regulatory direction with a deadline shorter than the
  normal path.

"Anxious to ship", "the sprint ends today" and "the reviewer is offline" are
**not** emergencies. Conflating urgency with emergency is how the normal gates
get bypassed for comfort, and the record below is written to make that visible.

## The path

An emergency change still follows the normal branch/PR shape — it does not
become a direct push to `master`. What differs is the minimum gate set and the
mandatory retrospective.

1. **Branch + PR** — as normal, on a short-lived branch.
2. **Minimum gates that may never be skipped:** the build must compile, the
   Rust unit and integration suites must pass, `verify` (the seventeen offline
   gates) must pass, and the change must not introduce a new
   `ProviderError::NotImplemented` token without declaring it. These are the
   gates that protect data and the money path; they are non-negotiable.
3. **Authorisation:** the repository maintainer authorises the emergency
   bypass of any remaining gate (for example `test-storybook`, a slow web
   suite, or `helm-check` needing a network) and names it in the PR.
4. **Merge** — the maintainer merges with the normal review, or with an
   explicitly recorded single-reviewer sign-off.

No gate is ever skipped silently. If `just ci` cannot complete, the PR records
exactly which gate was skipped, why, and by whose authority.

## The retrospective review

Within five business days of the merge, a retrospective review of the
emergency change is opened and completed:

- the skipped gate(s) are re-run against the merged tree, and any failure they
  find is tracked as its own issue;
- the change is reviewed again in full, at normal standard, by a reviewer who
  did not author the bypass decision;
- the threat assessment from [sdlc.md](sdlc.md) is applied if the change is
  security-relevant;
- the outcome (what was bypassed, what the full run found, whether the bypass
  was justified) is recorded on the PR and linked from this page's Status
  section.

The retrospective is not optional. An emergency change without one is a
bypass that never came back to the normal path.

## Status

**Policy written 2026-10-29 as part of the ISO 27001 secure-SDLC work
([#151](https://github.com/vaam-apps/vpay/issues/151)). No emergency change
has ever been made in this repository**, so this page is an unexercised
procedure: no emergency bypass is on record, no retrospective has been run, and
the procedure above has not yet been followed against a real incident. It is
written now so that the path exists and is auditable before it is needed. The
first time an emergency change is made, this Status section is updated to
record it, per the repository's rule that a status page that lags is worse
than none.
