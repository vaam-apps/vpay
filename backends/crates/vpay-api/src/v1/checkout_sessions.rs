//! `/v1/checkout/sessions` — create, retrieve, list, expire.
//!
//! Everything a **merchant's server** can do to a Checkout Session: the
//! object that sends a payer to a page vpay itself serves (Step 9, D1 of
//! `docs/plans/2026-09-04-step9-hosted-checkout.md`). The payer-facing half
//! is [`crate::browser::checkout_sessions`]; the two share nothing but the
//! repository and the rendered object.
//!
//! **Tenancy.** Every query takes the [`MerchantScope`] the authentication
//! middleware resolved. A merchant asking for another merchant's `cs_…` gets
//! the same 404, byte for byte, as one asking for an id that never existed —
//! and a merchant naming another merchant's `pi_…` in `payment_intent` gets
//! the same `400` as one naming an intent that does not exist.
//!
//! # A session references an intent; it never creates one
//!
//! D1. Amount, currency and rails stay on `payment_intents`, where every
//! existing invariant already guards them, and this resource adds no way to
//! set them. Whether a session may create its intent inline (Stripe's shape)
//! is left to the maintainer by the plan's "Decisions left to the
//! maintainer".
//!
//! # The three things `create` refuses, and why each is a `409` and not a `400`
//!
//! The intent must be `requires_payment_method`, must have no charge, and
//! must have no other open session. All three are facts about an *object's
//! state* rather than about the request's shape — the merchant sent a
//! perfectly well-formed `pi_…` and the answer may be different in a second
//! — which is exactly [`ApiError::Conflict`]'s definition. The last of the
//! three is enforced by a partial unique index and not only by the check
//! here: see [`create`].

use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use time::OffsetDateTime;
use vpay_core::{IntentStatus, ids};
use vpay_db::{
    Charges, CheckoutSessionRow, CheckoutSessions, NewCheckoutSession, PaymentIntents,
    Repositories, SessionListPage,
};

use crate::error::{ApiError, CHECKOUT_BASE_URL_MISSING, PUBLISHABLE_KEY_MISSING};
use crate::form::VpayQuery;
use crate::model::{CheckoutSessionObject, CheckoutSessionWithSecret, ListObject};
use crate::v1::paging::{self, CursorKind};
use crate::v1::payment_intents::{ClaimOutcome, PostRequest, json_response};
use crate::v1::{MerchantScope, ResourceConfig};

/// The object type this module speaks about, in the API's own vocabulary.
/// One constant so a 404 for a session can never be spelled two ways.
///
/// `pub(crate)` only so `crate::browser::checkout_sessions`' own test can
/// assert the two spellings have **not** converged — the merchant surface
/// says `checkout.session` (the object table an SDK matches on) and the
/// payer-facing one says `checkout session`.
pub(crate) const RESOURCE: &str = "checkout.session";

/// The list envelope's `url`, and the path a cursor page is read from.
const LIST_URL: &str = "/v1/checkout/sessions";

/// This resource's cursor vocabulary — `cs_…`.
pub(crate) const CURSOR: CursorKind = CursorKind {
    prefix: ids::CHECKOUT_SESSION_PREFIX,
    noun: "a checkout session id",
};

/// How long a session stays `open` on its own (D10).
///
/// Twenty-four hours, and it is a *product* constant rather than a
/// configuration value on purpose: it is the number
/// `docs/plans/2026-09-04-step9-hosted-checkout.md` fixes, it is the same
/// horizon `vpay_worker`'s poll ladder gives a payer, and a per-deployment
/// value would mean a merchant's integration behaves differently against
/// sandbox and production for a reason no code path explains.
///
/// Stored on the row as an absolute `expires_at` rather than recomputed on
/// read, so changing this constant does not retroactively expire — or
/// un-expire — a session a payer is already looking at.
const SESSION_LIFETIME: time::Duration = time::Duration::hours(24);

/// The one `status` in which a session is still being driven (D10).
///
/// A constant rather than four string literals across two modules: the
/// merchant surface refuses to expire anything else, and the **browser**
/// surface hands out the intent's `client_secret` only in this state
/// (`crate::browser::checkout_sessions`). Two spellings would mean one of
/// those two rules quietly stopped matching the other. `vpay-db` keeps its
/// own copy for its `WHERE` clauses, tied to migration `0028`'s CHECK.
pub(crate) const OPEN: &str = "open";

/// The ceiling on `success_url`, `cancel_url` and `return_url`.
///
/// The same 2048 characters `charges.return_url` is bounded to (migration
/// `0019`) and that `checkout_sessions`' own three CHECKs repeat (`0028`).
/// Checked here so the column's CHECK is a backstop rather than the guard:
/// trip the CHECK and a merchant's over-long URL comes back as a `500`
/// telling them vpay is broken; trip this and it comes back as a `400`
/// naming the parameter, which is the truth.
const URL_MAX_CHARS: usize = 2_048;

/// The schemes a payer's browser may be forwarded to.
///
/// A closed list rather than a denylist of dangerous schemes, exactly as
/// `payment_intents::RETURN_URL_SCHEMES` is: `javascript:` is the obvious
/// one, `data:` and `vbscript:` are the ones a denylist forgets, and the set
/// that legitimately belongs here is exactly two. Under
/// `deployment.livemode` only the second is accepted — see
/// [`checked_forward_url`].
const URL_SCHEMES: [&str; 2] = ["http://", "https://"];

/// The placeholder a merchant may put in any of the three URLs, which vpay
/// substitutes when it forwards the payer (D5).
///
/// Public because it is a **wire contract**: `frontends/apps/checkout` does
/// the substituting, `sdks/*` document it, and
/// `docs/runbooks/checkout.md` tells merchants to write it. Spelled once here
/// so the side that validates it and any Rust-side substituter cannot drift.
///
/// browser-checkout's D3 — "vpay appends nothing to `return_url`" — is
/// unchanged by it: nothing is *appended*, and a merchant who omits the
/// placeholder gets exactly the URL they wrote, with no correlation
/// parameter they did not ask for.
pub const CHECKOUT_SESSION_ID_PLACEHOLDER: &str = "{CHECKOUT_SESSION_ID}";

// ----------------------------------------------------------------- create

