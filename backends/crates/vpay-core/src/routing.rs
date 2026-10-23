//! Payment-method types, and the router that scores the rails for one
//! payment ([ADR-0029](../../../../docs/adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
//! D6, D7, D10, D12, D13; [RFC-0008](../../../../docs/rfc/0008-payment-method-routing.md)
//! as revised on 2026-10-23).
//!
//! A **method type** is what the payer uses, one per wallet network (D12):
//! [`MethodType::MtnWallet`], [`MethodType::OrangeWallet`]. A **rail** is the
//! adapter that processes it. Each adapter declares the types it serves, and
//! [`route`] ranks the ones that may take this payment.
//!
//! **The router scores.** The maintainer, on 2026-10-23: "I think routing
//! should be though again. We should have a score of each payment solution
//! for each checkout". So eligibility is a hard filter (D7), every eligible
//! rail gets a [`Score`] with its breakdown, the highest wins, and ties go to
//! the rail code. RFC-0008's 2026-10-23 revision designs the components.
//!
//! **The formula is provisional** ([`SCORE_FORMULA`]). It has one component,
//! [`Component::ConfiguredWeight`], because that is the only input that
//! exists today: fees (D8), health and latency have no source yet. Adding a
//! component is a variant on [`Component`] and one line in `score`; every
//! [`Decision`] names the formula it was scored under, so a row written
//! today stays readable after the formula changes.
//!
//! **This module names no rail.** It reads what each [`Candidate`] declares
//! and the deployment's weights, and nothing else (D9; RFC-0008's "Impact on
//! existing invariants"). `vpay-server`'s `the_router_names_no_rail` reads
//! this file and fails if any linked adapter's code appears in it.

use std::collections::BTreeMap;
use std::fmt;

use sha2::{Digest, Sha256};

/// What the payer uses. One per wallet network (ADR-0029 D12). The
/// maintainer confirmed one type per network on 2026-10-23 ("he himself
/// should choose orange or mtn"), and these two spellings the same day.
///
/// Changing one means this table, migration `0053`'s CHECKs on
/// `charges.method_type` and `routing_decisions.method_type`, and the
/// documented values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MethodType {
    /// An MTN Mobile Money wallet.
    MtnWallet,
    /// An Orange Money wallet.
    OrangeWallet,
}

impl MethodType {
    /// Every method type, in wire order.
    pub const ALL: [Self; 2] = [Self::MtnWallet, Self::OrangeWallet];

    /// The spelling on the wire, in configuration and in the database.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::MtnWallet => "mtn_wallet",
            Self::OrangeWallet => "orange_wallet",
        }
    }

    /// The method type a wire value names, or `None`.
    ///
    /// ```
    /// use vpay_core::routing::MethodType;
    ///
    /// assert_eq!(MethodType::from_wire("mtn_wallet"), Some(MethodType::MtnWallet));
    /// // A rail code is not a method type, which is what keeps D6's
    /// // backward compatibility: `mtn_momo` still means that rail.
    /// assert_eq!(MethodType::from_wire("mtn_momo"), None);
    /// // Reserved, and not built: see `RESERVED_METHOD_TYPES`.
    /// assert_eq!(MethodType::from_wire("card"), None);
    /// ```
    #[must_use]
    pub fn from_wire(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|method_type| method_type.as_wire_str() == value)
    }

    /// The bit this type holds in a [`MethodTypes`].
    const fn bit(self) -> u8 {
        match self {
            Self::MtnWallet => 1,
            Self::OrangeWallet => 1 << 1,
        }
    }
}

impl fmt::Display for MethodType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

/// Method-type spellings ADR-0029 D12 reserves and nothing serves yet:
/// `card` for RFC-0010's first card rail, `bank_transfer` for D4's two bank
/// shapes. A caller who sends one is told that, rather than "unknown".
pub const RESERVED_METHOD_TYPES: [&str; 2] = ["card", "bank_transfer"];

/// A set of [`MethodType`]s: what one rail serves.
///
/// A bit set rather than a slice so a rail's declaration is `Copy` and
/// comparable, and so `contains` is one instruction on the routing path.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct MethodTypes(u8);

impl MethodTypes {
    /// Serves nothing: the port's default, and what a rail that predates
    /// method types declares until its adapter says otherwise.
    pub const NONE: Self = Self(0);

