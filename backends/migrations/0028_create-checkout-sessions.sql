-- checkout_sessions: the object a merchant creates from its server to send a
-- payer to a page vpay serves (Step 9, D1 of
-- docs/plans/2026-09-04-step9-hosted-checkout.md).
--
-- WHY A SECOND OBJECT AND NOT THE INTENT'S SECRET IN A vpay URL
--
-- `success_url`, `cancel_url`, `return_url` and `ui_mode` have to live
-- somewhere, and none of them is a property of a PaymentIntent: they describe
-- one *checkout attempt* driven through vpay's own page. Putting them on the
-- intent would also mean the URL vpay mints for a payer carries the intent's
-- `client_secret` — the credential that authorises `confirm` — in a link a
-- merchant may email, log or put in a redirect chain. A session carries its
-- own credential instead (`client_secret_suffix` below), and the intent's
-- secret is handed to the page only by `/v1/browser/checkout/sessions/{id}`,
-- over a request that already proved it holds the session's own.
--
-- The session **references** an intent; it never creates one. Amount,
-- currency and rails stay on `payment_intents`, where every existing
-- invariant already guards them.
CREATE TABLE checkout_sessions (
    -- Caller-supplied `cs_…` id (vpay_core::ids::checkout_session_id),
    -- generated before the insert exactly as `pi_…` and `ch_…` are, so a
    -- crash mid-insert still leaves a name to reconcile by
    -- (docs/flows/crash-safety.md).
    id TEXT PRIMARY KEY,
    -- The list cursor, and the same argument as payment_intents.seq (0014)
    -- and events.seq (0018): `created_at` ties under a burst, and a cursor
    -- that can skip a row is a page a merchant never sees. GENERATED ALWAYS,
    -- so nothing may supply its own.
    seq BIGINT GENERATED ALWAYS AS IDENTITY,
    -- No FK: there is no merchants table (ADR-0003; see 0003's comment).
    -- Every query in vpay_db::checkout_sessions filters on it in SQL.
    merchant_id TEXT NOT NULL,
    -- The intent this session drives. A real foreign key, unlike
    -- `merchant_id`, because `payment_intents` is a table this schema owns —
    -- the same reason `charges.payment_intent_id` has one (0004).
    --
    -- The session's own `merchant_id` is stored rather than joined for the
    -- reason `charges` does not store one at all: `/v1` filters this table
    -- directly, and a tenancy filter that needed a join would be a filter a
    -- future query could forget. The two are kept in step by
    -- `vpay_api::v1::checkout_sessions::create`, which reads the intent
    -- through `PaymentIntents::get_for_merchant` — i.e. it cannot reference
    -- an intent outside the tenant it is about to stamp here.
    payment_intent_id TEXT NOT NULL REFERENCES payment_intents (id),
    -- Live or test money. Copied from the deployment's own configuration at
    -- creation, exactly as `payment_intents.livemode` is, and never updated.
    livemode BOOLEAN NOT NULL,
    -- `hosted` (vpay answers a `url` to redirect the payer to) or `embedded`
    -- (vpay answers a `client_secret` the merchant hands to
    -- `@vpay/stripe-js`, which mounts vpay's page in an iframe).
    ui_mode TEXT NOT NULL,
    -- D10's minimal lifecycle. `open` -> `complete` when the intent reaches
    -- `succeeded`; `open` -> `expired` on the 24-hour horizon, on an explicit
    -- `POST /v1/checkout/sessions/{id}/expire`, or when the intent reaches a
    -- terminal non-success state. There is no `canceled` and no reopening: a
    -- retry is a new intent (AGENTS.md, "one charge per intent, forever") and
    -- therefore a new session.
    status TEXT NOT NULL,
    -- `unpaid` -> `paid` or `failed`, written by the settlement transaction
    -- beside the intent it describes (vpay_db::settlement). Denormalised on
    -- purpose: a page that has just been handed back a session must be able
    -- to render the outcome without a second read, and the value is written
    -- in the *same* transaction as the intent's status, so the two cannot
    -- disagree.
    payment_status TEXT NOT NULL,
    -- Where the payer is forwarded after a hosted checkout, and where they
    -- are sent if they abandon it. Required together for `hosted`, refused
    -- for `embedded` — see `urls_match_ui_mode` below.
    success_url TEXT,
    cancel_url TEXT,
    -- Where an embedded checkout forwards the payer's *top-level* window.
    -- Required for `embedded`, refused for `hosted`.
    --
    -- Deliberately NOT `charges.return_url` (0019), which is the merchant's
    -- own destination for a `/v1` confirm. This one is the merchant's
    -- destination for a *session*; the URL vpay hands the rail for a
    -- session-driven charge is vpay's own return page, built from
    -- `checkout.public_base_url`, this row's `id` and `return_token`.
    return_url TEXT,
    -- The merchant publishable key every URL vpay mints for this session
    -- carries as `?key=` (Step 9, the integrator's ruling of 2026-09-04).
    --
    -- WHY IT IS A COLUMN AND NOT LOOKED UP AT RENDER TIME
    --
    -- All three `/v1/browser/checkout` routes authenticate by publishable key
    -- plus a session credential, and the *return* page is reached from a URL
    -- the **rail** replays — built once, at submit, and stored on the charge.
    -- Deriving the key at render time from `merchant_id` would mean a return
    -- URL that stopped resolving the moment a merchant rotated a key (add the
    -- new one, deploy, remove the old — the documented rotation), stranding
    -- every payer already in flight on a rail's page. Pinning the choice on
    -- the row is what makes the return URL stable for the life of the
    -- session.
    --
    -- It is also the reason a session cannot be created by a tenant with no
    -- publishable keys at all: there would be no `?key=` to put in the link,
    -- and the page it led to would answer the uniform 404 to every request.
    -- `vpay_api::v1::checkout_sessions` refuses that with
    -- `checkout_not_configured` before this insert.
    --
    -- NOT A SECRET. A publishable key names a tenant and authorises nothing
    -- (`vpay_config::MerchantClient::publishable_keys`); it is rendered into
    -- a merchant's own public checkout page by construction. It is visible in
    -- `CheckoutSessionRow`'s `Debug` for that reason, unlike the two columns
    -- below it.
    --
    -- The CHECK is a *shape* backstop only. The real rule — the key belongs
    -- to this session's merchant — is `vpay_config`'s registration list,
    -- which no database constraint can see (there is no merchants table;
    -- ADR-0003).
    publishable_key TEXT NOT NULL,
    -- The second half of this session's payer-facing `client_secret`; the
    -- first half is `id`. Exactly `payment_intents.client_secret_suffix`'s
    -- shape and exactly its reasoning (0026): only the suffix is stored
    -- because the other half is the primary key, and storing the joined
    -- string would be the id written twice. Not a hash, for 0026's reason —
    -- vpay mints this capability and renders it back, so there is nothing to
    -- verify against a hash without a second plaintext copy.
    client_secret_suffix TEXT NOT NULL,
    -- The **other** credential, and the reason it is a separate column
    -- rather than a reuse of the one above (D6).
    --
    -- A rail's redirect back to vpay cannot carry a URL fragment: the payer
    -- leaves vpay's origin entirely and the fragment does not survive the
    -- round trip, so the return page's credential has to travel in the query
    -- string. A query string is written to access logs, sent as a `Referer`
    -- by some clients, and kept in browser history — which is exactly why
    -- the value that travels there must NOT be the one that authorises
    -- reading the intent's own `client_secret`.
    --
    -- What this token authorises is deliberately smaller: reading the
    -- session and its intent *without* the intent's secret, which is enough
    -- to render an outcome and forward the payer, and nothing else. Same 160
    -- bits from the same generator, same constant-time compare, same uniform
    -- 404.
    return_token TEXT NOT NULL,
    -- The horizon after which this session is `expired` (D10: 24 hours from
    -- create). A stored instant rather than a computed one so the horizon a
    -- session was created under survives a change to the constant — and so
    -- the sweep is an index-able comparison rather than arithmetic on
    -- `created_at`.
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Maintained by the writers in vpay_db::checkout_sessions, not by a
    -- trigger — the same choice migration 0014 made and for the same reason
    -- (a trigger is a write nothing in the code names).
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT id_length CHECK (char_length(id) BETWEEN 1 AND 64),
    CONSTRAINT merchant_id_length CHECK (char_length(merchant_id) BETWEEN 1 AND 128),
    CONSTRAINT payment_intent_id_length CHECK (char_length(payment_intent_id) BETWEEN 1 AND 64),

    -- The three closed vocabularies, closed *at the database* for
    -- `events.type_is_a_documented_event`'s reason (0018): a label no code
    -- handles must not be storable, and the vocabulary is small and fixed.
    CONSTRAINT ui_mode_is_known CHECK (ui_mode IN ('hosted', 'embedded')),
    CONSTRAINT status_is_known CHECK (status IN ('open', 'complete', 'expired')),
    CONSTRAINT payment_status_is_known CHECK (payment_status IN ('unpaid', 'paid', 'failed')),

    -- WHICH URLS BELONG TO WHICH MODE
    --
    -- A hosted session forwards the payer's whole browser, so it needs both
    -- a success and a cancel destination; there is no iframe to hand control
    -- back to. An embedded session never navigates the top-level window on
    -- vpay's behalf — the merchant's own page does, on a `vpay:complete`
    -- message — so it has exactly one destination and no cancel.
    --
    -- The API refuses the wrong combination with a `400` naming the
    -- parameter before any insert; this is the backstop that makes "a
    -- session's URLs always match its mode" a property of the schema rather
    -- than of one handler, exactly as `0019`'s `return_url_length` backstops
    -- `checked_return_url`.
    CONSTRAINT urls_match_ui_mode CHECK (
        (ui_mode = 'hosted'
             AND success_url IS NOT NULL
             AND cancel_url IS NOT NULL
             AND return_url IS NULL)
        OR (ui_mode = 'embedded'
             AND return_url IS NOT NULL
             AND success_url IS NULL
             AND cancel_url IS NULL)
    ),

    -- The same 2048-character ceiling and the same closed scheme list as
    -- `charges.return_url` (0019), for the same two reasons that migration
    -- spells out at length: 2048 is the practical URL limit every browser
    -- and proxy agrees on, and a column that accepts `javascript:` is a
    -- stored XSS in whatever renders the object — here, vpay's *own*
    -- checkout page, which navigates to these values.
    --
    -- `lower(...) LIKE` rather than a regex, and lowercased because URL
    -- schemes are case-insensitive (RFC 3986 §3.1) — the API compares the
    -- same way, so `HTTPS://` is accepted by both or by neither. `http` is
    -- allowed alongside `https` because a merchant's local development host
    -- is plain HTTP; the livemode https-only rule is
    -- `vpay_api::v1::checkout_sessions`' own, decided from
    -- `deployment.livemode`, because it is deployment policy and not a
    -- property of the column.
    CONSTRAINT success_url_is_a_bounded_web_url CHECK (
        success_url IS NULL
        OR (char_length(success_url) <= 2048
            AND (lower(success_url) LIKE 'http://%' OR lower(success_url) LIKE 'https://%'))
    ),
    CONSTRAINT cancel_url_is_a_bounded_web_url CHECK (
        cancel_url IS NULL
        OR (char_length(cancel_url) <= 2048
            AND (lower(cancel_url) LIKE 'http://%' OR lower(cancel_url) LIKE 'https://%'))
    ),
    CONSTRAINT return_url_is_a_bounded_web_url CHECK (
        return_url IS NULL
        OR (char_length(return_url) <= 2048
            AND (lower(return_url) LIKE 'http://%' OR lower(return_url) LIKE 'https://%'))
    ),

    -- Both credentials, bounded exactly as `payment_intents`'
    -- `client_secret_suffix_length` is (0026). The floor is the load-bearing
    -- half: a short credential is a guessable one, and this is the only
    -- place a future writer cannot bypass. 32 is what
    -- `vpay_core::ids::client_secret_suffix` and
    -- `vpay_core::ids::return_token` mint (32 Crockford base32 characters =
    -- 160 bits).
    CONSTRAINT client_secret_suffix_length
        CHECK (char_length(client_secret_suffix) BETWEEN 32 AND 128),
    CONSTRAINT return_token_length
        CHECK (char_length(return_token) BETWEEN 32 AND 128),

    -- `pk_` plus 1 to 124 characters. Deliberately looser than
    -- `vpay_config`'s `pk_(test|live)_[A-Za-z0-9]{16,64}`: that rule includes
    -- a livemode agreement this table has no way to check, and a constraint
    -- that restated two thirds of a rule would be a second, drifting copy of
    -- it. What this catches is a writer that put something *else* in the
    -- column — a client id, a whole `cs_…_secret_…`, an empty string.
    --
    -- `\_` is a literal underscore: bare `_` is a single-character wildcard
    -- in `LIKE`, so `'pk_%'` would also accept `pkX…`.
    CONSTRAINT publishable_key_is_shaped_like_one
        CHECK (publishable_key LIKE 'pk\_%' AND char_length(publishable_key) BETWEEN 4 AND 128)
);

-- ONE OPEN SESSION PER INTENT, ENFORCED BY THE INDEX AND NOT BY A SELECT
--
-- Two open sessions on one intent are two `url`s a payer could be holding at
-- once, two return pages that would both forward, and — the case that costs
-- money — two pages racing the same `confirm`. `one_charge_per_intent`
-- (0004) is what actually stops the second charge, but the merchant-facing
-- failure would then be a `409` from a page a payer is looking at rather than
-- a `409` on the merchant's own `create` call, which is the one they can
-- handle.
--
-- Partial rather than total, and that is the deliberate half: a session that
-- has *finished* — `complete` or `expired` — blocks nothing, because the only
-- way to try again is a new PaymentIntent anyway, and a total unique index
-- would make an expired session permanently unreplaceable if that rule ever
-- relaxed. The check lives here rather than in a preceding `SELECT` for the
-- reason every guard in this schema does: between reading "no open session"
-- and inserting one, a concurrent create can commit one.
CREATE UNIQUE INDEX checkout_sessions_one_open_per_intent
    ON checkout_sessions (payment_intent_id)
    WHERE status = 'open';

-- An identity column is not implicitly unique, and every cursor below assumes
-- a total order — the same reasoning as `payment_intents_seq_key` (0014).
CREATE UNIQUE INDEX checkout_sessions_seq_key ON checkout_sessions (seq);

-- `GET /v1/checkout/sessions`: merchant-scoped, newest first. The same shape
-- as `payment_intents_merchant_seq_idx`.
CREATE INDEX checkout_sessions_merchant_seq_idx
    ON checkout_sessions (merchant_id, seq DESC);

-- The settlement transaction's lookup: "is there a session on this intent to
-- flip?", asked once per settled charge. Partial on `status = 'open'` so the
-- index stays the size of the *live* set rather than of every session ever
-- created — the same argument `events_pending_idx` (0018) and
-- `jobs_claimable_idx` (0021) make. `find_open_by_intent`, which lane 2's
-- confirm path calls to learn a session's return page, is the same lookup.
CREATE INDEX checkout_sessions_open_by_intent_idx
    ON checkout_sessions (payment_intent_id)
    WHERE status = 'open';

COMMENT ON TABLE checkout_sessions IS
    'One checkout attempt driven through vpay''s own hosted or embedded page (Step 9, D1). References an existing payment_intent; never creates one. Two payer credentials: client_secret_suffix (joined with id into cs_…_secret_…, rides in a URL fragment) and return_token (rides in the return page''s query string and authorises strictly less).';
COMMENT ON COLUMN checkout_sessions.publishable_key IS
    'The merchant publishable key every URL vpay mints for this session carries as ?key= — the hosted page, the embedded iframe and the return page. Pinned on the row rather than derived from merchant_id so a key rotation cannot strand a payer already on a rail''s page. NOT a secret: it names a tenant and authorises nothing.';
COMMENT ON COLUMN checkout_sessions.client_secret_suffix IS
    'The second half of this session''s payer-facing client_secret; the first half is `id`. Joined by vpay_core::ids::client_secret into `cs_…_secret_…`, rendered by POST/GET /v1/checkout/sessions and never by the list or by any browser route. Redacted in vpay_db::CheckoutSessionRow''s hand-written Debug.';
COMMENT ON COLUMN checkout_sessions.return_token IS
    'The credential the return page presents in a query string, because a URL fragment does not survive a rail''s redirect (D6). Authorises reading the session and its intent WITHOUT the intent''s client_secret — strictly less than client_secret_suffix above. Never rendered on any /v1 or /v1/browser response.';
COMMENT ON COLUMN checkout_sessions.payment_status IS
    'unpaid | paid | failed, written by the settlement transaction (vpay_db::settlement) in the SAME transaction as the intent status it describes, so the two cannot disagree.';
COMMENT ON CONSTRAINT urls_match_ui_mode ON checkout_sessions IS
    'A hosted session forwards the whole browser and needs success_url + cancel_url; an embedded one hands control back to the merchant''s page and needs return_url alone. Backstop for vpay_api::v1::checkout_sessions, which refuses the wrong combination with a 400 naming the parameter.';