/// `POST /v1/checkout/sessions`'s fields, as the form decoder produces them.
///
/// Every field is `Option<String>` for `CreateParams`' reason: the wire is
/// form-encoded, so every value arrives as text, and typing them here would
/// hand "not a `ui_mode`" to serde — which answers with `param: "body"` and a
/// sentence about the request's shape rather than naming the field.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CreateParams {
    payment_intent: Option<String>,
    ui_mode: Option<String>,
    success_url: Option<String>,
    cancel_url: Option<String>,
    return_url: Option<String>,
    /// Which of this merchant's publishable keys every URL vpay mints for
    /// the session should carry as `?key=`.
    ///
    /// Optional, defaulting to the tenant's **first configured key**. A
    /// merchant with one key — which is most of them — never sends it; a
    /// merchant mid-rotation, or one running two storefronts off two keys,
    /// sends the one whose page will read the session.
    ///
    /// It is a *choice among registered values*, never a free string: an
    /// unregistered key would mint a link whose page answers the uniform 404
    /// to every payer, with nothing in the response saying why.
    publishable_key: Option<String>,
}

/// The two `ui_mode` values, and which URLs each one requires.
///
/// A real enum rather than a validated `String`, unlike every other
/// vocabulary this API carries as text, because it is the only one that
/// *decides control flow* here: which URLs are required, which are refused,
/// and whether a `url` is minted at all. A `String` would put a
/// `match mode.as_str()` with an unreachable arm at three call sites, which
/// ADR-0007 forbids expressing as a `panic!` and which would otherwise have
/// to invent a default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiMode {
    /// vpay answers a `url`; the merchant redirects the payer's whole
    /// browser to it, and vpay forwards them to `success_url` or
    /// `cancel_url` when it is done.
    Hosted,
    /// vpay answers a `client_secret`; the merchant hands it to
    /// `@vaam-apps/vpay-stripe-js`, which mounts vpay's page in an iframe on their own
    /// site and receives a `vpay:complete` message.
    Embedded,
}

impl UiMode {
    /// The stored and rendered spelling — the `ui_mode_is_known` CHECK's own
    /// vocabulary (migration `0028`).
    const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Hosted => "hosted",
            Self::Embedded => "embedded",
        }
    }

    /// Parses the `ui_mode` field. Absent or empty means [`Self::Hosted`] —
    /// the wire contract's documented default, and the safer one: a merchant
    /// who omits it gets a redirect flow, which works without any front-end
    /// integration at all, rather than an iframe they have not built a page
    /// to hold.
    fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(Self::Hosted),
            Some("hosted") => Ok(Self::Hosted),
            Some("embedded") => Ok(Self::Embedded),
            Some(_) => Err(ApiError::invalid_param(
                "ui_mode",
                "`ui_mode` must be `hosted` (vpay returns a `url` to redirect the payer to) or \
                 `embedded` (vpay returns a `client_secret` for @vaam-apps/vpay-stripe-js).",
            )),
        }
    }
}

/// A create request that has passed every rule, in the shape the insert
/// needs.
///
/// A struct rather than a tuple for `ValidCreate`'s reason: three
/// `Option<String>` URLs next to each other in a tuple are one transposition
/// away from a session that forwards a successful payer to the cancel page.
///
/// `Debug` is derived and safe: every field is a merchant's own value and
/// none is a credential — the two secrets are minted by `create`, after this.
#[derive(Debug)]
struct ValidCreate {
    payment_intent: String,
    ui_mode: UiMode,
    success_url: Option<String>,
    cancel_url: Option<String>,
    return_url: Option<String>,
    /// Carried separately from the URLs because it is resolved against the
    /// *deployment's* registrations rather than checked for shape — see
    /// [`chosen_publishable_key`].
    publishable_key: String,
}

/// `POST /v1/checkout/sessions`.
///
/// # Ordering: the key is claimed first, then the deployment, then the
/// request, then the object
///
/// The `Idempotency-Key` claim runs before any rule, for
/// `payment_intents::create`'s reason: a replay must answer whatever the
/// original answered, whatever has changed since.
///
/// After that the deployment's own capability is checked *before* the
/// merchant's parameters, which is the one ordering decision worth writing
/// down. A merchant whose deployment serves no checkout page is not going to
/// be helped by being told their `success_url` is too long: the request
/// cannot succeed however they fix it, and the first answer they get should
/// be the one that says so.
///
/// # Why every failure releases the key
///
/// Nothing is written before the insert, so re-executing a corrected retry is
/// exactly equivalent to the request never having been made — and a merchant
/// who gets `checkout.public_base_url` deployed and retries under the same
/// key must get their session, not a 24-hour-old refusal that is no longer
/// true. That is `payment_intents::create`'s carve-out, applied here for the
/// same reason and to one more case.
///
/// # One open session per intent, and where that is actually enforced
///
/// [`CheckoutSessions::find_open_by_intent`] is checked below so the merchant
/// gets a sentence naming the session that is in the way. It is **not** the
/// guard: between that read and the insert, a concurrent create can commit
/// one. The guard is the partial unique index
/// `checkout_sessions_one_open_per_intent` (migration `0028`), and the
/// [`vpay_db::DbError::UniqueViolation`] it raises is turned into the same
/// `409` below — so two simultaneous creates produce one session and one
/// conflict, never two `url`s for one intent.
pub(crate) async fn create(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;

    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    // From here the key is claimed, so every path out of this function has to
    // end it. `finish` stores or releases; the `?`-free block below releases
    // before returning anything else.
    let prepared = prepare_create(&post, repositories.as_ref(), &config, &scope).await;
    let (validated, base_url) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            post.release(repositories.as_ref(), &scope, claim_id).await;
            return Err(error);
        }
    };

    let created_at = OffsetDateTime::now_utc();
    let new = NewCheckoutSession {
        id: ids::checkout_session_id(),
        merchant_id: scope.merchant_id().to_owned(),
        payment_intent_id: validated.payment_intent,
        livemode: config.livemode(),
        ui_mode: validated.ui_mode.as_wire_str().to_owned(),
        success_url: validated.success_url,
        cancel_url: validated.cancel_url,
        return_url: validated.return_url,
        publishable_key: validated.publishable_key,
        // Minted here, once, and never again — there is no rotation
        // endpoint, for the reason a payment intent has none: a retry is a
        // new intent and therefore a new session. Two independent draws from
        // the OS CSPRNG; see `vpay_core::ids::return_token` for why they are
        // two values and not one.
        client_secret_suffix: ids::client_secret_suffix(),
        return_token: ids::return_token(),
        expires_at: created_at.saturating_add(SESSION_LIFETIME),
        created_at,
    };

    let outcome = repositories
        .create(&new)
        .await
        .map_err(|error| create_error(error, &new.payment_intent_id))
        .and_then(|row| {
            // The one response that carries the credential a merchant hands
            // to a payer — through the `url` for hosted, through
            // `client_secret` for embedded. `create` is where they *get* it.
            session_response(StatusCode::CREATED, &row, Some(&base_url))
        });

    post.finish(repositories.as_ref(), &scope, claim_id, outcome)
        .await
}

