//! `/v1/browser/checkout/…` — the three routes **vpay's own checkout page**
//! calls, with no merchant credential anywhere in the picture.
//!
//! STATUS: all three are implemented. The page that calls them is
//! `frontends/apps/checkout` (lane 3 of Step 9); this module is the wire
//! contract it is written against.
//!
//! # Three routes, three different credentials, on purpose
//!
//! | Route | Presents | May read |
//! |---|---|---|
//! | `GET /checkout/sessions/{id}` | `key` + the **session's** `client_secret` | the session, `payment_intent` **expanded and carrying its own `client_secret`** |
//! | `GET /checkout/sessions/{id}/return` | `key` + the session's `return_token` | the session, `payment_intent` expanded **without** its secret |
//! | `GET /checkout/origins` | `key` alone | which origins may frame this tenant's page |
//!
//! # The intent is expanded here and an id on `/v1`
//!
//! Both reads answer a plain `checkout.session` object whose
//! `payment_intent` member is the **whole intent**, Stripe's `expand` shape
//! ([`ExpandableIntent`]). The page holds only a session id and a session
//! secret, and it needs the amount, the currency, the status,
//! `payment_method_types` (which rails to offer), `next_action` and
//! `last_payment_error` before it can paint anything — a second round trip
//! for that would mean two loading states instead of one. The session read
//! *also* answers the rail spec (`crate::model::CheckoutSessionForPayer::rails`,
//! built by [`build_rails`]) — what a payer sheet needs to draw each of
//! those rails, so a native client can render one it has never heard of
//! with no further round trip and no `if code == "mtn_momo"` of its own.
//!
//! The merchant surface keeps the id: a merchant already holds the intent
//! they created, and expanding on `GET /v1/checkout/sessions` would repeat
//! every amount once per row. Both shapes are documented in
//! `docs/reference/vpay-api.md`.
//!
//! That ladder is D6, and it is the whole design of this module. The session
//! secret rides in a **URL fragment**, which never leaves the browser, so it
//! is safe to hand it authority over the intent's own credential. The
//! `return_token` rides in a **query string** — it has to, because a fragment
//! does not survive a rail's redirect — so it is given strictly less: enough
//! to render an outcome and forward the payer, and not enough to confirm
//! anything. The origins route needs no secret at all, because an origin is
//! the merchant's own public website.
//!
//! # Every failure is the same 404
//!
//! Both session routes answer [`ApiError::NotFound`] with `resource:
//! "checkout session"`, byte-identically, for every one of their six ways to
//! refuse — see [`crate::browser`]'s module docs for why that is the entire
//! confidentiality property of an unauthenticated surface, and
//! [`authenticate`] for what the six are. The sixth is the **clock**: a
//! session past `expires_at` is refused by the read itself, not by the hourly
//! sweep that makes `status` honest to a merchant.
//!
//! The origins route answers `200 {"origins": []}` for an unknown key rather
//! than a 404, and that is the same property arrived at from the other side:
//! an empty list is what a *registered* tenant with no origins gets, so the
//! two are indistinguishable and nobody can enumerate a deployment's
//! merchants by trying keys. It is also the fail-closed answer — no origins
//! means no embedding.
//!
//! Stated plainly, because it is a real limit of that property: a key whose
//! tenant **has** origins is distinguishable from an unknown one — the answer
//! is a non-empty list. So this route confirms that a given publishable key is
//! registered *and* has embedded checkout configured. That is accepted rather
//! than fixed. A publishable key is 16–64 characters of `[A-Za-z0-9]` after
//! its prefix, which is not enumerable, and the fact learned about a key
//! someone already holds is the list of the merchant's own public websites —
//! which they published by putting vpay's iframe on them. What the uniform
//! empty list protects is the *unknown* key: nobody can tell "no such
//! deployment tenant" from "configured, embeds nowhere".

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use vpay_core::ids;
use vpay_db::{
    CheckoutSessionRow, CheckoutSessions, PaymentIntentRow, PaymentIntents, Repositories,
};
use vpay_provider::ProviderAdapter;

use crate::error::ApiError;
use crate::form::VpayQuery;
use crate::model::{
    CheckoutSessionForPayer, CheckoutSessionObject, ExpandableIntent, PaymentIntentObject,
    PaymentIntentWithSecret, RailDisplayName, RailSpec,
};
use crate::v1::payment_intents::json_response;
use crate::v1::{RailConfig, ResourceConfig};

