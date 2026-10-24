//! A probe of Notch Pay's real **sandbox**, for RFC-0012 § 10's sandbox items.
//! Run it on a machine whose egress presents Notch Pay's own certificate; see
//! `docs/runbooks/notchpay-sandbox-test.md`, which is the only supported way
//! to run it. Nothing in `just ci` runs it, and it is not a test.
//!
//! ```text
//! cargo run -p vpay-notchpay --example sandbox_probe -- read     --out <report.json>
//! cargo run -p vpay-notchpay --example sandbox_probe -- payments --out <report.json>
//! cargo run -p vpay-notchpay --example sandbox_probe -- webhooks --out <report.json> --webhook-url <url>
//! cargo run -p vpay-notchpay --example sandbox_probe -- search   --out <report.json> --reference <ref> [--reference <ref> …]
//! cargo run -p vpay-notchpay --example sandbox_probe -- refunds  --out <report.json>
//! cargo run -p vpay-notchpay --example sandbox_probe -- two-step --out <report.json>
//! cargo run -p vpay-notchpay --example sandbox_probe -- channels --out <report.json>
//! cargo run -p vpay-notchpay --example sandbox_probe -- webhook-receive --out <report.json> --webhook-url <url>
//! cargo run -p vpay-notchpay --example sandbox_probe -- webhook-update  --out <report.json> --reference <ep_…> --webhook-url <url>
//! cargo run -p vpay-notchpay --example sandbox_probe -- dashboard-deliveries --out <report.json>
//! cargo run -p vpay-notchpay --example sandbox_probe -- verify-signature --body <file> --signature <hex>
//! ```
//!
//! **`--ca-bundle <path>` (cloud only, off by default).** Every phase that
//! calls Notch Pay takes it: every PEM certificate at `<path>` is trusted
//! *in addition to* the vendored `webpki_roots`, for this probe's own HTTP
//! client only.
//! It exists for one environment, a cloud session whose egress gateway
//! re-terminates TLS with its own CA (`/root/.ccr/ca-bundle.crt` there), so
//! that step 1 can run from such a session. It is test tooling: this
//! example ships in no image, and the same option must **never** be added to
//! `vpay-server`, `vpay-worker`, `vpay_provider::http`'s shared constructors
//! or any shipping configuration, because ADR-0003 forbids an environment
//! branch in shipping code. The client it builds keeps both of
//! `vpay_provider::http`'s removals: redirects are not followed (so
//! `X-Grant` is never replayed at another host) and proxy variables are
//! ignored. A path that does not exist, does not parse, or holds no
//! certificate stops the probe before any request. See the runbook's
//! "Cloud sessions" section.
//!
//! **The keys never leave the process.** They are read from
//! `NOTCHPAY_TEST_PUBLIC_KEY`, `NOTCHPAY_TEST_PRIVATE_KEY` and (for
//! `verify-signature` only) `NOTCHPAY_TEST_HASH_KEY`. Nothing prints them.
//! The report is redacted before it is written: a string that contains a key
//! is replaced by the key's name, and a field whose name looks like a secret
//! (a webhook endpoint's signing secret, RFC-0012 § 10 item 13) is replaced by
//! its shape (`hsk_test_…`). **The probe refuses to run unless both keys
//! contain `test.` or `test_`**, the markers `vpay_config` reads.
//!
//! **What it exercises.** Every request body is serialised from this crate's
//! own `wire` types, and every answer is decoded with them, so a shape the
//! client would refuse shows up as `decoded_by_vpay: false`. The bodies are
//! sent with the same headers `Client` sends, over the same
//! `vpay_provider::http::client`, by hand rather than through `Client`,
//! because `Client` keeps no raw body and the report needs one. Each read is
//! also made through `Client` itself, and its outcome is recorded beside the
//! raw one. RFC-0012's step 5 (the adapter, through vpay-server) is the
//! end-to-end proof; this is the evidence its mapping is corrected from.
//!
//! **Phases, each a separate run, each appending to the same report.**
//! `read` changes nothing on the account. `payments` initializes and
//! processes six sandbox payments with Notch Pay's documented outcome
//! numbers. `webhooks` creates up to three endpoints at the URL you give and
//! then deletes every one it created. `refunds` (RFC-0012 § 10 items 6 and
//! 12) makes one sandbox payment on MTN's success number and asks
//! `POST /refunds` four times against it, twice with an identical body, to
//! see whether a repeated request refunds twice. The adapter does not call
//! `/refunds`: `notchpay::refund` stays a declared `NotImplemented` until
//! Notch Pay answers item 6 in writing.
//!
//! **Added 2026-10-23**, after ADR-0032 and RFC-0012 step 8 were built against
//! a stub. `two-step` initializes two payments and processes neither at
//! first: it re-sends one initialize to record the `409`, reads the
//! never-processed one on a schedule (`NOTCHPAY_PROBE_EXPIRY_CHECKS`, seconds,
//! default `0,60,300,900,1800,3600`) to see whether and when it expires,
//! processes the other after `NOTCHPAY_PROBE_LATE_PROCESS_SECONDS` (default
//! `300`), and at the end processes the never-processed one too. `channels`
//! sends every documented MTN and Orange outcome number on `cm.mtn` and
//! `cm.orange`, then each network's success number on the other network's
//! channel. `webhook-receive` creates **one** endpoint at the URL you give
//! (a capture inbox you can read), makes one successful and one failed
//! payment, waits `NOTCHPAY_PROBE_DELIVERY_WAIT` seconds (default `120`) for
//! deliveries, then deletes the endpoint and lists the account's endpoints
//! to show whether the delete worked. `webhook-update` reads one endpoint
//! this probe created (`--reference ep_…`) by id and sends it the documented
//! `PUT /webhooks/{id}` with the URL again, because the endpoint
//! `POST /webhooks` created is listed with `"url": null` and delivered
//! nothing. (On 2026-10-24 the listing showed that null for a dashboard
//! endpoint that delivers, too, so it is how the API lists every endpoint,
//! not proof the URL was dropped.) Reading the inbox and checking
//! `x-notch-signature` (with `verify-signature`) are done by hand; the probe
//! never talks to the capture service.
//!
//! **Added 2026-10-24.** `dashboard-deliveries` creates and deletes **no**
//! endpoint: the maintainer registers one by hand in Notch Pay's dashboard,
//! because the one `POST /webhooks` created delivered nothing. It lists
//! the account's endpoints, makes the same two payments as
//! `webhook-receive` (MTN's success and other-failure numbers), waits
//! `NOTCHPAY_PROBE_DELIVERY_WAIT` seconds, and lists the endpoints again.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use reqwest::Method;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde_json::{Value, json};
use sha2::Sha256;
use vpay_notchpay::wire::{
    CreateWebhook, InitializePayment, PAYMENT_EVENTS, PaymentEnvelope, ProcessData, ProcessPayment,
    WebhookEnvelope, WebhookPage,
};
use vpay_notchpay::{Answer, Client, Keys};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The markers `vpay_config::config::TEST_KEY_MARKERS` reads. A key with
/// neither stops the probe before any request.
const TEST_KEY_MARKERS: [&str; 2] = ["test.", "test_"];
const PUBLIC_KEY: &str = "NOTCHPAY_TEST_PUBLIC_KEY";
const PRIVATE_KEY: &str = "NOTCHPAY_TEST_PRIVATE_KEY";
const HASH_KEY: &str = "NOTCHPAY_TEST_HASH_KEY";
const DEFAULT_BASE_URL: &str = "https://api.notchpay.co";
/// The channel the adapter's setting takes (`vpay_adapter_notchpay::KNOWN_CHANNELS`),
/// and so the one a charge confirmed by the rail code is sent; a charge routed
/// by method type is sent `cm.mtn` or `cm.orange` instead.
const CHANNEL: &str = "cm.mobile";

