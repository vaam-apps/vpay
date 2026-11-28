//! The settings, read through a lookup function (so a test needs no `set_var`): the standard `OTEL_*` variables,
//! a subset of them, and strictly. Telemetry is **off unless an endpoint is set**.
//!
//! What is refused, on purpose: a protocol other than `http/json` (this crate speaks OTLP/HTTP with JSON bodies and
//! nothing else); `OTEL_EXPORTER_OTLP_HEADERS` (a header usually holds a credential, and an environment variable
//! shows in a process listing: the headers are read from the file `VPAY_OTEL_HEADERS_FILE` names); and
//! headers sent over plain `http` to a host that is not this machine. No message holds a header value or a URL's
//! credentials.

use std::collections::BTreeMap;
use std::time::Duration;

use url::Url;

/// Variables of the OpenTelemetry specification this exporter does not honour. Set, they stop the process at start:
/// a setting that is ignored in silence is worse than one that is refused.
const UNSUPPORTED: [&str; 28] = [
    "OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE",
    "OTEL_EXPORTER_OTLP_TRACES_COMPRESSION",
    "OTEL_EXPORTER_OTLP_METRICS_COMPRESSION",
    "OTEL_EXPORTER_OTLP_LOGS_COMPRESSION",
    "OTEL_SPAN_EVENT_COUNT_LIMIT",
    "OTEL_SPAN_LINK_COUNT_LIMIT",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
    "OTEL_EXPORTER_OTLP_METRICS_TIMEOUT",
    "OTEL_EXPORTER_OTLP_LOGS_TIMEOUT",
    "OTEL_EXPORTER_OTLP_TRACES_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_METRICS_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_LOGS_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_CLIENT_KEY",
    "OTEL_EXPORTER_OTLP_CLIENT_CERTIFICATE",
    "OTEL_EXPORTER_OTLP_COMPRESSION",
    "OTEL_BLRP_SCHEDULE_DELAY",
    "OTEL_BLRP_EXPORT_TIMEOUT",
    "OTEL_BLRP_MAX_QUEUE_SIZE",
    "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",
    "OTEL_BSP_EXPORT_TIMEOUT",
    "OTEL_BSP_MAX_QUEUE_SIZE",
    "OTEL_BSP_MAX_EXPORT_BATCH_SIZE",
    "OTEL_ATTRIBUTE_COUNT_LIMIT",
    "OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT",
    "OTEL_PROPAGATORS",
];

/// A refused setting. Fixed text: it names the variable, never its value.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingsError {
    /// The named variable is set but not usable.
    #[error("{0} is not usable")]
    Invalid(&'static str),
    /// The named variable is not supported by this exporter.
    #[error("{0} is not supported: {1}")]
    Unsupported(&'static str, &'static str),
    /// The headers file could not be read or parsed.
    #[error("VPAY_OTEL_HEADERS_FILE could not be read or is not `Name: value` lines")]
    HeadersFile,
    /// Headers would travel over plain http to another host.
    #[error("headers are only sent over https (or http to this machine)")]
    HeadersNeedHttps,
}

impl vpay_core::error::Classify for SettingsError {
    fn category(&self) -> vpay_core::error::Category {
        vpay_core::error::Category::Configuration
    }
}

/// Which signals go out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Signals {
    /// Spans.
    pub traces: bool,
    /// Metrics.
    pub metrics: bool,
    /// Logs.
    pub logs: bool,
}

/// What identifies the process in every signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    /// `service.name`.
    pub service_name: String,
    /// `service.version`: the release.
    pub service_version: String,
    /// `deployment.environment.name`, when set.
    pub environment: Option<String>,
    /// `service.instance.id`: `OTEL_RESOURCE_ATTRIBUTES`' value, else `HOSTNAME` (a pod's name), else a UUID made at
    /// start. Always set by `from_lookup`, so two replicas never share an identity.
    pub instance: Option<String>,
}