    /// The set of these types.
    ///
    /// ```
    /// use vpay_core::routing::{MethodType, MethodTypes};
    ///
    /// let both = MethodTypes::of(&[MethodType::MtnWallet, MethodType::OrangeWallet]);
    /// assert!(both.contains(MethodType::OrangeWallet));
    /// assert_eq!(both.len(), 2);
    /// assert!(MethodTypes::NONE.is_empty());
    /// ```
    #[must_use]
    pub const fn of(types: &[MethodType]) -> Self {
        let mut bits = 0;
        let mut rest = types;
        while let [first, tail @ ..] = rest {
            bits |= first.bit();
            rest = tail;
        }
        Self(bits)
    }

    /// Whether this set holds `method_type`.
    #[must_use]
    pub const fn contains(self, method_type: MethodType) -> bool {
        self.0 & method_type.bit() != 0
    }

    /// Whether this set is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many types this set holds.
    #[must_use]
    pub fn len(self) -> usize {
        self.iter().count()
    }

    /// The types this set holds, in wire order.
    pub fn iter(self) -> impl Iterator<Item = MethodType> {
        MethodType::ALL
            .into_iter()
            .filter(move |method_type| self.contains(*method_type))
    }

    /// The one type this set holds, when it holds exactly one.
    ///
    /// A charge confirmed by **rail code** on a rail that serves one type is
    /// recorded with that type: a `mtn_momo` charge is an MTN wallet charge,
    /// whoever routed it. On a rail serving several, the rail code says
    /// nothing about the network, and the charge records none.
    #[must_use]
    pub fn only(self) -> Option<MethodType> {
        let mut types = self.iter();
        match (types.next(), types.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
    }
}

/// A set is written as the list of its types' wire spellings, in wire order,
/// because [`MethodTypes`] sits on `vpay_provider::Capabilities`, which is
/// vpay's own serialised shape.
impl serde::Serialize for MethodTypes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter().map(MethodType::as_wire_str))
    }
}

/// The inverse of the `Serialize` impl: a list of wire spellings. An
/// unknown spelling is an error, never a silently smaller set.
impl<'de> serde::Deserialize<'de> for MethodTypes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let spellings = Vec::<String>::deserialize(deserializer)?;
        let mut types = Vec::with_capacity(spellings.len());
        for spelling in &spellings {
            let method_type = MethodType::from_wire(spelling).ok_or_else(|| {
                serde::de::Error::custom(format!("`{spelling}` is not a method type"))
            })?;
            types.push(method_type);
        }
        Ok(Self::of(&types))
    }
}

impl fmt::Debug for MethodTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

/// The formula a [`Score`] was computed under, stored on every decision row.
///
/// **Provisional** until RFC-0008's 2026-10-23 revision is decided: one
/// component, the configured weight. A formula change bumps this, so a row
/// says which rules produced its numbers.
pub const SCORE_FORMULA: &str = "provisional-1";

/// A rail's weight when the deployment configures none for it: every rail
/// equal, so the tie-break decides.
pub const DEFAULT_WEIGHT: u32 = 100;

/// The largest weight configuration may set. Bounded so a sum of components
/// can never overflow and an operator's typo of `1000000` is refused rather
/// than silently dominating every later component.
pub const MAX_WEIGHT: u32 = 1_000;

/// The deployment's configured weights (`routing.weights`), resolved most
/// specific first: a weight for this rail **and** this method type, else for
/// this rail, else [`DEFAULT_WEIGHT`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Weights {
    by_rail: BTreeMap<String, u32>,
    by_rail_and_type: BTreeMap<(String, MethodType), u32>,
}

impl Weights {
    /// No configured weights: every rail at [`DEFAULT_WEIGHT`].
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Sets this rail's weight for every type it serves, or, with a
    /// `method_type`, for that type only. A later call for the same key
    /// replaces the earlier one; configuration refuses duplicates before it
    /// gets here.
    #[must_use]
    pub fn with(mut self, rail: &str, method_type: Option<MethodType>, weight: u32) -> Self {
        let weight = weight.min(MAX_WEIGHT);
        match method_type {
            Some(method_type) => {
                self.by_rail_and_type
                    .insert((rail.to_owned(), method_type), weight);
            }
            None => {
                self.by_rail.insert(rail.to_owned(), weight);
            }
        }
        self
    }

