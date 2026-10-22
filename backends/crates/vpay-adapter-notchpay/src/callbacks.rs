//! RFC-0013 § 5's Notch Pay row: keep one webhook endpoint of vpay's own
//! registered with the account.
//!
//! # Recognised by its description, because Notch Pay does not echo the URL
//!
//! Notch Pay's sandbox answered `"url": null` for every endpoint, in the
//! create answers and in the list, although every create sent one
//! (2026-10-21, `docs/status/verification/2026-10-21-notchpay-sandbox-run.md`).
//! So vpay cannot find its endpoint by URL here, and RFC-0013 § 3's
//! "someone else's endpoint at vpay's URL" cannot be seen at all.
//!
//! The description is the only label Notch Pay keeps, and it carries both
//! things vpay needs: the marker (`vpay:{deployment}:{rail}:{account}`, who
//! created it) and a tag of the URL it was created for
//! (`url:{16 hex digits of SHA-256}`), so an endpoint left at a
//! `public_base_url` the deployment no longer uses is told apart from the
//! current one:
//!
//! ```text
//! vpay:prod:notchpay:default url:3f1c9a0b2d4e6f81
//! ```
//!
//! | The endpoint | Class | What this adapter does |
//! | --- | --- | --- |
//! | this marker, this URL's tag (or this URL, should Notch Pay ever echo it) | vpay's, current | nothing; reports [`EndpointState::Drifted`] if an event type is missing, because the client has no update call |
//! | this marker, another tag or URL | vpay's, superseded | nothing; its id is in [`EndpointReport::retirable`] |
//! | a second current one | vpay's, a duplicate | nothing; its id is in `retirable` |
//! | no marker, this URL | someone else's, at vpay's URL | nothing; [`EndpointState::Foreign`]. Only reachable if Notch Pay starts echoing URLs |
//! | anything else, another deployment's marker included | someone else's | nothing |
//!
//! Nothing is ever deleted or modified: Notch Pay's `DELETE /webhooks/{id}`
//! answered `404` for endpoints its own list had just returned (RFC-0012
//! § 10 item 20), and the client has no `PUT`.

use sha2::{Digest as _, Sha256};
use vpay_notchpay::wire::{CreateWebhook, PAYMENT_EVENTS, Webhook};
use vpay_notchpay::{Answer, Client, Refusal};
use vpay_provider::{EndpointCheck, EndpointReport, EndpointState, ProviderError};

use crate::mapping;

/// How many pages of endpoints a pass will read before it calls the answer
/// malformed. Notch Pay's default page is 30 endpoints, so this is 3 000 —
/// far past any account a deployment registers to, and a bound on a list
/// whose `last_page` keeps moving.
const MAX_PAGES: u32 = 100;

/// Hex digits of the URL's SHA-256 kept in the description: 64 bits, enough
/// that two URLs one deployment has used never share a tag.
const URL_TAG_HEX: usize = 16;

/// The description vpay gives an endpoint it creates for `url`.
pub(crate) fn description(marker: &str, url: &str) -> String {
    format!("{marker} {}", url_tag(url))
}

fn url_tag(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let hex = hex::encode(digest);
    format!("url:{}", &hex[..URL_TAG_HEX])
}

/// The pass's decision, before any write: report as found, or create.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Report(EndpointReport),
    Create { retirable: Vec<String> },
}

/// RFC-0013 § 4 steps 2–4 for one account: list every page, decide, and
/// create at most once.
pub(crate) async fn ensure(
    client: &Client,
    check: &EndpointCheck<'_>,
    url: &str,
) -> Result<EndpointReport, ProviderError> {
    let endpoints = list_every_page(client).await?;
    match decide(&endpoints, check, url)? {
        Decision::Report(report) => Ok(report),
        Decision::Create { retirable } => {
            let endpoint_id = create(client, check.marker, url).await?;
            tracing::info!(
                rail = crate::CODE,
                endpoint_id = %endpoint_id,
                "registered vpay's callback endpoint with the Notch Pay account"
            );
            Ok(EndpointReport {
                state: EndpointState::Created { endpoint_id },
                retirable,
            })
        }
    }
}

