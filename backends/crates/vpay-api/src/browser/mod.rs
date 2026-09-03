//! `/v1/browser` — the two routes a **payer's browser** may call, with no
//! merchant credential anywhere in the picture.
//!
//! STATUS: both routes are implemented and reach the same code `/v1` does.
//! There is deliberately no `create`, no `list` and no `cancel` here, and
//! there is no redirect-return endpoint: Step 5c ships push-only, and the
//! Orange return trip is a named gap owned by the step that builds provider
//! callbacks (D4, `docs/status.md`).
//!
//! This surface exists because `@stripe/stripe-js` cannot be pointed at
//! vpay — `StripeConstructorOptions` has no host, and the loader hardcodes
//! `js.stripe.com`. `sdks/stripe-js` (`@vpay/stripe-js`) is the drop-in
//! replacement, and *this* module is what it speaks to. The wire shape is
//! fixed by that package: query `key` + `client_secret` on the `GET`, form
//! `key`, `client_secret`, `payment_method_data[…]`, `return_url` on the
//! `POST`, and the Stripe envelope `{error:{type,code,message,param}}` on
//! every failure.
//!
//! # The credential model, and what it is not
//!
//! Two values, and both are needed:
//!
//! * a **publishable key** (`pk_test_…`/`pk_live_…`), which names a tenant
//!   and authorises nothing. It is rendered into a merchant's public
//!   checkout page by construction, so it is not a secret and is not treated
//!   as one — [`vpay_config::MerchantClient::publishable_keys`] prints it in
//!   `Debug` and `config/application.yml` carries it as a literal;
//! * the intent's own **`client_secret`** (`pi_…_secret_…`), 160 bits from
//!   the OS CSPRNG, minted at `create` and stored as
//!   `payment_intents.client_secret_suffix` (migration `0026`). This is the
//!   credential. It authorises exactly one payment intent, for its whole
//!   life, and there is no rotation endpoint: a retry is a new intent.
//!
//! Neither is a bearer token and neither can be exchanged for one. A payer
//! holding both can read one intent and confirm it once — nothing else on
//! this deployment, and nothing at all about any other intent.
//!
//! # Every failure is the same 404
//!
//! [`authenticate`] has four ways to refuse and answers all of them with a
//! byte-identical [`ApiError::NotFound`]. That is not politeness; it is the
//! whole confidentiality property of an unauthenticated surface. A distinct
//! answer for "unknown publishable key" would let anyone enumerate which
//! merchants a deployment serves; one for "wrong merchant" would turn a
//! stolen key into an oracle for which intents belong to whom; one for
//! "wrong secret" would separate "this intent exists" from "your secret is
//! wrong", which is the first half of a guessing attack.
//!
//! # Rate limiting: none, in this process, deliberately
//!
//! D5. What stands between a guesser and an intent is 160 bits, the uniform
//! 404, and one-charge-per-intent — not a counter. A per-process limiter
//! across N replicas is a limit of N times what it claims, and building one
//! that is not would be a shared-state design nobody has asked for. The
//! ingress requirement is stated in `docs/flows/browser-checkout.md` rather
//! than implied by a token bucket that would not hold.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
// `FromRequest` is named rather than let axum call it: `confirm` reads the
// whole request itself, exactly as `v1::payment_intents::confirm` does.
use axum::extract::{FromRequest as _, Path, Request, State};
use axum::response::Response;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Map, Value};
use vpay_core::ids;
use vpay_db::{PaymentIntentRow, PaymentIntents, Repositories};
use vpay_provider::ProviderAdapter;

use crate::error::ApiError;
use crate::form::{VpayForm, VpayQuery};
use crate::v1::payment_intents::{ConfirmParams, SecretRendering, confirm_once, rendered_intent};
use crate::v1::{MerchantScope, ResourceConfig, V1Route};

/// The object type this surface speaks about, in the words the 404 renders.
///
/// **`"payment intent"`, with a space — deliberately different from `/v1`'s
/// `"payment_intent"`.** The merchant surface's vocabulary is the API's own
/// (`docs/api/README.md`'s object table, which an SDK matches on); this
/// message is read by a *payer* in a browser, or by a merchant's front-end
/// developer, and neither has an object table in front of them.
/// `sdks/stripe-js/src/testing/browser-stub.ts` pins the resulting sentence
/// verbatim, so the two spellings cannot quietly converge.
const RESOURCE: &str = "payment intent";