/// Decodes the body, checks the deployment, checks every rule, and resolves
/// the intent — everything between "the key is claimed" and "write the row".
///
/// Split out of [`create`] so that "a failure from here must release the key"
/// is one call with one error path, rather than a dozen `?`s that would each
/// have to remember. Returns the checkout base URL alongside the validated
/// request because [`create`] needs it to render the `url` and re-reading it
/// from the config there would be a second chance for the two to disagree
/// about whether checkout is configured at all.
async fn prepare_create(
    post: &PostRequest,
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    scope: &MerchantScope,
) -> Result<(ValidCreate, String), ApiError> {
    let params: CreateParams = post.form().await?;

    // The deployment's own capability, first — see `create`'s ordering note.
    let base_url = config
        .checkout_public_base_url()
        .ok_or(ApiError::CheckoutNotConfigured(CHECKOUT_BASE_URL_MISSING))?
        .to_owned();

    // Before the URL rules, for the same reason the base URL is: a merchant
    // whose account has no publishable key cannot be helped by being told
    // their `success_url` is too long.
    let publishable_key = chosen_publishable_key(
        config,
        scope.merchant_id(),
        params.publishable_key.as_deref(),
    )?;

    let validated = validate_create(params, config.livemode(), publishable_key)?;

    // The intent, scoped. `get_for_merchant` and not `get_by_id`: the tenant
    // is a parameter of the lookup, so a foreign intent is indistinguishable
    // from a missing one and this handler has no way to compare tenants in
    // Rust and get it wrong.
    let intent = PaymentIntents::get_for_merchant(
        repositories,
        scope.merchant_id(),
        &validated.payment_intent,
    )
    .await?
    .ok_or_else(unknown_intent)?;

    // `requires_payment_method` is the only status a session can drive: a
    // `processing` or `requires_action` intent already has a payer acting on
    // it somewhere else, a `succeeded` one is paid, and a `canceled` one is
    // finished. Read-then-check rather than compare-and-swap, and that is
    // safe here for a reason worth stating: nothing about creating a session
    // *moves* the intent, so there is no lost update to lose. What a
    // concurrent confirm can do is make this check stale — and the charge
    // check below, plus `one_charge_per_intent`, is what stops that from
    // becoming a second payment.
    if intent.status != IntentStatus::INITIAL.as_wire_str() {
        return Err(ApiError::Conflict {
            message: format!(
                "A checkout session can only be created for a PaymentIntent that is still \
                 `{}`; this one is `{}`. Create a new PaymentIntent to try again.",
                IntentStatus::INITIAL.as_wire_str(),
                intent.status,
            ),
        });
    }

    // One charge per intent, forever. An intent that already has one — live
    // or terminal — cannot be paid again, so a session pointing at it would
    // mint a `url` that leads a payer to a `409`.
    if Charges::get_for_intent(repositories, &intent.id)
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict {
            message: "This PaymentIntent already has a charge. One charge per intent, forever \
                      — create a new payment intent, then a session for it."
                .to_owned(),
        });
    }

    // The friendly half of "one open session per intent"; the index is the
    // guard. See `create`'s own note.
    if let Some(open) = CheckoutSessions::find_open_by_intent(repositories, &intent.id).await? {
        return Err(open_session_conflict(&open.id));
    }

    Ok((validated, base_url))
}

/// Every rule that can be decided from the request alone.
fn validate_create(
    params: CreateParams,
    livemode: bool,
    publishable_key: String,
) -> Result<ValidCreate, ApiError> {
    let payment_intent = params
        .payment_intent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::invalid_param(
                "payment_intent",
                "A checkout session references a PaymentIntent you have already created; send \
                 its id as `payment_intent`.",
            )
        })?;
    // The *shape*, before the lookup, for `paging::validated_cursor`'s
    // reason: a `ch_…` or a truncated paste is a typo the merchant can fix
    // from the message, and letting it reach the database would answer the
    // deliberately opaque "no such intent" instead.
    if !ids::is_well_formed(ids::PAYMENT_INTENT_PREFIX, payment_intent) {
        return Err(ApiError::invalid_param(
            "payment_intent",
            "`payment_intent` must be a PaymentIntent id — `pi_` followed by 24 characters.",
        ));
    }
    let payment_intent = payment_intent.to_owned();

    let ui_mode = UiMode::parse(params.ui_mode.as_deref())?;

    // Blank is absent, everywhere: `success_url=` is what a client
    // templating an optional field emits when it has none, and treating it
    // as a present-but-malformed URL would give a merchant a message about a
    // value they did not send.
    let present = |value: Option<String>| {
        value
            .map(|raw| raw.trim().to_owned())
            .filter(|raw| !raw.is_empty())
    };
    let success_url = present(params.success_url);
    let cancel_url = present(params.cancel_url);
    let return_url = present(params.return_url);

    // Which URLs belong to which mode — the `urls_match_ui_mode` CHECK, at
    // the boundary, where it can name the parameter. Refusing the *wrong*
    // one rather than ignoring it is the point: a merchant who sends
    // `return_url` with `ui_mode: hosted` believes vpay will forward the
    // payer there, and silently dropping it is how a payer ends up on a page
    // the merchant never expected.
    match ui_mode {
        UiMode::Hosted => {
            let success_url = required(success_url, "success_url", "hosted")?;
            let cancel_url = required(cancel_url, "cancel_url", "hosted")?;
            refused(
                return_url.as_ref(),
                "return_url",
                "hosted",
                "success_url` and `cancel_url",
            )?;
            checked_forward_url(&success_url, "success_url", livemode)?;
            checked_forward_url(&cancel_url, "cancel_url", livemode)?;
            Ok(ValidCreate {
                payment_intent,
                ui_mode,
                success_url: Some(success_url),
                cancel_url: Some(cancel_url),
                return_url: None,
                publishable_key,
            })
        }
        UiMode::Embedded => {
            let return_url = required(return_url, "return_url", "embedded")?;
            refused(
                success_url.as_ref(),
                "success_url",
                "embedded",
                "return_url",
            )?;
            refused(cancel_url.as_ref(), "cancel_url", "embedded", "return_url")?;
            checked_forward_url(&return_url, "return_url", livemode)?;
            Ok(ValidCreate {
                payment_intent,
                ui_mode,
                success_url: None,
                cancel_url: None,
                return_url: Some(return_url),
                publishable_key,
            })
        }
    }
}

