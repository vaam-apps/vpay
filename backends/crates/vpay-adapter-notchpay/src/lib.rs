//! The `notchpay` rail: Notch Pay's direct mobile-money charge, behind the
//! provider port, on the [`vpay_notchpay`] client.
//!
//! A push rail, as `mtn_momo` is: the payer is prompted on their own handset
//! and the authenticated read is what moves money. Unlike `mtn_momo`, the
//! payment is addressed on the rail by **Notch Pay's** reference (`trx.…`),
//! returned by the initialize call and carried in `ref_extra` under
//! [`NOTCHPAY_REFERENCE`]: Notch Pay's sandbox answered `404` to every
//! process and read by our own reference (RFC-0012 § 10 item 2,
//! `docs/status/verification/2026-10-21-notchpay-sandbox-run.md`). Notch Pay is an aggregator, so
//! one account collects MTN MoMo and Orange Money in Cameroon: a charge routed
//! by the payer's method type is processed on `cm.mtn` or `cm.orange`, and one
//! confirmed by the rail code on the configured `cm.mobile` ([RFC-0012]).
//!
//! # Admitted under ADR-0031, and what that costs a merchant
//!
//! Notch Pay's read is documented as authenticated by the key Notch Pay calls
//! **public**, which ADR-0029 D1's third rule refused as written.
//! [ADR-0031] amended that rule on 2026-10-20, for every provider, on six
//! conditions. This crate meets the ones that are code: the private key is
//! sent on every call (the client does it), boot refuses a configuration
//! without it (`vpay_config`), [`Adapter::accepted_exposure`] declares what
//! the public key lets anyone else do, and a success is accepted only for the
//! charge's own amount and currency (`mapping`). The exposure itself is not
//! something code can close. `docs/flows/adapter-notchpay.md` § "Production
//! risks" is the long form.
//!
//! # What is not proven
//!
//! **Notch Pay's sandbox has been called through this crate, and its production
//! never has.** RFC-0012 step 5 took three test-mode payments through it on
//! 2026-10-21 (`docs/status/verification/2026-10-21-notchpay-step5.md`).
//! Its calls were transcribed from Notch Pay's documentation and corrected
//! from what Notch Pay's sandbox answered `vpay-notchpay`'s probe on
//! 2026-10-21 (step 1). Every wire assertion in this crate's tests still runs
//! against a WireMock stub, which proves the adapter against that stub and
//! nothing about Notch Pay. The `mapping` module's header names what was
//! confirmed and what is open.
//!
//! **Two calls, with a commit between them ([ADR-0032]).** Notch Pay's
//! reference exists only in the initialize answer, and Notch Pay finds a
//! payment by its own reference alone (`GET /payments?search=` does not
//! match ours). So this rail declares [`SubmitSteps::PrepareThenSubmit`]:
//! [`Adapter::prepare`] is initialize, which creates the payment and prompts
//! nobody, and returns the `trx.…`; the core commits it on the charge; only
//! then is [`Adapter::submit`] called, and `submit` is process, the prompt.
//! A crash between the two leaves a charge whose payer was provably never
//! asked, and the retry's `409` fails it cleanly
//! ([`PriorSubmits::None`]) so the payer can try again. The window this
//! closed was a crash after the prompt and before vpay held the `trx.…`,
//! which left a prompted payment vpay could never read.
//!
//! A charge submitted by the one-call build before ADR-0032, still in flight
//! across the deploy, may have been prompted, and the core says so with
//! [`PriorSubmits::Some`]: its `409` stays `NeedsReconciliation`, and the
//! worker makes it `unresolved`, alerting, on the first answer that says so
//! (vaam-apps/vpay#313). An operator reconciles it in Notch Pay's dashboard
//! (the maintainer's decision, 2026-10-21).
//!
//! The orphan a crash leaves behind — a payment initialized on Notch Pay and
//! never processed — expires there. Anyone holding the merchant's public key
//! could still process it to a number of their choosing; that is the
//! exposure ADR-0031 accepted, and not one this step adds.
//!
//! Refunds are a declared [`ProviderError::NotImplemented`] token
//! (`notchpay::refund`): Notch Pay documents `POST /refunds` in a guide only,
//! with no reference that would make a replay safe (RFC-0012 § 7).
//!
//! [RFC-0012]: ../../../../docs/rfc/0012-notchpay-adapter.md
//! [ADR-0031]: ../../../../docs/adr/0031-a-status-read-authenticated-by-a-server-side-key.md
//! [ADR-0032]: ../../../../docs/adr/0032-a-rail-issued-reference-is-committed-before-the-prompt.md