/// Notch Pay's documented sandbox outcome numbers
/// (<https://developer.notchpay.co/get-started/testing>, read 2026-10-20).
/// The page lists them under `cm.mtn` and `cm.orange`; whether `cm.mobile`
/// honours them is RFC-0012 § 10 item 3, which is what sending them on
/// `cm.mobile` measures. The page gives the five suffixes "for each
/// operator" (read again 2026-10-23); this list takes Orange's success number
/// only, and `channels` runs Orange's other four on `cm.orange`.
const OUTCOMES: [(&str, &str); 6] = [
    ("+237670000000", "MTN: success"),
    ("+237670000001", "MTN: insufficient funds"),
    ("+237670000002", "MTN: failure (other reason)"),
    ("+237670000003", "MTN: timeout (no response)"),
    ("+237670000004", "MTN: canceled by user"),
    ("+237690000000", "Orange: success"),
];

/// RFC-0012 step 8 (built 2026-10-23 against a stub): the channel a charge
/// routed by method type is processed on, with every outcome number the
/// testing page documents for that network, then each network's success
/// number on the other network's channel. The testing page lists the same
/// `XXX000000`–`XXX000004` pattern for every operator.
const CHANNEL_CASES: [(&str, &str, &str); 12] = [
    ("cm.mtn", "+237670000000", "cm.mtn, MTN: success"),
    ("cm.mtn", "+237670000001", "cm.mtn, MTN: insufficient funds"),
    (
        "cm.mtn",
        "+237670000002",
        "cm.mtn, MTN: failure (other reason)",
    ),
    (
        "cm.mtn",
        "+237670000003",
        "cm.mtn, MTN: timeout (no response)",
    ),
    ("cm.mtn", "+237670000004", "cm.mtn, MTN: canceled by user"),
    ("cm.orange", "+237690000000", "cm.orange, Orange: success"),
    (
        "cm.orange",
        "+237690000001",
        "cm.orange, Orange: insufficient funds",
    ),
    (
        "cm.orange",
        "+237690000002",
        "cm.orange, Orange: failure (other reason)",
    ),
    (
        "cm.orange",
        "+237690000003",
        "cm.orange, Orange: timeout (no response)",
    ),
    (
        "cm.orange",
        "+237690000004",
        "cm.orange, Orange: canceled by user",
    ),
    (
        "cm.orange",
        "+237670000000",
        "wrong network: MTN's success number on cm.orange",
    ),
    (
        "cm.mtn",
        "+237690000000",
        "wrong network: Orange's success number on cm.mtn",
    ),
];

/// The statuses after which a payment does not move again (RFC-0012 § 4).
const TERMINAL: [&str; 9] = [
    "complete",
    "failed",
    "canceled",
    "expired",
    "abandoned",
    "rejected",
    "refunded",
    "partialy-refunded",
    "incomplete",
];

struct Probe {
    http: reqwest::Client,
    client: Client,
    base_url: String,
    public_key: String,
    private_key: String,
    report: Vec<Value>,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| flag(&args, name);
    let phase = args.first().map(String::as_str).unwrap_or_default();
    if phase == "verify-signature" {
        let body = flag("--body").ok_or("verify-signature needs --body <file>")?;
        let signature = flag("--signature").ok_or("verify-signature needs --signature <hex>")?;
        return verify_signature(&PathBuf::from(body), &signature);
    }
    if !matches!(
        phase,
        "read"
            | "payments"
            | "webhooks"
            | "search"
            | "refunds"
            | "two-step"
            | "channels"
            | "webhook-receive"
            | "webhook-update"
            | "dashboard-deliveries"
    ) {
        return Err(
            "usage: sandbox_probe read|payments|webhooks|search|refunds|two-step|channels|webhook-receive|webhook-update|dashboard-deliveries \
             --out <report.json> [--webhook-url <url>] [--reference <ref> …] [--ca-bundle <pem>] \
             | verify-signature --body <file> --signature <hex>"
                .into(),
        );
    }
    let out = PathBuf::from(flag("--out").ok_or("--out <report.json> is required")?);
    // Built before the runtime, the keys and any request: a bad bundle path
    // fails here.
    let http = http_client(ca_bundle(&args)?.as_deref())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut probe = Probe::new(http)?;
        let started = now();
        let result = match phase {
            "read" => probe.read_phase().await,
            "payments" => probe.payments_phase().await,
            "search" => probe.search_phase(&all_flags(&args, "--reference")).await,
            "refunds" => probe.refunds_phase().await,
            "two-step" => probe.two_step_phase().await,
            "channels" => probe.channels_phase().await,
            "dashboard-deliveries" => probe.dashboard_deliveries_phase().await,
            "webhook-update" => {
                let url =
                    flag("--webhook-url").ok_or("webhook-update needs --webhook-url <url>")?;
                let id = flag("--reference").ok_or("webhook-update needs --reference <ep_…>")?;
                probe.webhook_update_phase(&id, &url).await
            }
            "webhook-receive" => {
                let url =
                    flag("--webhook-url").ok_or("webhook-receive needs --webhook-url <url>")?;
                probe.webhook_receive_phase(&url).await
            }
            _ => {
                let url = flag("--webhook-url").ok_or("webhooks needs --webhook-url <url>")?;
                probe.webhooks_phase(&url).await
            }
        };
        probe.write(
            &out,
            phase,
            started,
            result.as_ref().err().map(ToString::to_string),
        )?;
        result
    })
}

/// Every value after an occurrence of `name`.
fn all_flags(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter_map(|pair| match pair {
            [flag, value] if flag == name => Some(value.clone()),
            _ => None,
        })
        .collect()
}

