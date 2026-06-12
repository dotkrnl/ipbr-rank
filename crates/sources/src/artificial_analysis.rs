use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::{AliasIndex, RawRow, normalize_name};
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretRef, SecretStore, Source, SourceError, VerificationStatus,
    cache_json_path, read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "artificial_analysis";
const CACHE_KEY: &str = "artificial_analysis_llms";
const URL: &str = "https://artificialanalysis.ai/api/v2/data/llms/models";

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
        Duration::from_secs(24 * 3600)
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
    // canonical_id. The ingest layer prefers default, then medium; this sort
    // only keeps equal-priority variant ties deterministic.
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
        // AA's `id` is a UUID; use the human-readable `slug` (e.g.
        // "claude-opus-4-7") for alias matching, then fall back to `name`,
        // and finally the UUID `id` to keep parsing infallible.
        let model_name = item
            .get("slug")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .or_else(|| item.get("id").and_then(Value::as_str))
            .ok_or_else(|| {
                SourceError::Parse("Artificial Analysis row missing slug/name/id".into())
            })?;
        let vendor_hint = item
            .get("model_creator")
            .and_then(|value| value.get("slug"))
            .and_then(Value::as_str)
            .or_else(|| {
                item.get("model_creators")
                    .and_then(|value| value.get("slug"))
                    .and_then(Value::as_str)
            });

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

        // GPQA-Diamond and Humanity's Last Exam are reported as 0–1 fractions
        // in AA's payload. Average them into a unified reasoning signal that
        // populates both ArtificialAnalysisReasoning (PLAN group) and
        // GPQA_HLE_Reasoning (GEN group). Percentile normalization downstream
        // handles cross-benchmark calibration, so the equal-weight blend is
        // intentional. Falls back to the single metric available if only one
        // is present, and emits nothing if both are missing rather than
        // synthesizing from intelligence (the previous behaviour silently
        // turned the reasoning axis into a copy of the intelligence axis).
        let gpqa = number_at_paths(item, &[&["evaluations", "gpqa"], &["gpqa"]]);
        let hle = number_at_paths(item, &[&["evaluations", "hle"], &["hle"]]);
        let reasoning_blend = match (gpqa, hle) {
            (Some(g), Some(h)) => Some(((g + h) / 2.0) * 100.0),
            (Some(g), None) => Some(g * 100.0),
            (None, Some(h)) => Some(h * 100.0),
            (None, None) => None,
        };
        if let Some(value) = reasoning_blend {
            fields.insert(
                "ArtificialAnalysisReasoning".to_string(),
                Value::from(value),
            );
            fields.insert("GPQA_HLE_Reasoning".to_string(), Value::from(value));
        }

        // AA also publishes per-eval scores under `evaluations.{tau2,scicode,
        // ifbench}`. They're 0–1 fractions like gpqa/hle, so we scale to
        // 0–100 and emit as their own metrics. Skipped silently when absent
        // (older runs without these fields, smaller models, etc.)
        if let Some(tau2) = number_at_paths(item, &[&["evaluations", "tau2"], &["tau2"]]) {
            fields.insert("Tau2Bench".to_string(), Value::from(tau2 * 100.0));
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

        let row = RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: vendor_hint.map(ToOwned::to_owned),
            fields,
            synthesized_from: None,
            synthesis_category: None,
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
        } else if contains("medium") {
            Self::Medium
        } else if contains("non reasoning") {
            Self::NonReasoning
        } else if contains("low") {
            Self::Low
        } else if contains("thinking") || contains("reasoning") || contains("adaptive") {
            Self::Thinking
        } else if contains("high") {
            Self::High
        } else if contains("max") || contains("xhigh") {
            Self::Max
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
                    "terminalbench_hard": 0.50,
                    "livecodebench": 0.80,
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
        // Reasoning is now the equal-weight average of gpqa (0.93) and hle (0.40),
        // scaled to the 0-100 range: ((0.93 + 0.40) / 2) * 100 = 66.5.
        assert_eq!(
            row.fields
                .get("ArtificialAnalysisReasoning")
                .and_then(number_like),
            Some(66.5)
        );
        assert_eq!(
            row.fields.get("GPQA_HLE_Reasoning").and_then(number_like),
            Some(66.5)
        );
        assert_eq!(
            row.fields.get("TerminalBenchHard").and_then(number_like),
            Some(50.0)
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
}