/// The configuration of the exporter.
#[derive(Clone)]
pub struct Settings {
    /// The OTLP/HTTP URL of the traces; `None` with no traces endpoint or base, and then no spans go out.
    pub traces_url: Option<Url>,
    /// The metrics URL, as above.
    pub metrics_url: Option<Url>,
    /// The logs URL, as above.
    pub logs_url: Option<Url>,
    /// Which signals go out: a signal is on when it has a URL and is not `none`.
    pub signals: Signals,
    /// The PEM of the CA bundle that is the only trust for an `https` collector, from
    /// `OTEL_EXPORTER_OTLP_CERTIFICATE`'s file; `None` is the platform's roots.
    pub ca_pem: Option<Vec<u8>>,
    /// Request headers, from the file. Never printed.
    pub headers: Vec<(String, String)>,
    /// The process's identity.
    pub resource: Resource,
    /// The share of traces kept, `0.0..=1.0`, decided from the trace id at the root.
    pub sample_ratio: f64,
    /// How often metrics go out.
    pub metric_interval: Duration,
    /// How often spans and logs are flushed when a batch is not full.
    pub batch_delay: Duration,
    /// The timeout of one export request.
    pub timeout: Duration,
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("signals", &self.signals)
            .field("headers", &format_args!("<{} hidden>", self.headers.len()))
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

fn millis(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u64,
) -> Result<Duration, SettingsError> {
    match get(name) {
        None => Ok(Duration::from_millis(default)),
        Some(text) => match text.trim().parse::<u64>() {
            Ok(ms) if ms > 0 => Ok(Duration::from_millis(ms)),
            _ => Err(SettingsError::Invalid(name)),
        },
    }
}

fn url_of(
    base: Option<&str>,
    full: Option<String>,
    path: &str,
    full_name: &'static str,
) -> Result<Option<Url>, SettingsError> {
    let (text, appended) = match (full, base) {
        (Some(full), _) => (full, false),
        (None, Some(base)) => (base.to_owned(), true),
        (None, None) => return Ok(None),
    };
    let mut url = Url::parse(text.trim()).map_err(|_| SettingsError::Invalid(full_name))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(SettingsError::Invalid(full_name));
    }
    // A URL with a user or a password would end up in a connection error; refuse it here.
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
        return Err(SettingsError::Invalid(full_name));
    }
    if appended {
        let joined = format!("{}{path}", url.path().trim_end_matches('/'));
        url.set_path(&joined);
    }
    Ok(Some(url))
}

fn is_this_machine(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => name == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// `deployment.environment.name=prod,service.instance.id=pod-1` into a map: only the keys this crate sends.
fn resource_attributes(text: &str) -> BTreeMap<&'static str, String> {
    const KEYS: [&str; 3] = [
        "deployment.environment.name",
        "deployment.environment",
        "service.instance.id",
    ];
    text.split(',')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(_, v)| !v.trim().is_empty())
        .filter_map(|(k, v)| {
            let k = k.trim();
            KEYS.iter()
                .find(|allowed| **allowed == k)
                .map(|allowed| (*allowed, v.trim().to_owned()))
        })
        .collect()
}