/// Which publishable key this session pins, from the merchant's choice or
/// from their registration's first entry.
///
/// # Why a session pins a key at all
///
/// All three `/v1/browser/checkout` routes authenticate by publishable key
/// plus a session credential, so every URL vpay mints has to carry one:
/// `?key=` on the hosted page, on the embedded iframe, and on the **return**
/// page — which cannot use a fragment, because the payer arrives there from a
/// URL the *rail* replays.
///
/// Pinning it on the row rather than resolving it at render time is what
/// makes the return URL stable: the documented key rotation is "add the new
/// one, deploy, remove the old", and a return URL derived from `merchant_id`
/// would stop resolving the moment the old key came out — stranding every
/// payer already sitting on a rail's page. Migration `0028`'s comment on the
/// column carries the same argument.
///
/// # The three answers
///
/// * a key the merchant named, **and registered to them** → that one;
/// * no key named → their first configured key, in the order the operator
///   wrote it. `first()` and not "any": a merchant reading their own YAML can
///   predict which link they will get;
/// * a key that is not theirs → `400` naming `publishable_key`;
/// * **no keys at all** → [`ApiError::CheckoutNotConfigured`], the same code
///   a missing `checkout.public_base_url` answers with and a different
///   sentence. It is a `409`-shaped situation in spirit and deliberately not
///   one: nothing about the *intent* is wrong, and the fix is a line in the
///   merchant's registration, which is an operator's edit and a deploy.
///
/// # The unregistered-key answer is a `400` and not the uniform 404
///
/// The two surfaces answer differently on purpose. On `/v1/browser` an
/// unknown key is indistinguishable from every other failure, because the
/// caller is an unauthenticated payer and any distinction is an enumeration
/// oracle. Here the caller is the merchant, authenticated, asking about their
/// *own* registration — there is nothing to hide from them, and a uniform
/// refusal would leave them unable to tell a typo from a key they forgot to
/// register. The key is echoed back for the same reason: it is theirs, and it
/// is not a secret.
///
/// # Errors
///
/// [`ApiError::invalid_param`] on `publishable_key`, or
/// [`ApiError::CheckoutNotConfigured`].
fn chosen_publishable_key(
    config: &ResourceConfig,
    merchant_id: &str,
    requested: Option<&str>,
) -> Result<String, ApiError> {
    let registered = config.publishable_keys_for(merchant_id);

    let Some(requested) = requested.map(str::trim).filter(|key| !key.is_empty()) else {
        return registered
            .first()
            .cloned()
            .ok_or(ApiError::CheckoutNotConfigured(PUBLISHABLE_KEY_MISSING));
    };

    // A plain `==` scan and not a constant-time compare: a publishable key is
    // not a secret, the caller already holds a merchant token, and the answer
    // is about their own registration. `browser::secrets_match` exists for
    // the value that *is* a credential.
    if registered.iter().any(|key| key == requested) {
        return Ok(requested.to_owned());
    }
    // A tenant with none gets the configuration answer even when they named a
    // key, because "that key is not yours" would be true but useless: there
    // is no key of theirs to name instead.
    if registered.is_empty() {
        return Err(ApiError::CheckoutNotConfigured(PUBLISHABLE_KEY_MISSING));
    }
    Err(ApiError::invalid_param(
        "publishable_key",
        format!(
            "`{requested}` is not one of this account's registered publishable keys. Omit \
             `publishable_key` to use the first one, or register this key first."
        ),
    ))
}

/// A URL this mode requires, or a `400` naming it.
fn required(value: Option<String>, param: &'static str, mode: &str) -> Result<String, ApiError> {
    value.ok_or_else(|| {
        ApiError::invalid_param(
            param,
            format!("`{param}` is required when `ui_mode` is `{mode}`."),
        )
    })
}

/// A URL this mode does not have, refused rather than dropped.
fn refused(
    value: Option<&String>,
    param: &'static str,
    mode: &str,
    instead: &str,
) -> Result<(), ApiError> {
    if value.is_some() {
        return Err(ApiError::invalid_param(
            param,
            format!("`{param}` does not apply when `ui_mode` is `{mode}`; use `{instead}`."),
        ));
    }
    Ok(())
}

/// Refuses a URL that would be a `500` from a CHECK, a redirect a browser
/// would *execute* rather than navigate to, or a plaintext destination on a
/// deployment handling real money.
///
/// # Why `{CHECKOUT_SESSION_ID}` needs no special case
///
/// D5's placeholder is a literal substring of a URL a merchant writes, and
/// the three rules here are a scheme prefix, a character count and — under
/// livemode — the scheme again. None of them parses the URL, so a `{` in a
/// query string is simply a character. That is deliberate rather than
/// convenient: `url::Url::parse` would percent-encode the braces, and a
/// validator that *normalised* the value would either have to store the
/// normalised form (breaking the substitution the placeholder exists for) or
/// discard its own parse (proving nothing). The same argument
/// `payment_intents::checked_return_url` makes, plus this one.
///
/// # Errors
///
/// [`ApiError::invalid_param`] naming `param`.
fn checked_forward_url(url: &str, param: &'static str, livemode: bool) -> Result<(), ApiError> {
    // Lowercased because URL schemes are case-insensitive (RFC 3986 §3.1) —
    // the column's CHECK compares the same way, so `HTTPS://` is accepted by
    // both or by neither.
    let lowercase = url.to_lowercase();
    if !URL_SCHEMES
        .iter()
        .any(|scheme| lowercase.starts_with(scheme))
    {
        return Err(ApiError::invalid_param(
            param,
            format!(
                "`{param}` must be an `http://` or `https://` URL — it is where the payer's \
                 browser is sent."
            ),
        ));
    }
    // Under livemode, `https` alone. Not an environment branch (ADR-0003):
    // the rule is one line of *policy* read from `deployment.livemode`, the
    // same value `vpay_config`'s `validate_host` reads, and the code path is
    // identical either way. `http` is accepted otherwise because a
    // merchant's local development host is plain HTTP and refusing it would
    // push people to a worse workaround.
    if livemode && !lowercase.starts_with("https://") {
        return Err(ApiError::invalid_param(
            param,
            format!(
                "`{param}` must be an `https://` URL on a livemode deployment: the payer is \
                 forwarded there carrying the outcome of a real payment."
            ),
        ));
    }
    // Characters, not bytes: the column's CHECK is `char_length`, and
    // counting bytes here would refuse a legal URL whose query string is not
    // ASCII.
    if url.chars().count() > URL_MAX_CHARS {
        return Err(ApiError::invalid_param(
            param,
            format!("`{param}` must be at most {URL_MAX_CHARS} characters."),
        ));
    }
    Ok(())
}