mod callbacks;
mod mapping;

pub use crate::mapping::PRODUCED_FAILURE_CODES;

use async_trait::async_trait;
use uuid::Uuid;
use vpay_core::routing::{MethodType, MethodTypes};
use vpay_core::{FailureCode, Money, ProviderFlow};
use vpay_notchpay::status::PaymentStatus;
use vpay_notchpay::wire::{
    InitializePayment, PAYMENT_EVENTS, Payment, ProcessData, ProcessPayment,
};
use vpay_notchpay::{Answer, Client, Keys};
use vpay_provider::{
    AcceptedExposure, CallbackRef, CallbackRegistrationDefault, CallbackUrlMode, Capabilities,
    ChargeRef, ChargeStatus, EndpointCheck, EndpointReport, PayerField, PayerFieldKind,
    PhonePayerType, PriorSubmits, ProviderAdapter, ProviderConfig, ProviderError, RefExtra,
    RefundDestination, RefundTarget, Refunded, SubmitSteps, Submitted,
};

use crate::mapping::{Initialized, Processed};

/// The rail code, equal to the `payment_method_types` value.
const CODE: &str = "notchpay";

/// The `credentials` key holding the key Notch Pay calls public, sent as
/// `Authorization`.
const PUBLIC_KEY: &str = "public_key";
/// The `credentials` key holding the private key, sent as `X-Grant` on every
/// call (ADR-0031 condition 1).
const PRIVATE_KEY: &str = "private_key";
/// The `settings` key naming the channel the process call sends.
const CHANNEL: &str = "channel";
/// The optional `settings` key for the one free-text field Notch Pay receives.
const DESCRIPTION: &str = "description";

/// The `ref_extra` key holding Notch Pay's own reference for the payment
/// (`trx.…`), the only one its process and read paths answer for.
pub const NOTCHPAY_REFERENCE: &str = "notchpay_reference";

/// The channels `settings.channel` may name. `cm.mobile` only: Notch Pay's
/// channel table lists it as "MTN / Orange, Cameroon, XAF", so the network is
/// Notch Pay's to resolve and this crate carries no number-plan table.
/// Configuring `cm.mtn` or `cm.orange` for a whole deployment would fail
/// every payer on the other network, so neither is a setting: they are sent
/// per charge, from the payer's chosen method type ([`channel_for`]).
///
/// `vpay_config`'s boot table refuses any other value, so this check is the
/// adapter's own belt for a `ProviderConfig` built by something else.
pub const KNOWN_CHANNELS: [&str; 1] = ["cm.mobile"];

/// The channel for a charge routed by the payer's method type (ADR-0029 D6;
/// RFC-0012 delivery step 8, built 2026-10-23): Notch Pay's channel table
/// names `cm.mtn` for MTN Mobile Money and `cm.orange` for Orange Money in
/// Cameroon. **Neither has been sent to Notch Pay's sandbox**; `cm.mobile`
/// has (2026-10-21).
#[must_use]
pub const fn method_type_channel(method_type: MethodType) -> &'static str {
    match method_type {
        MethodType::MtnWallet => "cm.mtn",
        MethodType::OrangeWallet => "cm.orange",
    }
}

/// The only currency this adapter charges in. Notch Pay's XOF channels are
/// other countries, other channels and another RFC.
const CURRENCY: &str = "XAF";

/// One required phone field, declared exactly as `mtn_momo` declares its own:
/// the payer's mobile-money number, read at confirm from
/// `payment_method_data[notchpay][msisdn]`, validated as a Cameroonian mobile
/// number and normalised to digits-only E.164 by the core.
const PAYER_FIELDS: &[PayerField] = &[PayerField {
    name: "msisdn",
    kind: PayerFieldKind::Phone {
        region: "CM",
        phone_type: PhonePayerType::Mobile,
    },
    required: true,
    label_key: "msisdn.label",
}];