async fn list_every_page(client: &Client) -> Result<Vec<Webhook>, ProviderError> {
    let mut endpoints = Vec::new();
    for page in 1..=MAX_PAGES {
        match client.list_webhooks(page).await? {
            Answer::Success(listed) => {
                let empty = listed.endpoints.is_empty();
                endpoints.extend(listed.endpoints);
                let last = listed.last_page.unwrap_or(page);
                if empty || page >= last {
                    return Ok(endpoints);
                }
            }
            Answer::Refused(refusal) => {
                return Err(webhook_refusal(&refusal, "listing webhook endpoints"));
            }
        }
    }
    Err(ProviderError::malformed(format!(
        "notchpay: listing webhook endpoints did not reach its last page within {MAX_PAGES} \
         pages"
    )))
}

async fn create(client: &Client, marker: &str, url: &str) -> Result<String, ProviderError> {
    let description = description(marker, url);
    let body = CreateWebhook {
        url,
        description: Some(&description),
        events: &PAYMENT_EVENTS,
    };
    match client.create_webhook(&body).await? {
        Answer::Success(created) => created.endpoint.id.ok_or_else(|| {
            // Created, most likely; the next pass finds it by its description.
            ProviderError::malformed(
                "notchpay: creating a webhook endpoint answered success with no endpoint id",
            )
        }),
        Answer::Refused(refusal) => Err(webhook_refusal(&refusal, "creating a webhook endpoint")),
    }
}

/// § 3's table, as a pure function of the list.
///
/// # Errors
///
/// [`ProviderError::Malformed`] for a current endpoint of vpay's with no id,
/// which nothing could act on or report.
pub(crate) fn decide(
    endpoints: &[Webhook],
    check: &EndpointCheck<'_>,
    url: &str,
) -> Result<Decision, ProviderError> {
    let tag = url_tag(url);
    let mut current: Vec<&Webhook> = Vec::new();
    let mut retirable: Vec<String> = Vec::new();
    let mut foreign_at_ours: Option<&Webhook> = None;

    for endpoint in endpoints {
        let ours = endpoint.description.as_deref().and_then(|description| {
            let (identity, rest) = description.split_once(' ').unwrap_or((description, ""));
            (identity == check.marker).then_some(rest)
        });
        match ours {
            Some(rest) => {
                let at_our_url = match endpoint.url.as_deref() {
                    Some(echoed) => echoed == url,
                    None => rest == tag,
                };
                if at_our_url {
                    current.push(endpoint);
                } else if let Some(id) = &endpoint.id {
                    retirable.push(id.clone());
                }
            }
            None => {
                if foreign_at_ours.is_none() && endpoint.url.as_deref() == Some(url) {
                    foreign_at_ours = Some(endpoint);
                }
            }
        }
    }

    let mut current = current.into_iter();
    if let Some(first) = current.next() {
        let endpoint_id = first.id.clone().ok_or_else(|| {
            ProviderError::malformed(
                "notchpay: listing webhook endpoints returned vpay's endpoint with no id",
            )
        })?;
        retirable.extend(current.filter_map(|duplicate| duplicate.id.clone()));
        let missing: Vec<&str> = first.events.as_ref().map_or_else(Vec::new, |events| {
            PAYMENT_EVENTS
                .iter()
                .copied()
                .filter(|wanted| !events.iter().any(|event| event == wanted))
                .collect()
        });
        let state = if missing.is_empty() {
            EndpointState::Present {
                endpoint_id,
                repaired: false,
            }
        } else {
            EndpointState::Drifted {
                endpoint_id,
                detail: format!(
                    "subscribed without {}; Notch Pay's client has no update call, so it is \
                     left as found",
                    missing.join(", ")
                ),
            }
        };
        return Ok(Decision::Report(EndpointReport { state, retirable }));
    }

    if let Some(foreign) = foreign_at_ours {
        return Ok(Decision::Report(EndpointReport {
            state: EndpointState::Foreign {
                endpoint_id: foreign.id.clone().unwrap_or_default(),
            },
            retirable,
        }));
    }

    if !check.write {
        return Ok(Decision::Report(EndpointReport {
            state: EndpointState::Missing,
            retirable,
        }));
    }
    Ok(Decision::Create { retirable })
}

