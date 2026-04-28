use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretRef, SecretStore, Source, SourceError, VerificationStatus,
    cache_json_path, read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "aistupidlevel";
const CACHE_KEY: &str = "aistupidlevel_dashboard";

// Cached dashboard exposes both `modelScores[]` (current scores per model) and
// `historyMap` (per-model time series of axis breakdowns). The axis values in
// `historyMap[id][last].axes` are the per-category category scores the spec
// (Workspace/dotkrnl/llm_scoreboard.md) uses to compute AI_correctness, AI_spec,
// AI_code, AI_efficiency, AI_stability, AI_refusal, AI_recovery.
const PRIMARY_URL: &str = "https://aistupidlevel.info/api/dashboard/cached";
const FALLBACK_URLS: &[&str] = &[
    "https://aistupidlevel.info/dashboard/cached",
    "https://aistupidlevel.info/api/dashboard",
];

// Map ipbr-rank's spec axis name → live API axis key in `historyMap[].axes`.
//
// AIStupidLevel runs three distinct evaluation suites whose results all
// share the same `historyMap[modelId]` array:
//   * `hourly`  — quick correctness / format / codeQuality / efficiency /
//                 stability / safety / debugging / complexity / edgeCases.
//                 The original 7-axis set this source was designed for.
//   * `deep`    — longer tasks adding planCoherence, memoryRetention,
//                 hallucinationRate, contextWindow.
//   * `tooling` — function-calling suite with contextAwareness,
//                 taskCompletion, toolSelection, parameterAccuracy,
//                 errorHandling, safetyCompliance, plus efficiency.
//
// Walking only the newest entry (the previous behaviour) meant whichever
// suite ran most recently silently dropped 6-12 axes. The fixed parser
// walks history newest-first and takes each axis's first numeric value,
// merging across suites so models always carry their full axis surface.
//
// `contextWindow` is intentionally dropped — it overlaps OpenRouter's
// ContextWindow metric and conflates capability with config.
//
// Live values are floats in [0, 1]; we scale to 0-100 for the scoring
// layer. All AISL axes are emitted with higher = better semantics —
// upstream's `calculateHallucinationRate` already returns `1 - rate`
// (despite the misleading name), and `calculateSafety` /
// `calculateMemoryRetention` etc. follow the same convention. So no
// inversion is needed at our end.
const AXIS_MAPPINGS: &[(&str, &str)] = &[
    // hourly suite — code-quality basics
    ("AI_correctness", "correctness"),
    ("AI_spec", "format"),
    ("AI_code", "codeQuality"),
    ("AI_efficiency", "efficiency"),
    ("AI_stability", "stability"),
    ("AI_refusal", "safety"),
    ("AI_recovery", "debugging"),
    ("AI_complexity", "complexity"),
    ("AI_edge_cases", "edgeCases"),
    // deep suite — long-form work. `hallucinationRate` is upstream-named
    // but the value is already 1-rate per
    // <https://github.com/StudioPlatforms/aistupidmeter-api/blob/main/src/deepbench/index.ts>
    // `calculateHallucinationRate` (see the `return Math.max(0, 1 - rate)`
    // tail). Treating it as resistance is the right read.
    ("AI_hallucination_resistance", "hallucinationRate"),
    ("AI_plan_coherence", "planCoherence"),
    ("AI_memory_retention", "memoryRetention"),
    // tooling suite — function calling / agentic tool use.
    //
    // `errorHandling` is intentionally dropped: upstream defines it as
    // `recoveredFromErrors / failedCalls.length`, with the failedCalls=0
    // case returning 0 instead of 1 (see toolbench/session/benchmark-
    // session.ts line 380). That means a model that never fails gets the
    // same score as one that fails everything and recovers nothing — a
    // measurement quirk that creates spurious 0s for high-quality models.
    ("AI_context_awareness", "contextAwareness"),
    ("AI_task_completion", "taskCompletion"),
    ("AI_tool_selection", "toolSelection"),
    ("AI_parameter_accuracy", "parameterAccuracy"),
    ("AI_safety_compliance", "safetyCompliance"),
];

// No-op kept around so we don't have to edit the parser's match logic;
// future failure-rate axes (if AISL adds any) get added here. Empty today.
const INVERTED_AXIS_MAPPINGS: &[(&str, &str)] = &[];