/// The two routes mounted under `/v1/browser`, and the only place they are
/// listed.
///
/// A separate constant from [`crate::V1_ROUTES`], and that separation is
/// load-bearing rather than tidy: the integration test
/// `every_registered_v1_path_answers_401_without_a_token` walks `V1_ROUTES`
/// and asserts every entry is behind the merchant token boundary. These two
/// are outside it by design, so adding them there would either break that
/// test or, worse, make someone weaken it. The sibling assertion — that
/// exactly these two exist and that *neither* answers 401 — lives in
/// `backends/tests/integration/tests/browser_checkout.rs`.
///
/// [`V1Route`] is reused rather than a second route type: the shape (a path,
/// its methods, a `MethodRouter` builder) is identical, and one type is what
/// lets a test walk both tables with the same code.
pub const BROWSER_ROUTES: &[V1Route] = &[
    V1Route {
        path: "/payment_intents/{id}",
        methods: &["GET"],
        mount: || get(retrieve),
    },
    V1Route {
        path: "/payment_intents/{id}/confirm",
        methods: &["POST"],
        mount: || post(confirm),
    },
];

/// The `/v1/browser` router, built by folding [`BROWSER_ROUTES`].
///
/// Carries its own `.fallback` for the reason `crate::router`'s docs record
/// about the OP nest: without one, axum flattens this nest's routes into the
/// outer table and registers no catch-all, so `/v1/browser/anything_else`
/// would match `/v1/{*rest}` — the **authenticated** nest — and answer a
/// `401` telling a payer's browser to present a bearer token it can never
/// have. `the_browser_nest_answers_its_own_404` is what catches that.
pub(crate) fn routes() -> Router<crate::AppState> {
    BROWSER_ROUTES
        .iter()
        .fold(Router::new(), |router, route| {
            router.route(route.path, (route.mount)())
        })
        .fallback(crate::not_found)
}

/// The payer's authority to act on **one** payment intent, established by
/// [`authenticate`] and by nothing else.
///
/// Both fields are private and there is no public constructor, which is the
/// same device [`MerchantScope`] uses and for the same reason: a handler
/// cannot invent one, so "every browser query was scoped by a verified
/// key/secret pair" is a property of the type system rather than of review.
/// It is deliberately **not** a `FromRequestParts` extractor — unlike
/// `MerchantScope`, there is no middleware in front of these routes putting
/// one on the request, and an extractor that could fail closed at *some*
/// later point is exactly the shape that lets a handler forget to
/// authenticate at all.
///
/// `Debug` is derived and safe: an intent id and a tenant are both already
/// public, and the secret is not held here — [`authenticate`] compares it and
/// drops it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayerScope {
    merchant_id: String,
    intent_id: String,
}

impl PayerScope {
    /// The tenant the presented publishable key named, **and** the tenant the
    /// addressed intent belongs to — [`authenticate`] refuses unless they are
    /// the same string.
    #[must_use]
    pub fn merchant_id(&self) -> &str {
        &self.merchant_id
    }

    /// The one intent this scope authorises, as stored. Not the id the caller
    /// spelled: it is read back off the row, so a handler cannot act on a
    /// value that was never looked up.
    #[must_use]
    pub fn intent_id(&self) -> &str {
        &self.intent_id
    }

    /// The tenant filter every `/v1` repository call takes.
    ///
    /// # Why minting a `MerchantScope` here is not a hole
    ///
    /// [`MerchantScope`] means "queries may be filtered by this tenant", and
    /// that is exactly what has been established: the publishable key named
    /// the tenant, and the intent's own 160-bit secret proved the caller
    /// holds a credential for a row *inside* it. What a `MerchantScope` does
    /// **not** carry is a scope claim — and the browser surface needs none,
    /// because it mounts two routes and neither is `create`, `list` or
    /// `cancel`. A payer cannot reach any other merchant object with this,
    /// since the only two handlers that exist both address
    /// [`Self::intent_id`].
    fn as_merchant_scope(&self) -> MerchantScope {
        MerchantScope::for_payer(self.merchant_id.clone())
    }
}

