# Reference

One page per crate, answering a question the other two documentation tiers do
not: **why does this code look like this?**

- An [ADR](../adr/) records a decision that has been taken. Immutable —
  superseded, never edited.
- A [flow](../flows/) describes a process: what happens, in what order, what
  can go wrong, and what invariant holds throughout.
- A page here explains the *shape of the code* that implements them: the port
  boundaries, the orderings that are load-bearing, the alternatives that were
  tried and rejected, and the measurements behind a constant.

## Why the tier exists

Before Step 7 this material lived in the source, in module headers of 80 to
120 lines. That is the wrong place for three reasons, all of which this
repository had:

- rustdoc renders it on every item page, so the one paragraph a caller needs
  is buried under the history of the decision behind it;
- a reader who wants the reasoning has to open the file and scroll past it to
  find the code;
- it is unversioned prose sitting where `git blame` is about code, so it rots
  where nobody is looking.

The rule for what stays in the source: **one paragraph of what and why, plus a
link to the page here.** `# Errors`, `# Panics` and `# Examples` sections stay
in the code — they are the rustdoc contract, and `just test-doc` compiles the
examples. Everything that begins "measured", "review finding 2026-…" or "this
used to be" moves here.

`cargo xtask verify-docs` (printed by `just verify`) reports the doc-comment
line count per crate against its code line count. It is a **report and never a
gate** — Step 7's decision (4). A ratio that is enforced is a ratio people pass
by deleting the `# Errors` sections [ADR-0011](../adr/0011-error-modelling.md)
depends on.

## Pages

| Crate | What the page covers |
|---|---|
| [vpay-api.md](vpay-api.md) | The router and its middleware order, the merchant OP, resource-server JWT validation, the JWKS cache, the form decoder, the confirm path, boot |
| [vpay-config.md](vpay-config.md) | The boot sequence both binaries follow, exit codes, the flags that are optional in the parser and required in practice, the OAuth client shapes |
| [vpay-core.md](vpay-core.md) | The domain crate: ids, money's two provider encodings, the two state machines, the failure taxonomy, the `Classify` tiers, the metric vocabulary |
| [vpay-db.md](vpay-db.md) | The repository seam — why a trait object rather than a generic, why the transaction API is a closure, what stays `pub` — and one section per table family |
| [vpay-worker.md](vpay-worker.md) | The job loop and what it owns, one poll, the recovery table, settlement, and the webhook outbox's two transactions |
| [rails.md](rails.md) | One page for `vpay-provider` and both adapters: the port's shape, the shared token cache and its caller-supplied margin, the bounded rail read, and what each rail keeps to itself |

Not one page per crate everywhere: `rails.md` covers three crates, because the
argument for the port and the argument for an adapter are the same argument
seen from two ends, and splitting it would mean saying it twice.

**No page:** `vpay-ledger`, `vpay-testkit`, and the two binaries — whose boot
sequence is in [vpay-config.md](vpay-config.md), where the shared code lives.
Each rail's own wire is a flow document rather than a reference page
([adapter-mtn-momo.md](../flows/adapter-mtn-momo.md),
[adapter-orange-money.md](../flows/adapter-orange-money.md)). For the rest, the
reasoning is still in the source's module headers — this index lists what
exists, not what was planned.

A crate with no page here is not a crate whose reasoning is documented
elsewhere by default. See [../status.md](../status.md) for what is actually
built; this tier explains code that exists, and says nothing about whether it
has ever run.