/// What the public key lets anyone who holds it do on the merchant's account,
/// from Notch Pay's specification (RFC-0012 § "What rule 3 protects").
/// ADR-0031 condition 4: declared here, logged at every boot.
const EXPOSURE: AcceptedExposure = AcceptedExposure {
    credential: PUBLIC_KEY,
    operations: &[
        "read the status, amount and customer of any payment on the account",
        "initialize a payment on the account",
        "process a pending payment to any phone number (a USSD prompt in the merchant's name)",
        "cancel any pending payment",
    ],
    document: "docs/flows/adapter-notchpay.md",
};

/// The adapter, owning the outbound HTTP client it makes rail calls with.
///
/// It holds no key: a [`vpay_notchpay::Client`] is built per call from the
/// [`ProviderConfig`] the core hands in, so a merchant's keys live exactly as
/// long as the call that uses them, and the type has nothing to print.
#[derive(Debug)]
pub struct Adapter {
    /// Built once per process by the binary, refusing redirects
    /// (`vpay_provider::http`), and cloned in. A followed redirect would
    /// replay `X-Grant` to whatever host answered.
    http: reqwest::Client,
}

impl Adapter {
    /// Takes the process's one client rather than building its own.
    ///
    /// ```
    /// use vpay_core::ProviderFlow;
    /// use vpay_provider::ProviderAdapter as _;
    ///
    /// let http = vpay_provider::http::client().expect("the vendored-roots client builds");
    /// let adapter = vpay_adapter_notchpay::Adapter::new(http);
    /// assert_eq!(adapter.code(), "notchpay");
    /// assert_eq!(adapter.capabilities().flow, ProviderFlow::Push);
    /// // Admitted under ADR-0031: the exposure is declared, so boot logs it.
    /// assert_eq!(
    ///     adapter.accepted_exposure().map(|exposure| exposure.credential),
    ///     Some("public_key")
    /// );
    /// ```
    #[must_use]
    pub const fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// A client for the account `config` names, with its deadline.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Config`] naming the missing key — never its value —
    /// or a base URL no call could use.
    fn client(&self, config: &ProviderConfig) -> Result<Client, ProviderError> {
        let keys = Keys::new(
            credential(config, PUBLIC_KEY)?,
            credential(config, PRIVATE_KEY)?,
        )?;
        Ok(Client::new(self.http.clone(), &config.base_url, keys)?
            .with_request_timeout(config.request_timeout))
    }

    /// Call 2, `POST /payments/{notchpay_reference}`: the USSD prompt.
    async fn process(
        &self,
        client: &Client,
        charge: &ChargeRef,
        config: &ProviderConfig,
        phone: &str,
        notchpay_reference: &str,
    ) -> Result<Submitted, ProviderError> {
        let body = ProcessPayment {
            channel: channel_for(charge, config)?,
            data: ProcessData { phone },
        };
        match client.process_payment(notchpay_reference, &body).await? {
            Answer::Success(_) => Ok(submitted(notchpay_reference)),
            Answer::Refused(refusal) => match mapping::process_refusal(&refusal) {
                // Not a decline (RFC-0012 § 3). The payment left `pending`
                // under someone's process call — ours, before a crash — so it
                // is submitted, unless the read says it never moved.
                Processed::AlreadyProcessed => {
                    match self.read(client, charge, notchpay_reference).await? {
                        Some(PaymentStatus::Pending) => Err(ProviderError::malformed(
                            "notchpay: a process call was refused as already processed, and the \
                             payment still reads pending",
                        )),
                        _ => Ok(submitted(notchpay_reference)),
                    }
                }
                Processed::Refused(error) => Err(error),
            },
        }
    }

    /// The read, reduced to the payment's status, for the submit paths that
    /// have to decide whether to send the prompt. `None` is "no such
    /// payment".
    async fn read(
        &self,
        client: &Client,
        charge: &ChargeRef,
        notchpay_reference: &str,
    ) -> Result<Option<PaymentStatus>, ProviderError> {
        match client.retrieve_payment(notchpay_reference).await? {
            Answer::Success(envelope) => {
                require_our_payment(&envelope.transaction, charge)?;
                known_status(&envelope.transaction).map(Some)
            }
            Answer::Refused(refusal) => mapping::read_refusal(&refusal).map(|()| None),
        }
    }
}