/// The uniform refusal — see the module docs.
///
/// Built through one function so the four call sites cannot drift: the whole
/// property is that they are byte-identical, and four `ApiError::NotFound`
/// literals would be four chances for one of them to gain a different
/// `resource` or echo something back.
///
/// The id it echoes is the one the **caller** spelled, exactly as `/v1`'s
/// 404 does (`ApiError::NotFound::id`, bounded on the render path). The
/// `client_secret` never appears: `not_found`'s message renders the id, and
/// the id alone (§4).
fn not_found(id: &str) -> ApiError {
    ApiError::NotFound {
        resource: RESOURCE,
        id: id.to_owned(),
    }
}

/// Compares two byte strings without letting the *position* of the first
/// difference change how long it takes.
///
/// Returns the comparison **and how many bytes it looked at**, because the
/// second is the only part a test can observe: correctness assertions pass
/// just as well against `a == b`, so a test that only checked the boolean
/// would not notice the compare being replaced by a short-circuiting one.
/// `a_constant_time_compare_examines_every_byte_even_when_the_first_differs`
/// pins that property — but only for calls made directly against
/// [`ct_compare`]. It exercises this function, not [`secrets_match`], so it
/// cannot by itself catch [`secrets_match`]'s body being swapped for `==`:
/// every correctness case a boolean-only test can express (a same-length
/// single-byte difference, a NUL-padded suffix, equality) comes out
/// identical either way. What catches *that* mutation is that it leaves
/// [`ct_compare`] with no caller in a non-test build, which
/// `cargo xtask verify-no-mocks`'s sibling gate, `cargo clippy --all-targets
/// -- -D warnings`, fails on as dead code — see
/// `secrets_match_rejects_every_shape_a_boolean_test_can_express` below for
/// the wiring-level test and the honesty of what it does and does not prove.
/// The counter is two instructions per byte over a 40-odd byte string and is
/// itself branch-free.
///
/// # Why hand-rolled rather than `subtle`
///
/// `subtle` is in `Cargo.lock` (transitively), and its `ConstantTimeEq for
/// [u8]` short-circuits on a length mismatch and is otherwise a better
/// implementation than this one. It is not used here because a `Choice` it
/// returns is indistinguishable, from a test, from `==` — and this step owes
/// a *decisive* test that the compare was not skipped. That trade is worth
/// recording rather than silently taking: what is given up is a hardened,
/// audited primitive; what is bought is that removing the hardening fails
/// the build.
///
/// # What it does not hide
///
/// The **length** leaks, through the number of iterations. That is
/// acceptable and would be under `subtle` too: the expected secret's length
/// is a public constant (an id plus `_secret_` plus 32 characters), so an
/// attacker learns only whether their own guess was the right length, which
/// they already knew.
fn ct_compare(a: &[u8], b: &[u8]) -> (bool, usize) {
    // Seeded from the length inequality so a prefix can never compare equal:
    // the padding below reads `0` past the end of the shorter side, and
    // without this seed `"abc"` and `"abc\0"` would differ nowhere.
    let mut difference = u8::from(a.len() != b.len());
    let mut examined = 0_usize;
    let width = a.len().max(b.len());
    for index in 0..width {
        let left = a.get(index).copied().unwrap_or(0);
        let right = b.get(index).copied().unwrap_or(0);
        // `|=`, never a branch: an `if left != right { return }` here is
        // precisely the timing signal this function exists to remove.
        difference |= left ^ right;
        examined = examined.saturating_add(1);
        // Opaque to the optimiser, so LLVM cannot recognise the loop as a
        // `memcmp` and reintroduce the early exit it is allowed to (the
        // result would be identical, which is exactly why it would be a
        // legal transform).
        difference = std::hint::black_box(difference);
    }
    (difference == 0, examined)
}