    /// This rail's weight for this type.
    ///
    /// ```
    /// use vpay_core::routing::{MethodType, Weights, DEFAULT_WEIGHT};
    ///
    /// let weights = Weights::none()
    ///     .with("rail_a", None, 200)
    ///     .with("rail_a", Some(MethodType::OrangeWallet), 50);
    /// assert_eq!(weights.weight_of("rail_a", MethodType::MtnWallet), 200);
    /// assert_eq!(weights.weight_of("rail_a", MethodType::OrangeWallet), 50);
    /// assert_eq!(weights.weight_of("rail_b", MethodType::MtnWallet), DEFAULT_WEIGHT);
    /// ```
    #[must_use]
    pub fn weight_of(&self, rail: &str, method_type: MethodType) -> u32 {
        self.by_rail_and_type
            .get(&(rail.to_owned(), method_type))
            .or_else(|| self.by_rail.get(rail))
            .copied()
            .unwrap_or(DEFAULT_WEIGHT)
    }

    /// The weights that bear on `method_type`, as canonical text for
    /// [`Decision::config_version`].
    fn canonical(&self, method_type: MethodType) -> String {
        let by_rail = self
            .by_rail
            .iter()
            .map(|(rail, weight)| format!("{rail}={weight}"));
        let by_type = self
            .by_rail_and_type
            .iter()
            .filter(|((_, typed), _)| *typed == method_type)
            .map(|((rail, _), weight)| format!("{rail}/{method_type}={weight}"));
        by_rail.chain(by_type).collect::<Vec<_>>().join(",")
    }
}

/// One rail, as the router sees it for one payment.
///
/// Every field is a fact the caller looked up: the rail's declaration and the
/// deployment's configuration. The router decides nothing from a rail's
/// `rail` code but the tie-break and the weight lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate<'a> {
    /// The rail's code, `providers.code`.
    pub rail: &'a str,
    /// What its adapter declares it serves.
    pub serves: MethodTypes,
    /// `providers[].enabled` on this deployment.
    pub enabled: bool,
    /// Whether the rail settles this payment's currency.
    pub settles_currency: bool,
}

/// Why a rail was not a candidate: the hard filter (D7). Recorded per rail on
/// every [`Decision`], so "why did this payment go through that rail" is
/// answered from a row (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Exclusion {
    /// Its adapter does not serve the method type.
    DoesNotServe,
    /// The deployment configures it and has not enabled it.
    NotEnabled,
    /// It does not settle the payment's currency.
    CurrencyNotSettled,
    /// The merchant pinned another rail
    /// (`payment_method_options[<type>][rail]`).
    NotPinned,
}

impl Exclusion {
    /// The spelling stored in `routing_decisions.candidates`.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::DoesNotServe => "does_not_serve",
            Self::NotEnabled => "not_enabled",
            Self::CurrencyNotSettled => "currency_not_settled",
            Self::NotPinned => "not_pinned",
        }
    }
}

/// One input to a [`Score`]. RFC-0008's 2026-10-23 revision lists the
/// components it proposes; this enum holds the ones whose inputs exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Component {
    /// The deployment's `routing.weights` for this rail (and type), or
    /// [`DEFAULT_WEIGHT`].
    ConfiguredWeight,
}

impl Component {
    /// The spelling stored in a score breakdown.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::ConfiguredWeight => "configured_weight",
        }
    }
}

/// One component's contribution to a [`Score`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// Which component.
    pub component: Component,
    /// Its points, already multiplied by the component's coefficient.
    pub points: i64,
}

/// A candidate's score, and what it is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Score {
    /// The sum of [`Score::parts`].
    pub total: i64,
    /// Every component, in [`Component`] order, including a zero one: a
    /// breakdown that omits a component cannot show that it was weighed.
    pub parts: Vec<Part>,
}

/// One rail the router considered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Considered {
    /// The rail's code.
    pub rail: String,
    /// `None` for a candidate.
    pub excluded: Option<Exclusion>,
    /// A candidate's score; `None` for an excluded rail, which the hard
    /// filter removed before anything was scored.
    pub score: Option<Score>,
}

