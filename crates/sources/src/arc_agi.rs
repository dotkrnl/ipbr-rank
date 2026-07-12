//! ARC-AGI source — pure abstract-reasoning benchmark from the ARC Prize.
//!
//! ARC-AGI v2 is the only public benchmark that explicitly tests novel
//! pattern induction (every task is unfamiliar at evaluation time) — it's
//! orthogonal to GPQA/HLE which test learned knowledge. Frontier models
//! sit around 75-85% while humans top out near 100, so it discriminates
//! well at the top of the population.
//!
//! Data lives at two static endpoints fetched by the leaderboard's JS:
//!   * `https://arcprize.org/media/data/models.json`      — id → display name
//!   * `https://arcprize.org/media/data/evaluations.json` — score per dataset
//!
//! We pull both, join on `modelId`, and emit `ARC_AGI_2` / `ARC_AGI_3` for
//! every model that has a semi-private evaluation. Public ARC-AGI-2 numbers
//! exist too but are inflated by training-set exposure on some models, so we
//! skip them. ARC-AGI-3 remains diagnostic while its extremely low scores and
//! small model cohort make rank-sensitive weighting premature.

use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::{AliasIndex, RawRow, normalize_name};
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "arc_agi";
const CACHE_KEY: &str = "arc_agi";
const MODELS_URL: &str = "https://arcprize.org/media/data/models.json";
const EVALS_URL: &str = "https://arcprize.org/media/data/evaluations.json";
const PRIMARY_DATASET: &str = "v2_Semi_Private";
const DIAGNOSTIC_DATASET: &str = "v3_Semi_Private";

const DATASETS: [(&str, &str); 2] = [
    (PRIMARY_DATASET, "ARC_AGI_2"),
    (DIAGNOSTIC_DATASET, "ARC_AGI_3"),
];

#[derive(Debug, Default, Clone, Copy)]
pub struct ArcAgiSource;