/// The object type this surface's 404 speaks about.
///
/// **`"checkout session"`, with a space** — deliberately different from
/// `/v1`'s `"checkout.session"`, for the reason [`crate::browser`]'s
/// `RESOURCE` gives: the merchant surface's vocabulary is the API's own
/// object table, which an SDK matches on, while this message is read by a
/// *payer* in a browser or by a merchant's front-end developer, and neither
/// has an object table in front of them.
const RESOURCE: &str = "checkout session";

/// How much of a caller-supplied publishable key reaches a log line — see
/// [`bounded`].
const KEY_LOG_CHARS: usize = 40;

/// The uniform refusal for this surface.
///
/// Built through one function so the five call sites cannot drift — the whole
/// property is that they are byte-identical. The id it echoes is the one the
/// **caller** spelled, exactly as [`crate::browser::not_found`]'s is.
fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

/// The credential the session read carries: a publishable key and the
/// session's own `client_secret`.
///
/// Both `Option<String>` although both are required, for
/// [`crate::browser::PayerCredential`]'s reason: a missing parameter and an
/// empty one arrive indistinguishably, and serde's refusal would be a `400`
/// naming `query` — while a missing credential is not a shape error a payer
/// can fix by reading a message. It is the uniform 404.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct SessionCredential {
    key: Option<String>,
    client_secret: Option<String>,
}

/// The credential the **return** read carries: a publishable key and the
/// session's `return_token`.
///
/// `t`, not `return_token`, and that is the wire contract rather than
/// brevity: this parameter is appended by *the rail* to the URL vpay handed
/// it at submit, so every character of it is one more that has to survive
/// Orange's own URL handling and fit inside whatever length its `return_url`
/// field accepts.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct ReturnCredential {
    key: Option<String>,
    t: Option<String>,
}

/// The origins route's only parameter.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) struct OriginsQuery {
    key: Option<String>,
}

/// The origins route's answer.
///
/// An object with one key rather than a bare array, so the route can grow a
/// second fact about the tenant's page — a display name, a locale — without
/// every caller having to branch on the JSON's *type*.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct Origins {
    origins: Vec<String>,
}

