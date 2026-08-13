use std::collections::BTreeMap;
use std::time::Duration;

pub mod evaluations;

use ipbr_core::{AliasIndex, ModelRecord, RawRow, normalize_name};
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretRef, SecretStore, Source, SourceError, VerificationStatus,
    cache_json_path, read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "artificial_analysis";
const CACHE_KEY: &str = "artificial_analysis_llms";
// TODO(aa-v2-retirement): this legacy `/api/v2/data/*` route is being retired
// 2026-11-04. The documented replacement is the Free/Pro split under
// `/api/v2/language/models`. We intentionally stay on the legacy route until a
// Pro-tier key is available: the Pro route (`/api/v2/language/models`) preserves
// the full evaluations payload, whereas Free (`/api/v2/language/models/free`)
// carries only intelligence/coding/agentic indices and would silently drop ~11
// scored AA metrics (GPQA, HLE, MMLUPro, AIME25, SciCode, IFBench,
// LongContextRecall, Tau2Bench, TerminalBenchHard, LiveCodeBench, Math) plus
// relocate speed/TTFT into a `performance.*` sub-object. With the current
// Free-tier key the Pro route returns 403.
//
// Migration plan once a Pro key is in hand:
//   1. Point `URL` at `https://artificialanalysis.ai/api/v2/language/models`.
//   2. Envelope parsing is already `data[]`-agnostic (siblings like
//      `tier`/`pagination` are ignored), so no change there.
//   3. Add `performance.median_output_tokens_per_second` and
//      `performance.median_time_to_first_token_seconds` to the speed/TTFT
//      `number_at_paths` fallback lists.
//   4. If Pro is paginated like Free (`pagination.total_pages`), follow pages.
//   5. Refresh `data/fixtures/artificial_analysis_llms.json` from the new route
//      and regenerate the golden scoreboard.
const URL: &str = "https://artificialanalysis.ai/api/v2/data/llms/models";

pub(crate) fn automatic_fallback_note(label: &str) -> String {
    format!(
        "Routed-product observation: Artificial Analysis labels this configuration {label:?}; its vendor-automatic fallback is part of the served product."
    )
}

/// Prefer AA's stable slug, except when it has been reused for a newer dated
/// snapshot whose display label maps exactly to a separate catalog record.
/// This keeps historical generic labels on their original products while
/// allowing explicit identities such as DeepSeek V4 Flash 0731 to remain
/// separate from the April V4 Flash release.
fn preferred_aa_model_identity<'a>(
    slug: Option<&'a str>,
    label: Option<&'a str>,
    fallback: Option<&'a str>,
    vendor_hint: Option<&str>,
    alias_records: &[ModelRecord],
    alias_index: &AliasIndex<'_>,
) -> Option<&'a str> {
    let slug_match = slug.and_then(|value| alias_index.lookup_exact(value, vendor_hint));
    let label_match = label.and_then(|value| alias_index.lookup_exact(value, vendor_hint));

    match (slug_match, label_match) {
        (Some(slug_index), Some(label_index))
            if slug_index != label_index
                && canonical_has_dated_snapshot(&alias_records[label_index].canonical_id) =>
        {
            label
        }
        _ => slug.or(label).or(fallback),
    }
}

