//! The decisive test for this crate's authentication half: assertions minted
//! by [`vpay_sdk::auth::mint_client_assertion`] are handed to the **real**
//! verifier vpay will run — `authkestra_op::client_assertion::
//! verify_client_assertion` at the pinned `=0.7.1` — against a
//! `ClientRegistration` holding the matching public JWK.
//!
//! Why this rather than asserting the claim set against a table: a claim-set
//! assertion proves the SDK produces what *this repository believes* the OP
//! wants. Only running the OP's own code proves what the OP actually wants.
//! Every rule in `docs/flows/merchant-auth.md`'s "client assertion" table —
//! `iss == sub == client_id`, `aud` ∈ {token endpoint, issuer}, `exp` within
//! `MAX_CLIENT_ASSERTION_LIFETIME_SECS`, `kid` selection — is enforced by the
//! code under `verify_client_assertion`, not by anything in this file.
//!
//! What it does **not** prove: that vpay serves a token endpoint (nothing
//! does — see `docs/status.md`), that a `jti` is ever spent (replay tracking
//! is a `ClientAssertionStore` concern, deliberately outside this function),
//! or that a real deployment's registration would carry the right
//! `allowed_audiences`.

// See `tests/support/mod.rs` for why this allow list mirrors
// `backends/apps/vpay-server/tests/cli.rs`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::LazyLock;
use std::time::Duration;

use authkestra_op::client_assertion::verify_client_assertion;
use authkestra_op::{ClientRegistration, GrantType, TokenEndpointAuthMethod};
use serde_json::Value;
use vpay_sdk::auth::mint_client_assertion;
use vpay_sdk::{Client, ConfigError, Credentials};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

mod support;

const CLIENT_ID: &str = "merchant_acme";
const ISSUER: &str = "https://api.vpay.test/v1/oauth";
const TOKEN_ENDPOINT: &str = "https://api.vpay.test/v1/oauth/token";

/// Three keypairs, generated once per test binary. RSA generation is by far
/// the slowest thing in this suite; sharing the keys across tests keeps it
/// affordable without making any single verification fake — every test below
/// still signs with a real private key and verifies against the real public
/// half.
static KEY_A: LazyLock<support::TestKey> = LazyLock::new(|| support::generate_key(Some("key-a")));
static KEY_B: LazyLock<support::TestKey> = LazyLock::new(|| support::generate_key(Some("key-b")));
/// A keypair whose JWK carries **no `kid`** — the single-registered-key
/// shape. Named for what it is, not for one test's use of it: it plays the
/// registered key in most tests here and the *unregistered* one in
/// `an_assertion_signed_by_an_unregistered_keypair_is_refused`, which
/// registers `KEY_A` instead. (It was called `KEY_UNREGISTERED`, which was
/// wrong in every test but that one.)
static KEY_NO_KID: LazyLock<support::TestKey> = LazyLock::new(|| support::generate_key(None));

/// A merchant registration as vpay would build one from its YAML: no secret,
/// `client_credentials` only, `private_key_jwt` at the token endpoint, and
/// the merchant's public JWK Set inline.
fn registration(jwks: Value) -> ClientRegistration {
    ClientRegistration {
        client_id: CLIENT_ID.to_string(),
        client_secret_hash: None,
        redirect_uris: Vec::new(),
        grant_types: vec![GrantType::ClientCredentials],
        scopes: Vec::new(),
        // Deprecated at 0.7.0 and no longer read by any handler (PKCE is
        // unconditional on the authorization-code grant, which a merchant
        // client never uses); the field is still required to construct the
        // struct, so it is set and allowed rather than worked around.
        #[allow(deprecated)]
        require_pkce: false,
        allowed_audiences: vec!["vpay:v1".to_string()],
        token_endpoint_auth_method: Some(TokenEndpointAuthMethod::PrivateKeyJwt),
        jwks: Some(jwks),
    }
}

/// What the OP passes as `expected_audiences`: its token endpoint URL and its
/// issuer identifier (`handlers::token::authenticate_client`). The SDK sends
/// the token endpoint; both are accepted, and this list is what proves the
/// SDK's choice lands inside the accepted set rather than merely inside a set
/// this test invented.
fn expected_audiences() -> Vec<String> {
    vec![TOKEN_ENDPOINT.to_string(), ISSUER.to_string()]
}