/// Turns a `(publishable key, secret, id)` triple into an authenticated
/// session and its intent, or into the uniform 404.
///
/// # The order, and why each step is where it is
///
/// The same five steps [`crate::browser::authenticate`] takes, with one more
/// hop at the end:
///
/// 1. **`key` → tenant.** An unknown key resolves to nothing and no database
///    read happens, so it costs a caller no query and tells them nothing
///    about whether the id exists.
/// 2. **`id` → row**, by [`CheckoutSessions::get_by_id_unscoped`] and *not*
///    by `get_for_merchant`. The one place this surface reads unscoped, for
///    the reason that function's own doc gives: the tenant to filter by is
///    not yet trusted.
/// 3. **the row's tenant must be the key's tenant** — the case that makes
///    this a tenancy check and not a formality.
/// 4. **the credential**, rebuilt from the row by `compare` and compared with
///    [`crate::browser::secrets_match`], which is
///    [`crate::browser::ct_compare`] — the *same* constant-time compare the
///    payment-intent surface uses, not a second one. Rebuilding rather than
///    parsing what arrived means there is exactly one string this can succeed
///    against.
/// 5. **the clock.** A session past `expires_at` is refused whatever its
///    `status`, because the credential that addressed it is written down by
///    design — the `return_token` rides in a query string and therefore lands
///    in a rail's logs — and `expires_at` is the bound on how long that copy
///    is worth anything. `status` is deliberately not consulted here: a
///    `complete` session's return page is the screen the whole redirect leg
///    exists to reach.
/// 6. **the intent**, by `PaymentIntents::get_by_id`, unscoped for a
///    different and weaker reason: the session row's own foreign key names
///    it, so there is no caller input left to authorise. Its tenant is
///    checked anyway — a session and an intent that disagree about their
///    owner would be a broken foreign key, and answering the 404 is cheaper
///    than reasoning about what it would mean.
///
/// `compare` is a closure rather than a `&str`, so the caller supplies *which
/// credential this route accepts* without this function having to know that
/// two exist. The session read passes the joined `client_secret`; the return
/// read passes the `return_token`. Neither can accidentally accept the
/// other's, because neither is ever built here.
///
/// # Errors
///
/// [`ApiError::NotFound`], identically, for every one of the six, and
/// whatever the repository raises for a database that is unreachable.
async fn authenticate(
    config: &ResourceConfig,
    repositories: &dyn Repositories,
    id: &str,
    key: &str,
    presented: &str,
    expected: impl Fn(&CheckoutSessionRow) -> String,
) -> Result<(CheckoutSessionRow, PaymentIntentRow), ApiError> {
    let Some(merchant_id) = config.merchant_id_for_publishable_key(key) else {
        // Logged, because an operator debugging "every payer gets a 404"
        // needs to see that the key never resolved — and the key is not a
        // secret, so naming it is what makes the line actionable.
        tracing::debug!(
            // Bounded like every other reflected value on this API
            // (`crate::error`'s `key_hint`): a publishable key is not a
            // secret, but this one came from an *unauthenticated* caller who
            // may send a megabyte, and a log line is a place that gets
            // written whatever its length.
            publishable_key = %bounded(key),
            "a /v1/browser/checkout request named a publishable key this deployment has no \
             registration for; answering the uniform 404"
        );
        return Err(not_found(id));
    };

    let Some(session) = CheckoutSessions::get_by_id_unscoped(repositories, id).await? else {
        return Err(not_found(id));
    };

    if session.merchant_id != merchant_id {
        tracing::warn!(
            // Bounded for the reason above, though this one is a *registered*
            // key and therefore already bounded by configuration — the two
            // sites go through the same function so a future reordering
            // cannot make one of them unbounded.
            publishable_key = %bounded(key),
            checkout_session_id = %id,
            "a /v1/browser/checkout request presented a publishable key whose tenant does not \
             own the checkout session it addressed; answering the uniform 404"
        );
        return Err(not_found(id));
    }

    if !crate::browser::secrets_match(&expected(&session), presented) {
        tracing::warn!(
            checkout_session_id = %id,
            "a /v1/browser/checkout request presented the wrong credential; answering the \
             uniform 404"
        );
        return Err(not_found(id));
    }

    // The horizon, checked on the **read** and not left to the sweep.
    //
    // Both credentials outlive the session they address unless something
    // says otherwise, and the `return_token` in particular is written down
    // by design: it travels in a query string, so it lands in the rail's
    // logs, in the checkout app's access logs and in whatever sits between
    // them. `expires_at` is the bound on how long that copy is worth
    // anything (D10's 24 hours).
    //
    // It cannot be the hourly expiry sweep's job
    // (`vpay_worker::handlers::sweep_expired`): the sweep leaves a session
    // with a live charge `open` on purpose, it runs at most once an hour, and
    // a deployment whose worker is down would keep answering these reads for
    // as long as the outage lasted. The read is where the question has to be
    // asked, and the sweep is only what makes `status` honest to a merchant.
    //
    // Deliberately **not** conditioned on `status`: a `complete` session's
    // return page is the page a payer sees after a successful payment, and
    // refusing it would break the one screen the whole redirect leg exists
    // to reach. What ends both reads is the clock.
    if OffsetDateTime::now_utc() >= session.expires_at {
        tracing::debug!(
            checkout_session_id = %id,
            "a /v1/browser/checkout request presented a valid credential for a session past \
             its expiry; answering the uniform 404"
        );
        return Err(not_found(id));
    }

    let Some(intent) = PaymentIntents::get_by_id(repositories, &session.payment_intent_id).await?
    else {
        // The foreign key makes this unreachable. It is the uniform 404
        // rather than an `Internal` because a payer can do nothing with
        // either, and a `500` here would be a distinguishable answer on a
        // surface whose whole property is that its answers are not.
        tracing::error!(
            checkout_session_id = %id,
            payment_intent_id = %session.payment_intent_id,
            "a checkout session names a payment intent that does not exist; the foreign key \
             should make this impossible"
        );
        return Err(not_found(id));
    };
    if intent.merchant_id != session.merchant_id {
        tracing::error!(
            checkout_session_id = %id,
            payment_intent_id = %intent.id,
            "a checkout session and the payment intent it drives disagree about their tenant"
        );
        return Err(not_found(id));
    }

    Ok((session, intent))
}

/// Renders a session for a **payer**, which is the same object a merchant
/// sees minus the two things a payer must not be handed.
///
/// `url` is always `None` here, and that is a security decision rather than a
/// simplification. The hosted `url` carries the session's own `client_secret`
/// in its fragment, and the return read is authorised by the *`return_token`*
/// — the weaker credential, the one that travels in a query string. Rendering
/// the `url` there would let anyone holding a return token recover the
/// session secret, and the session secret is what
/// [`retrieve`] exchanges for the **intent's** secret, which confirms the
/// payment. That is a three-step escalation out of a value written to access
/// logs.
///
/// It costs the page nothing: a payer reading the session read is already on
/// the URL, and a payer on the return page has no use for it.
fn rendered_session(row: &CheckoutSessionRow) -> CheckoutSessionObject {
    CheckoutSessionObject::from_row(row, None)
}