fn canonical_has_dated_snapshot(canonical_id: &str) -> bool {
    let Some(suffix) = canonical_id.rsplit('-').next() else {
        return false;
    };
    if !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let mmdd = match suffix.len() {
        4 => suffix,
        8 => &suffix[4..],
        _ => return false,
    };
    let month = mmdd[..2].parse::<u8>().ok();
    let day = mmdd[2..].parse::<u8>().ok();
    matches!(month, Some(1..=12)) && matches!(day, Some(1..=31))
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ArtificialAnalysisSource;

#[async_trait::async_trait]
impl Source for ArtificialAnalysisSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        CACHE_KEY
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Verified
    }

    fn required_secret(&self) -> Option<SecretRef> {
        Some(SecretRef::AaApiKey)
    }

    fn cache_ttl(&self) -> Duration {
        Duration::from_secs(10 * 60)
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        let payload = if use_cached_json(opts, self.cache_key(), self.cache_ttl()) {
            let Some(dir) = opts.cache_dir else {
                return Err(SourceError::CacheMiss(format!(
                    "{} requires --cache in --offline mode",
                    self.id()
                )));
            };
            serde_json::from_slice::<Value>(&read_cached_bytes(&cache_json_path(
                dir,
                self.cache_key(),
            ))?)?
        } else {
            let api_key = secrets
                .get(SecretRef::AaApiKey)
                .ok_or_else(|| SourceError::MissingSecret(SOURCE_ID.to_string()))?;
            let headers = [("x-api-key", api_key)];
            let payload = http.get_json(URL, &headers).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_json(dir, self.cache_key(), &payload)?;
            }
            payload
        };
        parse_rows(&payload)
    }
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("Artificial Analysis payload missing data[]".into()))?;

    // AA ships multiple rows per logical model (e.g. default/medium/high/max
    // effort variants) and the alias matcher may collapse them into the same
    // canonical_id. Preserve one row per distinct effort here; the ingest
    // layer intentionally selects the strongest available effort
    // (max → xhigh → high → thinking → medium → default). This sort only
    // keeps equal-effort duplicate ties deterministic.
    let mut sorted: Vec<&Value> = data.iter().collect();
    sorted.sort_by(|a, b| {
        let intelligence = |item: &Value| -> f64 {
            number_at_paths(
                item,
                &[
                    &["evaluations", "artificial_analysis_intelligence_index"],
                    &["evaluations", "intelligence_index"],
                    &["evaluations", "intelligence"],
                ],
            )
            .unwrap_or(0.0)
        };
        intelligence(a)
            .partial_cmp(&intelligence(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut best_by_model: BTreeMap<(String, AaVariantPreference), (f64, RawRow)> = BTreeMap::new();
    for item in sorted {
        // Automatic fallback is part of the served product users invoke, so a
        // disclosed fallback row is direct ranked-product evidence. Preserve
        // that disclosure as an evidence note on every emitted observation.
        let automatic_fallback = item
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.to_ascii_lowercase().contains("fallback"));
        let vendor_hint = item
            .get("model_creator")
            .and_then(|value| value.get("slug"))
            .and_then(Value::as_str)
            .or_else(|| {
                item.get("model_creators")
                    .and_then(|value| value.get("slug"))
                    .and_then(Value::as_str)
            });
        // AA's `id` is a UUID. Prefer the human-readable slug unless AA has
        // reused it for a newer explicitly dated product identity.
        let model_name = preferred_aa_model_identity(
            item.get("slug").and_then(Value::as_str),
            item.get("name").and_then(Value::as_str),
            item.get("id").and_then(Value::as_str),
            vendor_hint,
            &alias_records,
            &alias_index,
        )
        .ok_or_else(|| SourceError::Parse("Artificial Analysis row missing slug/name/id".into()))?;

        let mut fields = BTreeMap::new();
        copy_if_present(&mut fields, "ModelId", item.get("id"));
        copy_if_present(&mut fields, "DisplayName", item.get("name"));
        copy_if_present(&mut fields, "CanonicalSlug", item.get("slug"));

        let intelligence = number_at_paths(
            item,
            &[
                &["evaluations", "intelligence_index"],
                &["evaluations", "intelligence"],
                &["evaluations", "artificial_analysis_intelligence_index"],
                &["intelligence_index"],
                &["intelligence"],
            ],
        );
        if let Some(intelligence) = intelligence {
            fields.insert(
                "ArtificialAnalysisIntelligence".to_string(),
                Value::from(intelligence),
            );
        }

        // GPQA-Diamond and Humanity's Last Exam are distinct evaluations,
        // reported as 0–1 fractions in AA's payload. Preserve them as separate
        // 0–100 observations so downstream scoring can normalize and combine
        // them once. Earlier revisions emitted the same gpqa+hle blend under
        // two metric names, which double-counted a single derived value across
        // the PLAN and GEN groups. A missing component remains genuinely
        // missing instead of changing the meaning of a blend.
        let gpqa = number_at_paths(item, &[&["evaluations", "gpqa"], &["gpqa"]]);
        let hle = number_at_paths(item, &[&["evaluations", "hle"], &["hle"]]);
        if let Some(gpqa) = gpqa {
            fields.insert("GPQA".to_string(), Value::from(gpqa * 100.0));
        }
        if let Some(hle) = hle {
            fields.insert("HLE".to_string(), Value::from(hle * 100.0));
        }

        // AA also publishes per-eval scores under `evaluations.{aime_25,tau2,
        // tau_banking,scicode,ifbench}`. They're 0–1 fractions like gpqa/hle, so we scale to
        // 0–100 and emit as their own metrics. Skipped silently when absent
        // (older runs without these fields, smaller models, etc.)
        if let Some(aime25) = number_at_paths(item, &[&["evaluations", "aime_25"], &["aime_25"]]) {
            fields.insert("AIME25".to_string(), Value::from(aime25 * 100.0));
        }
        if let Some(tau2) = number_at_paths(item, &[&["evaluations", "tau2"], &["tau2"]]) {
            fields.insert("Tau2Bench".to_string(), Value::from(tau2 * 100.0));
        }
        if let Some(tau_banking) =
            number_at_paths(item, &[&["evaluations", "tau_banking"], &["tau_banking"]])
        {
            // AA retained the historical payload key when the public suite
            // upgraded from tau2 to tau3-Banking. Emit the current semantic
            // name for scoring and the old name only for schema compatibility.
            fields.insert("Tau3Banking".to_string(), Value::from(tau_banking * 100.0));
            fields.insert("TauBanking".to_string(), Value::from(tau_banking * 100.0));
        }
        if let Some(scicode) = number_at_paths(item, &[&["evaluations", "scicode"], &["scicode"]]) {
            fields.insert("SciCode".to_string(), Value::from(scicode * 100.0));
        }
        if let Some(ifbench) = number_at_paths(item, &[&["evaluations", "ifbench"], &["ifbench"]]) {
            fields.insert("IFBench".to_string(), Value::from(ifbench * 100.0));
        }
        if let Some(terminalbench_hard) = number_at_paths(
            item,
            &[
                &["evaluations", "terminalbench_hard"],
                &["terminalbench_hard"],
            ],
        ) {
            fields.insert(
                "TerminalBenchHard".to_string(),
                Value::from(terminalbench_hard * 100.0),
            );
        }
        if let Some(terminalbench21) = number_at_paths(
            item,
            &[
                &["evaluations", "terminalbench_v2_1"],
                &["terminalbench_v2_1"],
            ],
        ) {
            fields.insert(
                "AATerminalBench21".to_string(),
                Value::from(terminalbench21 * 100.0),
            );
        }
        if let Some(livecodebench) = number_at_paths(
            item,
            &[&["evaluations", "livecodebench"], &["livecodebench"]],
        ) {
            fields.insert(
                "AALiveCodeBench".to_string(),
                Value::from(livecodebench * 100.0),
            );
        }
        if let Some(math) = number_at_paths(
            item,
            &[
                &["evaluations", "artificial_analysis_math_index"],
                &["evaluations", "math_index"],
                &["evaluations", "math"],
            ],
        ) {
            fields.insert("ArtificialAnalysisMath".to_string(), Value::from(math));
        }
        if let Some(mmlu_pro) =
            number_at_paths(item, &[&["evaluations", "mmlu_pro"], &["mmlu_pro"]])
        {
            fields.insert("MMLUPro".to_string(), Value::from(mmlu_pro * 100.0));
        }

        // Long Context Recall — AA's needle-in-haystack-style measurement of
        // how well a model retrieves information from large input windows.
        // Highly relevant to building (large codebases) and planning (multi-
        // step flows that reference earlier context).
        if let Some(lcr) = number_at_paths(item, &[&["evaluations", "lcr"], &["lcr"]]) {
            fields.insert("LongContextRecall".to_string(), Value::from(lcr * 100.0));
        }

        if let Some(coding) = number_at_paths(
            item,
            &[
                &["evaluations", "artificial_analysis_coding_index"],
                &["evaluations", "coding_index"],
                &["evaluations", "coding"],
                &["artificial_analysis_coding_index"],
                &["coding_index"],
                &["coding"],
            ],
        ) {
            fields.insert("ArtificialAnalysisCoding".to_string(), Value::from(coding));
        }

        // AA reports speed=0 and ttft=0 as "not yet measured" sentinels for
        // models they haven't benchmarked perf on (e.g. Kimi K2.6, GPT-5.4
        // Pro, several preview models). We skip those so they propagate as
        // genuinely missing through the pipeline rather than poisoning the
        // population with bogus zeros.
        if let Some(output_speed) = number_at_paths(
            item,
            &[
                &["median_output_tokens_per_second"],
                &["median_output_speed"],
                &["timescaleData", "median_output_speed"],
            ],
        ) && output_speed > 0.0
        {
            fields.insert("OutputSpeed".to_string(), Value::from(output_speed));
        }

        if let Some(ttft) = number_at_paths(
            item,
            &[
                &["median_time_to_first_token_seconds"],
                &["median_ttft"],
                &["timescaleData", "median_time_to_first_chunk"],
            ],
        ) && ttft > 0.0
        {
            fields.insert("TTFT".to_string(), Value::from(ttft));
        }

        let prompt = number_at_paths(
            item,
            &[
                &["pricing", "input_price_per_million"],
                &["pricing", "price_1m_input_tokens"],
                &["price_1m_input_tokens"],
            ],
        );
        let completion = number_at_paths(
            item,
            &[
                &["pricing", "output_price_per_million"],
                &["pricing", "price_1m_output_tokens"],
                &["price_1m_output_tokens"],
            ],
        );
        if let Some(prompt) = prompt {
            fields.insert("PromptPricePerMillion".to_string(), Value::from(prompt));
        }
        if let Some(completion) = completion {
            fields.insert(
                "CompletionPricePerMillion".to_string(),
                Value::from(completion),
            );
        }
        if let Some(blended) = number_at_paths(
            item,
            &[
                &["pricing", "blended_price_per_million"],
                &["pricing", "price_1m_blended_3_to_1"],
                &["pricing", "blended_price"],
                &["pricing", "blended"],
                &["price_1m_blended_3_to_1"],
            ],
        )
        .or_else(|| blend_cost(prompt, completion))
        .filter(|value| *value > 0.0)
        {
            fields.insert("BlendedCost".to_string(), Value::from(blended));
        }

        if automatic_fallback {
            let fallback_note = item
                .get("name")
                .and_then(Value::as_str)
                .map(automatic_fallback_note)
                .unwrap_or_else(|| automatic_fallback_note(model_name));
            let observed_metrics: Vec<String> = fields
                .iter()
                .filter_map(|(key, value)| value.as_f64().map(|_| key.clone()))
                .collect();
            for metric in observed_metrics {
                fields.insert(
                    format!("{metric}__evidence_note"),
                    Value::from(fallback_note.clone()),
                );
            }
            fields.insert(
                "UpstreamModelFallback".to_string(),
                Value::from("Upstream label discloses automatic product fallback"),
            );
        }

        let row = RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: vendor_hint.map(ToOwned::to_owned),
            fields,
        };
        let key = crate::alias_dedupe_key(
            &alias_records,
            &alias_index,
            &row.model_name,
            row.vendor_hint.as_deref(),
        );
        let preference = AaVariantPreference::from_row(&row);
        let priority = row
            .fields
            .get("ArtificialAnalysisIntelligence")
            .and_then(number_like)
            .unwrap_or(0.0);
        match best_by_model.get_mut(&(key.clone(), preference)) {
            Some((best_priority, best_row)) if priority > *best_priority => {
                *best_priority = priority;
                *best_row = row;
            }
            Some(_) => {}
            None => {
                best_by_model.insert((key, preference), (priority, row));
            }
        }
    }

    Ok(best_by_model.into_values().map(|(_, row)| row).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AaVariantPreference {
    Default,
    Medium,
    Thinking,
    NonReasoning,
    Low,
    High,
    XHigh,
    Max,
    Other,
}

impl AaVariantPreference {
    fn from_row(row: &RawRow) -> Self {
        let mut text = row.model_name.clone();
        for value in row.fields.values() {
            if let Some(s) = value.as_str() {
                text.push(' ');
                text.push_str(s);
            }
        }
        Self::from_text(&text)
    }

    fn from_text(text: &str) -> Self {
        let normalized = normalize_name(text);
        let contains = |phrase: &str| contains_phrase(&normalized, phrase);
        let has_effort_marker = [
            "default",
            "medium",
            "non reasoning",
            "minimal",
            "instant",
            "low",
            "high",
            "thinking",
            "reasoning",
            "adaptive",
            "max",
            "xhigh",
        ]
        .iter()
        .any(|phrase| contains(phrase));

        if contains("default") || !has_effort_marker {
            Self::Default
        } else if contains("non reasoning") {
            Self::NonReasoning
        } else if contains("minimal") || contains("instant") || contains("low") {
            Self::Low
        } else if contains("max") {
            Self::Max
        } else if contains("xhigh") {
            Self::XHigh
        } else if contains("high") {
            Self::High
        } else if contains("medium") {
            Self::Medium
        } else if contains("thinking") || contains("reasoning") || contains("adaptive") {
            Self::Thinking
        } else {
            Self::Other
        }
    }
}

fn contains_phrase(normalized_text: &str, phrase: &str) -> bool {
    let haystack = format!(" {normalized_text} ");
    let needle = format!(" {phrase} ");
    haystack.contains(&needle)
}

fn blend_cost(prompt: Option<f64>, completion: Option<f64>) -> Option<f64> {
    let (prompt, completion) = (prompt?, completion?);
    let blended = 0.75 * prompt + 0.25 * completion;
    (blended.is_finite() && blended > 0.0).then_some(blended)
}

fn number_at_paths(value: &Value, paths: &[&[&str]]) -> Option<f64> {
    paths
        .iter()
        .find_map(|path| follow_path(value, path).and_then(number_like))
}

fn follow_path<'a>(mut value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    for segment in path {
        value = value.get(*segment)?;
    }
    Some(value)
}

fn number_like(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn copy_if_present(fields: &mut BTreeMap<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value {
        fields.insert(key.to_string(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct PanicHttp;

    #[async_trait::async_trait]
    impl Http for PanicHttp {
        async fn get_json(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<Value, SourceError> {
            panic!("missing-secret fetch should not hit the network")
        }

        async fn get_text(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, SourceError> {
            panic!("unused")
        }
    }

    #[test]
    fn parse_rows_maps_nested_metrics() {
        let payload = json!({
            "data": [{
                "id": "openai/gpt-5.5",
                "name": "GPT-5.5",
                "slug": "gpt-5-5",
                "model_creator": { "slug": "openai", "name": "OpenAI" },
                "evaluations": {
                    "intelligence_index": 60.24,
                    "coding_index": 59.12,
                    "gpqa": 0.93,
                    "hle": 0.40,
                    "aime_25": 0.75,
                    "terminalbench_hard": 0.50,
                    "terminalbench_v2_1": 0.62,
                    "livecodebench": 0.80,
                    "tau_banking": 0.5,
                    "tau2": 0.64,
                    "artificial_analysis_math_index": 71.25,
                    "mmlu_pro": 0.88
                },
                "pricing": {
                    "input_price_per_million": 5.0,
                    "output_price_per_million": 30.0,
                    "blended_price_per_million": 11.25
                },
                "median_output_tokens_per_second": 90.37,
                "median_time_to_first_token_seconds": 30.78
            }]
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        let row = &rows[0];
        // Source prefers `slug` over `id` (AA's `id` is a UUID in production).
        assert_eq!(row.model_name, "gpt-5-5");
        assert_eq!(row.vendor_hint.as_deref(), Some("openai"));
        assert_eq!(
            row.fields
                .get("ArtificialAnalysisIntelligence")
                .and_then(number_like),
            Some(60.24)
        );
        assert_eq!(
            row.fields
                .get("ArtificialAnalysisCoding")
                .and_then(number_like),
            Some(59.12)
        );
        assert_eq!(row.fields.get("GPQA").and_then(number_like), Some(93.0));
        assert_eq!(row.fields.get("HLE").and_then(number_like), Some(40.0));
        assert!(!row.fields.contains_key("ArtificialAnalysisReasoning"));
        assert!(!row.fields.contains_key("GPQA_HLE_Reasoning"));
        assert_eq!(
            row.fields.get("TerminalBenchHard").and_then(number_like),
            Some(50.0)
        );
        assert_eq!(
            row.fields.get("AATerminalBench21").and_then(number_like),
            Some(62.0)
        );
        assert_eq!(row.fields.get("AIME25").and_then(number_like), Some(75.0));
        assert_eq!(
            row.fields.get("TauBanking").and_then(number_like),
            Some(50.0)
        );
        assert_eq!(
            row.fields.get("Tau3Banking").and_then(number_like),
            Some(50.0)
        );
        assert_eq!(
            row.fields.get("Tau2Bench").and_then(number_like),
            Some(64.0)
        );
        assert_eq!(
            row.fields.get("AALiveCodeBench").and_then(number_like),
            Some(80.0)
        );
        assert_eq!(
            row.fields
                .get("ArtificialAnalysisMath")
                .and_then(number_like),
            Some(71.25)
        );
        assert_eq!(row.fields.get("MMLUPro").and_then(number_like), Some(88.0));
        assert_eq!(
            row.fields.get("OutputSpeed").and_then(number_like),
            Some(90.37)
        );
        assert_eq!(row.fields.get("TTFT").and_then(number_like), Some(30.78));
        assert_eq!(
            row.fields.get("BlendedCost").and_then(number_like),
            Some(11.25)
        );
    }

    #[test]
    fn reused_deepseek_slugs_keep_dated_snapshots_separate() {
        let payload = json!({
            "data": [
                {
                    "slug": "deepseek-v4-flash-0420",
                    "name": "DeepSeek V4 Flash (Reasoning, Max Effort)",
                    "model_creator": {"slug": "deepseek"},
                    "evaluations": {"intelligence_index": 42.0}
                },
                {
                    "slug": "deepseek-v4-flash",
                    "name": "DeepSeek V4 Flash 0731 (Reasoning, Max Effort)",
                    "model_creator": {"slug": "deepseek"},
                    "evaluations": {"intelligence_index": 52.0}
                },
                {
                    "slug": "deepseek-v4-pro-0424",
                    "name": "DeepSeek V4 Pro (Reasoning, Max Effort)",
                    "model_creator": {"slug": "deepseek"},
                    "evaluations": {"intelligence_index": 45.0}
                },
                {
                    "slug": "deepseek-v4-pro",
                    "name": "DeepSeek V4 Pro 0813 (Reasoning, Max Effort)",
                    "model_creator": {"slug": "deepseek"},
                    "evaluations": {"intelligence_index": 53.0}
                }
            ]
        });

        let rows = parse_rows(&payload).expect("DeepSeek snapshots should parse");
        assert_eq!(rows.len(), 4);
        let mut records = crate::embedded_alias_records();
        let stats = ipbr_core::ingest_rows(&mut records, rows);
        assert_eq!(stats.matched, 4);
        assert!(stats.unmatched.is_empty());

        let intelligence = |canonical: &str| {
            records
                .iter()
                .find(|record| record.canonical_id == canonical)
                .and_then(|record| {
                    record
                        .raw_metrics
                        .get("ArtificialAnalysisIntelligence")
                        .copied()
                })
        };
        assert_eq!(intelligence("deepseek/deepseek-v4-flash"), Some(42.0));
        assert_eq!(intelligence("deepseek/deepseek-v4-flash-0731"), Some(52.0));
        assert_eq!(intelligence("deepseek/deepseek-v4-pro"), Some(45.0));
        assert_eq!(intelligence("deepseek/deepseek-v4-pro-0813"), Some(53.0));
    }

    #[tokio::test]
    async fn missing_secret_blocks_network_fetch() {
        let err = ArtificialAnalysisSource
            .fetch(
                &PanicHttp,
                crate::FetchOptions {
                    cache_dir: None,
                    offline: false,
                },
                &crate::SecretStore::default(),
            )
            .await
            .expect_err("missing secret should error before any network call");
        assert!(matches!(err, crate::SourceError::MissingSecret(_)));
    }

    #[test]
    fn skips_zero_blended_cost_sentinel() {
        let payload = json!({
            "data": [{
                "slug": "glm-5-turbo",
                "model_creator": {"slug": "zai"},
                "pricing": {
                    "price_1m_blended_3_to_1": 0.0,
                    "price_1m_input_tokens": 0.0,
                    "price_1m_output_tokens": 0.0
                }
            }]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].fields.contains_key("BlendedCost"));
    }

    #[test]
    fn retains_automatic_fallback_as_direct_product_evidence() {
        let payload = json!({
            "data": [
                {
                    "slug": "claude-fable-5",
                    "name": "Claude Fable 5 (Adaptive Reasoning, Max Effort, Opus 4.8 Fallback)",
                    "model_creator": {"slug": "anthropic"},
                    "evaluations": {"intelligence_index": 60.0}
                },
                {
                    "slug": "gpt-5-5",
                    "name": "GPT-5.5 (xhigh)",
                    "model_creator": {"slug": "openai"},
                    "evaluations": {"intelligence_index": 55.0}
                }
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 2);
        let fable = rows
            .iter()
            .find(|row| row.model_name == "claude-fable-5")
            .expect("Fable routed-product row should remain");
        assert_eq!(
            fable
                .fields
                .get("ArtificialAnalysisIntelligence")
                .and_then(number_like),
            Some(60.0)
        );
        assert_eq!(
            fable
                .fields
                .get("ArtificialAnalysisIntelligence__evidence_note")
                .and_then(Value::as_str),
            Some(automatic_fallback_note(
                "Claude Fable 5 (Adaptive Reasoning, Max Effort, Opus 4.8 Fallback)"
            ))
            .as_deref()
        );
        assert!(fable.fields.contains_key("UpstreamModelFallback"));
    }

    #[test]
    fn collapses_equal_effort_variants_by_intelligence() {
        let payload = json!({
            "data": [
                {
                    "slug": "gpt-5-2-medium",
                    "model_creator": {"slug": "openai"},
                    "evaluations": {
                        "intelligence_index": 30.3,
                        "coding_index": 46.7
                    }
                },
                {
                    "slug": "gpt-5.2-medium",
                    "model_creator": {"slug": "openai"},
                    "evaluations": {
                        "intelligence_index": 34.6,
                        "coding_index": 32.0
                    }
                }
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "gpt-5.2-medium");
        assert_eq!(
            rows[0]
                .fields
                .get("ArtificialAnalysisIntelligence")
                .and_then(number_like),
            Some(34.6)
        );
    }

    #[test]
    fn keeps_explicit_max_and_xhigh_as_distinct_effort_rows() {
        let payload = json!({
            "data": [
                {
                    "slug": "claude-opus-4-8-max",
                    "name": "Claude Opus 4.8 (max)",
                    "model_creator": {"slug": "anthropic"},
                    "evaluations": {"intelligence_index": 40.0}
                },
                {
                    "slug": "claude-opus-4-8-xhigh",
                    "name": "Claude Opus 4.8 (xhigh)",
                    "model_creator": {"slug": "anthropic"},
                    "evaluations": {"intelligence_index": 50.0}
                }
            ]
        });

        let rows = parse_rows(&payload).expect("payload should parse");

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.model_name.ends_with("-max")));
        assert!(rows.iter().any(|row| row.model_name.ends_with("-xhigh")));
    }

    #[test]
    fn keeps_explicit_medium_separate_from_generic_adaptive_reasoning() {
        let payload = json!({
            "data": [
                {
                    "slug": "claude-sonnet-5",
                    "name": "Claude Sonnet 5 (Adaptive Reasoning)",
                    "model_creator": {"slug": "anthropic"},
                    "evaluations": {"intelligence_index": 50.0}
                },
                {
                    "slug": "claude-sonnet-5",
                    "name": "Claude Sonnet 5 (Adaptive Reasoning, Medium Effort)",
                    "model_creator": {"slug": "anthropic"},
                    "evaluations": {"intelligence_index": 40.0}
                }
            ]
        });

        let rows = parse_rows(&payload).expect("payload should parse");

        assert_eq!(rows.len(), 2);
        assert_eq!(
            AaVariantPreference::from_text("Gemini 3 Flash (Thinking-Minimal)"),
            AaVariantPreference::Low
        );
        assert_eq!(
            AaVariantPreference::from_text("Kimi K2.5 Instant"),
            AaVariantPreference::Low
        );
    }
}