/// The refusal for a `payment_intent` this merchant cannot use.
///
/// **One function, two causes.** An id that names no intent and an id that
/// names another merchant's answer identically, for
/// `ApiError::NotFound`'s reason applied to a body parameter: a distinct
/// answer would make `POST /v1/checkout/sessions` an oracle for which `pi_…`
/// exist under some other tenant.
///
/// A `400` naming the parameter rather than a `404`, because the id is a
/// *field of the request* and not the resource being addressed — the same
/// distinction Stripe draws, and what lets an SDK point at the field.
fn unknown_intent() -> ApiError {
    ApiError::invalid_param(
        "payment_intent",
        "No such PaymentIntent for this account. A checkout session references an intent you \
         created with `POST /v1/payment_intents`.",
    )
}

/// The refusal when an intent already has an open session.
///
/// Names the session that is in the way, because that is the whole of what a
/// merchant does next: retrieve it (its `url` is still valid) or expire it.
/// The id is vpay's own and is already known to this merchant — it is theirs
/// — so naming it reflects nothing.
fn open_session_conflict(existing_id: &str) -> ApiError {
    ApiError::Conflict {
        message: format!(
            "This PaymentIntent already has an open checkout session ({existing_id}). Retrieve \
             it for its `url`, or expire it first."
        ),
    }
}

/// Turns the insert's storage errors into the merchant-facing answer.
///
/// The [`vpay_db::DbError::UniqueViolation`] arm is the one that matters:
/// it is the partial unique index firing, which means a concurrent create
/// won the race between `find_open_by_intent` and this insert. It has to
/// answer the same `409` the pre-check does, or a merchant would see two
/// different errors for one situation depending on timing.
///
/// The id of the winning session is not looked up: doing so would be a second
/// query on a path that has just lost a race, and the merchant's next step —
/// retrieve the intent's session — is the same either way.
fn create_error(error: vpay_db::DbError, payment_intent_id: &str) -> ApiError {
    match error {
        vpay_db::DbError::UniqueViolation { constraint, .. }
            if constraint == "checkout_sessions_one_open_per_intent" =>
        {
            ApiError::Conflict {
                message: format!(
                    "PaymentIntent {payment_intent_id} already has an open checkout session. \
                     Retrieve it for its `url`, or expire it first."
                ),
            }
        }
        // A foreign key violation here means the intent was deleted between
        // the scoped read above and this insert — which nothing in this
        // system does. It answers as "no such intent" rather than as a
        // storage failure, because that is what the merchant would see if
        // they retried.
        vpay_db::DbError::ForeignKeyViolation { .. } => unknown_intent(),
        other => ApiError::from(other),
    }
}

// --------------------------------------------------------------- retrieve

/// `GET /v1/checkout/sessions/{id}`.
///
/// Renders the `client_secret` and the `url`, exactly as `create` did, so a
/// merchant who lost the create response can recover both without creating a
/// second session. Their bearer token is what authorises this; the browser
/// route reaches a different function with a `client_secret` instead.
pub(crate) async fn retrieve(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let row = CheckoutSessions::get_for_merchant(repositories.as_ref(), scope.merchant_id(), &id)
        .await?
        .ok_or_else(|| not_found(&id))?;
    session_response(StatusCode::OK, &row, config.checkout_public_base_url())
}

// ------------------------------------------------------------------- list

/// `GET /v1/checkout/sessions`'s query parameters — text for the same reason
/// `CreateParams`' fields are.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ListParams {
    limit: Option<String>,
    starting_after: Option<String>,
    ending_before: Option<String>,
    payment_intent: Option<String>,
}

/// `GET /v1/checkout/sessions`.
///
/// **No `client_secret` and no `url` on any row**, and that is the whole
/// reason [`CheckoutSessionWithSecret`] is a separate type: one page would
/// otherwise hand a merchant's integration a live credential for every
/// session on it, and a list response is the one most likely to be logged
/// wholesale. The `url` is omitted for the same reason and not a weaker one —
/// it carries the same credential in its fragment.
pub(crate) async fn list(
    State(repositories): State<Arc<dyn Repositories>>,
    scope: MerchantScope,
    VpayQuery(params): VpayQuery<ListParams>,
) -> Result<Response, ApiError> {
    let page = paging::list_page(
        params.limit.as_deref(),
        params.starting_after,
        params.ending_before,
        CURSOR,
    )?;

    // The filter's *shape* is checked for `validated_cursor`'s reason: a
    // `cs_…` sent as `payment_intent` (an easy mistake, since both ids are on
    // the object) would otherwise return an empty page with nothing to fix.
    let payment_intent = params
        .payment_intent
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty());
    if let Some(intent) = payment_intent.as_deref()
        && !ids::is_well_formed(ids::PAYMENT_INTENT_PREFIX, intent)
    {
        return Err(ApiError::invalid_param(
            "payment_intent",
            "`payment_intent` must be a PaymentIntent id — `pi_` followed by 24 characters.",
        ));
    }

    let page = SessionListPage {
        limit: page.limit,
        starting_after: page.starting_after,
        ending_before: page.ending_before,
        payment_intent,
    };

    let (rows, has_more) =
        CheckoutSessions::list_page(repositories.as_ref(), scope.merchant_id(), &page).await?;
    let data: Vec<CheckoutSessionObject> = rows
        .iter()
        // `None`: no `url` on a list row. See this function's own doc.
        .map(|row| CheckoutSessionObject::from_row(row, None))
        .collect();

    json_response(StatusCode::OK, &ListObject::new(data, has_more, LIST_URL))
}

// ----------------------------------------------------------------- expire

/// `POST /v1/checkout/sessions/{id}/expire`.
///
/// The expiry is a compare-and-swap with a second guard, not a
/// read-then-write, for `payment_intents::cancel`'s reason: between reading a
/// session and writing `expired`, a payer's page may already have confirmed,
/// and expiring *then* would tell a merchant the checkout was abandoned while
/// the rail holds a live payment. Both guards — the status and `NOT EXISTS` a
/// live charge — are predicates of the `UPDATE`
/// (`vpay_db::checkout_sessions`), so there is no gap.
///
/// `Ok(None)` is ambiguous by construction, and the re-read below is what
/// turns it into the right answer: `404` if there is no such session for this
/// merchant, and one of two `409`s otherwise.
pub(crate) async fn expire(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    scope: MerchantScope,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let post = PostRequest::read(request).await?;
    let claim_id = match post.claim_or_answer(repositories.as_ref(), &scope).await? {
        ClaimOutcome::Owned(claim_id) => claim_id,
        ClaimOutcome::Answered(response) => return Ok(response),
    };

    let outcome = expire_once(repositories.as_ref(), &config, &scope, &id).await;
    post.finish(repositories.as_ref(), &scope, claim_id, outcome)
        .await
}