/// Adds the one thing about the *merchant* a payer is shown.
///
/// Both reads render it and both go through this function, so "which name did
/// this page get?" has one answer. The name comes from the **row's**
/// `merchant_id` and never from the key the caller presented: by the time this
/// runs [`authenticate`] has proved the two agree, and taking it from the
/// caller's side would be reading a tenant out of an attacker's input for the
/// sake of a string.
///
/// A session whose merchant configured no `display_name` renders no
/// `merchant` member at all — see [`ResourceConfig::merchant_display_name`]
/// for why nothing is the honest answer; the page tolerates the absence
/// (`frontends/apps/checkout/src/lib/api.ts`, Step 9 lane 3b) and paints a
/// neutral heading.
fn for_payer(
    config: &ResourceConfig,
    row: &CheckoutSessionRow,
    session: CheckoutSessionObject,
    rails: Vec<RailSpec>,
) -> CheckoutSessionForPayer {
    CheckoutSessionForPayer::new(
        session,
        config
            .merchant_display_name(&row.merchant_id)
            .map(str::to_owned),
        rails,
    )
}

/// Builds the server-driven rail spec (`crate::model::RailSpec`): one entry
/// per code in `payment_method_types`, in that order, describing a rail with
/// no `if code == "mtn_momo"` anywhere in the client that reads it — see
/// that type's own doc for the wire shape and what is deliberately reduced
/// out of it.
///
/// # Errors
///
/// [`ApiError::Internal`] if a code names no linked adapter. Not reachable
/// through anything a payer sent: `payment_method_types` only ever holds
/// codes that passed `config.enabled_rail(..).is_some()` when the intent was
/// created (`crate::v1::payment_intents::resolve_rail`), and boot itself
/// refuses to start with a configured rail and no linked adapter
/// (`vpay_config::ConfigError::ProviderWithoutAdapter`,
/// `crate::v1::boot::boot_seeds`) — so a miss here means that invariant
/// failed, on `crate::v1::account_holders::retrieve`'s terms.
fn build_rails(
    payment_method_types: &[String],
    config: &ResourceConfig,
    adapters: &BTreeMap<String, Box<dyn ProviderAdapter>>,
) -> Result<Vec<RailSpec>, ApiError> {
    payment_method_types
        .iter()
        .map(|code| {
            let adapter = adapters.get(code.as_str()).ok_or_else(|| {
                ApiError::Internal(format!(
                    "payment_intents.payment_method_types names `{code}`, which has no linked \
                     adapter; boot is supposed to have refused to start"
                ))
            })?;
            Ok(RailSpec {
                code: code.clone(),
                // The one `Capabilities` flag a payer's sheet acts on — see
                // `RailSpec`'s own doc for why the rest (refund flags,
                // `requires_ip_allowlist`) stay operator-facing.
                flow: adapter.capabilities().flow,
                label_key: format!("rail.{code}"),
                display_name: config
                    .rail(code)
                    .and_then(RailConfig::display_name)
                    .map(RailDisplayName::from),
                fields: adapter.payer_fields().to_vec(),
            })
        })
        .collect()
}