#[async_trait::async_trait]
impl Source for ArcAgiSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        CACHE_KEY
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Verified
    }

    fn required_secret(&self) -> Option<crate::SecretRef> {
        None
    }

    fn cache_ttl(&self) -> Duration {
        // ARC Prize updates ranks roughly weekly during active competition.
        Duration::from_secs(7 * 24 * 3600)
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        _secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        let combined = if use_cached_json(opts, self.cache_key(), self.cache_ttl()) {
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
            let models = http
                .get_json(MODELS_URL, &[("User-Agent", "ipbr-rank")])
                .await?;
            let evals = http
                .get_json(EVALS_URL, &[("User-Agent", "ipbr-rank")])
                .await?;
            let combined = serde_json::json!({
                "models": models,
                "evaluations": evals,
            });
            if let Some(dir) = opts.cache_dir {
                write_cache_json(dir, self.cache_key(), &combined)?;
            }
            combined
        };
        parse_rows(&combined)
    }
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("ARC-AGI payload missing models[]".into()))?;
    let evals = payload
        .get("evaluations")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("ARC-AGI payload missing evaluations[]".into()))?;

    let mut display: BTreeMap<&str, &str> = BTreeMap::new();
    for m in models {
        if let (Some(id), Some(name)) = (
            m.get("id").and_then(Value::as_str),
            m.get("displayName").and_then(Value::as_str),
        ) {
            display.insert(id, name);
        }
    }

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut best_by_model: BTreeMap<(String, String, ArcVariantPreference), (f64, RawRow)> =
        BTreeMap::new();
    for e in evals {
        let Some((_, metric)) = e
            .get("datasetId")
            .and_then(Value::as_str)
            .and_then(|dataset| DATASETS.iter().find(|(id, _)| *id == dataset))
        else {
            continue;
        };
        let Some(model_id) = e.get("modelId").and_then(Value::as_str) else {
            continue;
        };
        // Score is on a 0-1 scale in the JSON; rescale to 0-100 for parity
        // with the rest of the metric population.
        let Some(score_raw) = e.get("score").and_then(Value::as_f64) else {
            continue;
        };
        if !score_raw.is_finite() {
            continue;
        }
        let display_name = display.get(model_id).copied().unwrap_or(model_id);
        let mut fields = BTreeMap::new();
        fields.insert((*metric).to_string(), Value::from(score_raw * 100.0));
        let row = RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: display_name.to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
            synthesis_category: None,
        };
        let key = crate::alias_dedupe_key(&alias_records, &alias_index, display_name, None);
        let preference = ArcVariantPreference::from_text(display_name);
        let score = score_raw * 100.0;
        match best_by_model.get_mut(&(key.clone(), (*metric).to_string(), preference)) {
            Some((best_score, best_row)) if score > *best_score => {
                *best_score = score;
                *best_row = row;
            }
            Some(_) => {}
            None => {
                best_by_model.insert((key, (*metric).to_string(), preference), (score, row));
            }
        }
    }

    let rows: Vec<RawRow> = best_by_model.into_values().map(|(_, row)| row).collect();
    if !rows.iter().any(|row| row.fields.contains_key("ARC_AGI_2")) {
        return Err(SourceError::Parse(
            "ARC-AGI evaluations yielded no v2 semi-private rows".into(),
        ));
    }
    Ok(rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ArcVariantPreference {
    Default,
    Medium,
    Thinking,
    Low,
    High,
    Max,
    Other,
}

impl ArcVariantPreference {
    fn from_text(text: &str) -> Self {
        let normalized = normalize_name(text);
        let contains = |phrase: &str| contains_phrase(&normalized, phrase);
        let has_effort_marker = ["medium", "low", "high", "thinking", "max", "xhigh"]
            .iter()
            .any(|phrase| contains(phrase));
        if !has_effort_marker {
            Self::Default
        } else if contains("low") {
            Self::Low
        } else if contains("max") || contains("xhigh") {
            Self::Max
        } else if contains("high") {
            Self::High
        } else if contains("thinking") {
            Self::Thinking
        } else if contains("medium") {
            Self::Medium
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

#[cfg(test)]
mod tests {
    use super::*;
    use ipbr_core::alias::AliasIndex;
    use ipbr_core::required_aliases::load_embedded;
    use serde_json::json;

    #[test]
    fn parses_arc_fixture_and_resolves_flagships() {
        let bytes = include_bytes!("../../../data/fixtures/arc_agi.json");
        let payload: Value = serde_json::from_slice(bytes).expect("fixture must parse");
        let rows = parse_rows(&payload).expect("rows expected");
        assert!(rows.len() >= 10, "got {} rows", rows.len());
        assert!(rows.iter().any(|r| r.fields.contains_key("ARC_AGI_2")));
        assert!(rows.iter().any(|r| r.fields.contains_key("ARC_AGI_3")));

        let records = load_embedded().expect("aliases must load");
        let idx = AliasIndex::build(&records);
        let mut hits = 0;
        for r in &rows {
            if idx.match_record(&r.model_name, None).is_some() {
                hits += 1;
            }
        }
        assert!(
            hits >= 3,
            "expected ≥3 ARC rows to resolve to canonical IDs, got {hits}"
        );
    }

    #[test]
    fn keeps_best_same_effort_budget_per_canonical_model() {
        let payload = json!({
            "models": [
                {"id": "a", "displayName": "Claude Sonnet 4 (Thinking 1K)"},
                {"id": "b", "displayName": "Claude Sonnet 4 (Thinking 16K)"}
            ],
            "evaluations": [
                {"modelId": "a", "datasetId": PRIMARY_DATASET, "score": 0.01},
                {"modelId": "b", "datasetId": PRIMARY_DATASET, "score": 0.05}
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "Claude Sonnet 4 (Thinking 16K)");
        assert_eq!(
            rows[0].fields.get("ARC_AGI_2").and_then(Value::as_f64),
            Some(5.0)
        );
    }

    #[test]
    fn parses_arc_agi_3_as_a_separate_diagnostic_metric() {
        let payload = json!({
            "models": [
                {"id": "a", "displayName": "GPT-5.5 (High)"},
                {"id": "b", "displayName": "Claude Opus 4.8 (High)"},
                {"id": "v2", "displayName": "GPT-5.5 (High)"}
            ],
            "evaluations": [
                {"modelId": "a", "datasetId": DIAGNOSTIC_DATASET, "score": 0.0043},
                {"modelId": "b", "datasetId": DIAGNOSTIC_DATASET, "score": 0.0152},
                {"modelId": "v2", "datasetId": PRIMARY_DATASET, "score": 0.80}
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        let gpt_v3 = rows
            .iter()
            .find(|row| row.model_name == "GPT-5.5 (High)" && row.fields.contains_key("ARC_AGI_3"))
            .expect("GPT-5.5 ARC-AGI-3 row");
        assert_eq!(
            gpt_v3.fields.get("ARC_AGI_3").and_then(Value::as_f64),
            Some(0.43)
        );
        assert!(!gpt_v3.fields.contains_key("ARC_AGI_2"));
        assert!(rows.iter().any(|row| {
            row.model_name == "Claude Opus 4.8 (High)"
                && row.fields.get("ARC_AGI_3").and_then(Value::as_f64) == Some(1.52)
        }));
    }
}