/// The value after `name`, if `name` is present.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// `--ca-bundle <path>`, or `None` when the flag is absent: off by default.
/// A flag with no value is an error rather than "off", so a typo never
/// silently falls back to a client that cannot connect.
fn ca_bundle(args: &[String]) -> Result<Option<PathBuf>> {
    if !args.iter().any(|a| a == "--ca-bundle") {
        return Ok(None);
    }
    match flag(args, "--ca-bundle") {
        Some(path) if !path.starts_with("--") => Ok(Some(PathBuf::from(path))),
        _ => Err("--ca-bundle needs a path to a PEM file".into()),
    }
}

/// The probe's HTTP client. Without a bundle it is exactly
/// `vpay_provider::http::client()`, what `vpay-server` builds.
fn http_client(ca_bundle: Option<&Path>) -> Result<reqwest::Client> {
    match ca_bundle {
        None => Ok(vpay_provider::http::client()?),
        Some(path) => client_with_extra_roots(path),
    }
}

/// `vpay_provider::http::client()`'s configuration with the certificates at
/// `path` added to the vendored roots. Cloud-only test tooling: see the
/// module doc, and do not copy this into anything that ships (ADR-0003).
/// It repeats the shared builder rather than extending it on purpose,
/// because the shared builder must have no way to take extra roots.
fn client_with_extra_roots(path: &Path) -> Result<reqwest::Client> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject as _;

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let extra = CertificateDer::pem_file_iter(path)
        .map_err(|e| format!("--ca-bundle {}: {e}", path.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| format!("--ca-bundle {}: {e}", path.display()))?;
    if extra.is_empty() {
        return Err(format!("--ca-bundle {}: no PEM certificate in it", path.display()).into());
    }
    let (added, ignored) = roots.add_parsable_certificates(extra);
    if added == 0 {
        return Err(format!(
            "--ca-bundle {}: none of its {ignored} certificate(s) is a usable root",
            path.display()
        )
        .into());
    }
    say(&format!(
        "--ca-bundle: {added} extra root(s) trusted for this run ({ignored} ignored); cloud-only aid"
    ));
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        // The two removals `vpay_provider::http` makes, kept: a followed
        // redirect would replay `X-Grant` at whatever host answered.
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?)
}

fn test_key(name: &str) -> Result<String> {
    let value = std::env::var(name).map_err(|_| format!("{name} is not set"))?;
    if !TEST_KEY_MARKERS.iter().any(|m| value.contains(m)) {
        return Err(format!(
            "{name} contains neither `test.` nor `test_`: refusing to call Notch Pay with what may be a live key"
        )
        .into());
    }
    Ok(value)
}