fn signal_on(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<bool, SettingsError> {
    match get(name).as_deref().map(str::trim) {
        None | Some("otlp") => Ok(true),
        Some("none") => Ok(false),
        Some(_) => Err(SettingsError::Invalid(name)),
    }
}

impl Settings {
    /// The settings `lookup` gives, `None` when telemetry is off: no endpoint, or `OTEL_SDK_DISABLED=true`.
    /// `version` is the release, `service.version`.
    ///
    /// # Errors
    ///
    /// A [`SettingsError`] naming the variable that is set and not usable.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        version: &str,
        read_file: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, SettingsError> {
        let get = |name: &str| lookup(name).filter(|v| !v.trim().is_empty());
        if get("OTEL_SDK_DISABLED").is_some_and(|v| v.trim().eq_ignore_ascii_case("true")) {
            return Ok(None);
        }
        let base = get("OTEL_EXPORTER_OTLP_ENDPOINT");
        let traces = get("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT");
        let metrics = get("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT");
        let logs = get("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT");
        if base.is_none() && traces.is_none() && metrics.is_none() && logs.is_none() {
            return Ok(None);
        }
        match get("OTEL_EXPORTER_OTLP_PROTOCOL").as_deref().map(str::trim) {
            None | Some("http/json") => {}
            Some(_) => {
                return Err(SettingsError::Unsupported(
                    "OTEL_EXPORTER_OTLP_PROTOCOL",
                    "only http/json",
                ));
            }
        }
        if get("OTEL_EXPORTER_OTLP_HEADERS").is_some() {
            return Err(SettingsError::Unsupported(
                "OTEL_EXPORTER_OTLP_HEADERS",
                "use VPAY_OTEL_HEADERS_FILE",
            ));
        }
        for name in UNSUPPORTED {
            if get(name).is_some() {
                return Err(SettingsError::Unsupported(
                    name,
                    "not supported by this exporter; unset it",
                ));
            }
        }
        for name in [
            "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
            "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL",
            "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL",
        ] {
            if get(name).is_some_and(|v| v.trim() != "http/json") {
                return Err(SettingsError::Unsupported(name, "only http/json"));
            }
        }
        let base = base.as_deref();
        let traces_url = url_of(
            base,
            traces,
            "/v1/traces",
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        )?;
        let metrics_url = url_of(
            base,
            metrics,
            "/v1/metrics",
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
        )?;
        let logs_url = url_of(base, logs, "/v1/logs", "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT")?;
        let headers = match get("VPAY_OTEL_HEADERS_FILE") {
            None => Vec::new(),
            Some(path) => {
                let text = read_file(&path).ok_or(SettingsError::HeadersFile)?;
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(|line| {
                        line.split_once(':')
                            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                            .filter(|(k, _)| !k.is_empty())
                            .ok_or(SettingsError::HeadersFile)
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        if !headers.is_empty()
            && [&traces_url, &metrics_url, &logs_url]
                .into_iter()
                .flatten()
                .any(|url| url.scheme() != "https" && !is_this_machine(url))
        {
            return Err(SettingsError::HeadersNeedHttps);
        }
        let arg = match get("OTEL_TRACES_SAMPLER_ARG") {
            None => None,
            Some(text) => match text.trim().parse::<f64>() {
                Ok(ratio) if (0.0..=1.0).contains(&ratio) => Some(ratio),
                _ => return Err(SettingsError::Invalid("OTEL_TRACES_SAMPLER_ARG")),
            },
        };
        let sample_ratio = match get("OTEL_TRACES_SAMPLER").as_deref().map(str::trim) {
            None => arg.unwrap_or(1.0),
            Some("always_on" | "parentbased_always_on") => 1.0,
            Some("always_off" | "parentbased_always_off") => 0.0,
            Some("traceidratio" | "parentbased_traceidratio") => arg.unwrap_or(1.0),
            Some(_) => return Err(SettingsError::Invalid("OTEL_TRACES_SAMPLER")),
        };
        let ca_pem = match get("OTEL_EXPORTER_OTLP_CERTIFICATE") {
            None => None,
            Some(path) => Some(
                read_file(&path)
                    .map(String::into_bytes)
                    .ok_or(SettingsError::Invalid("OTEL_EXPORTER_OTLP_CERTIFICATE"))?,
            ),
        };
        let attributes = get("OTEL_RESOURCE_ATTRIBUTES")
            .map(|text| resource_attributes(&text))
            .unwrap_or_default();
        let resource = Resource {
            service_name: get("OTEL_SERVICE_NAME")
                .map(|v| v.trim().to_owned())
                .unwrap_or_else(|| "vpay".to_owned()),
            service_version: version.to_owned(),
            environment: attributes
                .get("deployment.environment.name")
                .or_else(|| attributes.get("deployment.environment"))
                .cloned(),
            // Replicas must not share an identity, or a metrics store merges their series: the attribute, else the
            // host name (a pod's name), else an id made now.
            instance: Some(
                attributes
                    .get("service.instance.id")
                    .cloned()
                    .or_else(|| {
                        get("HOSTNAME")
                            .map(|v| v.trim().to_owned())
                            .filter(|v| !v.is_empty())
                    })
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            ),
        };
        let signals = Signals {
            traces: traces_url.is_some() && signal_on(&get, "OTEL_TRACES_EXPORTER")?,
            metrics: metrics_url.is_some() && signal_on(&get, "OTEL_METRICS_EXPORTER")?,
            logs: logs_url.is_some() && signal_on(&get, "OTEL_LOGS_EXPORTER")?,
        };
        Ok(Some(Self {
            traces_url,
            metrics_url,
            logs_url,
            signals,
            ca_pem,
            headers,
            resource,
            sample_ratio,
            metric_interval: millis(&get, "OTEL_METRIC_EXPORT_INTERVAL", 15_000)?,
            batch_delay: millis(&get, "OTEL_BSP_SCHEDULE_DELAY", 5_000)?,
            timeout: millis(&get, "OTEL_EXPORTER_OTLP_TIMEOUT", 10_000)?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn settings(vars: &[(&str, &str)]) -> Result<Option<Settings>, SettingsError> {
        let vars: BTreeMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Settings::from_lookup(
            |name| vars.get(name).cloned(),
            "1.2.3",
            |path| (path == "/headers").then(|| "Authorization: Bearer s3cret\n# c\n".to_owned()),
        )
    }

    #[test]
    fn the_instance_id_is_the_attribute_then_the_hostname_then_a_fresh_uuid() {
        let on = ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318");
        let instance = |extra: &[(&str, &str)]| {
            let mut vars = vec![on];
            vars.extend_from_slice(extra);
            settings(&vars).unwrap().unwrap().resource.instance.unwrap()
        };
        assert_eq!(
            instance(&[
                ("OTEL_RESOURCE_ATTRIBUTES", "service.instance.id=pod-1"),
                ("HOSTNAME", "host-9")
            ]),
            "pod-1"
        );
        assert_eq!(instance(&[("HOSTNAME", " host-9 ")]), "host-9");
        // No attribute and no (or an empty) host name: a UUID, and a different one each start.
        let first = instance(&[("HOSTNAME", "  ")]);
        let second = instance(&[]);
        assert!(uuid::Uuid::parse_str(&first).is_ok(), "{first}");
        assert!(uuid::Uuid::parse_str(&second).is_ok(), "{second}");
        assert_ne!(first, second);
    }

    #[test]
    fn off_unless_an_endpoint_is_set() {
        assert!(settings(&[]).unwrap().is_none());
        assert!(
            settings(&[("OTEL_SERVICE_NAME", "x"), ("OTEL_TRACES_SAMPLER_ARG", "9")])
                .unwrap()
                .is_none()
        );
        assert!(
            settings(&[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318"),
                ("OTEL_SDK_DISABLED", "true")
            ])
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn one_endpoint_gives_the_three_urls_and_the_resource() {
        let s = settings(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318/otlp/"),
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "deployment.environment.name=prod,service.instance.id=pod-1,secret=x",
            ),
            ("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", "http://localhost:9/l"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            s.traces_url.as_ref().unwrap().as_str(),
            "http://localhost:4318/otlp/v1/traces"
        );
        assert_eq!(s.metrics_url.as_ref().unwrap().path(), "/otlp/v1/metrics");
        assert_eq!(
            s.logs_url.as_ref().unwrap().as_str(),
            "http://localhost:9/l"
        );
        assert_eq!(s.resource.service_name, "vpay");
        assert_eq!(s.resource.service_version, "1.2.3");
        assert_eq!(s.resource.environment.as_deref(), Some("prod"));
        assert_eq!(s.resource.instance.as_deref(), Some("pod-1"));
        assert!(s.signals.traces && s.signals.metrics && s.signals.logs);
    }

    #[test]
    fn refusals_name_the_variable_and_never_its_value() {
        let on = ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://c.example");
        for (vars, name) in [
            (
                vec![on, ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc")],
                "OTEL_EXPORTER_OTLP_PROTOCOL",
            ),
            (
                vec![on, ("OTEL_EXPORTER_OTLP_HEADERS", "a=hunter2")],
                "OTEL_EXPORTER_OTLP_HEADERS",
            ),
            (
                vec![("OTEL_EXPORTER_OTLP_ENDPOINT", "https://u:hunter2@c.example")],
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            ),
            (
                vec![("OTEL_EXPORTER_OTLP_ENDPOINT", "ftp://c.example")],
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            ),
            (
                vec![on, ("OTEL_TRACES_SAMPLER_ARG", "2")],
                "OTEL_TRACES_SAMPLER_ARG",
            ),
            (
                vec![on, ("OTEL_METRIC_EXPORT_INTERVAL", "0")],
                "OTEL_METRIC_EXPORT_INTERVAL",
            ),
            (
                vec![on, ("OTEL_LOGS_EXPORTER", "console")],
                "OTEL_LOGS_EXPORTER",
            ),
        ] {
            let error = settings(&vars).unwrap_err();
            let text = error.to_string();
            assert!(text.contains(name), "{text}");
            assert!(!text.contains("hunter2"), "{text}");
        }
    }

    #[test]
    fn headers_come_from_a_file_and_only_over_https_or_this_machine() {
        let with_file = |endpoint: &'static str| {
            settings(&[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint),
                ("VPAY_OTEL_HEADERS_FILE", "/headers"),
            ])
        };
        let s = with_file("https://c.example").unwrap().unwrap();
        assert_eq!(
            s.headers,
            [("Authorization".to_owned(), "Bearer s3cret".to_owned())]
        );
        assert!(!format!("{s:?}").contains("s3cret"));
        assert!(with_file("http://127.0.0.1:4318").unwrap().is_some());
        assert_eq!(
            with_file("http://c.example").unwrap_err(),
            SettingsError::HeadersNeedHttps
        );
        let missing = settings(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://c.example"),
            ("VPAY_OTEL_HEADERS_FILE", "/nowhere"),
        ]);
        assert_eq!(missing.unwrap_err(), SettingsError::HeadersFile);
    }

    #[test]
    fn one_signal_endpoint_alone_turns_on_that_signal_only() {
        let s = settings(&[(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            "http://localhost:4318/v1/traces",
        )])
        .unwrap()
        .unwrap();
        assert!(s.signals.traces && !s.signals.metrics && !s.signals.logs);
        assert!(s.metrics_url.is_none());
    }

    #[test]
    fn what_is_not_honoured_is_refused_and_what_is_honoured_is_read() {
        let on = ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://c.example");
        for name in [
            "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
            "OTEL_EXPORTER_OTLP_CLIENT_KEY",
            "OTEL_BLRP_SCHEDULE_DELAY",
            "OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE",
            "OTEL_EXPORTER_OTLP_TRACES_COMPRESSION",
            "OTEL_EXPORTER_OTLP_METRICS_COMPRESSION",
            "OTEL_EXPORTER_OTLP_LOGS_COMPRESSION",
            "OTEL_SPAN_EVENT_COUNT_LIMIT",
            "OTEL_SPAN_LINK_COUNT_LIMIT",
        ] {
            let error = settings(&[on, (name, "x")]).unwrap_err().to_string();
            assert!(error.contains(name), "{error}");
        }
        assert!(settings(&[on, ("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "grpc")]).is_err());
        assert!(settings(&[on, ("OTEL_TRACES_SAMPLER", "jaeger_remote")]).is_err());
        let off = settings(&[on, ("OTEL_TRACES_SAMPLER", "always_off")])
            .unwrap()
            .unwrap();
        assert!((off.sample_ratio - 0.0).abs() < f64::EPSILON);
        let ratio = settings(&[
            on,
            ("OTEL_TRACES_SAMPLER", "parentbased_traceidratio"),
            ("OTEL_TRACES_SAMPLER_ARG", "0.25"),
        ])
        .unwrap()
        .unwrap();
        assert!((ratio.sample_ratio - 0.25).abs() < f64::EPSILON);
        let with_ca = Settings::from_lookup(
            |n| {
                (n == "OTEL_EXPORTER_OTLP_ENDPOINT")
                    .then(|| "https://c.example".to_owned())
                    .or_else(|| {
                        (n == "OTEL_EXPORTER_OTLP_CERTIFICATE").then(|| "/ca.pem".to_owned())
                    })
            },
            "1",
            |p| (p == "/ca.pem").then(|| "PEM".to_owned()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(with_ca.ca_pem.as_deref(), Some(&b"PEM"[..]));
        let missing = settings(&[on, ("OTEL_EXPORTER_OTLP_CERTIFICATE", "/nowhere")]).unwrap_err();
        assert!(
            missing
                .to_string()
                .contains("OTEL_EXPORTER_OTLP_CERTIFICATE")
        );
    }

    #[test]
    fn a_signal_can_be_turned_off() {
        let s = settings(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318"),
            ("OTEL_LOGS_EXPORTER", "none"),
        ])
        .unwrap()
        .unwrap();
        assert!(s.signals.traces && s.signals.metrics && !s.signals.logs);
    }
}
