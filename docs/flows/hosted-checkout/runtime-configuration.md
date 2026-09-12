# Hosted checkout — runtime configuration (`branding.yaml`, `config.yaml`)

_Split out of [docs/flows/hosted-checkout.md](../hosted-checkout.md) on 2026-09-11 by exp57, which broke a 937-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## Runtime configuration

_New 2026-09-06 (the maintainer's requirement of 2026-09-05)._

**One image serves every operator.** The page's brand and its feature flags
are two YAML files an operator mounts, read at container start — not a
`tailwind.config.ts` edit, not a build argument, and not `NEXT_PUBLIC_*`
inlined by `next build`.

| File            | Default path                       | Environment override          |
| --------------- | ---------------------------------- | ----------------------------- |
| `branding.yaml` | `/etc/vpay/checkout/branding.yaml` | `VPAY_CHECKOUT_BRANDING_FILE` |
| `config.yaml`   | `/etc/vpay/checkout/config.yaml`   | `VPAY_CHECKOUT_CONFIG_FILE`   |

Worked examples with every key commented are checked in at
[`../../config/checkout/branding.example.yaml`](../../../config/checkout/branding.example.yaml)
and [`../../config/checkout/config.example.yaml`](../../../config/checkout/config.example.yaml),
and `compose.demo.yml` mounts both.

```yaml
# branding.yaml
display_name: Vaam Payments          # the OPERATOR's name, never the merchant's
logo_url: https://cdn.example/logo.svg
primary_color: "#f3c623"             # #rrggbb, six digits
support_contact: support@vaam.example
```

```yaml
# config.yaml
checkout:
  public_base_url: https://checkout.example
  allowed_methods: [mtn_momo, orange_money]
  features:
    page_memory: true
```

**Injected at startup, not fetched.** `src/config/runtime.ts` reads both files
once, on the first import in the server process, and memoises the result; each
server component passes the parts it needs to the client as props. A payer's
request never reads a file, and the operator's colour arrives in the same
response as the markup — there is no frame in which a payer sees the default
and then watches it change. A config change is a pod restart, which is what
ADR-0003 already says about administration.

`GET /config/v1` reports exactly what the container loaded, versioned in the
path. It is a **verification surface for an operator**, not something the page
uses: every value on it is already visible to any payer who opens the page. The
file paths and the problem list are deliberately _not_ on it — those are in the
container log, where the person who can act on them is looking.

**A missing file is a log line and defaults, never a blank page.** Absent,
unreadable, not YAML, a top level that is not a mapping, a key of the wrong
type, a value that fails its rule: each costs exactly that key, prints one
`WARN` naming the file and the key, and leaves the rest standing.

```
[vpay-checkout] configuration: no file at /etc/vpay/checkout/branding.yaml — using defaults
[vpay-checkout] configuration: config.yaml: checkout.allowed_methods is empty — ignored, every rail stays on offer
```

Three rules are worth stating because each is a decision rather than a
default:

- **`display_name` is the operator's, and never substitutes for the
  merchant's.** What a payer is told they are paying comes from the session
  (`merchant: { name }`, from the API's `merchant_clients[].display_name`).
  Where that is absent the page still says the neutral sentence; putting the
  operator's name there would name the wrong party.
- **`allowed_methods` can only narrow, and never silently.** A rail the intent
  offers and the operator excludes is listed as unsupported — the same
  treatment D9 gives a rail this page has no flow for. An **empty** list is
  refused with a `WARN` rather than honoured, because honouring it would
  refuse every payment on the deployment.
- **`primary_color` retints the theme at runtime.** daisyUI compiles a theme
  into `--p: L% C H` custom properties, so one
  ~~`:root[data-theme="bumblebee"]`~~ **`:root[data-theme="dark"]`
  (2026-09-12, the `@vaam-apps/ui` cutover — `@vpay/ui` is deleted and this
  app's theme moved with it, per decision 3: kept, retargeted)** block in
  `<head>` is enough. `src/config/theme.ts` converts sRGB to OKLCh
  with Ottosson's matrices and daisyUI's own foreground rule — no colour
  library on a payment page — and its tests assert the output against values
  produced by **daisyUI's own converter** for six colours, so a drift is a
  failing test rather than a page that is quietly the wrong colour. The
  colour that goes _on_ the primary is derived, not configured, so a
  combination nobody can read text on is not one this file can produce.