// Drop entirely — overlaps OpenRouter's ContextWindow metric and is more
// a config detail than a measured capability.
// `errorHandling` dropped (see comment above on the false-zero quirk).
const DROPPED_AXIS_KEYS: &[&str] = &["contextWindow", "errorHandling"];

#[derive(Debug, Default, Clone, Copy)]
pub struct AiStupidLevelSource;

#[async_trait::async_trait]
impl Source for AiStupidLevelSource {
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
        None
    }

    fn cache_ttl(&self) -> Duration {
        Duration::from_secs(3600)
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
            let payload = fetch_dashboard(http).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_json(dir, self.cache_key(), &payload)?;
            }
            payload
        };
        parse_rows(&payload)
    }
}

async fn fetch_dashboard(http: &dyn Http) -> Result<Value, SourceError> {
    let mut last_err: Option<SourceError> = None;
    for url in std::iter::once(PRIMARY_URL).chain(FALLBACK_URLS.iter().copied()) {
        match http.get_json(url, &[]).await {
            Ok(payload) => return Ok(payload),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err
        .unwrap_or_else(|| SourceError::Http("no aistupidlevel endpoints reachable".into())))
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let data = payload.get("data").unwrap_or(payload);
    let model_scores = data
        .get("modelScores")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Parse("AIStupidLevel payload missing data.modelScores[]".into())
        })?;
    let history_map = data.get("historyMap").and_then(Value::as_object);

    let mut rows = Vec::new();
    for entry in model_scores {
        let Some(name) = entry
            .get("name")
            .or_else(|| entry.get("model"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let model_name = name.trim();
        if model_name.is_empty() {
            continue;
        }
        let id = entry.get("id").map(value_to_string).unwrap_or_default();
        let axes = history_map
            .and_then(|m| m.get(&id).or_else(|| m.get(model_name)))
            .map(merged_axes)
            .unwrap_or_default();

        let mut fields = BTreeMap::new();
        for (metric_key, live_key) in AXIS_MAPPINGS {
            if let Some(value) = axes.get(*live_key).copied() {
                fields.insert((*metric_key).to_string(), Value::from(value * 100.0));
            }
        }
        for (metric_key, live_key) in INVERTED_AXIS_MAPPINGS {
            if let Some(value) = axes.get(*live_key).copied() {
                let inverted = (1.0 - value).clamp(0.0, 1.0);
                fields.insert((*metric_key).to_string(), Value::from(inverted * 100.0));
            }
        }
        if fields.is_empty() {
            // Fall back to current score so the model still contributes some
            // signal when historyMap is missing axes (e.g. brand-new model).
            if let Some(score) =
                value_to_f64(entry.get("currentScore")).or_else(|| value_to_f64(entry.get("score")))
            {
                fields.insert("AI_correctness".to_string(), Value::from(score));
            }
        }
        if fields.is_empty() {
            continue;
        }
        let vendor_hint = entry
            .get("vendor")
            .or_else(|| entry.get("provider"))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());

        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_name.to_string(),
            vendor_hint,
            fields,
            synthesized_from: None,
        });
    }

    if rows.is_empty() {
        return Err(SourceError::Parse(
            "AIStupidLevel: no models found in payload".into(),
        ));
    }
    Ok(rows)
}

/// Walk `historyMap[id]` newest-first and collect the first numeric value
/// for each axis. AISL's three suites (hourly / deep / tooling) each
/// expose different axes; merging across the history surface is the only
/// way to capture all of them. Newest-first ordering means a fresh
/// reading wins over a stale one when the same axis appears in multiple
/// suites.
fn merged_axes(history: &Value) -> BTreeMap<String, f64> {
    // The upstream `historyMap` is already newest-first (entry 0 carries
    // the most recent timestamp; index N is the oldest). Walking the
    // array in natural order with first-write-wins gives us the newest
    // value per axis. Earlier revisions used `.iter().rev()` here on the
    // mistaken assumption that the array was oldest-first; that quietly
    // pinned us to *oldest* values, which severely penalized models whose
    // older AISL runs predated their current quality (e.g. Claude Opus 4.7
    // landing at 0.8 on hallucinationRate from a four-day-old entry while
    // every other model's older entries also hovered near 1.0).
    let mut out: BTreeMap<String, f64> = BTreeMap::new();
    let Some(entries) = history.as_array() else {
        return out;
    };
    for entry in entries {
        let Some(axes) = entry.get("axes").and_then(Value::as_object) else {
            continue;
        };
        for (key, value) in axes {
            if DROPPED_AXIS_KEYS.contains(&key.as_str()) {
                continue;
            }
            if out.contains_key(key) {
                continue;
            }
            if let Some(number) = value_to_f64(Some(value)) {
                out.insert(key.clone(), number);
            }
        }
    }
    out
}

