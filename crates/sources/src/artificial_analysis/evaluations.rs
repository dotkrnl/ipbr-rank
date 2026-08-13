//! Public Artificial Analysis evaluation leaderboards.
//!
//! These pages render their leaderboard observations into schema.org
//! `Dataset` JSON-LD blocks. Parsing that server-rendered structured data is
//! both more stable and more precise than scraping the presentation table or
//! decoding the surrounding Next.js/RSC payload.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::{AliasIndex, ModelRecord, RawRow, normalize_name};
use scraper::{Html, Selector};
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_html_path,
    read_cached_string, use_cached_html, write_cache_html,
};

const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy)]
enum Transform {
    Identity,
    Percent,
    ComplementPercent,
}

impl Transform {
    fn apply(self, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        match self {
            Self::Identity => Some(value),
            Self::Percent => (0.0..=1.0).contains(&value).then_some(value * 100.0),
            Self::ComplementPercent => (0.0..=1.0)
                .contains(&value)
                .then_some((1.0 - value) * 100.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DatasetMetric {
    dataset_name: &'static str,
    upstream_key: &'static str,
    metric: &'static str,
    transform: Transform,
    rsc_transform: Transform,
    interval: bool,
    rsc_path: &'static [&'static str],
    rsc_lower_path: &'static [&'static str],
    rsc_upper_path: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct EvaluationConfig {
    source_id: &'static str,
    cache_key: &'static str,
    url: &'static str,
    datasets: &'static [DatasetMetric],
}

const GDPVAL_DATASETS: &[DatasetMetric] = &[DatasetMetric {
    dataset_name: "GDPval-AA v2 Leaderboard",
    upstream_key: "gdpvalAaElo",
    metric: "GDPvalAA2",
    transform: Transform::Identity,
    rsc_transform: Transform::Identity,
    interval: true,
    rsc_path: &["gdpval_v2_breakdown", "elo"],
    rsc_lower_path: &["gdpval_v2_breakdown", "lower_95ci"],
    rsc_upper_path: &["gdpval_v2_breakdown", "upper_95ci"],
}];

const CRITPT_DATASETS: &[DatasetMetric] = &[DatasetMetric {
    dataset_name: "CritPt: Score",
    upstream_key: "CritPt",
    metric: "CritPt",
    transform: Transform::Percent,
    rsc_transform: Transform::Percent,
    interval: false,
    rsc_path: &["critpt"],
    rsc_lower_path: &[],
    rsc_upper_path: &[],
}];

const OMNISCIENCE_DATASETS: &[DatasetMetric] = &[
    DatasetMetric {
        dataset_name: "AA-Omniscience Index: Score",
        upstream_key: "omniscienceIndex",
        metric: "AAOmniscienceIndex",
        transform: Transform::Identity,
        rsc_transform: Transform::Identity,
        interval: false,
        rsc_path: &["omniscience"],
        rsc_lower_path: &[],
        rsc_upper_path: &[],
    },
    DatasetMetric {
        dataset_name: "AA-Omniscience Accuracy",
        upstream_key: "omniscienceAccuracy",
        metric: "AAOmniscienceAccuracy",
        transform: Transform::Percent,
        rsc_transform: Transform::Percent,
        interval: false,
        rsc_path: &["omniscience_breakdown", "total", "accuracy"],
        rsc_lower_path: &[],
        rsc_upper_path: &[],
    },
    DatasetMetric {
        dataset_name: "AA-Omniscience Hallucination Rate",
        upstream_key: "omniscienceHallucinationRate",
        metric: "AAOmniscienceNonHallucination",
        transform: Transform::ComplementPercent,
        rsc_transform: Transform::Percent,
        interval: false,
        // The RSC object publishes the already-oriented non-hallucination
        // rate, whereas JSON-LD publishes hallucination rate.
        rsc_path: &["omniscience_breakdown", "total", "non_hallucination_rate"],
        rsc_lower_path: &[],
        rsc_upper_path: &[],
    },
];

const ENTERPRISE_OPS_DATASETS: &[DatasetMetric] = &[DatasetMetric {
    dataset_name: "EnterpriseOps-Gym-AA: Score",
    upstream_key: "EnterpriseOps-Gym-AA",
    metric: "EnterpriseOpsGymAA",
    transform: Transform::Percent,
    rsc_transform: Transform::Percent,
    interval: false,
    rsc_path: &["enterprise_ops_gym_breakdown", "summary", "success_rate"],
    rsc_lower_path: &[],
    rsc_upper_path: &[],
}];

const AUTOMATION_BENCH_DATASETS: &[DatasetMetric] = &[DatasetMetric {
    dataset_name: "AutomationBench-AA: Score",
    upstream_key: "automationBenchScore",
    metric: "AutomationBenchAA",
    transform: Transform::Percent,
    rsc_transform: Transform::Percent,
    interval: false,
    rsc_path: &["automation_bench_breakdown", "summary", "partial_score"],
    rsc_lower_path: &[],
    rsc_upper_path: &[],
}];

// AA re-implements IBM's ITBench SRE track: Kubernetes incident root-cause
// analysis scored as average precision at full recall. JSON-LD and RSC both
// publish the score as a fraction.
const ITBENCH_DATASETS: &[DatasetMetric] = &[DatasetMetric {
    dataset_name: "ITBench-AA: Score",
    upstream_key: "ITBench-AA",
    metric: "ITBenchAA",
    transform: Transform::Percent,
    rsc_transform: Transform::Percent,
    interval: false,
    rsc_path: &["it_bench_sre"],
    rsc_lower_path: &[],
    rsc_upper_path: &[],
}];

const GDPVAL_CONFIG: EvaluationConfig = EvaluationConfig {
    source_id: "aa_gdpval_v2",
    cache_key: "aa_gdpval_v2",
    url: "https://artificialanalysis.ai/evaluations/gdpval-aa",
    datasets: GDPVAL_DATASETS,
};

const CRITPT_CONFIG: EvaluationConfig = EvaluationConfig {
    source_id: "aa_critpt",
    cache_key: "aa_critpt",
    url: "https://artificialanalysis.ai/evaluations/critpt",
    datasets: CRITPT_DATASETS,
};

const OMNISCIENCE_CONFIG: EvaluationConfig = EvaluationConfig {
    source_id: "aa_omniscience",
    cache_key: "aa_omniscience",
    url: "https://artificialanalysis.ai/evaluations/omniscience",
    datasets: OMNISCIENCE_DATASETS,
};

const ENTERPRISE_OPS_CONFIG: EvaluationConfig = EvaluationConfig {
    source_id: "aa_enterprise_ops_gym",
    cache_key: "aa_enterprise_ops_gym",
    url: "https://artificialanalysis.ai/evaluations/enterprise-ops-gym-aa",
    datasets: ENTERPRISE_OPS_DATASETS,
};

const AUTOMATION_BENCH_CONFIG: EvaluationConfig = EvaluationConfig {
    source_id: "aa_automation_bench",
    cache_key: "aa_automation_bench",
    url: "https://artificialanalysis.ai/evaluations/automationbench-aa",
    datasets: AUTOMATION_BENCH_DATASETS,
};

const ITBENCH_CONFIG: EvaluationConfig = EvaluationConfig {
    source_id: "aa_itbench",
    cache_key: "aa_itbench",
    url: "https://artificialanalysis.ai/evaluations/itbench-aa",
    datasets: ITBENCH_DATASETS,
};

macro_rules! evaluation_source {
    ($name:ident, $config:ident) => {
        #[derive(Debug, Default, Clone, Copy)]
        pub struct $name;

        #[async_trait::async_trait]
        impl Source for $name {
            fn id(&self) -> &str {
                $config.source_id
            }

            fn cache_key(&self) -> &str {
                $config.cache_key
            }

            fn cache_paths(&self, cache_dir: &Path) -> Vec<PathBuf> {
                vec![cache_html_path(cache_dir, self.cache_key())]
            }

            fn status(&self) -> VerificationStatus {
                VerificationStatus::Verified
            }

            fn required_secret(&self) -> Option<crate::SecretRef> {
                None
            }

            fn cache_ttl(&self) -> Duration {
                CACHE_TTL
            }

            async fn fetch(
                &self,
                http: &dyn Http,
                opts: FetchOptions<'_>,
                _secrets: &SecretStore,
            ) -> Result<Vec<RawRow>, SourceError> {
                fetch_evaluation(http, opts, $config).await
            }
        }
    };
}

evaluation_source!(AaGdpvalV2Source, GDPVAL_CONFIG);
evaluation_source!(AaCritPtSource, CRITPT_CONFIG);
evaluation_source!(AaOmniscienceSource, OMNISCIENCE_CONFIG);
evaluation_source!(AaEnterpriseOpsGymSource, ENTERPRISE_OPS_CONFIG);
evaluation_source!(AaAutomationBenchSource, AUTOMATION_BENCH_CONFIG);
evaluation_source!(AaItBenchSource, ITBENCH_CONFIG);

async fn fetch_evaluation(
    http: &dyn Http,
    opts: FetchOptions<'_>,
    config: EvaluationConfig,
) -> Result<Vec<RawRow>, SourceError> {
    if use_cached_html(opts, config.cache_key, CACHE_TTL) {
        let Some(dir) = opts.cache_dir else {
            return Err(SourceError::CacheMiss(format!(
                "{} requires --cache in --offline mode",
                config.source_id
            )));
        };
        let html = read_cached_string(&cache_html_path(dir, config.cache_key))?;
        return parse_evaluation_rows(&html, config);
    }

    let html = http
        .get_text(config.url, &[("User-Agent", "ipbr-rank")])
        .await?;
    let rows = parse_evaluation_rows(&html, config)?;
    if let Some(dir) = opts.cache_dir {
        // Only replace the last known-good payload after the live response
        // passes both the JSON-LD schema gate and full RSC coverage checks.
        write_cache_html(dir, config.cache_key, &html)?;
    }
    Ok(rows)
}

#[derive(Debug)]
struct ParsedScore {
    mid: f64,
    lower: Option<f64>,
    upper: Option<f64>,
}

#[derive(Debug)]
struct PendingRow {
    model_name: String,
    label: String,
    details_url: Option<String>,
    revision: ModelRevision,
    fields: BTreeMap<String, Value>,
    field_transports: BTreeMap<String, ObservationTransport>,
    hybrid_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PendingKey {
    model: String,
    effort: AaEffort,
}

#[derive(Debug, Clone)]
struct StableModelIdentity {
    key: String,
    output_name: String,
    catalog_match: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ModelRevision {
    explicit_version: u32,
    release_date: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ObservationTransport {
    JsonLd,
    Rsc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AaEffort {
    Max,
    XHigh,
    High,
    Thinking,
    Medium,
    Default,
    Low,
    NonReasoning,
}

impl AaEffort {
    fn from_label(label: &str) -> Self {
        let normalized = normalize_name(label);
        let has_token = |needle: &str| normalized.split_whitespace().any(|token| token == needle);
        if normalized.contains("non reasoning") {
            Self::NonReasoning
        } else if has_token("instant") || has_token("minimal") || has_token("low") {
            Self::Low
        } else if has_token("max") {
            Self::Max
        } else if has_token("xhigh") {
            Self::XHigh
        } else if has_token("high") {
            Self::High
        } else if has_token("medium") {
            Self::Medium
        } else if has_token("thinking") || has_token("reasoning") || has_token("adaptive") {
            Self::Thinking
        } else {
            Self::Default
        }
    }
}

impl PendingRow {
    fn new(
        model_name: String,
        label: &str,
        details_url: Option<String>,
        revision: ModelRevision,
    ) -> Self {
        Self {
            model_name,
            label: label.to_string(),
            details_url,
            revision,
            fields: BTreeMap::new(),
            field_transports: BTreeMap::new(),
            hybrid_fallback: is_hybrid_fallback_label(label),
        }
    }

    /// Returns whether observations from this revision belong to the selected
    /// row. A newer revision atomically replaces the older one so component
    /// metrics cannot be accidentally mixed across releases.
    fn select_revision(
        &mut self,
        model_name: &str,
        label: &str,
        details_url: Option<&str>,
        revision: ModelRevision,
    ) -> bool {
        if revision < self.revision {
            return false;
        }
        if revision > self.revision {
            self.model_name = model_name.to_string();
            self.label = label.to_string();
            self.details_url = details_url.map(ToOwned::to_owned);
            self.revision = revision;
            self.fields.clear();
            self.field_transports.clear();
            self.hybrid_fallback = is_hybrid_fallback_label(label);
            return true;
        }

        if self.details_url.is_none() {
            self.details_url = details_url.map(ToOwned::to_owned);
        }
        merge_identity_label(self, label);
        true
    }

    fn merge_field(&mut self, key: String, value: Value, transport: ObservationTransport) {
        if self
            .field_transports
            .get(&key)
            .is_none_or(|existing| transport > *existing)
        {
            self.fields.insert(key.clone(), value);
            self.field_transports.insert(key, transport);
        }
    }
}

fn stable_model_identity(
    label: &str,
    details_url: Option<&str>,
    alias_records: &[ModelRecord],
    alias_index: &AliasIndex<'_>,
) -> StableModelIdentity {
    let slug = details_url.and_then(details_model_slug);
    let vendor = infer_vendor(label);
    // The details URL is the stable upstream identity; display labels are
    // mutable and only serve as a fallback when no model slug is available.
    let catalog_index = slug
        .and_then(|slug| alias_index.lookup_exact(slug, vendor))
        .or_else(|| alias_index.lookup_exact(label, vendor));
    if let Some(index) = catalog_index {
        let canonical = alias_records[index].canonical_id.clone();
        return StableModelIdentity {
            key: format!("canonical:{canonical}"),
            output_name: canonical,
            catalog_match: true,
        };
    }
    if let Some(slug) = slug {
        return StableModelIdentity {
            key: format!("slug:{}", normalize_name(slug)),
            output_name: slug.to_string(),
            catalog_match: false,
        };
    }
    StableModelIdentity {
        key: format!("label:{}", normalize_name(label)),
        output_name: label.to_string(),
        catalog_match: false,
    }
}

fn details_model_slug(url: &str) -> Option<&str> {
    let slug = url
        .split_once("/models/")
        .map(|(_, slug)| slug)
        .or_else(|| url.strip_prefix("models/"))?
        .split(['?', '#'])
        .next()?
        .trim_matches('/');
    (!slug.is_empty()).then_some(slug)
}

fn model_revision(label: &str, item: &Value) -> ModelRevision {
    let explicit_version = normalize_name(label)
        .split_whitespace()
        .filter_map(|token| token.strip_prefix('v'))
        .filter_map(|version| version.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    let release_date = item
        .get("release_date")
        .or_else(|| item.get("releaseDate"))
        .and_then(Value::as_str)
        .map(|date| {
            date.chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|digits| digits.parse::<u32>().ok())
        .unwrap_or(0);
    ModelRevision {
        explicit_version,
        release_date,
    }
}

fn parse_evaluation_rows(html: &str, config: EvaluationConfig) -> Result<Vec<RawRow>, SourceError> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(r#"script[type="application/ld+json"]"#)
        .map_err(|err| SourceError::Parse(format!("invalid JSON-LD selector: {err}")))?;
    let metrics_by_dataset: BTreeMap<&str, &DatasetMetric> = config
        .datasets
        .iter()
        .map(|metric| (metric.dataset_name, metric))
        .collect();
    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut pending: BTreeMap<PendingKey, PendingRow> = BTreeMap::new();
    let mut matched_datasets = BTreeSet::new();

    for script in document.select(&selector) {
        let raw = script.inner_html();
        let Ok(payload) = serde_json::from_str::<Value>(&raw) else {
            // Pages can carry unrelated JSON-LD blocks. A malformed unrelated
            // block must not hide a valid leaderboard block later in the page.
            continue;
        };
        for dataset in dataset_objects(&payload) {
            if dataset.get("@type").and_then(Value::as_str) != Some("Dataset") {
                continue;
            }
            let Some(name) = dataset.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(spec) = metrics_by_dataset.get(name).copied() else {
                continue;
            };
            matched_datasets.insert(spec.dataset_name);
            let Some(items) = dataset.get("data").and_then(Value::as_array) else {
                return Err(SourceError::Parse(format!(
                    "{} dataset {name:?} missing data[]",
                    config.source_id
                )));
            };
            for item in items {
                let Some(label) = item.get("label").and_then(Value::as_str) else {
                    continue;
                };
                let Some(score) = item
                    .get(spec.upstream_key)
                    .and_then(|value| parse_score(value, spec.interval))
                else {
                    continue;
                };
                if !score.mid.is_finite() {
                    continue;
                }
                let Some(mid) = spec.transform.apply(score.mid) else {
                    continue;
                };

                let details_url = item
                    .get("detailsUrl")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let identity = stable_model_identity(
                    label,
                    details_url.as_deref(),
                    &alias_records,
                    &alias_index,
                );
                let revision = model_revision(label, item);
                let key = PendingKey {
                    model: identity.key,
                    effort: AaEffort::from_label(label),
                };
                let row = pending.entry(key).or_insert_with(|| {
                    PendingRow::new(
                        identity.output_name.clone(),
                        label,
                        details_url.clone(),
                        revision,
                    )
                });
                if !row.select_revision(
                    &identity.output_name,
                    label,
                    details_url.as_deref(),
                    revision,
                ) {
                    continue;
                }

                let metric = spec.metric.to_string();
                row.merge_field(
                    metric.clone(),
                    Value::from(mid),
                    ObservationTransport::JsonLd,
                );
                if let Some(lower) = score.lower.and_then(|value| spec.transform.apply(value)) {
                    row.merge_field(
                        format!("{metric}CILow"),
                        Value::from(lower),
                        ObservationTransport::JsonLd,
                    );
                }
                if let Some(upper) = score.upper.and_then(|value| spec.transform.apply(value)) {
                    row.merge_field(
                        format!("{metric}CIHigh"),
                        Value::from(upper),
                        ObservationTransport::JsonLd,
                    );
                }
            }
        }
    }

    if matched_datasets.len() < config.datasets.len() {
        return Err(SourceError::Parse(format!(
            "{} found {}/{} expected JSON-LD datasets",
            config.source_id,
            matched_datasets.len(),
            config.datasets.len()
        )));
    }
    if pending.is_empty() {
        return Err(SourceError::Parse(format!(
            "{} contained no parseable leaderboard observations",
            config.source_id
        )));
    }

    // JSON-LD deliberately caps each chart at 20 rows. The streamed model
    // objects carry the complete observations used to render those charts;
    // supplementing from them matters when two charts sort differently (for
    // example Omniscience accuracy versus non-hallucination). JSON-LD remains
    // the schema gate and source of the visible raw labels.
    merge_rsc_rows(html, config, &alias_records, &alias_index, &mut pending)?;

    Ok(pending
        .into_values()
        .map(|mut row| {
            row.fields.insert(
                "UpstreamModelLabel".to_string(),
                Value::from(row.label.clone()),
            );
            if let Some(details_url) = row.details_url {
                row.fields
                    .insert("DetailsUrl".to_string(), Value::from(details_url));
            }
            if row.hybrid_fallback {
                let fallback_note = super::automatic_fallback_note(&row.label);
                row.fields.insert(
                    "UpstreamModelFallback".to_string(),
                    Value::from("Upstream label discloses automatic product fallback"),
                );
                for spec in config.datasets {
                    if row.fields.contains_key(spec.metric) {
                        row.fields.insert(
                            format!("{}__evidence_note", spec.metric),
                            Value::from(fallback_note.clone()),
                        );
                    }
                }
            }
            RawRow {
                source_id: config.source_id.to_string(),
                model_name: row.model_name,
                vendor_hint: infer_vendor(&row.label).map(ToOwned::to_owned),
                fields: row.fields,
            }
        })
        .collect())
}

fn merge_rsc_rows(
    html: &str,
    config: EvaluationConfig,
    alias_records: &[ModelRecord],
    alias_index: &AliasIndex<'_>,
    pending: &mut BTreeMap<PendingKey, PendingRow>,
) -> Result<(), SourceError> {
    // `additional_text` is present on every streamed model object, but AA does
    // not guarantee object-key order. Find the discriminator wherever it
    // appears, then balance backward to the containing object's opening brace.
    const MODEL_OBJECT_ANCHOR: &str = r#"\"additional_text\":"#;
    let mut cursor = 0usize;
    let mut anchors = 0usize;
    let mut parsed_objects = 0usize;
    let mut useful_observations = 0usize;
    while let Some(relative) = html[cursor..].find(MODEL_OBJECT_ANCHOR) {
        let anchor = cursor + relative;
        anchors += 1;
        let Some(start) = find_balanced_object_start(html, anchor) else {
            cursor = anchor + MODEL_OBJECT_ANCHOR.len();
            continue;
        };
        let Some(end) = find_balanced_object_end(html, start) else {
            cursor = anchor + MODEL_OBJECT_ANCHOR.len();
            continue;
        };
        cursor = end + 1;
        let decoded = unescape_rsc_json(&html[start..=end]);
        let Ok(item) = serde_json::from_str::<Value>(&decoded) else {
            continue;
        };
        if !item
            .as_object()
            .is_some_and(|object| object.contains_key("additional_text"))
        {
            continue;
        }
        parsed_objects += 1;

        let Some(label) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let details_url = item
            .get("slug")
            .and_then(Value::as_str)
            .map(|slug| format!("/models/{slug}"));
        let identity =
            stable_model_identity(label, details_url.as_deref(), alias_records, alias_index);
        let key = PendingKey {
            model: identity.key.clone(),
            effort: AaEffort::from_label(label),
        };

        // RSC includes hundreds of historical and untracked models. Retain
        // every row visible in the official JSON-LD chart plus any additional
        // row that maps to this ranking's current model catalog. This recovers
        // complete component coverage without turning routine scoring into a
        // fuzzy-match pass over the entire AA archive.
        if !pending.contains_key(&key) && !identity.catalog_match {
            continue;
        }

        let observations: Vec<(&DatasetMetric, f64, Option<f64>, Option<f64>)> = config
            .datasets
            .iter()
            .filter_map(|spec| {
                let mid = number_at_path(&item, spec.rsc_path)?;
                mid.is_finite().then(|| {
                    (
                        spec,
                        mid,
                        number_at_path(&item, spec.rsc_lower_path),
                        number_at_path(&item, spec.rsc_upper_path),
                    )
                })
            })
            .collect();
        if observations.is_empty() {
            continue;
        }

        let revision = model_revision(label, &item);
        let row = pending.entry(key).or_insert_with(|| {
            PendingRow::new(
                identity.output_name.clone(),
                label,
                details_url.clone(),
                revision,
            )
        });
        if !row.select_revision(
            &identity.output_name,
            label,
            details_url.as_deref(),
            revision,
        ) {
            continue;
        }
        for (spec, mid, lower, upper) in observations {
            let Some(mid) = spec.rsc_transform.apply(mid) else {
                continue;
            };
            useful_observations += 1;
            let metric = spec.metric.to_string();
            row.merge_field(metric.clone(), Value::from(mid), ObservationTransport::Rsc);
            if let Some(lower) = lower.and_then(|value| spec.rsc_transform.apply(value)) {
                row.merge_field(
                    format!("{metric}CILow"),
                    Value::from(lower),
                    ObservationTransport::Rsc,
                );
            }
            if let Some(upper) = upper.and_then(|value| spec.rsc_transform.apply(value)) {
                row.merge_field(
                    format!("{metric}CIHigh"),
                    Value::from(upper),
                    ObservationTransport::Rsc,
                );
            }
        }
    }

    if anchors == 0 {
        return Err(SourceError::Parse(format!(
            "{} missing required RSC model observations",
            config.source_id
        )));
    }
    if parsed_objects == 0 {
        return Err(SourceError::Parse(format!(
            "{} found RSC model anchors but could not decode any model object",
            config.source_id
        )));
    }
    if useful_observations == 0 {
        return Err(SourceError::Parse(format!(
            "{} decoded RSC model objects but found no useful leaderboard observations",
            config.source_id
        )));
    }
    Ok(())
}

fn find_balanced_object_start(html: &str, anchor: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    if anchor > bytes.len() {
        return None;
    }

    let mut depth = 0usize;
    let mut in_string = false;
    for index in (0..anchor).rev() {
        let byte = bytes[index];
        if byte == b'"' {
            let backslashes = bytes[..index]
                .iter()
                .rev()
                .take_while(|byte| **byte == b'\\')
                .count();
            // RSC embeds JSON inside a JavaScript string. JSON delimiters are
            // encoded as \" (one slash modulo four), while a literal quote in
            // JSON string content is encoded as \\\" (three modulo four).
            if backslashes % 4 == 1 {
                in_string = !in_string;
            }
            continue;
        }
        if in_string {
            continue;
        }
        match byte {
            b'}' => depth += 1,
            b'{' if depth == 0 => return Some(index),
            b'{' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn find_balanced_object_end(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut backslashes = 0usize;
    for (offset, byte) in bytes.get(start..)?.iter().enumerate() {
        if *byte == b'\\' {
            backslashes += 1;
            continue;
        }
        if *byte == b'"' {
            // RSC embeds JSON inside a JavaScript string. JSON delimiters are
            // encoded as \" (one slash modulo four), while a literal quote in
            // JSON string content is encoded as \\\" (three modulo four).
            if backslashes % 4 == 1 {
                in_string = !in_string;
            }
            backslashes = 0;
            continue;
        }
        backslashes = 0;
        if in_string {
            continue;
        }
        match *byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Unwinds the JSON-string layer used by Next.js streamed RSC chunks.
fn unescape_rsc_json(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    let bytes = escaped.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'\\' {
            match bytes[i + 1] {
                b'"' => {
                    out.push('"');
                    i += 2;
                    continue;
                }
                b'\\' => {
                    out.push('\\');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn number_at_path(value: &Value, path: &[&str]) -> Option<f64> {
    if path.is_empty() {
        return None;
    }
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    number_like(current)
}

fn dataset_objects(payload: &Value) -> Vec<&Value> {
    match payload {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .get("@graph")
            .and_then(Value::as_array)
            .map(|items| items.iter().collect())
            .unwrap_or_else(|| vec![payload]),
        _ => Vec::new(),
    }
}

fn parse_score(value: &Value, interval: bool) -> Option<ParsedScore> {
    if !interval {
        return number_like(value).map(|mid| ParsedScore {
            mid,
            lower: None,
            upper: None,
        });
    }
    let values = value.as_array()?;
    let named = |name: &str| {
        values.iter().find_map(|item| {
            (item.get("name").and_then(Value::as_str) == Some(name))
                .then(|| item.get("value").and_then(number_like))
                .flatten()
        })
    };
    Some(ParsedScore {
        mid: named("mid")?,
        lower: named("lower"),
        upper: named("upper"),
    })
}

fn number_like(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

/// Upstream sometimes evaluates a served product with its automatic fallback.
/// The fallback disclosure remains sticky and is attached to each primary
/// observation as provenance because the routed endpoint is the ranked unit.
pub fn is_hybrid_fallback_label(label: &str) -> bool {
    normalize_name(label).contains("fallback")
}

/// Fallback is sticky for a model identity. The page can abbreviate a label in
/// one transport while spelling out the fallback disclosure in another; once
/// either transport identifies the row as routed, its provenance remains
/// attached even when another transport abbreviates the label.
fn merge_identity_label(row: &mut PendingRow, label: &str) {
    let incoming_hybrid = is_hybrid_fallback_label(label);
    if incoming_hybrid && !row.hybrid_fallback {
        row.hybrid_fallback = true;
    }

    // Prefer a disclosed fallback label over an abbreviated one, then the more
    // explicit spelling within the same disclosure class.
    if (incoming_hybrid && !is_hybrid_fallback_label(&row.label))
        || (incoming_hybrid == is_hybrid_fallback_label(&row.label)
            && label.len() > row.label.len())
    {
        row.label = label.to_string();
    }
}

fn infer_vendor(label: &str) -> Option<&'static str> {
    let normalized = normalize_name(label);
    let first = normalized.split_whitespace().next()?;
    match first {
        "claude" => Some("anthropic"),
        "gpt" | "chatgpt" | "o1" | "o3" | "o4" => Some("openai"),
        "gemini" | "gemma" => Some("google"),
        "grok" => Some("xai"),
        "kimi" => Some("moonshot"),
        "glm" => Some("zai"),
        "qwen" => Some("alibaba"),
        "deepseek" => Some("deepseek"),
        "mistral" | "ministral" => Some("mistral"),
        "minimax" => Some("minimax"),
        "llama" => Some("meta"),
        "mimo" => Some("xiaomi"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticHtmlHttp(&'static str);

    #[async_trait::async_trait]
    impl Http for StaticHtmlHttp {
        async fn get_json(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<Value, SourceError> {
            panic!("evaluation sources should fetch HTML")
        }

        async fn get_text(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, SourceError> {
            Ok(self.0.to_string())
        }
    }

    fn numeric(row: &RawRow, metric: &str) -> Option<f64> {
        row.fields.get(metric).and_then(number_like)
    }

    fn upstream_label(row: &RawRow) -> &str {
        row.fields
            .get("UpstreamModelLabel")
            .and_then(Value::as_str)
            .expect("parsed rows retain their upstream display label")
    }

    #[test]
    fn parses_gdpval_interval_and_keeps_routed_product_primary() {
        let rows = parse_evaluation_rows(
            include_str!("../../../../data/fixtures/aa_gdpval_v2.html"),
            GDPVAL_CONFIG,
        )
        .expect("GDPval fixture should parse");
        assert_eq!(rows.len(), 3);
        let gpt = rows
            .iter()
            .find(|row| upstream_label(row) == "GPT-5.5 (xhigh)")
            .expect("GPT row");
        assert_eq!(numeric(gpt, "GDPvalAA2"), Some(1493.72));
        assert_eq!(numeric(gpt, "GDPvalAA2CILow"), Some(1477.17));
        assert_eq!(numeric(gpt, "GDPvalAA2CIHigh"), Some(1510.28));

        let fable = rows
            .iter()
            .find(|row| upstream_label(row).contains("Fable"))
            .expect("Fable fallback row should remain visible");
        assert_eq!(numeric(fable, "GDPvalAA2"), Some(1759.6));
        assert_eq!(numeric(fable, "GDPvalAA2CILow"), Some(1740.2));
        assert_eq!(numeric(fable, "GDPvalAA2CIHigh"), Some(1779.0));
        assert_eq!(
            fable
                .fields
                .get("GDPvalAA2__evidence_note")
                .and_then(Value::as_str),
            Some(super::super::automatic_fallback_note(
                "Claude Fable 5 (with fallback)"
            ))
            .as_deref()
        );
        assert!(fable.fields.contains_key("UpstreamModelFallback"));
    }

    #[test]
    fn parses_critpt_fraction_as_percent() {
        let rows = parse_evaluation_rows(
            include_str!("../../../../data/fixtures/aa_critpt.html"),
            CRITPT_CONFIG,
        )
        .expect("CritPt fixture should parse");
        let row = rows
            .iter()
            .find(|row| upstream_label(row) == "GPT-5.5 Pro (xhigh)")
            .expect("GPT-5.5 Pro row");
        assert!((numeric(row, "CritPt").unwrap() - 30.5714285714286).abs() < 1e-10);
    }

    #[test]
    fn joins_omniscience_datasets_and_inverts_hallucination() {
        let rows = parse_evaluation_rows(
            include_str!("../../../../data/fixtures/aa_omniscience.html"),
            OMNISCIENCE_CONFIG,
        )
        .expect("Omniscience fixture should parse");
        assert_eq!(rows.len(), 3);
        let gpt = rows
            .iter()
            .find(|row| upstream_label(row) == "GPT-5.5 (xhigh)")
            .expect("GPT row");
        assert_eq!(numeric(gpt, "AAOmniscienceIndex"), Some(25.25));
        assert!((numeric(gpt, "AAOmniscienceAccuracy").unwrap() - 56.9).abs() < 1e-10);
        assert!((numeric(gpt, "AAOmniscienceNonHallucination").unwrap() - 68.35).abs() < 1e-10);

        let fable = rows
            .iter()
            .find(|row| upstream_label(row).contains("Fable"))
            .expect("Fable row");
        assert_eq!(numeric(fable, "AAOmniscienceAccuracy"), Some(61.35));
        assert_eq!(
            fable
                .fields
                .get("AAOmniscienceAccuracy__evidence_note")
                .and_then(Value::as_str),
            Some(super::super::automatic_fallback_note(
                "Claude Fable 5 (with fallback)"
            ))
            .as_deref()
        );
    }

    #[test]
    fn parses_enterprise_and_automation_scores() {
        let enterprise = parse_evaluation_rows(
            include_str!("../../../../data/fixtures/aa_enterprise_ops_gym.html"),
            ENTERPRISE_OPS_CONFIG,
        )
        .expect("EnterpriseOps fixture should parse");
        let gpt = enterprise
            .iter()
            .find(|row| upstream_label(row) == "GPT-5.5 (xhigh)")
            .expect("GPT EnterpriseOps row");
        assert!((numeric(gpt, "EnterpriseOpsGymAA").unwrap() - 46.64279319606088).abs() < 1e-10);

        let automation = parse_evaluation_rows(
            include_str!("../../../../data/fixtures/aa_automation_bench.html"),
            AUTOMATION_BENCH_CONFIG,
        )
        .expect("AutomationBench fixture should parse");
        let gpt = automation
            .iter()
            .find(|row| upstream_label(row) == "GPT-5.5 (xhigh)")
            .expect("GPT AutomationBench row");
        assert!((numeric(gpt, "AutomationBenchAA").unwrap() - 44.25).abs() < 1e-10);
    }

    #[test]
    fn parses_itbench_fraction_and_keeps_max_effort_identity() {
        let rows = parse_evaluation_rows(
            include_str!("../../../../data/fixtures/aa_itbench.html"),
            ITBENCH_CONFIG,
        )
        .expect("ITBench fixture should parse");
        assert_eq!(rows.len(), 3);

        let sol = rows
            .iter()
            .find(|row| upstream_label(row) == "GPT-5.6 Sol (max)")
            .expect("GPT-5.6 Sol row");
        assert_eq!(sol.model_name, "openai/gpt-5.6-sol");
        assert!((numeric(sol, "ITBenchAA").unwrap() - 56.2146892655367).abs() < 1e-10);

        let opus = rows
            .iter()
            .find(|row| upstream_label(row) == "Claude Opus 4.7 (max)")
            .expect("Claude Opus 4.7 row");
        assert_eq!(opus.model_name, "anthropic/claude-opus-4.7");
        assert!((numeric(opus, "ITBenchAA").unwrap() - 46.6572504708098).abs() < 1e-10);
    }

    #[test]
    fn supports_json_ld_graph_and_string_numbers() {
        let html = r#"<script type="application/ld+json">{
          "@graph": [{"@type":"Dataset","name":"CritPt: Score","data":[{
            "label":"GPT-5.5 (xhigh)","CritPt":"0.25","detailsUrl":"/models/gpt-5-5"
          }]}]
        }</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"GPT-5.5 (xhigh)\",\"slug\":\"gpt-5-5\",\"critpt\":0.25}"])</script>"#;
        let rows = parse_evaluation_rows(html, CRITPT_CONFIG).expect("graph should parse");
        assert_eq!(numeric(&rows[0], "CritPt"), Some(25.0));
    }

    #[test]
    fn fallback_disclosure_is_sticky_across_json_ld_and_rsc_labels() {
        let html = r#"
        <script type="application/ld+json">{
          "@type":"Dataset","name":"GDPval-AA v2 Leaderboard","data":[
            {"label":"Claude Test A","gdpvalAaElo":[
              {"name":"mid","value":1000},{"name":"lower","value":990},{"name":"upper","value":1010}
            ],"detailsUrl":"/models/test-a"},
            {"label":"Claude Test B (with fallback)","gdpvalAaElo":[
              {"name":"mid","value":1100},{"name":"lower","value":1090},{"name":"upper","value":1110}
            ],"detailsUrl":"/models/test-b"}
          ]
        }</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"Claude Test A (with fallback)\",\"slug\":\"test-a\",\"gdpval_v2_breakdown\":{\"elo\":1200,\"lower_95ci\":1190,\"upper_95ci\":1210}}"])</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"Claude Test B\",\"slug\":\"test-b\",\"gdpval_v2_breakdown\":{\"elo\":1300,\"lower_95ci\":1290,\"upper_95ci\":1310}}"])</script>
        "#;
        let rows = parse_evaluation_rows(html, GDPVAL_CONFIG).expect("labels should merge");
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert!(upstream_label(row).contains("fallback"));
            assert!(row.fields.contains_key("GDPvalAA2"));
            assert!(row.fields.contains_key("GDPvalAA2CILow"));
            assert!(row.fields.contains_key("GDPvalAA2CIHigh"));
            assert!(row.fields.contains_key("GDPvalAA2__evidence_note"));
            assert!(row.fields.contains_key("UpstreamModelFallback"));
        }
        let a = rows
            .iter()
            .find(|row| upstream_label(row).contains("Test A"))
            .expect("test A row");
        let b = rows
            .iter()
            .find(|row| upstream_label(row).contains("Test B"))
            .expect("test B row");
        assert_eq!(numeric(a, "GDPvalAA2"), Some(1200.0));
        assert_eq!(numeric(b, "GDPvalAA2"), Some(1300.0));
    }

    #[test]
    fn stable_slug_keeps_efforts_separate_and_selects_latest_revision() {
        let html = r#"
        <script type="application/ld+json">{
          "@type":"Dataset","name":"CritPt: Score","data":[
            {"label":"Grok 4.20 0309 v2 (Reasoning)","CritPt":0.065714,"detailsUrl":"/models/grok-4-20"},
            {"label":"Grok 4.20 0309 (Reasoning)","CritPt":0.06,"detailsUrl":"/models/grok-4-20"},
            {"label":"Grok 4.20 0309 v2 (Non-reasoning)","CritPt":0.03,"detailsUrl":"/models/grok-4-20"},
            {"label":"Grok 4.20 0309 (Non-reasoning)","CritPt":0.02,"detailsUrl":"/models/grok-4-20"}
          ]
        }</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"Grok 4.20 0309 v2 (Reasoning)\",\"slug\":\"grok-4-20\",\"release_date\":\"2026-04-07\",\"critpt\":0.065714}"])</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"Grok 4.20 0309 (Reasoning)\",\"slug\":\"grok-4-20\",\"release_date\":\"2026-03-10\",\"critpt\":0.06}"])</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"Grok 4.20 0309 v2 (Non-reasoning)\",\"slug\":\"grok-4-20\",\"release_date\":\"2026-04-07\",\"critpt\":0.03}"])</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"Grok 4.20 0309 (Non-reasoning)\",\"slug\":\"grok-4-20\",\"release_date\":\"2026-03-10\",\"critpt\":0.02}"])</script>
        "#;

        let rows = parse_evaluation_rows(html, CRITPT_CONFIG).expect("Grok rows should parse");
        assert_eq!(rows.len(), 2, "reasoning efforts remain distinct");
        assert!(
            rows.iter().all(|row| row.model_name == "xai/grok-4.20"),
            "the stable canonical identity should be emitted"
        );
        let reasoning = rows
            .iter()
            .find(|row| {
                let label = upstream_label(row);
                label.contains("Reasoning") && !label.contains("Non-reasoning")
            })
            .expect("reasoning row");
        let non_reasoning = rows
            .iter()
            .find(|row| upstream_label(row).contains("Non-reasoning"))
            .expect("non-reasoning row");
        assert!(upstream_label(reasoning).contains("v2"));
        assert!(upstream_label(non_reasoning).contains("v2"));
        assert!((numeric(reasoning, "CritPt").unwrap() - 6.5714).abs() < 1e-10);
        assert_eq!(numeric(non_reasoning, "CritPt"), Some(3.0));
    }

    #[test]
    fn stable_slug_takes_precedence_over_a_mutable_display_label() {
        let records = crate::embedded_alias_records();
        let index = AliasIndex::build(&records);
        let identity = stable_model_identity(
            "GPT-5.5 (xhigh)",
            Some("/models/claude-opus-4-8"),
            &records,
            &index,
        );

        assert_eq!(identity.output_name, "anthropic/claude-opus-4.8");
        assert!(identity.catalog_match);
    }

    #[test]
    fn fails_when_required_rsc_transport_is_missing() {
        let html = r#"<script type="application/ld+json">{
          "@type":"Dataset","name":"CritPt: Score","data":[{
            "label":"GPT-5.5 (xhigh)","CritPt":0.25,"detailsUrl":"/models/gpt-5-5"
          }]
        }</script>"#;
        let error = parse_evaluation_rows(html, CRITPT_CONFIG)
            .expect_err("JSON-LD alone is a capped leaderboard view");
        assert!(error.to_string().contains("missing required RSC"));
    }

    #[test]
    fn fails_when_rsc_decodes_but_contains_no_useful_metric() {
        let html = r#"<script type="application/ld+json">{
          "@type":"Dataset","name":"CritPt: Score","data":[{
            "label":"GPT-5.5 (xhigh)","CritPt":0.25,"detailsUrl":"/models/gpt-5-5"
          }]
        }</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"GPT-5.5 (xhigh)\",\"slug\":\"gpt-5-5\",\"unrelated_metric\":0.99}"])</script>"#;
        let error = parse_evaluation_rows(html, CRITPT_CONFIG)
            .expect_err("RSC schema drift must not silently truncate coverage");
        assert!(
            error
                .to_string()
                .contains("no useful leaderboard observations")
        );
    }

    #[test]
    fn rsc_object_scanner_ignores_braces_inside_string_fields() {
        let html = r#"<script type="application/ld+json">{
          "@type":"Dataset","name":"CritPt: Score","data":[{
            "label":"GPT-5.5 (xhigh)","CritPt":0.25,"detailsUrl":"/models/gpt-5-5"
          }]
        }</script>
        <script>self.__next_f.push([1,"{\"additional_text\":\"literal } closing and { opening braces\",\"name\":\"GPT-5.5 (xhigh)\",\"slug\":\"gpt-5-5\",\"critpt\":0.25}"])</script>"#;
        let rows = parse_evaluation_rows(html, CRITPT_CONFIG)
            .expect("braces inside a string must not truncate the object");
        assert_eq!(numeric(&rows[0], "CritPt"), Some(25.0));
    }

    #[test]
    fn rsc_object_scanner_allows_reordered_and_nested_leading_fields() {
        let html = r#"<script type="application/ld+json">{
          "@type":"Dataset","name":"CritPt: Score","data":[{
            "label":"GPT-5.5 (xhigh)","CritPt":0.20,"detailsUrl":"/models/gpt-5-5"
          }]
        }</script>
        <script>self.__next_f.push([1,"{\"metadata\":{\"note\":\"literal } and { plus \\\"quote\\\"\"},\"aa_analyst_agent\":null,\"additional_text\":null,\"name\":\"GPT-5.5 (xhigh)\",\"slug\":\"gpt-5-5\",\"critpt\":0.25}"])</script>"#;
        let rows = parse_evaluation_rows(html, CRITPT_CONFIG)
            .expect("model fields before the discriminator should parse");
        assert_eq!(numeric(&rows[0], "CritPt"), Some(25.0));
    }

    #[tokio::test]
    async fn invalid_live_payload_does_not_replace_last_good_cache() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cached = include_str!("../../../../data/fixtures/aa_critpt.html");
        write_cache_html(tmp.path(), CRITPT_CONFIG.cache_key, cached).expect("cache should write");
        let cache_path = cache_html_path(tmp.path(), CRITPT_CONFIG.cache_key);
        let status = std::process::Command::new("touch")
            .args(["-t", "202001010000"])
            .arg(&cache_path)
            .status()
            .expect("touch should run");
        assert!(status.success(), "touch should mark fixture cache stale");

        let invalid_live = r#"<script type="application/ld+json">{
          "@type":"Dataset","name":"CritPt: Score","data":[{
            "label":"GPT-5.5 (xhigh)","CritPt":0.25,"detailsUrl":"/models/gpt-5-5"
          }]
        }</script>"#;
        let error = fetch_evaluation(
            &StaticHtmlHttp(invalid_live),
            FetchOptions {
                cache_dir: Some(tmp.path()),
                offline: false,
            },
            CRITPT_CONFIG,
        )
        .await
        .expect_err("invalid live data must fail validation");

