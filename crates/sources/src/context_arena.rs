//! Context Arena GDM-MRCRv2 long-context benchmark.
//!
//! The public API exposes one row per model and reasoning mode. We consume the
//! full eight-needle track, prefer the highest available reasoning mode for
//! each model, and emit AUC through 128k as the comparable primary signal.
//! AUC through 1M is retained as a diagnostic because not every model supports
//! the full context range.

use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "context_arena";
const CACHE_KEY: &str = "context_arena";
const URL: &str = "https://contextarena.ai/api/needle-summary?needles=8";
const EXPECTED_NEEDLES: u64 = 8;
const EXPECTED_MODE: &str = "full";
const MIN_MODEL_COHORT: usize = 5;

#[derive(Debug, Default, Clone, Copy)]
pub struct ContextArenaSource;

#[async_trait::async_trait]
impl Source for ContextArenaSource {
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
        Duration::from_secs(24 * 3600)
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        _secrets: &SecretStore,
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
            let payload = http.get_json(URL, &[("User-Agent", "ipbr-rank")]).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_json(dir, self.cache_key(), &payload)?;
            }
            payload
        };
        parse_rows(&payload)
    }
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let request_params = payload
        .get("request_params")
        .ok_or_else(|| SourceError::Parse("Context Arena payload missing request_params".into()))?;
    let needles = request_params.get("needles").and_then(Value::as_u64);
    let mode = request_params.get("mode").and_then(Value::as_str);
    if needles != Some(EXPECTED_NEEDLES) || mode != Some(EXPECTED_MODE) {
        return Err(SourceError::Parse(format!(
            "Context Arena response parameters changed: expected needles={EXPECTED_NEEDLES}, mode={EXPECTED_MODE:?}; got needles={needles:?}, mode={mode:?}"
        )));
    }

    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("Context Arena payload missing models[]".into()))?;

    // Lower numeric priority wins. The ranking intentionally follows the
    // repository-wide max-effort policy rather than selecting whichever row
    // happened to receive the highest benchmark score.
    let mut best: BTreeMap<String, (u8, RawRow)> = BTreeMap::new();
    for model in models {
        let Some(slug) = model.get("model_slug").and_then(Value::as_str) else {
            continue;
        };
        let mode = model
            .get("reasoning_mode")
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some(metrics) = model.get("overall_metrics") else {
            continue;
        };
        let Some(auc_128k) = metrics.get("auc_128k").and_then(valid_auc) else {
            continue;
        };

        let (vendor, base_name) = slug.split_once('/').unwrap_or(("", slug));
        let model_name = display_name(base_name, mode);
        let mut fields = BTreeMap::new();
        fields.insert(
            "ContextArenaMRCR128k".to_string(),
            Value::from(auc_128k * 100.0),
        );
        if let Some(auc_1m) = metrics.get("auc_1m").and_then(valid_auc) {
            fields.insert(
                "ContextArenaMRCR1M".to_string(),
                Value::from(auc_1m * 100.0),
            );
        }
        let row = RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name,
            vendor_hint: (!vendor.is_empty()).then(|| vendor.to_string()),
            fields,
            synthesized_from: None,
            synthesis_category: None,
        };
        let priority = effort_priority(mode);
        match best.get(slug) {
            Some((current, _)) if *current <= priority => {}
            _ => {
                best.insert(slug.to_string(), (priority, row));
            }
        }
    }

    let rows: Vec<_> = best.into_values().map(|(_, row)| row).collect();
    if rows.len() < MIN_MODEL_COHORT {
        return Err(SourceError::Parse(format!(
            "Context Arena payload yielded only {} scored models; expected at least {MIN_MODEL_COHORT}",
            rows.len()
        )));
    }
    Ok(rows)
}

fn valid_auc(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
}

fn effort_priority(mode: &str) -> u8 {
    match mode.trim().to_ascii_lowercase().as_str() {
        "max" | "pro" => 0,
        "xhigh" | "x-high" => 1,
        "high" => 2,
        "enabled" | "adaptive" | "thinking" => 3,
        "medium" => 4,
        "low" => 5,
        "" | "default" | "none" | "disabled" => 6,
        _ => 7,
    }
}

fn display_name(base_name: &str, mode: &str) -> String {
    match mode.trim().to_ascii_lowercase().as_str() {
        "" | "default" | "none" | "disabled" => base_name.to_string(),
        "enabled" => format!("{base_name} (reasoning)"),
        other => format!("{base_name} ({other})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_max_effort_and_emits_both_auc_views() {
        let payload = json!({
            "request_params": {"needles": 8, "mode": "full"},
            "models": [
                {
                    "model_slug": "openai/gpt-5.5",
                    "reasoning_mode": "medium",
                    "overall_metrics": {"auc_128k": 0.87, "auc_1m": 0.42}
                },
                {
                    "model_slug": "openai/gpt-5.5",
                    "reasoning_mode": "xhigh",
                    "overall_metrics": {"auc_128k": 0.91, "auc_1m": 0.50}
                },
                {
                    "model_slug": "openai/gpt-5.4",
                    "reasoning_mode": "xhigh",
                    "overall_metrics": {"auc_128k": 0.80, "auc_1m": 0.38}
                },
                {
                    "model_slug": "anthropic/claude-opus-4.8",
                    "reasoning_mode": "max",
                    "overall_metrics": {"auc_128k": 0.88, "auc_1m": 0.42}
                },
                {
                    "model_slug": "anthropic/claude-opus-4.7",
                    "reasoning_mode": "xhigh",
                    "overall_metrics": {"auc_128k": 0.30, "auc_1m": 0.07}
                },
                {
                    "model_slug": "google/gemini-3.1-pro-preview",
                    "reasoning_mode": "high",
                    "overall_metrics": {"auc_128k": 0.77, "auc_1m": 0.40}
                }
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 5);
        let gpt = rows
            .iter()
            .find(|row| row.model_name == "gpt-5.5 (xhigh)")
            .expect("GPT-5.5 row");
        assert_eq!(gpt.vendor_hint.as_deref(), Some("openai"));
        assert_eq!(
            gpt.fields
                .get("ContextArenaMRCR128k")
                .and_then(Value::as_f64),
            Some(91.0)
        );
        assert_eq!(
            gpt.fields.get("ContextArenaMRCR1M").and_then(Value::as_f64),
            Some(50.0)
        );
    }

    #[test]
    fn rejects_wrong_track_parameters() {
        let payload = json!({
            "request_params": {"needles": 1, "mode": "full"},
            "models": []
        });
        let error = parse_rows(&payload).expect_err("wrong needle track must fail");
        assert!(error.to_string().contains("response parameters changed"));
    }

    #[test]
    fn rejects_out_of_bounds_auc_and_tiny_cohort() {
        let payload = json!({
            "request_params": {"needles": 8, "mode": "full"},
            "models": [
                {
                    "model_slug": "openai/gpt-5.5",
                    "reasoning_mode": "xhigh",
                    "overall_metrics": {"auc_128k": 1.01, "auc_1m": -0.1}
                },
                {
                    "model_slug": "openai/gpt-5.4",
                    "reasoning_mode": "xhigh",
                    "overall_metrics": {"auc_128k": 0.80, "auc_1m": 0.38}
                }
            ]
        });
        let error = parse_rows(&payload).expect_err("invalid AUC cannot form a cohort");
        assert!(error.to_string().contains("only 1 scored models"));
    }
}
