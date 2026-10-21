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
//! cargo run -p vpay-notchpay --example sandbox_probe -- verify-signature --body <file> --signature <hex>
//! ```
//!
//! **`--ca-bundle <path>` (cloud only, off by default).** The three calling
//! phases take it: every PEM certificate at `<path>` is trusted *in addition
//! to* the vendored `webpki_roots`, for this probe's own HTTP client only.
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
//! then deletes every one it created.

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
/// The only channel the adapter sends (`vpay_adapter_notchpay::KNOWN_CHANNELS`).
const CHANNEL: &str = "cm.mobile";

/// Notch Pay's documented sandbox outcome numbers
/// (<https://developer.notchpay.co/get-started/testing>, read 2026-10-20).
/// The page lists them under `cm.mtn` and `cm.orange`; whether `cm.mobile`
/// honours them is RFC-0012 § 10 item 3, which is what sending them on
/// `cm.mobile` measures. Orange's failure suffixes are not documented.
const OUTCOMES: [(&str, &str); 6] = [
    ("+237670000000", "MTN: success"),
    ("+237670000001", "MTN: insufficient funds"),
    ("+237670000002", "MTN: failure (other reason)"),
    ("+237670000003", "MTN: timeout (no response)"),
    ("+237670000004", "MTN: canceled by user"),
    ("+237690000000", "Orange: success"),
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
    if !matches!(phase, "read" | "payments" | "webhooks" | "search") {
        return Err(
            "usage: sandbox_probe read|payments|webhooks|search --out <report.json> \
             [--webhook-url <url>] [--reference <ref> …] [--ca-bundle <pem>] | verify-signature --body <file> --signature <hex>"
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
/// their original order: what Node's `JSON.stringify(req.body)` gives for an
/// ordinary body, which is what Notch Pay's setup page signs. `serde_json`
/// would sort the keys instead.
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