impl Probe {
    fn new(http: reqwest::Client) -> Result<Self> {
        let public_key = test_key(PUBLIC_KEY)?;
        let private_key = test_key(PRIVATE_KEY)?;
        let base_url =
            std::env::var("NOTCHPAY_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
        let client = Client::new(
            http.clone(),
            &base_url,
            Keys::new(&public_key, &private_key)?,
        )?
        .with_request_timeout(Duration::from_secs(30));
        Ok(Self {
            http,
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            public_key,
            private_key,
            report: Vec::new(),
        })
    }

    /// One raw exchange, recorded redacted. `decode` is the vpay wire type
    /// the answer is checked against.
    async fn exchange(
        &mut self,
        item: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
        decode: Decode,
    ) -> Result<(u16, Value)> {
        let mut request = self
            .http
            .request(method.clone(), format!("{}{path}", self.base_url))
            .header(AUTHORIZATION, sensitive(&self.public_key)?)
            .header("X-Grant", sensitive(&self.private_key)?)
            .header(ACCEPT, "application/json")
            .timeout(Duration::from_secs(30));
        if let Some(body) = &body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(body)?);
        }
        let response = request.send().await.map_err(|e| e.without_url())?;
        let status = response.status();
        let bytes = response.bytes().await?;
        let parsed: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({ "_non_json": String::from_utf8_lossy(&bytes) }));
        let decoded = status.is_success().then(|| decode.check(&bytes));
        let keys: Vec<&String> = parsed
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        let entry = json!({
            "item": item,
            "at": now(),
            "method": method.as_str(),
            "path": path,
            "request_body": body,
            "status": status.as_u16(),
            "top_level_fields": keys,
            "decoded_by_vpay": decoded,
            "body": parsed,
        });
        self.record(entry);
        Ok((status.as_u16(), parsed))
    }

    fn record(&mut self, entry: Value) {
        let redacted = self.redact(entry, None);
        say(&summary(&redacted));
        self.report.push(redacted);
    }

    fn note(&mut self, item: &str, text: impl Into<String>) {
        self.record(json!({ "item": item, "at": now(), "note": text.into() }));
    }

    /// The same read through `Client`, so the report says what vpay's own
    /// call path made of the answer.
    async fn client_read(&mut self, item: &str, reference: &str) {
        let outcome = match self.client.retrieve_payment(reference).await {
            Ok(Answer::Success(envelope)) => json!({
                "answer": "success",
                "status": envelope.transaction.status,
                "amount_minor": envelope.transaction.amount_minor(),
                "currency": envelope.transaction.currency,
                "id_present": envelope.transaction.id.is_some(),
                "reference_present": envelope.transaction.reference.is_some(),
                "trxref_present": envelope.transaction.trxref.is_some(),
            }),
            Ok(Answer::Refused(refusal)) => json!({
                "answer": "refused",
                "http_status": refusal.http_status,
                "message": refusal.message,
            }),
            Err(error) => json!({ "answer": "error", "error": error.to_string() }),
        };
        self.record(json!({
            "item": item,
            "at": now(),
            "via": "vpay_notchpay::Client::retrieve_payment",
            "reference": reference,
            "outcome": outcome,
        }));
    }

    async fn read_phase(&mut self) -> Result<()> {
        let missing = format!("vpay-probe-{}", uuid::Uuid::new_v4());
        self.exchange(
            "read-only first call; 15",
            Method::GET,
            &format!("/payments/{missing}"),
            None,
            Decode::Payment,
        )
        .await?;
        self.client_read("read-only first call", &missing).await;
        self.exchange(
            "16; 15",
            Method::GET,
            "/webhooks?page=1",
            None,
            Decode::WebhookPage,
        )
        .await?;
        let mode = self
            .report
            .iter()
            .flat_map(|entry| mode_hints(entry.get("body").unwrap_or(&Value::Null)))
            .collect::<Vec<_>>();
        self.note(
            "mode",
            if mode.is_empty() {
                "no field naming test, sandbox or live mode in these answers; check the dashboard says Test mode before `payments`".to_owned()
            } else {
                format!("fields naming a mode: {mode:?}. STOP if any says live.")
            },
        );
        Ok(())
    }

    async fn payments_phase(&mut self) -> Result<()> {
        let poll_for = Duration::from_secs(
            std::env::var("NOTCHPAY_PROBE_POLL_SECONDS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(180),
        );
        let mut success_reference = None;
        for (index, (phone, outcome)) in OUTCOMES.iter().enumerate() {
            // Varied amounts: Notch Pay's fraud system watches for many
            // identical payments to one number. 100 is the documented minimum.
            let amount = 100 + 7 * i64::try_from(index)?;
            let reference = uuid::Uuid::new_v4().to_string();
            let item = format!("2, 3, 5: {outcome}");
            let init = InitializePayment {
                amount,
                currency: "XAF",
                phone,
                reference: &reference,
                description: Some("vpay sandbox probe (RFC-0012 step 1)"),
            };
            let (status, body) = self
                .exchange(
                    &item,
                    Method::POST,
                    "/payments",
                    Some(serde_json::to_value(&init)?),
                    Decode::Payment,
                )
                .await?;
            if !(200..300).contains(&status) {
                self.note(&item, "initialize refused; nothing more for this number");
                continue;
            }
            let trx = body
                .pointer("/transaction/reference")
                .or_else(|| body.pointer("/data/reference"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let process = serde_json::to_value(ProcessPayment {
                channel: CHANNEL,
                data: ProcessData { phone },
            })?;
            // Item 2: the merchant's reference first, as the adapter sends it;
            // Notch Pay's own only if that is refused.
            let (status, _) = self
                .exchange(
                    &format!("2: process by our reference ({outcome})"),
                    Method::POST,
                    &format!("/payments/{reference}"),
                    Some(process.clone()),
                    Decode::Payment,
                )
                .await?;
            if !(200..300).contains(&status)
                && let Some(trx) = trx.as_deref().filter(|t| *t != reference)
            {
                self.exchange(
                    &format!("2: process by Notch Pay's reference ({outcome})"),
                    Method::POST,
                    &format!("/payments/{trx}"),
                    Some(process),
                    Decode::Payment,
                )
                .await?;
            }
            self.poll(&item, &reference, trx.as_deref(), poll_for)
                .await?;
            if index == 0 {
                success_reference = Some((reference, amount, *phone));
            }
        }
        if let Some((reference, amount, phone)) = success_reference {
            let again = InitializePayment {
                amount,
                currency: "XAF",
                phone,
                reference: &reference,
                description: Some("vpay sandbox probe (RFC-0012 step 1) - duplicate"),
            };
            self.exchange(
                "4: the same reference again",
                Method::POST,
                "/payments",
                Some(serde_json::to_value(&again)?),
                Decode::Payment,
            )
            .await?;
            self.client_read("4: the read after the duplicate", &reference)
                .await;
        }
        self.note("11", "item 11 (an incomplete payment's expiry) is a written question; no outcome number produces `incomplete`");
        Ok(())
    }

    /// Reads by both references until a terminal status or the deadline,
    /// recording every change of status.
    async fn poll(
        &mut self,
        item: &str,
        reference: &str,
        trx: Option<&str>,
        poll_for: Duration,
    ) -> Result<()> {
        let deadline = std::time::Instant::now() + poll_for;
        let mut last = None;
        loop {
            let (_, body) = self
                .exchange(
                    &format!("{item}: read by our reference"),
                    Method::GET,
                    &format!("/payments/{reference}"),
                    None,
                    Decode::Payment,
                )
                .await?;
            let status = body
                .pointer("/transaction/status")
                .or_else(|| body.pointer("/data/status"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if status != last {
                self.note(item, format!("status now {status:?}"));
                last.clone_from(&status);
            }
            let done = status.as_deref().is_some_and(|s| TERMINAL.contains(&s));
            if done || std::time::Instant::now() >= deadline {
                if let Some(trx) = trx.filter(|t| *t != reference) {
                    self.exchange(
                        &format!("2: {item}: read by Notch Pay's reference"),
                        Method::GET,
                        &format!("/payments/{trx}"),
                        None,
                        Decode::Payment,
                    )
                    .await?;
                }
                self.client_read(item, reference).await;
                if !done {
                    self.note(
                        item,
                        format!("no terminal status after {}s", poll_for.as_secs()),
                    );
                }
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    /// RFC-0012 § 10 items 6 and 12: does `POST /refunds` exist, what does it
    /// answer, and does a repeated request refund twice? The refunds guide
    /// documents `payment`, `amount`, `reason` and `metadata`, and no
    /// reference or idempotency header (RFC-0012 § 7). One fresh payment on
    /// MTN's success number, then four requests of 50 XAF against it: an
    /// identical pair, and a pair carrying a `reference` of ours, which the
    /// guide does not document. Then every read the guide names. Sandbox
    /// money only, and at most the payment's own 200 XAF.
    async fn refunds_phase(&mut self) -> Result<()> {
        let (phone, amount) = ("+237670000000", 200);
        let reference = uuid::Uuid::new_v4().to_string();
        let item = "6: the payment to refund";
        let init = InitializePayment {
            amount,
            currency: "XAF",
            phone,
            reference: &reference,
            description: Some("vpay sandbox probe (RFC-0012 § 10 item 6)"),
        };
        let (status, body) = self
            .exchange(
                item,
                Method::POST,
                "/payments",
                Some(serde_json::to_value(&init)?),
                Decode::Payment,
            )
            .await?;
        let trx = trx_reference(&body)
            .filter(|_| (200..300).contains(&status))
            .ok_or("the payment to refund was not initialized; no refund was requested")?;
        // Notch Pay's `trx.…` reference: the only one its payment path takes
        // (item 2), and what the adapter records as `provider_txn_id`.
        let process = serde_json::to_value(ProcessPayment {
            channel: CHANNEL,
            data: ProcessData { phone },
        })?;
        self.exchange(
            item,
            Method::POST,
            &format!("/payments/{trx}"),
            Some(process),
            Decode::Payment,
        )
        .await?;
        let status = self.poll_trx(item, &trx, Duration::from_secs(120)).await?;
        if status.as_deref() != Some("complete") {
            self.note(
                item,
                format!("the payment read {status:?}, not complete; no refund was requested"),
            );
            return Ok(());
        }

        let reason = "vpay sandbox probe (RFC-0012 § 10 item 6)";
        let plain = json!({
            "payment": trx,
            "amount": 50,
            "reason": reason,
            "metadata": { "vpay_refund": uuid::Uuid::new_v4().to_string() },
        });
        let ours = format!("vpay-refund-{}", uuid::Uuid::new_v4());
        let with_reference = json!({
            "payment": trx,
            "amount": 50,
            "reference": ours,
            "reason": reason,
            "metadata": { "vpay_refund": ours },
        });
        let mut ids = Vec::new();
        for (label, request) in [
            ("6, 12: the first refund", &plain),
            ("6: the identical request again (a replay)", &plain),
            (
                "6: with a `reference` of ours (undocumented)",
                &with_reference,
            ),
            ("6: the same `reference` again", &with_reference),
        ] {
            let (_, answer) = self
                .exchange(
                    label,
                    Method::POST,
                    "/refunds",
                    Some(request.clone()),
                    Decode::Nothing,
                )
                .await?;
            ids.extend(refund_ids(&answer));
        }
        ids.sort();
        ids.dedup();
        self.note(
            "6",
            format!(
                "{} distinct refund id(s) in the four answers; reading back in 15 s",
                ids.len()
            ),
        );
        tokio::time::sleep(Duration::from_secs(15)).await;
        self.exchange(
            "6: the payment's refunds",
            Method::GET,
            &format!("/payments/{trx}/refunds"),
            None,
            Decode::Nothing,
        )
        .await?;
        for id in &ids {
            self.exchange(
                "6: one refund",
                Method::GET,
                &format!("/refunds/{id}"),
                None,
                Decode::Nothing,
            )
            .await?;
        }
        self.exchange(
            "6: the payment after its refunds",
            Method::GET,
            &format!("/payments/{trx}"),
            None,
            Decode::Payment,
        )
        .await?;
        Ok(())
    }

    /// Reads by Notch Pay's `trx.…` reference every 5 s until a terminal
    /// status or `poll_for`, recording each change. Returns the last status.
    async fn poll_trx(
        &mut self,
        item: &str,
        trx: &str,
        poll_for: Duration,
    ) -> Result<Option<String>> {
        let deadline = std::time::Instant::now() + poll_for;
        let mut last = None;
        loop {
            let (_, body) = self
                .exchange(
                    item,
                    Method::GET,
                    &format!("/payments/{trx}"),
                    None,
                    Decode::Payment,
                )
                .await?;
            let status = payment_status(&body);
            if status != last {
                self.note(item, format!("status now {status:?}"));
                last.clone_from(&status);
            }
            let done = status.as_deref().is_some_and(|s| TERMINAL.contains(&s));
            if done || std::time::Instant::now() >= deadline {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    /// RFC-0012 § 10 item 19: does `GET /payments?search=` find a payment by
    /// the merchant's own reference? Read-only. Each reference is also read
    /// directly, for comparison.
    async fn search_phase(&mut self, references: &[String]) -> Result<()> {
        if references.is_empty() {
            return Err("search needs at least one --reference <ref>".into());
        }
        for reference in references {
            let query: String = reference
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                        char::from(b).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect();
            self.exchange(
                "19: search",
                Method::GET,
                &format!("/payments?search={query}"),
                None,
                Decode::Nothing,
            )
            .await?;
            self.exchange(
                "19: direct read, for comparison",
                Method::GET,
                &format!("/payments/{query}"),
                None,
                Decode::Payment,
            )
            .await?;
        }
        Ok(())
    }

    /// Initializes one payment and processes nothing. Returns our reference
    /// and Notch Pay's `trx.…`.
    async fn initialize_only(
        &mut self,
        item: &str,
        phone: &str,
        amount: i64,
    ) -> Result<(String, String)> {
        let reference = uuid::Uuid::new_v4().to_string();
        let init = InitializePayment {
            amount,
            currency: "XAF",
            phone,
            reference: &reference,
            description: Some("vpay sandbox probe (ADR-0032 / RFC-0012 step 8)"),
        };
        let (status, body) = self
            .exchange(
                item,
                Method::POST,
                "/payments",
                Some(serde_json::to_value(&init)?),
                Decode::Payment,
            )
            .await?;
        let trx = trx_reference(&body)
            .filter(|_| (200..300).contains(&status))
            .ok_or_else(|| format!("{item}: initialize answered {status}, no `trx.…`"))?;
        Ok((reference, trx))
    }

    /// `POST /payments/{trx}` on `channel`, recorded.
    async fn process(&mut self, item: &str, trx: &str, channel: &str, phone: &str) -> Result<u16> {
        let body = serde_json::to_value(ProcessPayment {
            channel,
            data: ProcessData { phone },
        })?;
        let (status, _) = self
            .exchange(
                item,
                Method::POST,
                &format!("/payments/{trx}"),
                Some(body),
                Decode::Payment,
            )
            .await?;
        Ok(status)
    }

    /// ADR-0032 on Notch Pay's side: what an initialized, never-processed
    /// payment does over time, whether a late `process` still prompts, and
    /// the `409` a repeated initialize gets.
    async fn two_step_phase(&mut self) -> Result<()> {
        let checks: Vec<u64> = std::env::var("NOTCHPAY_PROBE_EXPIRY_CHECKS")
            .unwrap_or_else(|_| "0,60,300,900,1800,3600".to_owned())
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        let late: u64 = std::env::var("NOTCHPAY_PROBE_LATE_PROCESS_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);
        let phone = "+237670000000";
        let never = "ADR-0032: initialized, never processed";
        let (reference_a, trx_a) = self.initialize_only(never, phone, 151).await?;
        let later = "ADR-0032: processed late";
        let (_, trx_b) = self.initialize_only(later, phone, 157).await?;
        let again = InitializePayment {
            amount: 151,
            currency: "XAF",
            phone,
            reference: &reference_a,
            description: Some("vpay sandbox probe (ADR-0032) - the same reference again"),
        };
        self.exchange(
            "ADR-0032: the never-processed payment's reference, initialized again",
            Method::POST,
            "/payments",
            Some(serde_json::to_value(&again)?),
            Decode::Payment,
        )
        .await?;

        let start = std::time::Instant::now();
        let mut processed_late = false;
        let mut last = None;
        for at in checks {
            if !processed_late && late <= at {
                sleep_until(start, late).await;
                self.note(later, format!("processing {late}s after initialize"));
                self.process(later, &trx_b, CHANNEL, phone).await?;
                self.poll_trx(later, &trx_b, Duration::from_secs(180))
                    .await?;
                self.client_read(later, &trx_b).await;
                processed_late = true;
            }
            sleep_until(start, at).await;
            let (_, body) = self
                .exchange(
                    never,
                    Method::GET,
                    &format!("/payments/{trx_a}"),
                    None,
                    Decode::Payment,
                )
                .await?;
            let status = payment_status(&body);
            self.note(
                never,
                format!(
                    "{}s after initialize: status {status:?}",
                    start.elapsed().as_secs()
                ),
            );
            let done = status.as_deref().is_some_and(|s| TERMINAL.contains(&s));
            last = status;
            if done {
                break;
            }
        }
        if !processed_late {
            self.process(later, &trx_b, CHANNEL, phone).await?;
            self.poll_trx(later, &trx_b, Duration::from_secs(180))
                .await?;
        }
        self.client_read(never, &trx_a).await;
        // What a resubmit after a long crash meets: the held `trx.…` processed
        // late, whatever the payment now reads.
        self.note(
            never,
            format!(
                "now processing it, {}s after initialize, last read {last:?}",
                start.elapsed().as_secs()
            ),
        );
        self.process(never, &trx_a, CHANNEL, phone).await?;
        self.poll_trx(never, &trx_a, Duration::from_secs(180))
            .await?;
        Ok(())
    }

    /// RFC-0012 step 8 on Notch Pay's side: each [`CHANNEL_CASES`] row
    /// initialized, processed by its `trx.…` on its channel, and read to a
    /// terminal status or `NOTCHPAY_PROBE_POLL_SECONDS` (default 180).
    async fn channels_phase(&mut self) -> Result<()> {
        let poll_for = Duration::from_secs(
            std::env::var("NOTCHPAY_PROBE_POLL_SECONDS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(180),
        );
        for (index, (channel, phone, label)) in CHANNEL_CASES.iter().enumerate() {
            // Varied amounts, as in `payments`: the fraud system watches for
            // identical payments to one number.
            let amount = 110 + 3 * i64::try_from(index)?;
            let item = format!("step 8: {label}");
            let (_, trx) = match self.initialize_only(&item, phone, amount).await {
                Ok(pair) => pair,
                Err(error) => {
                    self.note(&item, format!("initialize failed, nothing more: {error}"));
                    continue;
                }
            };
            let status = self.process(&item, &trx, channel, phone).await?;
            if !(200..300).contains(&status) {
                self.note(&item, format!("process answered {status}; reading once"));
            }
            self.poll_trx(&item, &trx, poll_for).await?;
            self.client_read(&item, &trx).await;
        }
        Ok(())
    }

    /// RFC-0012 § 10 item 7 with an endpoint registered by hand in the
    /// dashboard (2026-10-24): nothing is created or deleted here, so the
    /// endpoint whose deliveries are checked is the one an operator makes,
    /// the route vpay's `off` default leaves them.
    async fn dashboard_deliveries_phase(&mut self) -> Result<()> {
        let wait: u64 = std::env::var("NOTCHPAY_PROBE_DELIVERY_WAIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        self.exchange(
            "7: list before",
            Method::GET,
            "/webhooks?page=1",
            None,
            Decode::WebhookPage,
        )
        .await?;
        self.note("7", format!("payments start at unix {}", now()));
        for (phone, amount, label) in [
            ("+237670000000", 151_i64, "7: a payment that succeeds"),
            ("+237670000002", 157_i64, "7: a payment that fails"),
        ] {
            let (_, trx) = self.initialize_only(label, phone, amount).await?;
            self.process(label, &trx, CHANNEL, phone).await?;
            self.poll_trx(label, &trx, Duration::from_secs(180)).await?;
        }
        self.note("7", format!("waiting {wait}s for deliveries"));
        tokio::time::sleep(Duration::from_secs(wait)).await;
        self.exchange(
            "7: list after",
            Method::GET,
            "/webhooks?page=1",
            None,
            Decode::WebhookPage,
        )
        .await?;
        Ok(())
    }

    /// RFC-0012 § 10 items 7 and 17 with a URL that can actually receive:
    /// one endpoint, two payments, a wait for deliveries, and the endpoint
    /// deleted (or the attempt recorded, since `DELETE` answered `404` on
    /// 2026-10-21).
    async fn webhook_receive_phase(&mut self, url: &str) -> Result<()> {
        let wait: u64 = std::env::var("NOTCHPAY_PROBE_DELIVERY_WAIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        self.exchange(
            "7: list before",
            Method::GET,
            "/webhooks?page=1",
            None,
            Decode::WebhookPage,
        )
        .await?;
        let mut events: Vec<&str> = PAYMENT_EVENTS.to_vec();
        events.push("payment.processing");
        let body = CreateWebhook {
            url,
            description: Some("vpay sandbox probe (RFC-0012 item 7) - delete me"),
            events: &events,
        };
        let (status, answer) = self
            .exchange(
                "7, 17: create one endpoint",
                Method::POST,
                "/webhooks",
                Some(serde_json::to_value(&body)?),
                Decode::Webhook,
            )
            .await?;
        let id = answer
            .pointer("/endpoint/id")
            .or_else(|| answer.pointer("/webhook/id"))
            .or_else(|| answer.pointer("/data/id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if !(200..300).contains(&status) {
            self.note("7", "the endpoint was not created; nothing to receive");
            return Ok(());
        }
        let started = now();
        self.note("7", format!("endpoint created at unix {started}: {id:?}"));
        for (phone, amount, label) in [
            ("+237670000000", 133_i64, "7: a payment that succeeds"),
            ("+237670000002", 139_i64, "7: a payment that fails"),
        ] {
            let (_, trx) = self.initialize_only(label, phone, amount).await?;
            self.process(label, &trx, CHANNEL, phone).await?;
            self.poll_trx(label, &trx, Duration::from_secs(180)).await?;
        }
        self.note("7", format!("waiting {wait}s for deliveries"));
        tokio::time::sleep(Duration::from_secs(wait)).await;
        let Some(id) = id else {
            self.note(
                "cleanup",
                "created, but no id in the answer: delete it in the dashboard",
            );
            return Ok(());
        };
        let (deleted, _) = self
            .exchange(
                "cleanup",
                Method::DELETE,
                &format!("/webhooks/{id}"),
                None,
                Decode::Nothing,
            )
            .await?;
        self.exchange(
            "cleanup: list after",
            Method::GET,
            "/webhooks?page=1",
            None,
            Decode::WebhookPage,
        )
        .await?;
        self.note(
            "cleanup",
            if (200..300).contains(&deleted) {
                format!("endpoint {id} deleted ({deleted})")
            } else {
                format!("endpoint {id} NOT deleted ({deleted}): delete it in the dashboard")
            },
        );
        Ok(())
    }

    /// `GET /webhooks/{id}` and the documented `PUT /webhooks/{id}` for one
    /// endpoint this probe created, then the list: does an update store the
    /// URL that the create dropped?
    async fn webhook_update_phase(&mut self, id: &str, url: &str) -> Result<()> {
        if !id.starts_with("ep_test.")
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_'))
        {
            return Err("--reference must be a sandbox endpoint id, `ep_test.…`".into());
        }
        self.exchange(
            "7: read the endpoint by id",
            Method::GET,
            &format!("/webhooks/{id}"),
            None,
            Decode::Webhook,
        )
        .await?;
        let mut events: Vec<&str> = PAYMENT_EVENTS.to_vec();
        events.push("payment.processing");
        let body = json!({
            "url": url,
            "events": events,
            "description": "vpay sandbox probe (RFC-0012 item 7) - delete me",
            "active": true,
        });
        self.exchange(
            "7: PUT the URL again",
            Method::PUT,
            &format!("/webhooks/{id}"),
            Some(body),
            Decode::Webhook,
        )
        .await?;
        self.exchange(
            "7: list after the update",
            Method::GET,
            "/webhooks?page=1",
            None,
            Decode::WebhookPage,
        )
        .await?;
        Ok(())
    }

    async fn webhooks_phase(&mut self, url: &str) -> Result<()> {
        let mut created = Vec::new();
        let attempts: [(&str, Vec<&str>); 3] = [
            ("13, 15, 16: create", PAYMENT_EVENTS.to_vec()),
            ("14: create again at the same URL", PAYMENT_EVENTS.to_vec()),
            (
                "17: create with payment.processing",
                vec!["payment.processing"],
            ),
        ];
        self.exchange(
            "16: list before",
            Method::GET,
            "/webhooks?page=1",
            None,
            Decode::WebhookPage,
        )
        .await?;
        for (item, events) in &attempts {
            let body = CreateWebhook {
                url,
                description: Some("vpay sandbox probe (RFC-0012 step 1) - delete me"),
                events,
            };
            let (status, answer) = self
                .exchange(
                    item,
                    Method::POST,
                    "/webhooks",
                    Some(serde_json::to_value(&body)?),
                    Decode::Webhook,
                )
                .await?;
            let id = answer
                .pointer("/endpoint/id")
                .or_else(|| answer.pointer("/webhook/id"))
                .or_else(|| answer.pointer("/data/id"))
                .and_then(Value::as_str);
            match id {
                Some(id) if (200..300).contains(&status) => created.push(id.to_owned()),
                _ if (200..300).contains(&status) => {
                    self.note(item, "created, but no id found in the answer: find and delete it in the dashboard");
                }
                _ => {}
            }
        }
        self.exchange(
            "16: list after",
            Method::GET,
            "/webhooks?page=1",
            None,
            Decode::WebhookPage,
        )
        .await?;
        let mut left = Vec::new();
        for id in &created {
            let (status, _) = self
                .exchange(
                    "cleanup",
                    Method::DELETE,
                    &format!("/webhooks/{id}"),
                    None,
                    Decode::Nothing,
                )
                .await?;
            if !(200..300).contains(&status) {
                left.push(id.clone());
            }
        }
        self.note(
            "cleanup",
            if left.is_empty() {
                format!("{} endpoint(s) created, all deleted", created.len())
            } else {
                format!("NOT deleted, delete by hand in the dashboard: {left:?}")
            },
        );
        Ok(())
    }

    fn write(&self, out: &PathBuf, phase: &str, started: u64, error: Option<String>) -> Result<()> {
        let mut report: Value = fs::read(out)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| json!({ "tool": "vpay-notchpay sandbox_probe", "runs": [] }));
        if let Some(runs) = report.get_mut("runs").and_then(Value::as_array_mut) {
            runs.push(json!({
                "phase": phase,
                "started_unix": started,
                "base_url": self.base_url,
                "channel": CHANNEL,
                "error": error.map(|e| self.redact_str(&e)),
                "exchanges": self.report,
            }));
        }
        fs::write(out, serde_json::to_vec_pretty(&report)?)?;
        say(&format!(
            "report written to {} (redacted; review it before you paste it)",
            out.display()
        ));
        Ok(())
    }

    fn redact(&self, value: Value, field: Option<&str>) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| {
                        let v = self.redact(v, Some(&k));
                        (k, v)
                    })
                    .collect(),
            ),
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|v| self.redact(v, field)).collect())
            }
            Value::String(s) if field.is_some_and(secret_named) && !s.is_empty() => {
                Value::String(shape(&s))
            }
            Value::String(s) => Value::String(self.redact_str(&s)),
            other => other,
        }
    }

    fn redact_str(&self, s: &str) -> String {
        let mut s = s.replace(&self.public_key, &format!("<{PUBLIC_KEY}>"));
        s = s.replace(&self.private_key, &format!("<{PRIVATE_KEY}>"));
        if let Ok(hash) = std::env::var(HASH_KEY)
            && !hash.is_empty()
        {
            s = s.replace(&hash, &format!("<{HASH_KEY}>"));
        }
        s
    }
}

/// Which vpay wire type an answer must decode as.
#[derive(Clone, Copy)]
enum Decode {
    Payment,
    Webhook,
    WebhookPage,
    Nothing,
}

impl Decode {
    fn check(self, bytes: &[u8]) -> Value {
        let ok = match self {
            Self::Payment => serde_json::from_slice::<PaymentEnvelope>(bytes).is_ok(),
            Self::Webhook => serde_json::from_slice::<WebhookEnvelope>(bytes).is_ok(),
            Self::WebhookPage => serde_json::from_slice::<WebhookPage>(bytes).is_ok(),
            Self::Nothing => return Value::Null,
        };
        Value::Bool(ok)
    }
}

fn sensitive(value: &str) -> Result<HeaderValue> {
    let mut header = HeaderValue::from_str(value)?;
    header.set_sensitive(true);
    Ok(header)
}

fn secret_named(field: &str) -> bool {
    let f = field.to_ascii_lowercase();
    ["secret", "hash", "key", "token", "signature", "password"]
        .iter()
        .any(|word| f.contains(word))
}

/// `hsk_test_…`, `sk_test.…`: the letters, separator and mode word, and
/// nothing after them.
fn shape(value: &str) -> String {
    for mode in ["test", "live"] {
        if let Some(at) = value.find(mode)
            && at <= 8
            && let Some(prefix) = value.get(..(at + mode.len() + 1).min(value.len()))
        {
            return format!("<redacted: present, shape {prefix}…>");
        }
    }
    "<redacted: present>".to_owned()
}

/// Fields whose name says which mode an answer came from.
fn mode_hints(body: &Value) -> Vec<String> {
    let mut found = Vec::new();
    walk(body, &mut |key, value| {
        let k = key.to_ascii_lowercase();
        if ["mode", "livemode", "sandbox", "environment", "env", "test"]
            .iter()
            .any(|w| k == *w || k.ends_with(&format!("_{w}")))
        {
            found.push(format!("{key}={value}"));
        }
    });
    found
}

fn walk(value: &Value, f: &mut impl FnMut(&str, &Value)) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                f(k, v);
                walk(v, f);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| walk(v, f)),
        _ => {}
    }
}

/// Notch Pay's own `trx.…` reference in a payment answer.
fn trx_reference(body: &Value) -> Option<String> {
    body.pointer("/transaction/reference")
        .or_else(|| body.pointer("/data/reference"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// A payment answer's status.
fn payment_status(body: &Value) -> Option<String> {
    body.pointer("/transaction/status")
        .or_else(|| body.pointer("/data/status"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The refund ids in a `POST /refunds` answer, under whichever envelope it
/// uses. The guide does not show one. Only ids that are safe to put in a path
/// segment are kept: they go into `GET /refunds/{id}`.
fn refund_ids(body: &Value) -> Vec<String> {
    ["/refund/id", "/data/id", "/transaction/id", "/id"]
        .iter()
        .filter_map(|pointer| body.pointer(pointer).and_then(Value::as_str))
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
        .map(str::to_owned)
        .collect()
}

fn summary(entry: &Value) -> String {
    let item = entry
        .get("item")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(note) = entry.get("note").and_then(Value::as_str) {
        return format!("[{item}] {note}");
    }
    if let Some(outcome) = entry.get("outcome") {
        return format!("[{item}] via Client: {outcome}");
    }
    format!(
        "[{item}] {} {} -> {} fields {} decoded_by_vpay {}",
        entry
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        entry.get("status").unwrap_or(&Value::Null),
        entry.get("top_level_fields").unwrap_or(&Value::Null),
        entry.get("decoded_by_vpay").unwrap_or(&Value::Null),
    )
}

/// RFC-0012 § 10 item 7: is `x-notch-signature` an HMAC-SHA256 of the raw
/// body, or of a re-serialisation? Prints which matched, never the key.
fn verify_signature(body: &PathBuf, signature: &str) -> Result<()> {
    let key = std::env::var(HASH_KEY).map_err(|_| format!("{HASH_KEY} is not set"))?;
    let raw = fs::read(body)?;
    serde_json::from_slice::<Value>(&raw)?;
    let reserialised = minify(&raw);
    let expected = signature.trim().to_ascii_lowercase();
    for (label, bytes) in [
        ("raw body", &raw),
        (
            "whitespace-free re-serialisation (JSON.stringify)",
            &reserialised,
        ),
    ] {
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes())?;
        mac.update(bytes);
        let matches = hex::encode(mac.finalize().into_bytes()) == expected;
        say(&format!(
            "HMAC-SHA256 over the {label}: {}",
            if matches { "MATCHES" } else { "does not match" }
        ));
    }
    if signature.trim() != expected {
        say("note: the signature was not all lower-case hex");
    }
    Ok(())
}

/// The body with every whitespace byte outside a string removed, keys in
/// their original order and escapes kept. **Not** what Node's
/// `JSON.stringify(req.body)` gives for every body (corrected 2026-10-24):
/// Notch Pay escapes `/` as `\/`, and `JSON.stringify` of the parsed body
/// writes a bare `/`. On all four webhooks the sandbox delivered that day the
/// raw body's HMAC matched and a `JSON.stringify` re-serialisation's did not.
/// For a body Notch Pay sends already compact, this equals the raw body.
fn minify(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let (mut in_string, mut escaped) = (false, false);
    for &byte in raw {
        if in_string {
            out.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
            out.push(byte);
        } else if !byte.is_ascii_whitespace() {
            out.push(byte);
        }
    }
    out
}

/// One line to stdout. `print_stdout` is denied workspace-wide; this is the
/// probe's whole interface, so it writes through `io::Write` and ignores a
/// closed pipe rather than panicking.
fn say(line: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stdout(), "{line}");
}

/// Sleeps until `at` seconds after `start`; returns at once if that is past.
async fn sleep_until(start: std::time::Instant, at: u64) {
    let target = start + Duration::from_secs(at);
    let now = std::time::Instant::now();
    if target > now {
        tokio::time::sleep(target - now).await;
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_ca_bundle_is_off_unless_the_flag_is_given() {
        let none = ca_bundle(&args(&["read", "--out", "r.json"])).unwrap();
        assert_eq!(none, None);
        let some = ca_bundle(&args(&["read", "--ca-bundle", "/x.pem", "--out", "r.json"])).unwrap();
        assert_eq!(some, Some(PathBuf::from("/x.pem")));
    }

    #[test]
    fn a_ca_bundle_flag_without_a_path_is_refused() {
        assert!(ca_bundle(&args(&["read", "--out", "r.json", "--ca-bundle"])).is_err());
        assert!(ca_bundle(&args(&["read", "--ca-bundle", "--out", "r.json"])).is_err());
    }

    #[test]
    fn a_missing_bundle_fails_before_any_request() {
        let error = http_client(Some(Path::new("/nonexistent/vpay-probe/ca.pem")))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--ca-bundle"), "{error}");
    }

    #[test]
    fn a_bundle_with_no_certificate_is_refused() {
        let dir = std::env::temp_dir().join(format!("vpay-probe-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty.pem");
        fs::write(&empty, "not a certificate\n").unwrap();
        let error = http_client(Some(&empty)).unwrap_err().to_string();
        fs::remove_dir_all(&dir).unwrap();
        assert!(error.contains("no PEM certificate"), "{error}");
    }

    #[test]
    fn refund_ids_are_found_under_any_envelope_and_only_if_path_safe() {
        assert_eq!(
            refund_ids(&json!({ "refund": { "id": "rf_123" } })),
            ["rf_123"]
        );
        assert_eq!(
            refund_ids(&json!({ "data": { "id": "rf.test_9" } })),
            ["rf.test_9"]
        );
        assert_eq!(refund_ids(&json!({ "id": "rf-1" })), ["rf-1"]);
        assert!(refund_ids(&json!({ "refund": { "id": "../payments/x" } })).is_empty());
        assert!(refund_ids(&json!({ "refund": { "id": "" } })).is_empty());
        assert!(refund_ids(&json!({ "message": "Not found" })).is_empty());
    }

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/fixtures/isrg-root-x1.pem")
    }

    /// The client built with a bundle still refuses redirects: a 307 to a
    /// second listener is returned as-is, and the second listener is never
    /// reached, so `X-Grant` is not replayed.
    #[tokio::test]
    async fn a_client_with_a_bundle_does_not_follow_redirects() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio::net::TcpListener;

        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target.local_addr().unwrap();
        let redirector = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirector_addr = redirector.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = redirector.accept().await.unwrap();
            let mut buf = [0_u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let answer = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{target_addr}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            socket.write_all(answer.as_bytes()).await.unwrap();
        });

        let client = http_client(Some(&fixture())).unwrap();
        let response = client
            .get(format!("http://{redirector_addr}/payments/x"))
            .header("X-Grant", "probe-grant-not-a-key")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 307);
        let reached = tokio::time::timeout(Duration::from_millis(300), target.accept()).await;
        assert!(reached.is_err(), "the redirect was followed");
    }
}