/// Whether the presented secret is the expected one, in constant time.
///
/// "In constant time" is a property of [`ct_compare`], proved once, directly
/// against it — see that function's doc for why no test at *this* level can
/// reprove it. What a test here can and does pin is the wiring: that this
/// function still delegates to `ct_compare` at all, rather than, say, `==`
/// (functionally indistinguishable for every case in
/// `secrets_match_rejects_every_shape_a_boolean_test_can_express`).
fn secrets_match(expected: &str, presented: &str) -> bool {
    ct_compare(expected.as_bytes(), presented.as_bytes()).0
}

/// Turns a `(publishable key, client_secret, id)` triple into the authority
/// to act on one intent, or into the uniform 404.
///
/// # The order, and why each step is where it is
///
/// 1. **`key` → tenant.** A key this deployment does not know resolves to
///    nothing, and nothing further happens — in particular no database read,
///    so an unknown key costs a caller no query and tells them nothing about
///    whether the id exists.
/// 2. **`id` → row**, by [`PaymentIntents::get_by_id`] and *not* by
///    `get_for_merchant`. This is the one place in the HTTP layer that
///    deliberately reads unscoped, because the tenant to filter by is not
///    yet trusted: it came from an unauthenticated caller. The comparison in
///    step 3 is what establishes it, and doing it in this order is what lets
///    step 3 be an equality rather than a "the query found nothing, probably
///    because of tenancy".
/// 3. **the row's tenant must be the key's tenant.** A valid key for
///    merchant A presented against merchant B's intent is refused here —
///    which is the case that makes this a *tenancy* check and not a
///    formality, since B's `client_secret` is what step 4 would otherwise
///    accept on A's behalf.
/// 4. **the secret**, rebuilt from `(row.id, row.client_secret_suffix)` by
///    `ids::client_secret` and compared with `secrets_match` (private, just
///    above this function in the source — a public item cannot link to it).
///    Rebuilding
///    rather than parsing what arrived means there is exactly one string
///    this can succeed against.
///
/// # Errors
///
/// [`ApiError::NotFound`], identically, for every one of the four — see the
/// module docs — and whatever [`PaymentIntents::get_by_id`] raises for a
/// database that is unreachable, which is a `503` about vpay and not about
/// the caller.
pub async fn authenticate(
    config: &ResourceConfig,
    repositories: &dyn Repositories,
    id: &str,
    key: &str,
    secret: &str,
) -> Result<(PayerScope, PaymentIntentRow), ApiError> {
    let Some(merchant_id) = config.merchant_id_for_publishable_key(key) else {
        // Logged, because an operator debugging "every payer gets a 404"
        // needs to see that the key never resolved — and the key is not a
        // secret, so naming it is what makes the line actionable.
        tracing::debug!(
            publishable_key = %key,
            "a /v1/browser request named a publishable key this deployment has no registration \
             for; answering the uniform 404"
        );
        return Err(not_found(id));
    };

    let Some(row) = PaymentIntents::get_by_id(repositories, id).await? else {
        return Err(not_found(id));
    };

    if row.merchant_id != merchant_id {
        tracing::warn!(
            publishable_key = %key,
            payment_intent_id = %id,
            "a /v1/browser request presented a publishable key whose tenant does not own the \
             payment intent it addressed; answering the uniform 404"
        );
        return Err(not_found(id));
    }

    // The expected value, derived — never a column read back and compared as
    // a whole secret, and never the caller's string re-split. `row.id` rather
    // than `id`: they are equal (the row was found by it), and using the
    // stored one means the compared string is built entirely from data this
    // process owns.
    let expected = ids::client_secret(&row.id, &row.client_secret_suffix);
    if !secrets_match(&expected, secret) {
        tracing::warn!(
            payment_intent_id = %id,
            "a /v1/browser request presented the wrong client_secret; answering the uniform 404"
        );
        return Err(not_found(id));
    }

    Ok((
        PayerScope {
            merchant_id: row.merchant_id.clone(),
            intent_id: row.id.clone(),
        },
        row,
    ))
}