/// The expiry itself, and the re-read that names which of the three things
/// `Ok(None)` meant.
async fn expire_once(
    repositories: &dyn Repositories,
    config: &ResourceConfig,
    scope: &MerchantScope,
    id: &str,
) -> Result<Response, ApiError> {
    if let Some(row) = CheckoutSessions::expire(repositories, scope.merchant_id(), id).await? {
        return session_response(StatusCode::OK, &row, config.checkout_public_base_url());
    }

    // The guard refused. Re-read *scoped*, so a session of another
    // merchant's is still indistinguishable from one that does not exist.
    let current = CheckoutSessions::get_for_merchant(repositories, scope.merchant_id(), id)
        .await?
        .ok_or_else(|| not_found(id))?;

    // A session that is not `open` is the ordinary double-tap: the merchant
    // (or the settlement transaction, or a previous call) already finished
    // it. Told apart from the live-charge case, because the two need
    // different things done about them — nothing, versus wait and poll.
    if current.status != OPEN {
        return Err(ApiError::Conflict {
            message: format!(
                "This checkout session is already `{}` and cannot be expired.",
                current.status
            ),
        });
    }

    Err(ApiError::Conflict {
        message: format!(
            "A charge for this session's PaymentIntent is being resolved with the rail; poll \
             GET /v1/payment_intents/{} — expiring the session now would tell you a payment \
             was abandoned while the rail may still take it.",
            current.payment_intent_id
        ),
    })
}

// --------------------------------------------------------------- plumbing

/// The 404 for a session, built through one function so its two call sites
/// cannot drift.
fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

/// The payer link for a hosted session, or `None`.
///
/// **The one place the fragment is joined**, on the minting side, so "which
/// URL did a payer actually get?" has one answer.
///
/// `None` for an embedded session — there is nothing to redirect to, the
/// merchant's own page mounts the iframe from the `client_secret` — and
/// `None` when the deployment configures no checkout app, which a merchant
/// can only see on a session created before that configuration was removed:
/// `create` refuses outright ([`ApiError::CheckoutNotConfigured`]).
///
/// # What is in the query string and what is in the fragment, and why
///
/// `?key=` is the merchant's publishable key. It has to be a query
/// parameter: the page reads it *server-side*, in `middleware.ts`, to look up
/// the tenant's `checkout_origins` and set `frame-ancestors` on the HTML
/// response before any script runs (D4) — and a fragment is never sent to a
/// server. It is safe there because a publishable key names a tenant and
/// authorises nothing.
///
/// `#{client_secret}` is the credential, and it is in the fragment for the
/// opposite reason (D6): a `#` never leaves the browser, so it is not written
/// to the checkout app's access logs, not sent as a `Referer`, and not
/// visible to any proxy in between. The two live on opposite sides of the
/// `#` precisely because they need opposite things.
///
/// Order matters to URL syntax and not to anything else: a query string comes
/// before a fragment, and everything after the first `#` is the fragment —
/// which is what keeps the secret out of `?key=`'s half no matter what is
/// added to it later.
///
/// `base` has already had its trailing slash removed by
/// [`ResourceConfig::from_config`], so this is a plain `format!`.
fn hosted_url(row: &CheckoutSessionRow, base: Option<&str>) -> Option<String> {
    if row.ui_mode != "hosted" {
        return None;
    }
    let base = base?;
    let secret = ids::client_secret(&row.id, &row.client_secret_suffix);
    Some(format!(
        "{base}/c/{}?key={}#{secret}",
        row.id, row.publishable_key
    ))
}

/// Renders a stored session **with** its `client_secret` and its `url`.
///
/// The only renderer the merchant surface's create, retrieve and expire use,
/// so "which `/v1` responses carry the credential?" has one answer: all three
/// of those, and never the list — which builds its rows directly and is the
/// reason [`CheckoutSessionWithSecret`] exists at all.
///
/// # Errors
///
/// [`ApiError::Internal`] if the object will not serialise, which for a wire
/// DTO means a bug in a `Serialize` impl.
fn session_response(
    status: StatusCode,
    row: &CheckoutSessionRow,
    base_url: Option<&str>,
) -> Result<Response, ApiError> {
    let object = CheckoutSessionObject::from_row(row, hosted_url(row, base_url));
    let secret = ids::client_secret(&row.id, &row.client_secret_suffix);
    json_response(status, &CheckoutSessionWithSecret::new(object, secret))
}

#[cfg(test)]
mod tests {
    use vpay_core::Classify as _;

    use super::*;