fn value_to_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_payload() -> Value {
        json!({
            "success": true,
            "data": {
                "modelScores": [
                    {
                        "id": "1",
                        "name": "claude-opus-4-7",
                        "vendor": "anthropic",
                        "currentScore": 88.0,
                        "score": 87.5,
                        "standardError": 1.2
                    },
                    {
                        "id": "2",
                        "name": "gpt-5.5",
                        "vendor": "openai",
                        "currentScore": 80.0,
                        "score": 79.0,
                        "standardError": 1.6
                    }
                ],
                "historyMap": {
                    "1": [
                        {
                            "timestamp": "2026-04-26T00:00:00Z",
                            "score": 87.5,
                            "axes": {
                                "correctness": 0.95,
                                "format": 0.90,
                                "codeQuality": 0.88,
                                "efficiency": 0.70,
                                "stability": 0.92,
                                "safety": 0.99,
                                "debugging": 0.85
                            }
                        }
                    ],
                    "2": [
                        {
                            "timestamp": "2026-04-26T00:00:00Z",
                            "score": 79.0,
                            "axes": {
                                "correctness": 0.85,
                                "format": 0.82,
                                "codeQuality": 0.78,
                                "efficiency": 0.65,
                                "stability": 0.80,
                                "safety": 0.95,
                                "debugging": 0.72
                            }
                        }
                    ]
                }
            }
        })
    }

    #[test]
    fn parse_rows_extracts_per_model_axes() {
        let rows = parse_rows(&sample_payload()).expect("should parse");
        assert_eq!(rows.len(), 2);

        let opus = rows
            .iter()
            .find(|r| r.model_name == "claude-opus-4-7")
            .unwrap();
        assert_eq!(opus.vendor_hint.as_deref(), Some("anthropic"));
        assert_eq!(
            opus.fields.get("AI_correctness").and_then(Value::as_f64),
            Some(95.0)
        );
        assert_eq!(
            opus.fields.get("AI_spec").and_then(Value::as_f64),
            Some(90.0)
        );
        assert_eq!(
            opus.fields.get("AI_code").and_then(Value::as_f64),
            Some(88.0)
        );
        assert_eq!(
            opus.fields.get("AI_efficiency").and_then(Value::as_f64),
            Some(70.0)
        );
        assert_eq!(
            opus.fields.get("AI_stability").and_then(Value::as_f64),
            Some(92.0)
        );
        assert_eq!(
            opus.fields.get("AI_refusal").and_then(Value::as_f64),
            Some(99.0)
        );
        assert_eq!(
            opus.fields.get("AI_recovery").and_then(Value::as_f64),
            Some(85.0)
        );
    }

    #[test]
    fn parse_rows_falls_back_to_current_score_when_history_missing() {
        let payload = json!({
            "data": {
                "modelScores": [
                    { "id": "9", "name": "novel-model", "vendor": "x", "currentScore": 71.0 }
                ],
                "historyMap": {}
            }
        });
        let rows = parse_rows(&payload).expect("should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].fields.get("AI_correctness").and_then(Value::as_f64),
            Some(71.0)
        );
        assert!(!rows[0].fields.contains_key("AI_spec"));
    }

    #[test]
    fn parse_rows_errors_when_no_models() {
        let payload = json!({"data": {"modelScores": []}});
        assert!(parse_rows(&payload).is_err());
    }

    #[test]
    fn parse_rows_accepts_top_level_modelscores() {
        // Older clients/fallbacks may return modelScores at the top level.
        let payload = json!({
            "modelScores": [
                {
                    "id": "1",
                    "name": "foo",
                    "vendor": "v",
                    "currentScore": 50.0
                }
            ],
            "historyMap": {}
        });
        let rows = parse_rows(&payload).expect("should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "foo");
    }
}