/// `GET /v1/browser/checkout/sessions/{id}?key=…&client_secret=…`.
///
/// What the checkout page calls first, with the session secret it read out of
/// its own URL fragment.
///
/// Answers the session **and the intent's own `client_secret`**, which is the
/// point of the route: the page then drives
/// `GET`/`POST /v1/browser/payment_intents/{id}[/confirm]` — the routes that
/// already exist, already CORS-enabled, already proven — with no new
/// confirm path and no second way to move money. Handing over that credential
/// is exactly as strong as handing over the session's, which the caller has
/// just proved it holds.
pub(super) async fn retrieve(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    State(adapters): State<Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>>,
    Path(id): Path<String>,
    VpayQuery(credential): VpayQuery<SessionCredential>,
) -> Result<Response, ApiError> {
    let (key, secret) = parts(
        credential.key.as_deref(),
        credential.client_secret.as_deref(),
        &id,
    )?;
    let (session, intent) = authenticate(&config, repositories.as_ref(), &id, key, secret, |row| {
        ids::client_secret(&row.id, &row.client_secret_suffix)
    })
    .await?;

    // The one place on this surface that renders an intent credential, and
    // the variant name is what says so. `ExpandedWithSecret` on the *return*
    // read below would be a visible, reviewable line — not a forgotten
    // field.
    //
    // And only while the session is `open`. The credential exists so this
    // page can drive `POST /v1/browser/payment_intents/{id}/confirm`; once
    // the session is `complete` or `expired` there is nothing left to
    // confirm, and handing it out again would keep a live intent credential
    // in circulation for a checkout that is over. The page loses nothing: it
    // read the secret on its first call, and everything it does afterwards
    // is polling with the copy it already holds.
    let intent_object = PaymentIntentObject::try_from(&intent)?;
    let rails = build_rails(&intent_object.payment_method_types, &config, &adapters)?;
    let expanded = if session.status == crate::v1::checkout_sessions::OPEN {
        ExpandableIntent::ExpandedWithSecret(Box::new(PaymentIntentWithSecret::new(
            intent_object,
            ids::client_secret(&intent.id, &intent.client_secret_suffix),
        )))
    } else {
        ExpandableIntent::Expanded(Box::new(intent_object))
    };
    json_response(
        StatusCode::OK,
        &for_payer(
            &config,
            &session,
            rendered_session(&session).with_expanded_intent(expanded),
            rails,
        ),
    )
}

/// `GET /v1/browser/checkout/sessions/{id}/return?key=…&t=…`.
///
/// Where a redirect rail sends the payer back (D2/D6). The `t` is the
/// session's `return_token`, which vpay put into the `return_url` it handed
/// the rail at submit — a query parameter, because a fragment does not
/// survive that round trip.
///
/// Answers the session and the intent **without** the intent's
/// `client_secret`, which is the whole reason this is a second route rather
/// than a second credential on the first. Everything the return page has to
/// do — render the outcome, substitute `{CHECKOUT_SESSION_ID}`, forward the
/// payer — needs the intent's `status` and nothing more.
///
/// It could not confirm anyway: by the time a payer is on this page the
/// charge exists, so `confirm` is a `409`. That is a *second* line of
/// defence and deliberately not the argument — a token that authorised
/// confirming would be one route change away from mattering, and the
/// [`SessionWithIntent<PaymentIntentObject>`] here is what makes it a type
/// error instead.
pub(super) async fn retrieve_for_return(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    Path(id): Path<String>,
    VpayQuery(credential): VpayQuery<ReturnCredential>,
) -> Result<Response, ApiError> {
    let (key, token) = parts(credential.key.as_deref(), credential.t.as_deref(), &id)?;
    let (session, intent) = authenticate(&config, repositories.as_ref(), &id, key, token, |row| {
        // The stored token verbatim: unlike a `client_secret` there is no id
        // half to join, because this value never appears beside an id in a
        // single string — it is its own query parameter.
        row.return_token.clone()
    })
    .await?;

    // `Expanded`, never `ExpandedWithSecret`: everything the return page has
    // to do needs the intent's `status` and nothing more. See this
    // function's own doc for why that is a type-level fact rather than a
    // cleared field.
    let expanded = ExpandableIntent::Expanded(Box::new(PaymentIntentObject::try_from(&intent)?));
    json_response(
        StatusCode::OK,
        &for_payer(
            &config,
            &session,
            rendered_session(&session).with_expanded_intent(expanded),
            // No adapter map on this route (this handler takes no
            // `State<BTreeMap<..>>`) and, more to the point, nothing to ask
            // it: by the time a payer reaches the return page a charge
            // already exists, `POST .../confirm` is a `409`, and there is no
            // instrument left to collect — see this function's own doc, "It
            // could not confirm anyway". An empty rail spec here is the
            // honest answer, not a shortcut.
            Vec::new(),
        ),
    )
}