/// The credential both routes carry, however it was encoded.
///
/// One struct with `#[serde(flatten)]` at each call site rather than two, so
/// the `GET`'s query parameters and the `POST`'s form fields cannot be
/// spelled differently — `@vpay/stripe-js` sends the same two names in both
/// places (`sdks/stripe-js/src/client.ts`).
///
/// Both fields are `Option<String>` although both are required: the wire is
/// form-encoded, so a missing field and an empty one arrive
/// indistinguishably to a required `String`'s deserializer, and serde's
/// refusal would be a `400` naming `query` or `body`. A missing credential
/// is not a shape error a payer can fix by reading a message — it is the
/// uniform 404, decided below.
#[derive(Debug, Deserialize)]
struct PayerCredential {
    key: Option<String>,
    client_secret: Option<String>,
}

impl PayerCredential {
    /// Both halves, or the uniform 404.
    ///
    /// Deliberately *not* a `400` for a missing parameter. A browser calling
    /// this surface without a credential is either a mistyped integration —
    /// which a merchant's developer debugs against
    /// `docs/flows/browser-checkout.md`, not against a parameter name — or a
    /// probe, and a probe that can tell "you sent no key" from "your key is
    /// wrong" has learned the surface exists and is enumerable.
    fn parts<'a>(&'a self, id: &str) -> Result<(&'a str, &'a str), ApiError> {
        match (self.key.as_deref(), self.client_secret.as_deref()) {
            (Some(key), Some(secret)) if !key.is_empty() && !secret.is_empty() => Ok((key, secret)),
            _ => Err(not_found(id)),
        }
    }
}

/// `GET /v1/browser/payment_intents/{id}?key=…&client_secret=…`.
///
/// The polling endpoint: `@vpay/stripe-js`'s `waitForPaymentIntent` calls it
/// every couple of seconds until the intent stops moving, which is how a
/// merchant's page learns a push confirm succeeded without a webhook.
///
/// Answers exactly what `GET /v1/payment_intents/{id}` answers, through the
/// same function — including `next_action` on a redirect rail, and including
/// the `client_secret` the caller just presented. Echoing the credential back
/// is not a leak: it was in the request, and returning it is what makes the
/// response the same `PaymentIntentWithSecret` shape a merchant's page
/// already holds (`sdks/stripe-js/src/types.ts`).
async fn retrieve(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    Path(id): Path<String>,
    VpayQuery(credential): VpayQuery<PayerCredential>,
) -> Result<Response, ApiError> {
    let (key, secret) = credential.parts(&id)?;
    let (_scope, row) = authenticate(&config, repositories.as_ref(), &id, key, secret).await?;
    rendered_intent(repositories.as_ref(), &row, SecretRendering::Include).await
}

/// `POST /v1/browser/payment_intents/{id}/confirm`'s form, beyond the
/// credential.
///
/// The two fields a payer may influence, and the reason
/// [`ConfirmParams::from_payer`] exists: this struct has no
/// `#[serde(flatten)] unsupported` and therefore no way for a payer's body
/// to reach the Stripe fields `/v1` refuses. `payment_method_data` is an
/// untyped map for the same ADR-0002 reason `/v1`'s is — the instrument is
/// nested under the *rail's own code*, so a typed struct here would name
/// `mtn_momo` as a field.
#[derive(Debug, Deserialize)]
struct BrowserConfirmParams {
    payment_method_data: Option<Map<String, Value>>,
    return_url: Option<String>,
    #[serde(flatten)]
    credential: PayerCredential,
}