        assert!(error.to_string().contains("missing required RSC"));
        assert_eq!(
            std::fs::read_to_string(cache_path).expect("cache should remain readable"),
            cached
        );
    }

    #[test]
    fn rejects_out_of_range_fraction_scores() {
        let html = r#"<script type="application/ld+json">{
          "@type":"Dataset","name":"CritPt: Score","data":[{
            "label":"GPT-5.5 (xhigh)","CritPt":1.25,"detailsUrl":"/models/gpt-5-5"
          }]
        }</script>
        <script>self.__next_f.push([1,"{\"additional_text\":null,\"name\":\"GPT-5.5 (xhigh)\",\"slug\":\"gpt-5-5\",\"critpt\":1.25}"])</script>"#;
        let error = parse_evaluation_rows(html, CRITPT_CONFIG)
            .expect_err("fraction metrics outside [0,1] must fail");
        assert!(
            error
                .to_string()
                .contains("no parseable leaderboard observations")
        );
    }

    #[test]
    fn fails_loudly_when_expected_dataset_disappears() {
        let error = parse_evaluation_rows(
            r#"<script type="application/ld+json">{"@type":"Dataset","name":"Other","data":[]}</script>"#,
            CRITPT_CONFIG,
        )
        .expect_err("schema drift should fail");
        assert!(error.to_string().contains("expected JSON-LD datasets"));
    }
}