#[test]
fn an_assertion_without_a_kid_is_accepted_when_one_key_is_registered() {
    let credentials = Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap();
    let assertion =
        mint_client_assertion(&credentials, TOKEN_ENDPOINT, Duration::from_secs(60)).unwrap();

    let client = registration(support::jwks(&[&KEY_NO_KID]));
    let verified = verify_client_assertion(&assertion, &client, &expected_audiences())
        .expect("the real OP verifier accepts an assertion this SDK minted");

    // The `jti` the OP would spend is the one the SDK generated — proof the
    // claim reached the verifier intact, not merely that verification passed.
    assert!(!verified.jti.is_empty());
    assert!(uuid::Uuid::parse_str(&verified.jti).is_ok());
}

#[test]
fn a_kid_selects_the_matching_key_out_of_two_registered_keys() {
    let jwks = support::jwks(&[&KEY_A, &KEY_B]);

    for key in [&*KEY_A, &*KEY_B] {
        let kid = key.kid.as_deref().expect("both shared keys carry a kid");
        let credentials = Credentials::rsa_pem(CLIENT_ID, &key.pem)
            .unwrap()
            .with_kid(kid);
        let assertion =
            mint_client_assertion(&credentials, TOKEN_ENDPOINT, Duration::from_secs(60)).unwrap();

        verify_client_assertion(
            &assertion,
            &registration(jwks.clone()),
            &expected_audiences(),
        )
        .unwrap_or_else(|e| panic!("assertion signed with {kid} should verify, got {e:?}"));
    }
}

#[test]
fn an_assertion_naming_a_kid_it_did_not_sign_with_is_refused() {
    // Signed by KEY_A but claiming KEY_B's `kid`. `select_key` looks up
    // strictly by `kid` with no "try them all" fallback, so this must fail —
    // if it passed, a merchant could have a signature attributed to a key it
    // does not hold.
    let credentials = Credentials::rsa_pem(CLIENT_ID, &KEY_A.pem)
        .unwrap()
        .with_kid("key-b");
    let assertion =
        mint_client_assertion(&credentials, TOKEN_ENDPOINT, Duration::from_secs(60)).unwrap();

    let client = registration(support::jwks(&[&KEY_A, &KEY_B]));
    assert!(verify_client_assertion(&assertion, &client, &expected_audiences()).is_err());
}

#[test]
fn an_assertion_signed_by_an_unregistered_keypair_is_refused() {
    let credentials = Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem)
        .unwrap()
        .with_kid("key-a");
    let assertion =
        mint_client_assertion(&credentials, TOKEN_ENDPOINT, Duration::from_secs(60)).unwrap();

    // Registered JWKS holds KEY_A under `key-a`; the assertion was signed by
    // a different private key entirely.
    let client = registration(support::jwks(&[&KEY_A]));
    assert!(verify_client_assertion(&assertion, &client, &expected_audiences()).is_err());
}

#[test]
fn an_assertion_minted_for_another_audience_is_refused() {
    let credentials = Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap();
    let assertion = mint_client_assertion(
        &credentials,
        "https://another-op.example/token",
        Duration::from_secs(60),
    )
    .unwrap();

    let client = registration(support::jwks(&[&KEY_NO_KID]));
    assert!(verify_client_assertion(&assertion, &client, &expected_audiences()).is_err());
}

#[test]
fn an_assertion_for_a_different_client_id_is_refused() {
    // `iss`/`sub` are both `client_id` by construction in this SDK, so the
    // only way to reach the OP's `iss == sub == client_id` check is to verify
    // against a registration for someone else — which is exactly the shape a
    // stolen assertion would have.
    let credentials = Credentials::rsa_pem("merchant_other", &KEY_NO_KID.pem).unwrap();
    let assertion =
        mint_client_assertion(&credentials, TOKEN_ENDPOINT, Duration::from_secs(60)).unwrap();

    let client = registration(support::jwks(&[&KEY_NO_KID]));
    assert!(verify_client_assertion(&assertion, &client, &expected_audiences()).is_err());
}

#[test]
fn an_assertion_at_the_maximum_lifetime_is_still_accepted() {
    // The boundary the OP enforces (`exp > now + 300` is refused) and the
    // boundary the builder allows must be the same number, or the SDK would
    // permit a configuration that always fails on the wire.
    let credentials = Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap();
    let assertion =
        mint_client_assertion(&credentials, TOKEN_ENDPOINT, Duration::from_secs(300)).unwrap();

    let client = registration(support::jwks(&[&KEY_NO_KID]));
    assert!(verify_client_assertion(&assertion, &client, &expected_audiences()).is_ok());
}