/// A push rail's `Submitted`: no redirect, and Notch Pay's reference as the
/// key material, because it is the only address Notch Pay answers for.
fn submitted(notchpay_reference: &str) -> Submitted {
    Submitted {
        ref_extra: notchpay_ref_extra(notchpay_reference),
        redirect_url: None,
    }
}

/// The key material this rail is addressed by: Notch Pay's reference alone.
/// What [`Adapter::prepare`] returns and every [`Submitted`] carries.
fn notchpay_ref_extra(notchpay_reference: &str) -> RefExtra {
    RefExtra::from([(NOTCHPAY_REFERENCE.to_owned(), notchpay_reference.to_owned())])
}

/// The charge's Notch Pay reference from `ref_extra`, if it holds a
/// non-empty one.
fn held_reference(charge: &ChargeRef) -> Option<&str> {
    charge
        .ref_extra
        .get(NOTCHPAY_REFERENCE)
        .map(String::as_str)
        .filter(|reference| !reference.is_empty())
}

/// Notch Pay's reference out of the initialize answer.
///
/// # Errors
///
/// [`ProviderError::Malformed`] if the answer carries none, or carries ours
/// back in its place: without Notch Pay's own reference the payment cannot
/// be processed or read, so nothing is sent and the charge's fate is left
/// unknown.
fn notchpay_reference_of(payment: &Payment, charge: &ChargeRef) -> Result<String, ProviderError> {
    require_our_payment(payment, charge)?;
    payment
        .reference
        .as_deref()
        .filter(|reference| !reference.is_empty() && *reference != charge.reference_id.to_string())
        .map(str::to_owned)
        .ok_or_else(|| {
            ProviderError::malformed(
                "notchpay: the initialize answer carries no Notch Pay reference to address the \
                 payment by; nothing was sent to the payer",
            )
        })
}

/// Refuses an answer about another payment: its `trxref` must be our
/// reference. Notch Pay's sandbox put ours there on every payment answer on
/// 2026-10-21. A read addressed by a `trx.…` that came back naming another
/// merchant reference must not settle this charge.
fn require_our_payment(payment: &Payment, charge: &ChargeRef) -> Result<(), ProviderError> {
    if payment.trxref.as_deref() == Some(charge.reference_id.to_string().as_str()) {
        return Ok(());
    }
    Err(ProviderError::malformed(
        "notchpay: the payment answer does not name this charge's reference as its trxref",
    ))
}

/// The charge's Notch Pay reference from `ref_extra`.
///
/// # Errors
///
/// [`ProviderError::NeedsReconciliation`] if there is none: that is key
/// material vpay lost for a payment Notch Pay may well have settled, not a
/// payment Notch Pay never heard of, so it is never
/// [`ChargeStatus::NotFound`] (`docs/flows/crash-safety.md`; Orange's
/// missing `pay_token` is the same rule). Raised before any request.
///
/// It was [`ProviderError::Config`] until 2026-10-23, which dead-letters the
/// poll job: the alert fired once and the polling stopped, with the charge
/// left `submitting` or `pending` where the `unresolved` query of
/// `docs/runbooks/unresolved-charges.md` never finds it. Now the worker
/// makes it `unresolved` and keeps the hourly poll (vaam-apps/vpay#313).
fn held_notchpay_reference(charge: &ChargeRef) -> Result<&str, ProviderError> {
    held_reference(charge).ok_or_else(|| {
        ProviderError::needs_reconciliation(format!(
            "notchpay: the charge holds no `{NOTCHPAY_REFERENCE}`, so Notch Pay cannot be asked \
             about it; reconcile it in Notch Pay's dashboard"
        ))
    })
}

/// A payment status this adapter knows, from a successful answer.
fn known_status(payment: &Payment) -> Result<PaymentStatus, ProviderError> {
    payment
        .status
        .as_deref()
        .and_then(PaymentStatus::parse)
        .ok_or_else(|| {
            ProviderError::malformed(
                "notchpay: a payment answer carries no status Notch Pay documents",
            )
        })
}

/// A `credentials` value, by name, or a `Config` error naming it.
fn credential<'a>(config: &'a ProviderConfig, key: &str) -> Result<&'a str, ProviderError> {
    config
        .credentials
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ProviderError::Config(format!("notchpay: credentials.{key} is missing")))
}

