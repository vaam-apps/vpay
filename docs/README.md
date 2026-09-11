# The vpay documentation

**Start with [status.md](status.md).** It is the only page that says what is
actually built, and it is machine-checked: `cargo xtask verify-status` fails the
build if shipping code carries a `ProviderError::NotImplemented` token that page
does not declare, and fails the other way too. Everything else here explains
code — it says nothing about whether that code has ever run.

## Which page answers which question

| You want to know                                                            | Read                                                |
| --------------------------------------------------------------------------- | --------------------------------------------------- |
| **what works today**, and what does not                                     | [status.md](status.md)                              |
| what worked on a given day, and what a gate actually printed                | [status/](status/README.md)                         |
| **why a decision was taken**, and what it rules out                         | [adr/](adr/) — immutable; superseded, never edited  |
| **what happens, in what order**, and what can go wrong                      | [flows/](flows/README.md)                           |
| **why this code looks like this** — the seams, the orderings, the constants | [reference/](reference/README.md)                   |
| **what to do when it is 3 a.m.** and something is wrong                     | [runbooks/](runbooks/README.md)                     |
| what a merchant's SDK offers, and which gaps are dated                      | [sdks/parity.md](sdks/parity.md)                    |
| what the HTTP wire looks like                                               | [api/](api/README.md)                               |
| what is planned, in what order                                              | [roadmap.md](roadmap.md)                            |
| what a past step or experiment actually did                                 | [plans/](plans/) — dated records, not current truth |
| a proposal that has not been decided                                        | [rfc/](rfc/README.md)                               |

## The documents a gate reads

A document here is prose unless a gate reads it. These are read:

| Document                                                               | Gate                | What it refuses                                                           |
| ---------------------------------------------------------------------- | ------------------- | ------------------------------------------------------------------------- |
| [status.md](status.md)                                                 | `verify-status`     | an undeclared `NotImplemented` token, or a declaration no code carries    |
| [sdks/parity.md](sdks/parity.md)                                       | `verify-sdk-parity` | an SDK capability with no row, a row naming no capability, a renamed test |
| [adr/0016-engineering-standards.md](adr/0016-engineering-standards.md) | `verify-serde`      | a serialisable type that does not spell the wire convention, unexempted   |
| every `.md` in the tree                                                | `verify-links`      | a link that resolves to no tracked path                                   |

`verify-links` is the one that touches this page, and it is worth knowing what
it does not do: **it checks the destination path and never the `#anchor`**, so a
link to a heading that has been renamed or moved still passes the build. Two
such links were found stale during the 2026-09-11 split and fixed; one remains
open, in [flows/invoices.md](flows/invoices.md), and it is named here rather
than quietly left.

## Why some pages are an overview and a directory

On 2026-09-11 the eleven longest documents were split, because they had stopped
being readable:

| Page                                                 | Was   | Is  | Where the rest went                              |
| ---------------------------------------------------- | ----- | --- | ------------------------------------------------ |
| [status.md](status.md)                               | 6 151 | 259 | [status/](status/README.md)                      |
| [reference/vpay-db.md](reference/vpay-db.md)         | 3 330 | 335 | [reference/vpay-db/](reference/vpay-db/)         |
| [reference/vpay-api.md](reference/vpay-api.md)       | 1 566 | 203 | [reference/vpay-api/](reference/vpay-api/)       |
| [roadmap.md](roadmap.md)                             | 1 504 | 110 | [roadmap/](roadmap/)                             |
| [runbooks/demo.md](runbooks/demo.md)                 | 1 426 | 309 | [runbooks/demo/](runbooks/demo/)                 |
| [flows/hosted-checkout.md](flows/hosted-checkout.md) | 937   | 434 | [flows/hosted-checkout/](flows/hosted-checkout/) |
| [flows/customers.md](flows/customers.md)             | 888   | 259 | [flows/customers/](flows/customers/)             |
| [flows/dashboard.md](flows/dashboard.md)             | 865   | 151 | [flows/dashboard/](flows/dashboard/)             |
| [flows/webhooks.md](flows/webhooks.md)               | 811   | 249 | [flows/webhooks/](flows/webhooks/)               |
| [flows/merchant-auth.md](flows/merchant-auth.md)     | 748   | 484 | [flows/merchant-auth/](flows/merchant-auth/)     |
| [flows/dashboard-auth.md](flows/dashboard-auth.md)   | 732   | 282 | [flows/dashboard-auth/](flows/dashboard-auth/)   |

**Nothing was deleted.** This repository's habit is to record what it got
wrong — "this said X until date Y and was wrong", a struck-through claim, a
number that turned out to be arithmetic rather than a measurement — and those
paragraphs are the reason the documents are worth trusting. Every one of them
moved **verbatim**; the only edits to moved text are the `../` a relative link
gains when its file moves down a directory, and a link whose heading landed on a
different page. A whitespace-tolerant diff of each original against its new set
of pages reports zero lines missing, and that check is what this split was
judged by. The markdown under `docs/` went from **75 956 lines in 206 files to
76 729 in 292** — it grew, because every new page carries a header saying where
its text came from.

Each split page keeps its own path, so every inbound link and every `docs/…`
citation in a code comment still resolves; the page is an overview with an index
in it.

**Two documents over 800 lines were deliberately not split**, and the reason is
the same for both: [api/README.md](api/README.md) (855) is a wire contract
people grep rather than read in order, and
[reference/vpay-worker.md](reference/vpay-worker.md) (833) is one process
described once. `plans/` was not touched at all — those are dated records of
what was run on a day, and rewriting one falsifies it.

## What is not here

- **Generated API reference.** There is none. `docs/api/` is hand-written.
- **Anything about production.** Nothing in this repository has run outside a
  developer machine or a CI runner. [status.md](status.md)'s banner is the
  authority on that and it has not moved since 2026-09-03.