#[test]
fn a_lifetime_beyond_the_op_ceiling_is_refused_by_the_builder_not_by_the_op() {
    // `docs/flows/merchant-auth.md`: "Cannot happen: the SDK refuses to be
    // configured that way." This asserts the refusal happens at construction,
    // where the caller can see it — not as an `invalid_client` from a server
    // that does not exist yet.
    let built = Client::builder("https://api.vpay.test")
        .credentials(Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap())
        .assertion_lifetime(Duration::from_secs(301))
        .build();
    assert!(matches!(
        built,
        Err(ConfigError::InvalidAssertionLifetime { .. })
    ));

    // And directly, for anyone minting an assertion without a `Client`.
    assert!(matches!(
        mint_client_assertion(
            &Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap(),
            TOKEN_ENDPOINT,
            Duration::from_secs(301),
        ),
        Err(ConfigError::InvalidAssertionLifetime { .. })
    ));
}

#[test]
fn the_issuer_is_accepted_as_an_audience_too() {
    // OIDC Core §9 lets a client address the assertion to the OP's issuer
    // identifier instead of its token endpoint. The SDK sends the token
    // endpoint; this pins the fact that the *other* accepted value is the
    // issuer, so overriding `token_endpoint` to the issuer would still
    // authenticate rather than silently breaking.
    let credentials = Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap();
    let assertion = mint_client_assertion(&credentials, ISSUER, Duration::from_secs(60)).unwrap();

    let client = registration(support::jwks(&[&KEY_NO_KID]));
    assert!(verify_client_assertion(&assertion, &client, &expected_audiences()).is_ok());
}

/// Drives a real [`Client`] through one token exchange against a local stub
/// and hands back the `client_assertion` it actually put on the wire.
///
/// The stub's address is `http://127.0.0.1:<port>` — an address reachable
/// only from this process, standing in for `http://vpay-server:8080` inside a
/// compose network or a private DNS name in production. That is the whole
/// setup this pair of tests needs: the URL the merchant POSTs to is not one
/// of the two names the OP calls itself.
async fn assertion_sent_by(build: impl FnOnce(&MockServer) -> Client) -> String {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(support::token_response("tok_1", 300)),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/balance"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "balance",
            "available": [],
            "pending": [],
        })))
        .mount(&server)
        .await;

    let client = build(&server);
    client.balance().retrieve().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|r| r.url.path() == "/v1/oauth/token")
        .expect("the token endpoint was called");
    let pairs = support::form_pairs(&token_request.body);
    let raw = support::form_field(&pairs, "client_assertion").expect("a client_assertion was sent");
    support::percent_decode(raw)
}

#[tokio::test]
async fn the_real_verifier_refuses_a_client_that_reaches_vpay_internally_and_sets_no_audience() {
    // The defect, run through the OP's own code rather than argued about.
    // Everything else in the assertion is correct — the signature verifies,
    // `iss`/`sub` are the client id, the lifetime is in range — and the OP
    // still refuses it, because `aud` names a URL the OP has never called
    // itself.
    let assertion = assertion_sent_by(|server| {
        Client::builder(server.uri())
            .credentials(Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap())
            .build()
            .unwrap()
    })
    .await;

    let client = registration(support::jwks(&[&KEY_NO_KID]));
    assert!(
        verify_client_assertion(&assertion, &client, &expected_audiences()).is_err(),
        "an assertion addressed to the internal URL must be refused"
    );
}

#[tokio::test]
async fn the_real_verifier_accepts_the_same_client_once_assertion_audience_is_set() {
    // The fix, through the same verifier and the same registration: only
    // `assertion_audience` differs from the test above. The token request
    // still goes to the internal address; the claim names the OP.
    let assertion = assertion_sent_by(|server| {
        Client::builder(server.uri())
            .credentials(Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap())
            .assertion_audience(TOKEN_ENDPOINT)
            .build()
            .unwrap()
    })
    .await;

    let client = registration(support::jwks(&[&KEY_NO_KID]));
    verify_client_assertion(&assertion, &client, &expected_audiences())
        .expect("the real OP verifier accepts the assertion once its audience names the OP");
}

#[tokio::test]
async fn the_issuer_works_as_an_assertion_audience_too() {
    // `expected_audiences` holds both names; a merchant that configured the
    // issuer rather than the token endpoint must authenticate as well.
    let assertion = assertion_sent_by(|server| {
        Client::builder(server.uri())
            .credentials(Credentials::rsa_pem(CLIENT_ID, &KEY_NO_KID.pem).unwrap())
            .assertion_audience(ISSUER)
            .build()
            .unwrap()
    })
    .await;

    let client = registration(support::jwks(&[&KEY_NO_KID]));
    verify_client_assertion(&assertion, &client, &expected_audiences())
        .expect("the OP accepts its issuer identifier as the assertion audience");
}