/// The channel this charge is processed on: its method type's, when the
/// payer chose one, else the configured one.
///
/// The configured channel is checked either way, so a bad setting is
/// refused for every charge rather than only for the ones that read it.
fn channel_for<'a>(
    charge: &ChargeRef,
    config: &'a ProviderConfig,
) -> Result<&'a str, ProviderError> {
    let configured = channel(config)?;
    Ok(match charge.method_type {
        Some(method_type) => method_type_channel(method_type),
        None => configured,
    })
}

/// The configured channel, refused unless it is one this adapter knows.
fn channel(config: &ProviderConfig) -> Result<&str, ProviderError> {
    let value = config
        .settings
        .get(CHANNEL)
        .map(String::as_str)
        .ok_or_else(|| ProviderError::Config("notchpay: settings.channel is missing".to_owned()))?;
    KNOWN_CHANNELS
        .iter()
        .find(|known| **known == value)
        .copied()
        .ok_or_else(|| {
            ProviderError::Config(format!(
                "notchpay: settings.channel must be one of {}",
                KNOWN_CHANNELS.join(", ")
            ))
        })
}

/// The payer's number as Notch Pay's examples write it, `+` and digits.
///
/// The core hands a push rail the digits-only E.164 its confirm path
/// normalised (`vpay_api::v1::payer_fields`); Notch Pay's specification and
/// its testing page write numbers with the `+`. Whether it accepts both is
/// not documented, and the documented form is the one sent.
fn payer_phone(charge: &ChargeRef) -> Result<String, ProviderError> {
    let digits = charge
        .payer_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProviderError::Config("notchpay: payer_ref required on a push rail".to_owned())
        })?;
    Ok(if digits.starts_with('+') {
        digits.to_owned()
    } else {
        format!("+{digits}")
    })
}

