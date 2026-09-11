# `vpay-api` — boot (`boot.rs`)

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## Boot (`boot.rs`)

`vpay_api::boot` is the one derivation both binaries run before they serve
anything: the linked adapters keyed by `providers.code`, the YAML joined
against them into reference-table seeds, and the connect/migrate/reconcile
sequence in the order [configuration.md](../../flows/configuration.md) fixes. See
[vpay-config.md § the boot sequence](../vpay-config.md#the-boot-sequence) for the
ordering itself and why each step is where it is.

`vpay-api` is the home because it is the only crate both boot paths already
link that also depends on `vpay-config` (the YAML), `vpay-provider` (the port)
and `vpay-db` (the seed types). No new dependency edge exists because of it.

It used to live in both binaries — and until 2026-09-07 there were two.
`vpay-server`'s `main.rs` and
`vpay-worker-bin`'s carried verbatim copies of `adapters_by_code`, `boot_seeds`,
`flow_label` and `display_name_for` — about 150 lines each, with comments
explaining that the duplication was deliberate. It was not safe: the two
processes reconcile the _same two tables_ in the same database, so a change to
one copy and not the other is a rollout where `providers.display_name` or
`providers.flow` flips back and forth depending on which binary restarted last,
with nothing to report it. The previous arrangement had no drift guard of any
kind — not a shared test, not a compile-time link.

What stayed per-binary was the thing that was then genuinely per-binary: the
four-line `adapters()` list of linked rails (Step 2's D6). A worker that
learned which rails exist from `vpay-server`'s crate would have made its
capabilities a function of the API server, and the two deployed independently.

**They do not deploy independently any more** (issue #77, 2026-09-07): one
package, one binary, one image, with the worker behind a subcommand. The second
copy of `adapters()` was deleted and `backends/apps/vpay-server/src/worker.rs`
calls `vpay_server::adapters`, so the drift this section is about is now
impossible for the rail list too, and not merely discouraged.
`cargo xtask verify-no-mocks` walks the dependency graph from each binary root,
so the list has to be reachable from that root to be checked — there is one
root now, and `SHIPPING_PACKAGES` in `.xtask` is still an array.

### Every adapter comes back wrapped in `Measured`

`adapters_by_code` is where `vpay_provider_requests_total` and
`vpay_provider_request_duration_seconds` get their one seam. The wrap happens
there rather than in the binary's `adapters()` list because that list _was_
deliberately duplicated per binary (until #77) and a metric mounted in a
duplicated list is one the copies eventually disagree about. The reason has
outlived the duplication and the wrap stays where it is: everything that
resolves a rail goes through that function — both of `vpay-server`'s boot
paths, and the integration suite's own harness — which is a stronger property
than "every copy remembered to wrap".

`Measured` delegates every method to the adapter it wraps and returns it as a
plain `Box<dyn ProviderAdapter>`, so no caller can tell — or branch on — whether
it is holding a wrapper. It is not a substitute for a rail and adds no code path
that exists only outside production
([ADR-0006](../../adr/0006-no-mocks-in-main-processes.md)); it is the shipping
process measuring itself.

The conformance suite constructs adapters directly and is therefore _not_
measured, which is correct: it exercises one adapter against a stub, and its
counts would say nothing about a deployment.
