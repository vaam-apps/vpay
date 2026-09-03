-- Where a redirect rail sends the payer back, as the merchant gave it to us.
--
-- WHY A COLUMN AND NOT `provider_ref_extra`
--
-- `provider_ref_extra` is `vpay_provider::RefExtra`, and the port's own doc
-- comment defines it as *rail-supplied* key material ("Rail key material the
-- core must persist to query status later — Orange's `pay_token`, for
-- instance. Opaque to the core."). `return_url` is the opposite of that: the
-- merchant supplied it, the core owns it, and no adapter reads it. Two
-- reasons that distinction is worth a column rather than one more JSON key:
--
--   1. `parse_callback` returns a `ref_extra` the reconciler is meant to be
--      able to use to *repair* a charge whose write was lost
--      (docs/flows/crash-safety.md, docs/flows/reconciler.md). A repair that
--      replaced the JSON document would silently take the merchant's
--      `return_url` with it, and `GET /v1/payment_intents/{id}` would stop
--      being able to render the `next_action` it already returned once.
--   2. `next_action.redirect_to_url.return_url` must be reproducible on
--      every later read of the intent (docs/api/README.md's object table:
--      the object always carries every key). A column is the thing a query
--      can select and a constraint can guard; a key inside an opaque bag is
--      neither.
--
-- Nullable, because only redirect rails have one: a push rail prompts the
-- payer's own handset and there is no browser to send anywhere. The API
-- refuses a redirect confirm that omits it, so the pairing that matters
-- ("`redirect_url` implies `return_url`") is enforced where the request is
-- validated rather than by a CHECK here — a charge is inserted with the
-- merchant's `return_url` *before* the rail is called, and only learns its
-- `redirect_url` afterwards, so the two columns are legitimately out of step
-- for the duration of the submit.
ALTER TABLE charges ADD COLUMN return_url TEXT;

ALTER TABLE charges
    -- 2048 is the practical URL limit every browser and proxy agrees on,
    -- and `vpay_api::v1::payment_intents::checked_return_url` refuses the
    -- same length *and the same schemes* before the insert, so a merchant
    -- sending a 4 KB `return_url` gets a `400` naming the parameter rather
    -- than the `503` an unguarded CHECK violation becomes. This constraint
    -- is the backstop for a writer that forgets, not the primary guard —
    -- which is exactly why both exist.
    ADD CONSTRAINT return_url_length
        CHECK (return_url IS NULL OR char_length(return_url) <= 2048);

ALTER TABLE charges
    -- WHY A SCHEME, AND NOT JUST A LENGTH
    --
    -- Both of these columns are URLs that end up in a *browser*:
    -- `return_url` is the merchant's, rendered back as
    -- `next_action.redirect_to_url.return_url`; `redirect_url` is the
    -- rail's own hosted-payment page, rendered as
    -- `next_action.redirect_to_url.url` and followed by a payer. A column
    -- that accepts `javascript:…` is a stored XSS in whatever renders the
    -- intent — a merchant's checkout, or this project's own dashboard —
    -- and neither of the two writers is in a position to be the only
    -- guard: the merchant supplies one and a *rail* supplies the other.
    --
    -- `lower(...) LIKE` rather than a regex, and lowercased because URL
    -- schemes are case-insensitive (RFC 3986 §3.1): both writers compare
    -- case-insensitively too, so `HTTPS://` is accepted by all three or by
    -- none. `http` is allowed alongside `https` because the WireMock stub
    -- rails and `compose.yml` serve over plain HTTP; the livemode
    -- https-only rule is `vpay_config`'s `validate_host`, which is where a
    -- deployment-wide policy belongs.
    ADD CONSTRAINT return_url_is_a_web_url
        CHECK (
            return_url IS NULL
            OR lower(return_url) LIKE 'http://%'
            OR lower(return_url) LIKE 'https://%'
        );

ALTER TABLE charges
    -- The rail's half of the same rule. `redirect_url` has been unbounded
    -- and unchecked since `0004`; the Step 3 security review found that
    -- `vpay_adapter_orange_money::mapping::submitted` only tested it for
    -- non-emptiness, so whatever the rail put in `payment_url` became the
    -- URL a payer was sent to. The adapter now refuses a non-`http(s)` or
    -- over-long value itself (`checked_redirect_url`) — this is the
    -- constraint that makes that refusal a property of the *system* rather
    -- than of one adapter, and that a second rail cannot forget.
    ADD CONSTRAINT redirect_url_is_a_bounded_web_url
        CHECK (
            redirect_url IS NULL
            OR (
                char_length(redirect_url) <= 2048
                AND (
                    lower(redirect_url) LIKE 'http://%'
                    OR lower(redirect_url) LIKE 'https://%'
                )
            )
        );

COMMENT ON CONSTRAINT redirect_url_is_a_bounded_web_url ON charges IS
    'A rail-supplied URL a payer is redirected to must be a bounded http(s) URL. Backstop for vpay_adapter_*::mapping, which refuses the same values before the write.';

COMMENT ON COLUMN charges.return_url IS
    'Merchant-supplied return destination for a redirect rail, persisted before the rail is called and rendered back as next_action.redirect_to_url.return_url. NOT rail material — that is provider_ref_extra, which a callback repair may overwrite wholesale.';