/// `GET /v1/browser/checkout/origins?key=…`.
///
/// The only route on any vpay surface that takes a publishable key and
/// **nothing else**, which is worth stating rather than assuming: it is safe
/// because the answer is not about any object. It is the list of the
/// merchant's own public websites, which they published by putting vpay's
/// iframe on them.
///
/// Called **server-side** by the checkout app's `middleware.ts`, before any
/// script runs, because `Content-Security-Policy: frame-ancestors` has to be
/// on the HTML response itself. That is also why it carries no secret: a
/// credential in a server-side lookup would end up in the Next server's logs
/// (D4).
///
/// An unknown key answers `{"origins": []}` and a `200` — see this module's
/// header for why that is the same confidentiality property the 404 gives,
/// and why it is the fail-closed answer rather than the lenient one.
pub(super) async fn origins(
    State(config): State<Arc<ResourceConfig>>,
    VpayQuery(query): VpayQuery<OriginsQuery>,
) -> Result<Response, ApiError> {
    let origins = query
        .key
        .as_deref()
        .filter(|key| !key.is_empty())
        .and_then(|key| config.merchant_id_for_publishable_key(key))
        .map(|merchant_id| config.checkout_origins_for(merchant_id).to_vec())
        .unwrap_or_default();

    json_response(StatusCode::OK, &Origins { origins })
}

/// Both halves of a credential, or the uniform 404.
///
/// Deliberately not a `400` for a missing parameter, for
/// [`crate::browser::PayerCredential::parts`]'s reason: a probe that can tell
/// "you sent no key" from "your key is wrong" has learned the surface exists
/// and is enumerable.
///
/// Takes two `Option<&str>` rather than being a method on the two credential
/// structs, because there *are* two structs — the parameters are spelled
/// differently on the wire — and the rule is the same one. Two copies of it
/// would be two chances for one route to start answering `400`.
/// The first [`KEY_LOG_CHARS`] characters of a caller-supplied publishable
/// key, for a log line.
///
/// Not a redaction — a publishable key is public by design and the whole
/// value of logging it is that an operator can compare it with what the
/// merchant's page renders. It is a *bound*: the value arrives from an
/// unauthenticated request, and 40 characters is past the longest key
/// `vpay_config` will accept (`pk_live_` plus 64) being distinguishable while
/// being far short of what a caller could send. A truncated key is marked, so
/// nobody reads a prefix as a whole value.
fn bounded(key: &str) -> String {
    let mut out: String = key.chars().take(KEY_LOG_CHARS).collect();
    if key.chars().nth(KEY_LOG_CHARS).is_some() {
        out.push('…');
    }
    out
}