/// The router's answer for one payment and one method type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// What the payer chose.
    pub method_type: MethodType,
    /// The highest-scoring candidate, or `None` when no rail may take the
    /// payment.
    pub chosen: Option<String>,
    /// The rail the merchant pinned, if any.
    pub pinned: Option<String>,
    /// Every rail considered: the candidates ranked, best first, then the
    /// excluded ones by rail code.
    pub considered: Vec<Considered>,
    /// [`SCORE_FORMULA`] at the time of the decision.
    pub formula: &'static str,
    /// 16 hex digits identifying the configuration the decision was made
    /// under: the formula, the weights that bear on this type, and every
    /// rail's code, declaration and `enabled`. The payment's own facts
    /// (currency, pin) are not in it.
    pub config_version: String,
}

/// Ranks the rails that may take a payment of `method_type`, and picks the
/// best (RFC-0008 as revised on 2026-10-23; ADR-0029 D7, D10).
///
/// A rail is a candidate when it serves the type, is enabled, settles the
/// currency, and is the pinned rail when one is pinned. Every candidate is
/// scored; the highest total wins, and a tie goes to the lower rail code.
/// The result is a function of its arguments alone: the same rails in any
/// input order give the same [`Decision`].
///
/// ```
/// use vpay_core::routing::{route, Candidate, MethodType, MethodTypes, Weights};
///
/// let wallet = MethodTypes::of(&[MethodType::MtnWallet]);
/// let rails = [
///     Candidate { rail: "rail_b", serves: wallet, enabled: true, settles_currency: true },
///     Candidate { rail: "rail_a", serves: wallet, enabled: true, settles_currency: true },
/// ];
/// // No configured weights: a tie, and the rail code breaks it.
/// let decision = route(MethodType::MtnWallet, &rails, &Weights::none(), None);
/// assert_eq!(decision.chosen.as_deref(), Some("rail_a"));
/// // A heavier rail wins.
/// let weights = Weights::none().with("rail_b", None, 150);
/// let decision = route(MethodType::MtnWallet, &rails, &weights, None);
/// assert_eq!(decision.chosen.as_deref(), Some("rail_b"));
/// assert_eq!(decision.considered[0].score.as_ref().map(|s| s.total), Some(150));
/// ```
#[must_use]
pub fn route(
    method_type: MethodType,
    candidates: &[Candidate<'_>],
    weights: &Weights,
    pin: Option<&str>,
) -> Decision {
    let mut rails: Vec<Candidate<'_>> = candidates.to_vec();
    rails.sort_by(|left, right| left.rail.cmp(right.rail));
    rails.dedup_by(|left, right| left.rail == right.rail);

    let exclusion = |candidate: &Candidate<'_>| {
        if !candidate.serves.contains(method_type) {
            Some(Exclusion::DoesNotServe)
        } else if !candidate.enabled {
            Some(Exclusion::NotEnabled)
        } else if !candidate.settles_currency {
            Some(Exclusion::CurrencyNotSettled)
        } else if pin.is_some_and(|pinned| pinned != candidate.rail) {
            Some(Exclusion::NotPinned)
        } else {
            None
        }
    };

    let mut ranked: Vec<Considered> = Vec::new();
    let mut excluded: Vec<Considered> = Vec::new();
    for candidate in &rails {
        match exclusion(candidate) {
            None => ranked.push(Considered {
                rail: candidate.rail.to_owned(),
                excluded: None,
                score: Some(score(candidate, method_type, weights)),
            }),
            Some(reason) => excluded.push(Considered {
                rail: candidate.rail.to_owned(),
                excluded: Some(reason),
                score: None,
            }),
        }
    }
    // Highest total first. `rails` is in code order and the sort is stable,
    // so a tie keeps the lower code first: the deterministic tie-break.
    ranked.sort_by_key(|considered| {
        std::cmp::Reverse(considered.score.as_ref().map_or(i64::MIN, |s| s.total))
    });

    Decision {
        method_type,
        chosen: ranked.first().map(|considered| considered.rail.clone()),
        pinned: pin.map(str::to_owned),
        considered: ranked.into_iter().chain(excluded).collect(),
        formula: SCORE_FORMULA,
        config_version: config_version(method_type, &rails, weights),
    }
}

/// The provisional formula: a weighted sum of [`Component`]s, each with
/// coefficient 1 because there is one. A new component is one more entry.
fn score(candidate: &Candidate<'_>, method_type: MethodType, weights: &Weights) -> Score {
    let parts = vec![Part {
        component: Component::ConfiguredWeight,
        points: i64::from(weights.weight_of(candidate.rail, method_type)),
    }];
    Score {
        total: parts.iter().map(|part| part.points).sum(),
        parts,
    }
}