    /// A stored row, for the two renderers that take one. Built here rather
    /// than shared with `vpay-db`'s own test fixture, because a `pub` test
    /// helper is a `pub` item nothing ships.
    fn row() -> CheckoutSessionRow {
        CheckoutSessionRow {
            id: "cs_0123456789abcdefghjkmnpq".to_owned(),
            seq: 1,
            merchant_id: "acme-cameroon-tenant".to_owned(),
            payment_intent_id: "pi_0123456789abcdefghjkmnpq".to_owned(),
            livemode: false,
            ui_mode: "hosted".to_owned(),
            status: "open".to_owned(),
            payment_status: "unpaid".to_owned(),
            success_url: Some(OK_URL.to_owned()),
            cancel_url: Some(OK_URL.to_owned()),
            return_url: None,
            publishable_key: PK.to_owned(),
            client_secret_suffix: "neverlogthissessioncredential000".to_owned(),
            return_token: "neverlogthisreturntoken000000000".to_owned(),
            expires_at: OffsetDateTime::UNIX_EPOCH,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn params(
        ui_mode: Option<&str>,
        success: Option<&str>,
        cancel: Option<&str>,
        ret: Option<&str>,
    ) -> CreateParams {
        CreateParams {
            payment_intent: Some("pi_0123456789abcdefghjkmnpq".to_owned()),
            ui_mode: ui_mode.map(str::to_owned),
            success_url: success.map(str::to_owned),
            cancel_url: cancel.map(str::to_owned),
            return_url: ret.map(str::to_owned),
            // The key is resolved before `validate_create` runs (see
            // `prepare_create`), so it is never read from these params by the
            // function under test — `chosen_publishable_key` has its own.
            publishable_key: None,
        }
    }

    const OK_URL: &str = "https://shop.example/done";

    /// The tenant's registered key, and the one every link below carries.
    const PK: &str = "pk_test_acmecameroonsandbox01";
    /// A second registered key, for the "the merchant chose one" case.
    const PK_SECOND: &str = "pk_test_acmecameroonsandbox02";
    const MERCHANT: &str = "acme-cameroon-tenant";

    /// The default is `hosted`, and it is the default because it is the mode
    /// that works with no front-end integration at all.
    #[test]
    fn an_absent_or_blank_ui_mode_is_hosted_and_an_unknown_one_is_a_400() {
        for raw in [None, Some(""), Some("   "), Some("hosted")] {
            assert_eq!(
                UiMode::parse(raw).expect("a hosted spelling"),
                UiMode::Hosted,
                "{raw:?}"
            );
        }
        assert_eq!(
            UiMode::parse(Some("embedded")).expect("embedded"),
            UiMode::Embedded
        );

        for raw in ["Hosted", "HOSTED", "iframe", "redirect", "embed", "none"] {
            let error = UiMode::parse(Some(raw)).expect_err("{raw} is not a ui_mode");
            assert_eq!(error.param(), Some("ui_mode"), "{raw}");
        }
    }

    /// The wire contract's URL rules, as a table: which are required, which
    /// are refused, and that the refusal names the field the merchant sent.
    ///
    /// The *refusal* rows are the ones worth having. Ignoring a `return_url`
    /// on a hosted session would pass every "the session was created" test
    /// and would then forward a payer somewhere the merchant did not choose.
    #[test]
    fn each_ui_mode_requires_its_own_urls_and_refuses_the_others() {
        // hosted: both, and no return_url.
        let ok = validate_create(
            params(Some("hosted"), Some(OK_URL), Some(OK_URL), None),
            false,
            PK.to_owned(),
        )
        .expect("a complete hosted session");
        assert_eq!(ok.ui_mode, UiMode::Hosted);
        assert_eq!(ok.success_url.as_deref(), Some(OK_URL));
        assert_eq!(ok.cancel_url.as_deref(), Some(OK_URL));
        assert_eq!(ok.return_url, None);

        // embedded: return_url alone.
        let ok = validate_create(
            params(Some("embedded"), None, None, Some(OK_URL)),
            false,
            PK.to_owned(),
        )
        .expect("a complete embedded session");
        assert_eq!(ok.ui_mode, UiMode::Embedded);
        assert_eq!(ok.return_url.as_deref(), Some(OK_URL));
        assert_eq!(ok.success_url, None);
        assert_eq!(ok.cancel_url, None);

        let cases: [(CreateParams, &str); 6] = [
            (
                params(Some("hosted"), None, Some(OK_URL), None),
                "success_url",
            ),
            (
                params(Some("hosted"), Some(OK_URL), None, None),
                "cancel_url",
            ),
            (
                params(Some("hosted"), Some(OK_URL), Some(OK_URL), Some(OK_URL)),
                "return_url",
            ),
            (params(Some("embedded"), None, None, None), "return_url"),
            (
                params(Some("embedded"), Some(OK_URL), None, Some(OK_URL)),
                "success_url",
            ),
            (
                params(Some("embedded"), None, Some(OK_URL), Some(OK_URL)),
                "cancel_url",
            ),
        ];
        for (params, expected_param) in cases {
            let mode = params.ui_mode.clone();
            let error = validate_create(params, false, PK.to_owned())
                .expect_err("the URL combination must be refused");
            assert_eq!(
                error.param(),
                Some(expected_param),
                "{mode:?}: the refusal must name the field the merchant sent"
            );
        }

        // Blank is absent, not malformed: `success_url=` from a client
        // templating an optional field must read as "you did not send one".
        let error = validate_create(
            params(Some("hosted"), Some("  "), Some(OK_URL), None),
            false,
            PK.to_owned(),
        )
        .expect_err("a blank success_url is a missing one");
        assert_eq!(error.param(), Some("success_url"));
    }

    /// `payment_intent` is required, and its *shape* is checked before any
    /// lookup — so a `cs_…` or a truncated paste is a message the merchant
    /// can act on rather than the deliberately opaque "no such intent".
    #[test]
    fn the_payment_intent_reference_is_required_and_shape_checked() {
        for raw in [None, Some(""), Some("   ")] {
            let mut p = params(Some("hosted"), Some(OK_URL), Some(OK_URL), None);
            p.payment_intent = raw.map(str::to_owned);
            let error =
                validate_create(p, false, PK.to_owned()).expect_err("payment_intent is required");
            assert_eq!(error.param(), Some("payment_intent"));
        }
        for raw in [
            "cs_0123456789abcdefghjkmnpq",
            "ch_0123456789abcdefghjkmnpq",
            "pi_short",
            "pi_iiiiiiiiiiiiiiiiiiiiiiii",
            "pi_0123456789abcdefghjkmnpqZ",
        ] {
            let mut p = params(Some("hosted"), Some(OK_URL), Some(OK_URL), None);
            p.payment_intent = Some(raw.to_owned());
            let error =
                validate_create(p, false, PK.to_owned()).expect_err("{raw} is not a pi_ id");
            assert_eq!(error.param(), Some("payment_intent"), "{raw}");
        }
    }

    /// The scheme rule, the livemode rule and the length rule, and the one
    /// value that must pass all three: a URL carrying D5's placeholder.
    ///
    /// Decisive on the placeholder: make [`checked_forward_url`] parse the
    /// URL with `url::Url::parse` and compare against its re-serialisation,
    /// and the `{CHECKOUT_SESSION_ID}` rows fail — the parser
    /// percent-encodes the braces.
    #[test]
    fn a_forward_url_must_be_http_s_bounded_and_https_under_livemode() {
        for good in [
            "https://shop.example/ok",
            "http://localhost:3000/ok",
            "HTTPS://Shop.Example/OK",
            "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
            "https://shop.example/{CHECKOUT_SESSION_ID}",
        ] {
            checked_forward_url(good, "success_url", false)
                .unwrap_or_else(|error| panic!("{good} must be accepted in sandbox: {error}"));
        }

        for bad in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "//shop.example/ok",
            "shop.example/ok",
            "",
        ] {
            let error =
                checked_forward_url(bad, "success_url", false).expect_err("{bad} must be refused");
            assert_eq!(error.param(), Some("success_url"), "{bad}");
        }

        // Livemode: https only, and the placeholder still survives.
        checked_forward_url(
            "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
            "success_url",
            true,
        )
        .expect("an https URL with the placeholder is fine in livemode");
        let error = checked_forward_url("http://shop.example/ok", "success_url", true)
            .expect_err("http is refused under livemode");
        assert_eq!(error.param(), Some("success_url"));
        // …and accepted when the deployment is not live, which is the whole
        // reason the rule takes a parameter rather than being unconditional.
        checked_forward_url("http://shop.example/ok", "success_url", false)
            .expect("http is a merchant's local development host in sandbox");

        // The bound is characters and the column's CHECK is `char_length`,
        // so they agree exactly at the boundary.
        let at_limit = format!("https://shop.example/{}", "a".repeat(URL_MAX_CHARS - 21));
        assert_eq!(at_limit.chars().count(), URL_MAX_CHARS);
        checked_forward_url(&at_limit, "success_url", false).expect("2048 characters is allowed");
        let error = checked_forward_url(&format!("{at_limit}a"), "success_url", false)
            .expect_err("2049 characters is not");
        assert_eq!(error.param(), Some("success_url"));
    }

    /// The placeholder is a wire contract shared with a Next app and two
    /// SDKs that cannot import this constant.
    ///
    /// Pinned as a literal for `CLIENT_SECRET_INFIX`'s reason: a rename that
    /// only touched the constant would compile, pass every other test, and
    /// leave every merchant's `success_url` carrying an unsubstituted
    /// placeholder into their own analytics.
    #[test]
    fn the_placeholder_is_spelled_the_way_merchants_write_it() {
        assert_eq!(CHECKOUT_SESSION_ID_PLACEHOLDER, "{CHECKOUT_SESSION_ID}");
    }

    /// The hosted `url` is `{base}/c/{id}#{client_secret}` — the fragment,
    /// never a query string (D6) — and an embedded session has none.
    #[test]
    fn a_hosted_url_carries_the_secret_in_its_fragment_and_an_embedded_one_has_no_url() {
        let mut row = row();

        let url = hosted_url(&row, Some("https://checkout.example"))
            .expect("a hosted session on a configured deployment has a url");
        assert_eq!(
            url,
            format!(
                "https://checkout.example/c/{}?key={PK}#{}",
                row.id,
                ids::client_secret(&row.id, &row.client_secret_suffix)
            )
        );
        // The two halves, and which value belongs on which side of the `#`.
        // The query string carries the publishable key — the page reads it
        // server-side to set `frame-ancestors`, and a fragment never reaches
        // a server. The credential is after the `#` and nowhere else: in the
        // query string it would be in the checkout app's access log.
        let (before_fragment, fragment) = url.split_once('#').expect("a fragment");
        // The messages below describe the shape and never print the URL:
        // it carries the credential, and a panic message is a log line.
        assert!(
            !before_fragment.contains(&row.client_secret_suffix),
            "the credential must not appear before the fragment (query part is {} chars)",
            before_fragment.len()
        );
        assert!(
            before_fragment.ends_with(&format!("?key={PK}")),
            "the publishable key must be the last query parameter before the fragment"
        );
        assert!(
            !fragment.contains('?') && !fragment.contains('&'),
            "the fragment is the credential and nothing else (fragment is {} chars)",
            fragment.len()
        );

        // A deployment with no checkout app renders no url rather than a
        // link to nothing. `create` refuses outright; this is the read path.
        assert_eq!(hosted_url(&row, None), None);

        row.ui_mode = "embedded".to_owned();
        assert_eq!(
            hosted_url(&row, Some("https://checkout.example")),
            None,
            "an embedded session is mounted by the merchant's own page and has nowhere to \
             redirect to"
        );
    }

    /// The refusal a merchant gets on a deployment that serves no checkout
    /// page names the key, carries its own code, and does not tell them to
    /// retry.
    ///
    /// The last of those is the one that would be silently wrong: classified
    /// as `Category::Storage` — the category whose status is the `503` the
    /// plan asked for — this would answer `Retry::AfterBackoff`, i.e. tell an
    /// SDK to retry a request that cannot succeed until someone deploys.
    #[test]
    fn an_unconfigured_deployment_answers_checkout_not_configured_and_never_retry() {
        // Two gaps, one code, two sentences — see the variant's own doc.
        for (error, must_name) in [
            (
                ApiError::CheckoutNotConfigured(CHECKOUT_BASE_URL_MISSING),
                "checkout.public_base_url",
            ),
            (
                ApiError::CheckoutNotConfigured(PUBLISHABLE_KEY_MISSING),
                "publishable_keys",
            ),
        ] {
            assert_eq!(error.code(), "checkout_not_configured");
            assert_eq!(error.retry(), vpay_core::Retry::Never);
            assert_eq!(error.category(), vpay_core::Category::Configuration);
            let message = error.public_message();
            assert!(
                message.contains(must_name),
                "the message must name the key an operator has to set: {message}"
            );
        }
        // …and the two sentences are genuinely different, or the code would
        // be all a merchant had and the distinction would be decoration.
        assert_ne!(CHECKOUT_BASE_URL_MISSING, PUBLISHABLE_KEY_MISSING);
    }

    /// Which key a session pins: the merchant's choice when they name a
    /// registered one, their first configured key when they do not, a `400`
    /// for a key that is not theirs, and the configuration answer for a
    /// tenant with none.
    ///
    /// The **order** assertion is the one that would otherwise rot: "the
    /// first configured key" is a rule a merchant can predict from their own
    /// YAML, and a `BTreeMap` somewhere in the projection would silently make
    /// it alphabetical instead.
    #[test]
    fn a_session_pins_the_named_key_or_the_tenants_first_one() {
        let config = |keys: &[&str]| {
            let mut config = crate::v1::tests::config();
            config
                .merchant_clients
                .first_mut()
                .expect("the fixture registers one merchant")
                .publishable_keys = keys.iter().map(|key| (*key).to_owned()).collect();
            ResourceConfig::from_config(&config).expect("the fixture projects onto the port")
        };

        // Deliberately *not* alphabetical: `PK_SECOND` sorts after `PK`, so a
        // reversed registration is what tells a `BTreeMap` apart from the
        // operator's own order.
        let two = config(&[PK_SECOND, PK]);
        assert_eq!(
            chosen_publishable_key(&two, MERCHANT, None).expect("a default"),
            PK_SECOND,
            "the default is the *first configured* key, not the first alphabetically"
        );
        assert_eq!(
            chosen_publishable_key(&two, MERCHANT, Some(PK)).expect("a registered key"),
            PK
        );
        // Blank is absent, everywhere.
        assert_eq!(
            chosen_publishable_key(&two, MERCHANT, Some("  ")).expect("blank is absent"),
            PK_SECOND
        );

        // A key that is not theirs: a `400` naming the parameter, because
        // this caller is the merchant asking about their own registration —
        // the payer-facing uniform 404 exists for a caller who must not be
        // able to tell.
        let error = chosen_publishable_key(&two, MERCHANT, Some("pk_test_somebodyelseskey0001"))
            .expect_err("an unregistered key must be refused");
        assert_eq!(error.param(), Some("publishable_key"));

        // A tenant with none gets the configuration answer either way: there
        // is no key of theirs to name instead.
        let none = config(&[]);
        for requested in [None, Some(PK)] {
            let error = chosen_publishable_key(&none, MERCHANT, requested)
                .expect_err("a tenant with no keys cannot have a checkout link built");
            assert_eq!(error.code(), "checkout_not_configured");
            assert!(
                error.public_message().contains("publishable_keys"),
                "{}",
                error.public_message()
            );
        }
    }
}
