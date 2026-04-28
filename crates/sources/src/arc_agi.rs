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
//! We pull both, join on `modelId`, and emit `ARC_AGI_2` for every model that
//! has a `v2_Semi_Private` evaluation (the contamination-controlled track).
//! `v2_Public_Eval` numbers exist too but are inflated by training-set
//! exposure on some models, so we skip them.

use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::RawRow;
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

    let mut rows: Vec<RawRow> = Vec::new();
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    for e in evals {
        if e.get("datasetId").and_then(Value::as_str) != Some(PRIMARY_DATASET) {
            continue;
        }
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
        if seen.contains_key(display_name) {
            continue;
        }
        seen.insert(display_name.to_string(), ());
        let mut fields = BTreeMap::new();
        fields.insert("ARC_AGI_2".to_string(), Value::from(score_raw * 100.0));
        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: display_name.to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
        });
    }

    if rows.is_empty() {
        return Err(SourceError::Parse(
            "ARC-AGI evaluations yielded no v2 semi-private rows".into(),
        ));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipbr_core::alias::AliasIndex;
    use ipbr_core::required_aliases::load_embedded;

    #[test]
    fn parses_arc_fixture_and_resolves_flagships() {
        let bytes = include_bytes!("../../../data/fixtures/arc_agi.json");
        let payload: Value = serde_json::from_slice(bytes).expect("fixture must parse");
        let rows = parse_rows(&payload).expect("rows expected");
        assert!(rows.len() >= 10, "got {} rows", rows.len());
        assert!(rows.iter().all(|r| r.fields.contains_key("ARC_AGI_2")));

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
}