/// The configuration a decision was made under, as 16 hex digits of SHA-256
/// over a canonical text: the formula, the type, the weights that bear on
/// it, and each rail's code, declaration and `enabled`, in code order.
fn config_version(method_type: MethodType, rails: &[Candidate<'_>], weights: &Weights) -> String {
    let mut text = format!(
        "routing\nformula={SCORE_FORMULA}\ntype={method_type}\nweights={}",
        weights.canonical(method_type)
    );
    for rail in rails {
        let serves: Vec<&str> = rail.serves.iter().map(MethodType::as_wire_str).collect();
        text.push_str(&format!(
            "\nrail={} serves={} enabled={}",
            rail.rail,
            serves.join(","),
            rail.enabled
        ));
    }
    let digest = Sha256::digest(text.as_bytes());
    let prefix: Vec<u8> = digest.iter().take(8).copied().collect();
    hex::encode(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MTN: MethodTypes = MethodTypes::of(&[MethodType::MtnWallet]);
    const ORANGE: MethodTypes = MethodTypes::of(&[MethodType::OrangeWallet]);
    const BOTH: MethodTypes = MethodTypes::of(&[MethodType::MtnWallet, MethodType::OrangeWallet]);

    fn rail(code: &str, serves: MethodTypes) -> Candidate<'_> {
        Candidate {
            rail: code,
            serves,
            enabled: true,
            settles_currency: true,
        }
    }

    fn find<'d>(decision: &'d Decision, code: &str) -> &'d Considered {
        decision
            .considered
            .iter()
            .find(|considered| considered.rail == code)
            .expect("every rail is considered")
    }

    #[test]
    fn every_method_type_round_trips_through_its_wire_spelling() {
        for method_type in MethodType::ALL {
            assert_eq!(
                MethodType::from_wire(method_type.as_wire_str()),
                Some(method_type)
            );
        }
        for reserved in RESERVED_METHOD_TYPES {
            assert_eq!(MethodType::from_wire(reserved), None, "{reserved}");
        }
    }

    #[test]
    fn a_set_holds_what_it_was_built_from_and_names_its_only_member() {
        assert_eq!(BOTH.iter().collect::<Vec<_>>(), MethodType::ALL.to_vec());
        assert_eq!(MTN.only(), Some(MethodType::MtnWallet));
        assert_eq!(BOTH.only(), None);
        assert_eq!(MethodTypes::NONE.only(), None);
        assert!(!ORANGE.contains(MethodType::MtnWallet));
        assert_eq!(format!("{BOTH:?}"), "{MtnWallet, OrangeWallet}");
    }

    #[test]
    fn a_set_serialises_as_its_wire_spellings_and_refuses_an_unknown_one() {
        let json = serde_json::to_string(&BOTH).expect("serialises");
        assert_eq!(json, r#"["mtn_wallet","orange_wallet"]"#);
        let back: MethodTypes = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, BOTH);
        assert!(serde_json::from_str::<MethodTypes>(r#"["card"]"#).is_err());
    }

    #[test]
    fn the_most_specific_weight_wins_and_none_is_the_default() {
        let weights = Weights::none()
            .with("rail_a", None, 300)
            .with("rail_a", Some(MethodType::MtnWallet), 20)
            .with("rail_b", Some(MethodType::OrangeWallet), MAX_WEIGHT + 1);
        assert_eq!(weights.weight_of("rail_a", MethodType::MtnWallet), 20);
        assert_eq!(weights.weight_of("rail_a", MethodType::OrangeWallet), 300);
        assert_eq!(
            weights.weight_of("rail_b", MethodType::MtnWallet),
            DEFAULT_WEIGHT
        );
        assert_eq!(
            weights.weight_of("rail_b", MethodType::OrangeWallet),
            MAX_WEIGHT,
            "clamped: configuration refuses the excess before it gets here"
        );
    }

    #[test]
    fn each_exclusion_is_recorded_with_its_own_reason_and_no_score() {
        let rails = [
            rail("rail_a", ORANGE),
            Candidate {
                enabled: false,
                ..rail("rail_b", MTN)
            },
            Candidate {
                settles_currency: false,
                ..rail("rail_c", MTN)
            },
            rail("rail_d", BOTH),
        ];
        let decision = route(MethodType::MtnWallet, &rails, &Weights::none(), None);
        assert_eq!(decision.chosen.as_deref(), Some("rail_d"));
        for (code, reason) in [
            ("rail_a", Exclusion::DoesNotServe),
            ("rail_b", Exclusion::NotEnabled),
            ("rail_c", Exclusion::CurrencyNotSettled),
        ] {
            assert_eq!(find(&decision, code).excluded, Some(reason));
            assert_eq!(find(&decision, code).score, None);
        }
        assert_eq!(
            find(&decision, "rail_d").score,
            Some(Score {
                total: i64::from(DEFAULT_WEIGHT),
                parts: vec![Part {
                    component: Component::ConfiguredWeight,
                    points: i64::from(DEFAULT_WEIGHT),
                }],
            })
        );
        assert_eq!(decision.considered.len(), rails.len());
        assert_eq!(decision.formula, SCORE_FORMULA);
    }

    #[test]
    fn a_pin_narrows_the_candidates_to_one_and_cannot_add_one() {
        let rails = [
            rail("rail_a", MTN),
            rail("rail_b", MTN),
            rail("rail_c", ORANGE),
        ];
        // rail_a is heavier, and the pin still wins: a pin is a filter, not
        // a weight.
        let weights = Weights::none().with("rail_a", None, 900);
        let pinned = route(MethodType::MtnWallet, &rails, &weights, Some("rail_b"));
        assert_eq!(pinned.chosen.as_deref(), Some("rail_b"));
        assert_eq!(pinned.pinned.as_deref(), Some("rail_b"));
        assert_eq!(find(&pinned, "rail_a").excluded, Some(Exclusion::NotPinned));

        // A pin to a rail that does not serve the type chooses nothing: it
        // narrows the candidates and never overrides a rule.
        let refused = route(MethodType::MtnWallet, &rails, &weights, Some("rail_c"));
        assert_eq!(refused.chosen, None);
        assert_eq!(
            find(&refused, "rail_c").excluded,
            Some(Exclusion::DoesNotServe)
        );
    }

    #[test]
    fn no_rail_serving_the_type_is_a_decision_with_no_choice() {
        let decision = route(
            MethodType::OrangeWallet,
            &[rail("rail_a", MTN)],
            &Weights::none(),
            None,
        );
        assert_eq!(decision.chosen, None);
        assert_eq!(decision.considered.len(), 1);
        assert_eq!(
            route(MethodType::OrangeWallet, &[], &Weights::none(), None).chosen,
            None
        );
    }

    #[test]
    fn the_configuration_version_moves_with_the_configuration_and_nothing_else() {
        let rails = [rail("rail_a", MTN), rail("rail_b", BOTH)];
        let none = Weights::none();
        let base = route(MethodType::MtnWallet, &rails, &none, None).config_version;
        assert_eq!(base.len(), 16);
        assert!(base.chars().all(|c| c.is_ascii_hexdigit()));

        // A pin and the currency are the payment's, not the configuration's.
        assert_eq!(
            route(MethodType::MtnWallet, &rails, &none, Some("rail_b")).config_version,
            base
        );
        let foreign = [
            Candidate {
                settles_currency: false,
                ..rails[0]
            },
            rails[1],
        ];
        assert_eq!(
            route(MethodType::MtnWallet, &foreign, &none, None).config_version,
            base
        );
        // A weight for another type does not bear on this one.
        let orange_only = Weights::none().with("rail_b", Some(MethodType::OrangeWallet), 7);
        assert_eq!(
            route(MethodType::MtnWallet, &rails, &orange_only, None).config_version,
            base
        );

        // A weight that bears on it, a disabled rail and another declaration do.
        let heavier = Weights::none().with("rail_a", None, 7);
        assert_ne!(
            route(MethodType::MtnWallet, &rails, &heavier, None).config_version,
            base
        );
        let disabled = [
            Candidate {
                enabled: false,
                ..rails[0]
            },
            rails[1],
        ];
        assert_ne!(
            route(MethodType::MtnWallet, &disabled, &none, None).config_version,
            base
        );
        assert_ne!(
            route(
                MethodType::MtnWallet,
                &[rail("rail_a", BOTH), rails[1]],
                &none,
                None
            )
            .config_version,
            base
        );
    }

    /// Every assignment of six shapes to up to four rails, routed under three
    /// weightings and with and without a pin. The properties RFC-0008 asks of
    /// the ranking, checked over the whole space rather than sampled:
    ///
    /// 1. the input order of the rails never changes the decision;
    /// 2. a rail is a candidate if and only if no hard-filter rule refuses it,
    ///    and only candidates carry a score;
    /// 3. the candidates lead, by total descending, then rail code;
    /// 4. the chosen rail is the first candidate, and each total is the sum of
    ///    its parts.
    #[test]
    fn the_ranking_is_total_deterministic_and_obeys_every_rule_over_every_arrangement() {
        let codes = ["rail_a", "rail_b", "rail_c", "rail_d"];
        // (serves, enabled, settles): the six shapes a rail can take here.
        let shapes = [
            (MTN, true, true),
            (BOTH, true, true),
            (ORANGE, true, true),
            (MTN, false, true),
            (MTN, true, false),
            (MethodTypes::NONE, true, true),
        ];
        let weightings = [
            Weights::none(),
            Weights::none()
                .with("rail_c", None, 400)
                .with("rail_d", None, 400),
            Weights::none()
                .with("rail_b", Some(MethodType::MtnWallet), 0)
                .with("rail_a", None, 250),
        ];
        let mut checked = 0_u32;
        for count in 0..=codes.len() {
            let arrangements = (0..count).fold(1_usize, |total, _| total * shapes.len());
            for arrangement in 0..arrangements {
                // `arrangement`'s digits in base `shapes.len()`, one per rail.
                let rails: Vec<Candidate<'_>> = codes
                    .iter()
                    .take(count)
                    .scan(arrangement, |rest, code| {
                        let shape = *rest % shapes.len();
                        *rest /= shapes.len();
                        let (serves, enabled, settles_currency) =
                            *shapes.get(shape).expect("a digit below the base");
                        Some(Candidate {
                            rail: code,
                            serves,
                            enabled,
                            settles_currency,
                        })
                    })
                    .collect();
                let reversed: Vec<Candidate<'_>> = rails.iter().rev().copied().collect();
                let last = rails.last().map(|candidate| candidate.rail);
                for method_type in MethodType::ALL {
                    for weights in &weightings {
                        for pin in [None, last] {
                            let decision = route(method_type, &rails, weights, pin);
                            assert_eq!(decision, route(method_type, &reversed, weights, pin));
                            check(&decision, &rails, method_type, weights, pin);
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 10_000, "the space was walked: {checked}");
    }

    fn check(
        decision: &Decision,
        rails: &[Candidate<'_>],
        method_type: MethodType,
        weights: &Weights,
        pin: Option<&str>,
    ) {
        let eligible = |candidate: &Candidate<'_>| {
            candidate.serves.contains(method_type)
                && candidate.enabled
                && candidate.settles_currency
                && pin.is_none_or(|pinned| pinned == candidate.rail)
        };
        let mut expected: Vec<&Candidate<'_>> = rails.iter().filter(|c| eligible(c)).collect();
        expected.sort_by_key(|candidate| {
            (
                std::cmp::Reverse(weights.weight_of(candidate.rail, method_type)),
                candidate.rail,
            )
        });
        assert_eq!(
            decision.chosen.as_deref(),
            expected.first().map(|candidate| candidate.rail)
        );
        assert_eq!(decision.considered.len(), rails.len());
        for candidate in rails {
            let considered: Vec<&Considered> = decision
                .considered
                .iter()
                .filter(|considered| considered.rail == candidate.rail)
                .collect();
            let [only] = considered.as_slice() else {
                panic!(
                    "{} considered once, not {}",
                    candidate.rail,
                    considered.len()
                );
            };
            assert_eq!(only.excluded.is_none(), eligible(candidate));
            assert_eq!(only.score.is_some(), eligible(candidate));
            if let Some(score) = &only.score {
                assert_eq!(
                    score.total,
                    score.parts.iter().map(|p| p.points).sum::<i64>()
                );
            }
        }
        let leading: Vec<&str> = decision
            .considered
            .iter()
            .take_while(|considered| considered.excluded.is_none())
            .map(|considered| considered.rail.as_str())
            .collect();
        let expected: Vec<&str> = expected.iter().map(|candidate| candidate.rail).collect();
        assert_eq!(leading, expected, "the candidates lead, best first");
    }
}