fn parts<'a>(
    key: Option<&'a str>,
    secret: Option<&'a str>,
    id: &str,
) -> Result<(&'a str, &'a str), ApiError> {
    match (key, secret) {
        (Some(key), Some(secret)) if !key.is_empty() && !secret.is_empty() => Ok((key, secret)),
        _ => Err(not_found(id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The credential gate refuses a missing or blank half with the same 404
    /// everything else on this surface answers — never a `400` naming a
    /// parameter.
    #[test]
    fn a_missing_or_blank_credential_is_the_uniform_404() {
        let id = "cs_00000000000000000000000x";
        let full = ApiError::NotFound {
            resource: RESOURCE,
            id: id.to_owned(),
        };

        for (key, secret) in [
            (None, None),
            (Some("pk_test_something0000000"), None),
            (None, Some("cs_x_secret_y")),
            (Some(""), Some("cs_x_secret_y")),
            (Some("pk_test_something0000000"), Some("")),
        ] {
            let error = parts(key, secret, id).expect_err("an incomplete credential is refused");
            assert_eq!(
                format!("{error}"),
                format!("{full}"),
                "every refusal on this surface must be the identical 404"
            );
        }

        let (key, secret) = parts(Some("pk_test_something0000000"), Some("cs_x_secret_y"), id)
            .expect("a complete credential passes the gate");
        assert_eq!(key, "pk_test_something0000000");
        assert_eq!(secret, "cs_x_secret_y");
    }

    /// The 404's noun is the payer-facing spelling, not the API's object
    /// name.
    ///
    /// Both are pinned as literals because they are two different wire
    /// contracts that must not converge: `checkout.session` is what
    /// `docs/api/README.md`'s object table and both merchant SDKs match on,
    /// and `checkout session` is what a payer reads.
    #[test]
    fn the_uniform_404_says_checkout_session_the_way_a_payer_reads_it() {
        assert_eq!(RESOURCE, "checkout session");
        assert_eq!(
            vpay_core::Classify::public_message(&not_found("cs_1")),
            "No such checkout session: cs_1"
        );
        assert_ne!(
            RESOURCE,
            crate::v1::checkout_sessions::RESOURCE,
            "the merchant surface's object name and the payer-facing noun are two different \
             contracts and must not converge"
        );
    }

    /// A test double naming only what [`build_rails`] reads off an adapter:
    /// `capabilities().flow` and `payer_fields()`. This is where a real
    /// deployment caught a bug a unit test alone would not have (see
    /// `vpay_provider::measured::tests::payer_fields_is_forwarded_…`): the
    /// `Measured` decorator every real adapter is wrapped in before it
    /// reaches this map did not forward `payer_fields()` and silently
    /// answered the trait's `&[]` default. This double is unwrapped
    /// (exactly the map shape `crate::v1::boot::boot_seeds` builds, whose
    /// values are `Box<dyn ProviderAdapter>` — `Measured` included), so it
    /// cannot by itself prove that forwarding is correct; it proves
    /// `build_rails` reads the two methods it should, and the `Measured`
    /// test proves the wrapper does not swallow the answer between here and
    /// a real adapter.
    #[derive(Debug)]
    struct StubAdapter {
        code: &'static str,
        flow: vpay_core::ProviderFlow,
        fields: &'static [vpay_provider::PayerField],
    }

    #[async_trait::async_trait]
    impl ProviderAdapter for StubAdapter {
        fn code(&self) -> &'static str {
            self.code
        }

        fn capabilities(&self) -> vpay_provider::Capabilities {
            vpay_provider::Capabilities {
                flow: self.flow,
                supports_refunds: false,
                supports_partial_refunds: false,
                delivers_callbacks: false,
                requires_ip_allowlist: false,
                supports_account_holder_lookup: false,
                refund_destination: vpay_provider::RefundDestination::Origin,
            }
        }

        fn payer_fields(&self) -> &'static [vpay_provider::PayerField] {
            self.fields
        }

        async fn submit(
            &self,
            _charge: &vpay_provider::ChargeRef,
            _config: &vpay_provider::ProviderConfig,
        ) -> Result<vpay_provider::Submitted, vpay_provider::ProviderError> {
            panic!("this rail-spec test double is never asked to drive a charge")
        }

        async fn query_status(
            &self,
            _charge: &vpay_provider::ChargeRef,
            _config: &vpay_provider::ProviderConfig,
        ) -> Result<vpay_provider::ChargeStatus, vpay_provider::ProviderError> {
            panic!("this rail-spec test double is never asked to drive a charge")
        }

        fn parse_callback(
            &self,
            _body: &[u8],
        ) -> Result<vpay_provider::CallbackRef, vpay_provider::ProviderError> {
            panic!("this rail-spec test double is never asked to drive a charge")
        }
    }

    const MTN_FIELDS: &[vpay_provider::PayerField] = &[vpay_provider::PayerField {
        name: "msisdn",
        kind: vpay_provider::PayerFieldKind::Phone {
            region: "CM",
            phone_type: vpay_provider::PhonePayerType::Mobile,
        },
        required: true,
        label_key: "msisdn.label",
    }];

    /// A minimal, self-contained `Config` naming `mtn_momo` and
    /// `orange_money` — deliberately not `crate::v1::tests::config()`
    /// (that fixture's module is private to `v1` and unreachable from
    /// `crate::browser`, and this test wants to vary exactly one field
    /// anyway: `mtn_momo`'s `display_name`).
    fn rail_spec_test_config(
        mtn_display_name: Option<vpay_config::RailDisplayName>,
    ) -> vpay_config::Config {
        vpay_config::Config {
            deployment: vpay_config::Deployment {
                name: "test".to_owned(),
                livemode: false,
                public_base_url: "https://api.vpay.test".to_owned(),
                surfaces: None,
            },
            providers: vec![
                vpay_config::ProviderHost {
                    code: "mtn_momo".to_owned(),
                    enabled: true,
                    host: vpay_config::HostEntry {
                        url: "https://mtn.example".to_owned(),
                        label: "mtn".to_owned(),
                    },
                    settings: std::collections::BTreeMap::new(),
                    callback_url: None,
                    currency: "XAF".to_owned(),
                    credentials: std::collections::BTreeMap::new(),
                    display_name: mtn_display_name,
                },
                vpay_config::ProviderHost {
                    code: "orange_money".to_owned(),
                    enabled: true,
                    host: vpay_config::HostEntry {
                        url: "https://orange.example".to_owned(),
                        label: "orange".to_owned(),
                    },
                    settings: std::collections::BTreeMap::new(),
                    callback_url: None,
                    currency: "XAF".to_owned(),
                    credentials: std::collections::BTreeMap::new(),
                    display_name: None,
                },
            ],
            currencies: vec![vpay_config::CurrencyEntry {
                code: "XAF".to_owned(),
                exponent: 0,
            }],
            merchant_clients: vec![crate::test_fixtures::merchant(
                "acme-cameroon",
                &["payments:write"],
            )],
            webhooks: vpay_config::WebhookPolicy::default(),
            checkout: vpay_config::CheckoutConfig::default(),
            dashboard_client: None,
            staff_auth: vpay_config::StaffAuth::default(),
        }
    }

    fn rail_test_adapters() -> BTreeMap<String, Box<dyn ProviderAdapter>> {
        let mut adapters: BTreeMap<String, Box<dyn ProviderAdapter>> = BTreeMap::new();
        adapters.insert(
            "mtn_momo".to_owned(),
            Box::new(StubAdapter {
                code: "mtn_momo",
                flow: vpay_core::ProviderFlow::Push,
                fields: MTN_FIELDS,
            }),
        );
        adapters.insert(
            "orange_money".to_owned(),
            Box::new(StubAdapter {
                code: "orange_money",
                flow: vpay_core::ProviderFlow::Redirect,
                fields: &[],
            }),
        );
        adapters
    }

    /// The rail spec, end to end: a push rail's declared field survives
    /// into the wire type, a redirect rail's is the empty array Stripe-style
    /// "every key, every time" would render as `[]` rather than an absent
    /// key, and a rail with no configured `display_name` omits the key
    /// entirely (`crate::model::RailSpec::display_name`'s own doc) rather
    /// than rendering `null` — a client falls back to `label_key`.
    #[test]
    fn build_rails_renders_flow_fields_and_the_configured_display_name() {
        let config = rail_spec_test_config(Some(vpay_config::RailDisplayName {
            en: Some("MTN Mobile Money".to_owned()),
            fr: None,
        }));
        let resource_config =
            ResourceConfig::from_config(&config).expect("the fixture config is valid");

        let rails = build_rails(
            &["mtn_momo".to_owned(), "orange_money".to_owned()],
            &resource_config,
            &rail_test_adapters(),
        )
        .expect("both codes have a linked test adapter");

        // A slice pattern rather than `rails[0]`/`rails[1]`:
        // `clippy::indexing_slicing` is denied workspace-wide and, unlike
        // unwrap/expect/panic, has no test exemption (clippy.toml,
        // docs/adr/0007-lint-policy.md). It also states the arity once.
        let [mtn, orange] = rails.as_slice() else {
            panic!("build_rails must answer exactly the two codes it was given");
        };

        assert_eq!(mtn.code, "mtn_momo");
        assert_eq!(mtn.flow, vpay_core::ProviderFlow::Push);
        assert_eq!(mtn.label_key, "rail.mtn_momo");
        assert_eq!(mtn.fields.len(), 1);
        let msisdn = mtn
            .fields
            .first()
            .expect("asserted exactly one field just above");
        assert_eq!(msisdn.name, "msisdn");
        let display_name = mtn
            .display_name
            .as_ref()
            .expect("mtn_momo was configured with a display_name in this test");
        assert_eq!(display_name.en.as_deref(), Some("MTN Mobile Money"));
        assert_eq!(display_name.fr, None);

        assert_eq!(orange.code, "orange_money");
        assert_eq!(orange.flow, vpay_core::ProviderFlow::Redirect);
        assert_eq!(orange.label_key, "rail.orange_money");
        assert!(
            orange.fields.is_empty(),
            "a redirect rail collects no instrument"
        );
        assert!(
            orange.display_name.is_none(),
            "orange_money was not configured with a display_name in this test"
        );

        assert_eq!(
            serde_json::to_value(orange)
                .expect("a wire DTO always serialises")
                .get("display_name"),
            None,
            "an unconfigured display_name must be an absent key, never `null`"
        );
    }

    /// A code that names no linked adapter is our own invariant failing
    /// (boot is supposed to have refused to start), never a caller's
    /// mistake — see [`build_rails`]'s own doc.
    #[test]
    fn a_code_with_no_linked_adapter_is_an_internal_error() {
        let config = rail_spec_test_config(None);
        let resource_config =
            ResourceConfig::from_config(&config).expect("the fixture config is valid");

        let error = build_rails(
            &["a_rail_nothing_links".to_owned()],
            &resource_config,
            &rail_test_adapters(),
        )
        .expect_err("no adapter is linked for this code");
        assert!(matches!(error, ApiError::Internal(_)));
    }
}