/// `POST /v1/browser/payment_intents/{id}/confirm`.
///
/// # No `Idempotency-Key`, and what stands in for it
///
/// `/v1`'s POSTs require one (D7), and this one cannot: a browser request
/// carrying a custom header is CORS-preflighted, and Stripe.js — which this
/// surface is shaped after — sends none (§0 S4). So this handler calls
/// [`confirm_once`] directly rather than going through `PostRequest`, which
/// is where the key is read.
///
/// The protection that remains is the one that was always doing the work:
/// [`confirm_once`] refuses before any insert if the intent already has a
/// charge, and `one_charge_per_intent` is a unique index that refuses even if
/// two requests get past that check together. A payer who double-taps gets a
/// `200` and a `409`, never two charges — asserted end to end by
/// `a_second_browser_confirm_is_the_409_and_not_a_second_charge`.
///
/// What is genuinely lost is *replay*: a merchant's retry under a key
/// replays the stored response, and a payer's retry re-executes and is
/// refused. That is the honest trade and it is stated in
/// `docs/flows/browser-checkout.md` rather than papered over — a 409 telling
/// the payer to poll is a worse experience than a replayed 200, and it is
/// not a second charge.
async fn confirm(
    State(repositories): State<Arc<dyn Repositories>>,
    State(config): State<Arc<ResourceConfig>>,
    State(adapters): State<Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>>,
    Path(id): Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let VpayForm(params) = VpayForm::<BrowserConfirmParams>::from_request(request, &()).await?;
    let (key, secret) = params.credential.parts(&id)?;
    let (scope, _row) = authenticate(&config, repositories.as_ref(), &id, key, secret).await?;

    confirm_once(
        repositories.as_ref(),
        &config,
        &adapters,
        &scope.as_merchant_scope(),
        // `scope.intent_id()`, not the path's `id`: they are equal, and using
        // the authenticated one means the row that was authorised is the row
        // that is confirmed even if a future refactor changes how the path is
        // extracted.
        scope.intent_id(),
        ConfirmParams::from_payer(params.payment_method_data, params.return_url),
        SecretRendering::Include,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decisive test for the constant-time compare: a difference in the
    /// **first** byte must still cost every byte.
    ///
    /// Replace [`ct_compare`]'s body with anything that short-circuits — the
    /// natural `if left != right { return }`, or `a == b` with a length
    /// stand-in for the counter — and this fails. The correctness assertions
    /// below would not: `==` satisfies every one of them, which is why the
    /// examined count exists at all.
    #[test]
    fn a_constant_time_compare_examines_every_byte_even_when_the_first_differs() {
        let expected = "pi_00000000000000000000000x_secret_".to_owned() + &"a".repeat(32);
        let width = expected.len();

        // Differs in the first byte, the middle, and the last.
        for position in [0, width / 2, width - 1] {
            let mut guess = expected.clone().into_bytes();
            let byte = guess.get_mut(position).expect("inside the string");
            *byte ^= 0xff;

            let (equal, examined) = ct_compare(expected.as_bytes(), &guess);
            assert!(
                !equal,
                "a mutated byte at {position} must not compare equal"
            );
            assert_eq!(
                examined, width,
                "a difference at byte {position} must not stop the comparison early"
            );
        }
    }

    #[test]
    fn a_constant_time_compare_is_still_a_correct_comparison() {
        let secret = "pi_abc_secret_".to_owned() + &"z".repeat(32);

        assert!(secrets_match(&secret, &secret));
        assert!(!secrets_match(&secret, ""));
        assert!(!secrets_match("", &secret));
        // A prefix, which the length seed is what refuses: without it the
        // zero-padding would make these compare equal.
        assert!(!secrets_match(&secret, &secret[..secret.len() - 1]));
        assert!(!secrets_match(&secret[..secret.len() - 1], &secret));
        // A NUL-extended copy, the specific input the padding would accept.
        assert!(!secrets_match(&secret, &format!("{secret}\0")));
        // Two empty strings are equal, which is why `parts` refuses an empty
        // credential *before* this function is ever reached.
        assert!(secrets_match("", ""));
    }

    /// A wiring-level test, called out by name in both [`ct_compare`]'s and
    /// [`secrets_match`]'s doc comments — read those first.
    ///
    /// Every case here is deliberately one a plain `expected == presented`
    /// would also get right: an equal-length difference at the very first
    /// byte, one at the very last byte, and a NUL-padded suffix (the
    /// specific shape the length-inequality seed in [`ct_compare`] exists to
    /// refuse). That is not an oversight — it is the point being recorded.
    /// A boolean result can never distinguish a constant-time compare from a
    /// correct short-circuiting one; only the byte count `ct_compare`
    /// returns can, and this function's signature does not expose that
    /// count. So this test proves `secrets_match` is still *correct* through
    /// whatever it delegates to, not that the delegate is *`ct_compare`*.
    ///
    /// Revert-proof, but not with this test: swap `secrets_match`'s body for
    /// `expected == presented` and every assertion below still passes. What
    /// fails is `cargo clippy -p vpay-api --all-targets -- -D warnings`,
    /// because `ct_compare` — called only from `secrets_match` in
    /// non-test code, and from
    /// `a_constant_time_compare_examines_every_byte_even_when_the_first_differs`
    /// only under `#[cfg(test)]` — becomes dead code in the plain `lib`
    /// target that `--all-targets` also builds.
    #[test]
    fn secrets_match_rejects_every_shape_a_boolean_test_can_express() {
        let secret = "pi_abc_secret_".to_owned() + &"z".repeat(32);
        let width = secret.len();

        let mut differs_at_last_byte = secret.clone();
        differs_at_last_byte.replace_range(width - 1..width, "Q");
        assert_eq!(
            differs_at_last_byte.len(),
            width,
            "the mutation must not change the length, or this would silently \
             become the already-covered length-mismatch case"
        );
        assert!(!secrets_match(&secret, &differs_at_last_byte));

        let mut differs_at_first_byte = secret.clone();
        differs_at_first_byte.replace_range(0..1, "Q");
        assert_eq!(differs_at_first_byte.len(), width);
        assert!(!secrets_match(&secret, &differs_at_first_byte));

        // The NUL-padded suffix: the shape `ct_compare`'s length-inequality
        // seed exists specifically to refuse, exercised again here at the
        // `secrets_match` level rather than only via `ct_compare` directly.
        assert!(!secrets_match(&secret, &format!("{secret}\0")));
    }

    /// The credential gate refuses a missing or blank half with the same 404
    /// everything else answers — never a `400` naming a parameter.
    #[test]
    fn a_missing_or_blank_credential_is_the_uniform_404() {
        let id = "pi_00000000000000000000000x";
        let full = ApiError::NotFound {
            resource: RESOURCE,
            id: id.to_owned(),
        };

        for credential in [
            PayerCredential {
                key: None,
                client_secret: None,
            },
            PayerCredential {
                key: Some("pk_test_something0000000".to_owned()),
                client_secret: None,
            },
            PayerCredential {
                key: None,
                client_secret: Some("pi_x_secret_y".to_owned()),
            },
            PayerCredential {
                key: Some(String::new()),
                client_secret: Some("pi_x_secret_y".to_owned()),
            },
            PayerCredential {
                key: Some("pk_test_something0000000".to_owned()),
                client_secret: Some(String::new()),
            },
        ] {
            let error = credential
                .parts(id)
                .expect_err("an incomplete credential must be refused");
            assert_eq!(
                format!("{error}"),
                format!("{full}"),
                "every refusal on this surface must be the identical 404"
            );
        }

        let complete = PayerCredential {
            key: Some("pk_test_something0000000".to_owned()),
            client_secret: Some("pi_x_secret_y".to_owned()),
        };
        let (key, secret) = complete
            .parts(id)
            .expect("a complete credential passes the gate");
        assert_eq!(key, "pk_test_something0000000");
        assert_eq!(secret, "pi_x_secret_y");
    }

    /// The table is exactly two routes, and neither is a write this surface
    /// must not offer.
    ///
    /// The end-to-end half — that `create`, `list` and `cancel` really are
    /// 404 on this prefix, and that neither of these two answers `401` — is
    /// `backends/tests/integration/tests/browser_checkout.rs`, because it
    /// needs the mounted router.
    #[test]
    fn the_browser_surface_offers_two_read_and_confirm_routes_and_nothing_else() {
        let paths: Vec<(&str, &[&str])> = BROWSER_ROUTES
            .iter()
            .map(|route| (route.path, route.methods))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("/payment_intents/{id}", ["GET"].as_slice()),
                ("/payment_intents/{id}/confirm", ["POST"].as_slice()),
            ]
        );
    }

    /// The 404's sentence is a wire contract with a package that cannot
    /// import `RESOURCE`: `sdks/stripe-js/src/testing/browser-stub.ts`'s
    /// `notFoundEnvelope` builds this exact string, and every error case in
    /// that package's suite is written against it.
    #[test]
    fn the_uniform_404_says_payment_intent_the_way_the_browser_package_expects() {
        assert_eq!(RESOURCE, "payment intent");
        assert_eq!(
            vpay_core::Classify::public_message(&not_found("pi_1")),
            "No such payment intent: pi_1"
        );
    }
}