#[async_trait]
impl ProviderAdapter for Adapter {
    fn code(&self) -> &'static str {
        CODE
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // The direct charge sends a USSD prompt; Notch Pay's hosted page,
            // Collect, is the alternative RFC-0012 considered and did not
            // choose.
            flow: ProviderFlow::Push,
            // A claim about the rail: Notch Pay's refunds guide documents
            // full and partial refunds. `refund` below is a token, which is
            // vpay's gap and not Notch Pay's (RFC-0012 § 7).
            supports_refunds: true,
            supports_partial_refunds: true,
            // Notch Pay posts webhooks to an endpoint registered with the
            // account. vpay registers it itself since RFC-0013 steps 2–4
            // (`callback_url_mode` below); by hand before that.
            delivers_callbacks: true,
            // Notch Pay documents an IP whitelist "for API transfers", which
            // this adapter never calls. Whether refunds need it is RFC-0012
            // § 10 item 12.
            requires_ip_allowlist: false,
            supports_account_holder_lookup: false,
            // "A full refund returns the entire payment amount to the
            // customer", with no payee in the request: the second `Origin`
            // rail, and it takes the port's `parse_destination` default.
            refund_destination: RefundDestination::Origin,
            // The initialize call carries no callback URL; Notch Pay posts to
            // the endpoints registered with the account (`/webhooks`), so the
            // `ensure_callbacks` pass keeps one of vpay's own there
            // (`ensure_callback_endpoint` below, RFC-0013 § 5).
            callback_url_mode: CallbackUrlMode::Registered,
            submit_steps: SubmitSteps::PrepareThenSubmit,
            // An aggregator: both networks through one account, the rail
            // that made a method type and a rail two things (ADR-0029 D6).
            // `submit` sends the chosen network's channel (`channel_for`).
            method_types: MethodTypes::of(&[MethodType::MtnWallet, MethodType::OrangeWallet]),
        }
    }

    fn payer_fields(&self) -> &'static [PayerField] {
        PAYER_FIELDS
    }

    fn accepted_exposure(&self) -> Option<AcceptedExposure> {
        Some(EXPOSURE)
    }

    /// `Off`: with `providers[].callbacks` unset, vpay asks the account
    /// nothing, and the operator registers vpay's callback URL in Notch Pay's
    /// dashboard (or sets `register: ensure`). Since 2026-10-23, when the
    /// maintainer chose "Default to `off` for Notch Pay".
    ///
    /// The sandbox measured that day
    /// (`docs/status/verification/2026-10-23-notchpay-sandbox-two-step-and-channels.md`):
    /// an endpoint created through `POST /webhooks` with a public URL is
    /// stored with `"url": null`, delivered nothing in 21 minutes, and
    /// cannot be read, updated or deleted by its id. Four payments processed
    /// after one was created stayed `processing` for 3 to 55 minutes, where
    /// eleven before it settled in about 5 s — a correlation, not a shown
    /// cause, and enough not to write to a merchant's account unasked.
    fn default_callback_registration(&self) -> CallbackRegistrationDefault {
        CallbackRegistrationDefault::Off
    }

    /// Call 1, `POST /payments` (RFC-0012 § 3): the payment is created on
    /// Notch Pay and **nobody is prompted**. Returns Notch Pay's `trx.…`
    /// reference, which the core commits on the charge before it calls
    /// [`submit`](Adapter::submit) (ADR-0032).
    ///
    /// | Call 1 answered | Then |
    /// | --- | --- |
    /// | `201` with Notch Pay's reference | that reference, under [`NOTCHPAY_REFERENCE`] |
    /// | `201` without it, or about another `trxref` | `Malformed`: nothing exists that vpay could address |
    /// | `409`, and `prior` is [`PriorSubmits::None`] | `Rejected`, `provider_unavailable`: the charge fails, and the payer can try again |
    /// | `409`, and `prior` is [`PriorSubmits::Some`] | `NeedsReconciliation`: an operator reconciles it |
    /// | anything else | `mapping::initialize_refusal` |
    ///
    /// The `409` is the crash-retry case: an initialize that reached Notch
    /// Pay and lost its answer. Notch Pay cannot find the payment by our
    /// reference (2026-10-21: `search` matches its own reference only), so
    /// what the `409` means turns on what was done with it since, which is
    /// the core's evidence and arrives as `prior`. With no `submit` ever
    /// recorded, nothing asked Notch Pay to prompt the payer — the core
    /// records a `submit` before every process call, and calls process only
    /// with the `trx.…` committed — so the payment is an orphan nobody was
    /// asked to pay, and it expires on Notch Pay's side. With one recorded,
    /// the charge was submitted by the one-call build before ADR-0032, whose
    /// process may have prompted: the fate is unknown, never a decline.
    ///
    /// A charge that already holds Notch Pay's reference is answered from it
    /// with no request. The core never calls this for one; a second
    /// initialize would only be a `409`.
    ///
    /// # Errors
    ///
    /// `Config` for a missing key, setting or payer, or a charge in another
    /// currency, before any request; `Rejected` for a refusal, our refused
    /// keys, or the `409` above with no submit behind it;
    /// `NeedsReconciliation` for the `409` with one; `Transport` and
    /// `Malformed` leave the charge's fate unknown, and in either case no
    /// payer was prompted.
    async fn prepare(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
        prior: PriorSubmits,
    ) -> Result<RefExtra, ProviderError> {
        if let Some(held) = held_reference(charge) {
            return Ok(notchpay_ref_extra(held));
        }
        if charge.amount.currency().code() != CURRENCY {
            return Err(ProviderError::Config(format!(
                "notchpay: this adapter charges in {CURRENCY} only"
            )));
        }
        // Read before any request, so a missing setting is a refusal that
        // created nothing rather than a payment initialized and never
        // prompted. `submit` reads it again, for the process body.
        channel(config)?;
        let client = self.client(config)?;
        let phone = payer_phone(charge)?;
        let reference = charge.reference_id.to_string();
        let body = InitializePayment {
            amount: charge.amount.minor(),
            currency: CURRENCY,
            phone: &phone,
            reference: &reference,
            description: config.settings.get(DESCRIPTION).map(String::as_str),
        };

        let answer = client.initialize_payment(&body).await?;
        tracing::debug!(
            rail = CODE,
            reference_id = %charge.reference_id,
            accepted = matches!(answer, Answer::Success(_)),
            "initialize answered"
        );
        match answer {
            Answer::Success(envelope) => {
                let notchpay_reference = notchpay_reference_of(&envelope.transaction, charge)?;
                // A status Notch Pay does not document is refused here, as it
                // was when this was half of `submit`: an answer vpay cannot
                // read is not one to prompt a payer on.
                known_status(&envelope.transaction)?;
                Ok(notchpay_ref_extra(&notchpay_reference))
            }
            Answer::Refused(refusal) => match mapping::initialize_refusal(&refusal) {
                Initialized::AlreadyExists => match prior {
                    PriorSubmits::None => Err(ProviderError::Rejected {
                        code: FailureCode::ProviderUnavailable,
                        message: "notchpay: initialize answered 409 for this reference, and no \
                                  submit was ever made for the charge: an earlier initialize \
                                  created the payment and lost its answer, and nobody asked \
                                  Notch Pay to prompt the payer for it (ADR-0032)"
                            .to_owned(),
                    }),
                    PriorSubmits::Some => Err(ProviderError::needs_reconciliation(
                        "notchpay: initialize answered 409 for this reference and the charge \
                         holds no Notch Pay reference; a submit made before ADR-0032 may have \
                         prompted the payer, and Notch Pay cannot find the payment by ours, so \
                         its fate is unknown and an operator must reconcile it",
                    )),
                },
                Initialized::Refused(error) => Err(error),
            },
        }
    }

    /// Call 2, `POST /payments/{notchpay_reference}` (RFC-0012 § 3): the
    /// USSD prompt, by the `trx.…` [`prepare`](Adapter::prepare) returned and
    /// the core committed first (ADR-0032).
    ///
    /// | The charge | Then |
    /// | --- | --- |
    /// | holds Notch Pay's reference | process by it; `Submitted` carries it |
    /// | holds none | `NeedsReconciliation`, before any request |
    ///
    /// Process is safe to repeat: a second one is refused as "already
    /// processed", and the read decides whether the first prompt went out
    /// (`process` below), so a resubmit after a lost answer never prompts
    /// twice.
    ///
    /// A charge with no Notch Pay reference never reaches here from the
    /// core, which calls `prepare` and commits its answer first; one that
    /// does has lost the only address Notch Pay answers for, and is a case
    /// for a person rather than for an initialize, which is what used to
    /// open the window ADR-0032 closed.
    ///
    /// # Errors
    ///
    /// `Config` for a missing key, setting or payer, or a charge in another
    /// currency; `NeedsReconciliation` for a charge with no Notch Pay
    /// reference; `Rejected` for a refusal; `Transport` and `Malformed`
    /// leave the charge's fate unknown, never a decline.
    async fn submit(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<Submitted, ProviderError> {
        if charge.amount.currency().code() != CURRENCY {
            return Err(ProviderError::Config(format!(
                "notchpay: this adapter charges in {CURRENCY} only"
            )));
        }
        channel(config)?;
        let client = self.client(config)?;
        let phone = payer_phone(charge)?;
        let notchpay_reference = held_reference(charge).ok_or_else(|| {
            ProviderError::needs_reconciliation(format!(
                "notchpay: asked to prompt for a charge that holds no `{NOTCHPAY_REFERENCE}`; the \
                 core commits prepare's answer before it submits (ADR-0032), so this charge has \
                 lost the only reference Notch Pay answers for, and an operator must reconcile it"
            ))
        })?;
        self.process(&client, charge, config, &phone, notchpay_reference)
            .await
    }

    /// Call 3, `GET /payments/{notchpay_reference}`: the authoritative read,
    /// and the only thing that moves money. The answer must name this
    /// charge's reference as its `trxref`. `mapping::charge_status` and
    /// `mapping::read_refusal` are the whole table.
    ///
    /// # Errors
    ///
    /// `Config` for a missing key, or a `404` that is not Notch Pay's
    /// "Payment not found"; `Rejected` only for our own refused keys;
    /// `Transport` and `Malformed` as for `submit`; `NeedsReconciliation` for
    /// a charge with no Notch Pay reference in `ref_extra`, and for a
    /// success for a sum that is not the charge's (`mapping`). A payment
    /// Notch Pay has no record of is [`ChargeStatus::NotFound`], not an
    /// error.
    async fn query_status(
        &self,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> Result<ChargeStatus, ProviderError> {
        let notchpay_reference = held_notchpay_reference(charge)?;
        let client = self.client(config)?;
        let answer = client.retrieve_payment(notchpay_reference).await?;
        tracing::debug!(
            rail = CODE,
            reference_id = %charge.reference_id,
            answered = matches!(answer, Answer::Success(_)),
            "payment read answered"
        );
        match answer {
            Answer::Success(envelope) => {
                require_our_payment(&envelope.transaction, charge)?;
                mapping::charge_status(&envelope.transaction, charge.amount)
            }
            Answer::Refused(refusal) => {
                mapping::read_refusal(&refusal).map(|()| ChargeStatus::NotFound)
            }
        }
    }

    /// Identifiers only, out of the event Notch Pay POSTs.
    ///
    /// # This request is not authenticated by anything vpay checks
    ///
    /// Notch Pay signs its webhooks (`x-notch-signature`, HMAC-SHA256), and
    /// the port hands this method the body and no headers, so the signature
    /// cannot be checked (RFC-0010 open question 4). And on this rail the
    /// public key may be in other hands (ADR-0031). So the body is a hint:
    /// the event's own status is not modelled by the client at all, and
    /// `ref_extra` is always empty, so nothing from an unauthenticated
    /// request can reach a charge.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Malformed`] for a body that is not an event, a `type`
    /// other than the five payment events in
    /// [`vpay_notchpay::wire::PAYMENT_EVENTS`], or a `data` whose `trxref` and
    /// `reference` are both missing or not ours (a UUID). The message never
    /// quotes the body.
    fn parse_callback(&self, body: &[u8]) -> Result<CallbackRef, ProviderError> {
        let event = vpay_notchpay::parse_event(body)?;
        let handled = event
            .event_type
            .as_deref()
            .is_some_and(|event_type| PAYMENT_EVENTS.contains(&event_type));
        if !handled {
            return Err(ProviderError::malformed(
                "notchpay: the webhook is not one of the five payment events vpay handles",
            ));
        }
        // `data.trxref` first: on every payment answer Notch Pay's sandbox
        // gave (2026-10-21), `trxref` was ours and `reference` its own
        // `trx.…`. No webhook was received to confirm the event's `data` has
        // the same shape, so `data.reference` stays as the fallback for the
        // specification's form.
        let reference_id = [
            event.data.trxref.as_deref(),
            event.data.reference.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find_map(|reference| Uuid::parse_str(reference).ok())
        .ok_or_else(|| {
            ProviderError::malformed(
                "notchpay: the webhook's data names no vpay reference (neither trxref nor \
                     reference is one)",
            )
        })?;
        Ok(CallbackRef {
            reference_id,
            ref_extra: RefExtra::new(),
        })
    }

    /// A declared gap, not a fact about Notch Pay.
    ///
    /// Notch Pay's refunds guide documents `POST /refunds`; the specification
    /// and the API reference do not, and the request carries no reference
    /// that would make a replay safe. A lost answer re-sent would be a second
    /// refund. RFC-0012 § 7 is the argument, and this becomes a call when
    /// Notch Pay confirms in writing a refund that is safe to replay
    /// (§ 10 item 6). `supports_refunds` stays `true`, because answering
    /// `Unsupported` would be a claim about Notch Pay.
    ///
    /// # Errors
    ///
    /// Always [`ProviderError::NotImplemented`], declared in `docs/status.md`.
    async fn refund(
        &self,
        _charge: &ChargeRef,
        _amount: Money,
        _destination: Option<&RefundTarget>,
        _config: &ProviderConfig,
    ) -> Result<Refunded, ProviderError> {
        Err(ProviderError::NotImplemented("notchpay::refund"))
    }

    /// RFC-0013 § 5: list every page of the account's webhook endpoints, and
    /// create one of vpay's own only if none carries this deployment's
    /// marker for this URL. Never modifies or deletes one; `callbacks`' module
    /// header has the table, and why the description carries a tag of the
    /// URL (Notch Pay answers `"url": null`).
    ///
    /// # Errors
    ///
    /// `Config` for a missing key, a wrong base URL, or a URL or event list
    /// Notch Pay refused; `Rejected` when it refuses our keys; `Transport`
    /// and `Malformed` as for every call. None of them touches a charge.
    async fn ensure_callback_endpoint(
        &self,
        check: &EndpointCheck<'_>,
        config: &ProviderConfig,
    ) -> Result<EndpointReport, ProviderError> {
        let client = self.client(config)?;
        callbacks::ensure(&client, check, &config.callback_url).await
    }
}

#[cfg(test)]
mod tests;