/// A refused list or create.
///
/// | Notch Pay answered | Result |
/// | --- | --- |
/// | `401`, `403` | `Rejected { provider_account_blocked }` — our own keys |
/// | `400`, `422` | `Config` — the URL or the events this deployment sent were refused; Notch Pay's message, bounded, names which |
/// | `404` | `Config` — a base URL with a wrong path |
/// | anything else | `Malformed` |
fn webhook_refusal(refusal: &Refusal, doing: &str) -> ProviderError {
    match refusal.http_status {
        401 | 403 => mapping::keys_refused(refusal, doing),
        400 | 422 => ProviderError::Config(format!(
            "notchpay: {doing} answered {}: {}",
            refusal.http_status,
            refusal
                .message
                .as_deref()
                .map_or_else(|| "no message".to_owned(), mapping::truncated),
        )),
        404 => ProviderError::Config(format!(
            "notchpay: {doing} answered 404; the base URL is wrong"
        )),
        other => ProviderError::malformed(format!(
            "notchpay: {doing} answered an undocumented {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &str = "vpay:prod:notchpay:default";
    const URL: &str = "https://pay.example/provider/notchpay/callback";
    const OLD_URL: &str = "https://old.example/provider/notchpay/callback";

    fn endpoint(id: &str, url: Option<&str>, description: Option<&str>) -> Webhook {
        Webhook {
            id: Some(id.to_owned()),
            url: url.map(str::to_owned),
            description: description.map(str::to_owned),
            events: Some(PAYMENT_EVENTS.iter().map(|e| (*e).to_owned()).collect()),
        }
    }

    fn ensure_mode() -> EndpointCheck<'static> {
        EndpointCheck {
            marker: MARKER,
            write: true,
        }
    }

    fn report(decision: Decision) -> EndpointReport {
        match decision {
            Decision::Report(report) => report,
            Decision::Create { .. } => panic!("expected a report, got a create"),
        }
    }

    #[test]
    fn the_description_is_the_marker_and_a_tag_of_the_url() {
        let described = description(MARKER, URL);
        assert!(described.starts_with("vpay:prod:notchpay:default url:"));
        assert_eq!(described.len(), MARKER.len() + " url:".len() + URL_TAG_HEX);
        assert_ne!(described, description(MARKER, OLD_URL));
    }

    #[test]
    fn an_empty_account_is_created_in_ensure_mode_and_missing_in_check_mode() {
        assert_eq!(
            decide(&[], &ensure_mode(), URL).expect("decides"),
            Decision::Create { retirable: vec![] }
        );
        let check_only = EndpointCheck {
            marker: MARKER,
            write: false,
        };
        assert_eq!(
            report(decide(&[], &check_only, URL).expect("decides")).state,
            EndpointState::Missing
        );
    }

    /// The case the sandbox forces: Notch Pay answers `"url": null`, so the
    /// description's tag is the only evidence of where an endpoint points.
    #[test]
    fn vpays_endpoint_is_found_by_its_description_when_notch_pay_echoes_no_url() {
        let ours = endpoint("ep_1", None, Some(&description(MARKER, URL)));
        let decided = report(decide(&[ours], &ensure_mode(), URL).expect("decides"));
        assert_eq!(
            decided.state,
            EndpointState::Present {
                endpoint_id: "ep_1".to_owned(),
                repaired: false
            }
        );
        assert!(decided.retirable.is_empty());
    }

    #[test]
    fn an_endpoint_for_an_old_url_is_superseded_and_a_new_one_is_created() {
        let old = endpoint("ep_old", None, Some(&description(MARKER, OLD_URL)));
        assert_eq!(
            decide(&[old], &ensure_mode(), URL).expect("decides"),
            Decision::Create {
                retirable: vec!["ep_old".to_owned()]
            }
        );
    }

    #[test]
    fn a_duplicate_of_the_current_endpoint_is_reported_and_never_touched() {
        let first = endpoint("ep_1", None, Some(&description(MARKER, URL)));
        let second = endpoint("ep_2", None, Some(&description(MARKER, URL)));
        let decided = report(decide(&[first, second], &ensure_mode(), URL).expect("decides"));
        assert!(
            matches!(decided.state, EndpointState::Present { ref endpoint_id, .. } if endpoint_id == "ep_1")
        );
        assert_eq!(decided.retirable, vec!["ep_2".to_owned()]);
    }

    /// Another deployment sharing the sandbox account: its marker differs
    /// only in the deployment's name, and its endpoint is neither ours nor
    /// retirable.
    #[test]
    fn another_deployments_endpoint_is_never_ours() {
        let theirs = endpoint(
            "ep_theirs",
            None,
            Some(&description("vpay:staging:notchpay:default", URL)),
        );
        assert_eq!(
            decide(&[theirs], &ensure_mode(), URL).expect("decides"),
            Decision::Create { retirable: vec![] }
        );
    }

    /// A marker that is a prefix of ours (`…:default2`) is another account's.
    #[test]
    fn a_marker_is_matched_whole_never_as_a_prefix() {
        let other_account = endpoint(
            "ep_other",
            None,
            Some(&description("vpay:prod:notchpay:default2", URL)),
        );
        assert_eq!(
            decide(&[other_account], &ensure_mode(), URL).expect("decides"),
            Decision::Create { retirable: vec![] }
        );
    }

    #[test]
    fn an_endpoint_at_our_url_without_our_marker_is_foreign_and_nothing_is_created() {
        let hand_registered = endpoint("ep_hand", Some(URL), Some("registered by hand"));
        let decided = report(decide(&[hand_registered], &ensure_mode(), URL).expect("decides"));
        assert_eq!(
            decided.state,
            EndpointState::Foreign {
                endpoint_id: "ep_hand".to_owned()
            }
        );
    }

    #[test]
    fn a_current_endpoint_missing_an_event_is_drifted_and_left_as_found() {
        let mut ours = endpoint("ep_1", None, Some(&description(MARKER, URL)));
        ours.events = Some(vec!["payment.complete".to_owned()]);
        let decided = report(decide(&[ours], &ensure_mode(), URL).expect("decides"));
        let EndpointState::Drifted {
            endpoint_id,
            detail,
        } = decided.state
        else {
            panic!("expected drift, got {:?}", decided.state);
        };
        assert_eq!(endpoint_id, "ep_1");
        assert!(detail.contains("payment.failed") && detail.contains("payment.expired"));
        assert!(!detail.contains("payment.complete"));
    }

    #[test]
    fn a_current_endpoint_with_no_id_is_malformed_rather_than_guessed_at() {
        let mut ours = endpoint("ignored", None, Some(&description(MARKER, URL)));
        ours.id = None;
        assert!(matches!(
            decide(&[ours], &ensure_mode(), URL),
            Err(ProviderError::Malformed { .. })
        ));
    }

    #[test]
    fn a_refusal_of_our_keys_pages_and_a_refused_url_is_the_deployments_to_fix() {
        let refusal = |http_status| Refusal {
            http_status,
            message: Some("The url field must be a valid URL.".to_owned()),
            errors: std::collections::BTreeMap::new(),
        };
        assert!(matches!(
            webhook_refusal(&refusal(401), "listing webhook endpoints"),
            ProviderError::Rejected { .. }
        ));
        let ProviderError::Config(message) =
            webhook_refusal(&refusal(422), "creating a webhook endpoint")
        else {
            panic!("a 422 on create is the deployment's configuration");
        };
        assert!(message.contains("422") && message.contains("valid URL"));
        assert!(matches!(
            webhook_refusal(&refusal(404), "listing webhook endpoints"),
            ProviderError::Config(_)
        ));
        assert!(matches!(
            webhook_refusal(&refusal(409), "listing webhook endpoints"),
            ProviderError::Malformed { .. }
        ));
    }
}
