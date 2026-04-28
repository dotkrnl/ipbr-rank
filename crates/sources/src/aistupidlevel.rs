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
const CANARY_HEALTH_METRIC: &str = "AI_canary_health";

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

// Map ipbr-rank's spec axis name → live API axis key and suite in
// `historyMap[].axes`.
//
// AIStupidLevel runs three distinct evaluation suites whose results all
// share the same `historyMap[modelId]` array:
//   * `hourly`  — full/speed correctness / format / codeQuality / efficiency /
//                 stability / safety / debugging / complexity / edgeCases.
//   * `deep`    — longer tasks adding planCoherence, memoryRetention,
//                 hallucinationRate, contextWindow.
//   * `tooling` — function-calling suite with contextAwareness,
//                 taskCompletion, toolSelection, parameterAccuracy,
//                 errorHandling, safetyCompliance, plus efficiency.
//
// Canary rows are intentionally not mapped here. They are a drift-detection
// heartbeat, not a replacement for the full 4-hour speed benchmark. Some axis
// keys overlap across suites (`correctness`, `efficiency`), so each metric is
// read from the suite whose methodology matches that metric.
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
struct AxisMapping {
    metric_key: &'static str,
    live_key: &'static str,
    suite: &'static str,
}

const AXIS_MAPPINGS: &[AxisMapping] = &[
    // hourly suite — code-quality basics
    AxisMapping {
        metric_key: "AI_correctness",
        live_key: "correctness",
        suite: "hourly",
    },
    AxisMapping {
        metric_key: "AI_spec",
        live_key: "format",
        suite: "hourly",
    },
    AxisMapping {
        metric_key: "AI_code",
        live_key: "codeQuality",
        suite: "hourly",
    },
    AxisMapping {
        metric_key: "AI_efficiency",
        live_key: "efficiency",
        suite: "hourly",
    },
    AxisMapping {
        metric_key: "AI_stability",
        live_key: "stability",
        suite: "hourly",
    },
    AxisMapping {
        metric_key: "AI_refusal",
        live_key: "safety",
        suite: "hourly",
    },
    AxisMapping {
        metric_key: "AI_recovery",
        live_key: "debugging",
        suite: "hourly",
    },
    AxisMapping {
        metric_key: "AI_complexity",
        live_key: "complexity",
        suite: "hourly",
    },
    AxisMapping {
        metric_key: "AI_edge_cases",
        live_key: "edgeCases",
        suite: "hourly",
    },
    // deep suite — long-form work. `hallucinationRate` is upstream-named
    // but the value is already 1-rate per
    // <https://github.com/StudioPlatforms/aistupidmeter-api/blob/main/src/deepbench/index.ts>
    // `calculateHallucinationRate` (see the `return Math.max(0, 1 - rate)`
    // tail). Treating it as resistance is the right read.
    AxisMapping {
        metric_key: "AI_hallucination_resistance",
        live_key: "hallucinationRate",
        suite: "deep",
    },
    AxisMapping {
        metric_key: "AI_plan_coherence",
        live_key: "planCoherence",
        suite: "deep",
    },
    AxisMapping {
        metric_key: "AI_memory_retention",
        live_key: "memoryRetention",
        suite: "deep",
    },
    // tooling suite — function calling / agentic tool use.
    //
    // `errorHandling` is intentionally dropped: upstream defines it as
    // `recoveredFromErrors / failedCalls.length`, with the failedCalls=0
    // case returning 0 instead of 1 (see toolbench/session/benchmark-
    // session.ts line 380). That means a model that never fails gets the
    // same score as one that fails everything and recovers nothing — a
    // measurement quirk that creates spurious 0s for high-quality models.
    AxisMapping {
        metric_key: "AI_context_awareness",
        live_key: "contextAwareness",
        suite: "tooling",
    },
    AxisMapping {
        metric_key: "AI_task_completion",
        live_key: "taskCompletion",
        suite: "tooling",
    },
    AxisMapping {
        metric_key: "AI_tool_selection",
        live_key: "toolSelection",
        suite: "tooling",
    },
    AxisMapping {
        metric_key: "AI_parameter_accuracy",
        live_key: "parameterAccuracy",
        suite: "tooling",
    },
    AxisMapping {
        metric_key: "AI_safety_compliance",
        live_key: "safetyCompliance",
        suite: "tooling",
    },
];

// No-op kept around so we don't have to edit the parser's match logic;
// future failure-rate axes (if AISL adds any) get added here. Empty today.
const INVERTED_AXIS_MAPPINGS: &[AxisMapping] = &[];

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
        let history = history_map.and_then(|m| m.get(&id).or_else(|| m.get(model_name)));

        let mut fields = BTreeMap::new();
        for mapping in AXIS_MAPPINGS {
            if let Some(value) =
                history.and_then(|h| axis_value(h, mapping.live_key, mapping.suite))
            {
                fields.insert(mapping.metric_key.to_string(), Value::from(value * 100.0));
            }
        }
        for mapping in INVERTED_AXIS_MAPPINGS {
            if let Some(value) =
                history.and_then(|h| axis_value(h, mapping.live_key, mapping.suite))
            {
                let inverted = (1.0 - value).clamp(0.0, 1.0);
                fields.insert(
                    mapping.metric_key.to_string(),
                    Value::from(inverted * 100.0),
                );
            }
        }
        if let Some(value) = canary_health(data, history, &id, model_name) {
            fields.insert(CANARY_HEALTH_METRIC.to_string(), Value::from(value));
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

/// Walk `historyMap[id]` newest-first and collect the first numeric value for
/// an axis in the requested suite. Suite-less entries are kept as a legacy
/// fallback for older endpoint shapes, but explicit `canary` rows are never
/// used for steady-state capability metrics.
fn axis_value(history: &Value, live_key: &str, suite: &str) -> Option<f64> {
    // The upstream `historyMap` is already newest-first (entry 0 carries
    // the most recent timestamp; index N is the oldest). Walking the array in
    // natural order gives us the newest matching suite reading.
    let mut legacy_fallback = None;
    let Some(entries) = history.as_array() else {
        return None;
    };
    for entry in entries {
        let entry_suite = entry.get("suite").and_then(Value::as_str);
        let suite_matches = entry_suite == Some(suite);
        let suite_missing = entry_suite.is_none();
        if !suite_matches && !suite_missing {
            continue;
        }
        let Some(axes) = entry.get("axes").and_then(Value::as_object) else {
            continue;
        };
        if DROPPED_AXIS_KEYS.contains(&live_key) {
            continue;
        }
        let Some(number) = axes
            .get(live_key)
            .and_then(|value| value_to_f64(Some(value)))
        else {
            continue;
        };
        if suite_matches {
            return Some(number);
        }
        if legacy_fallback.is_none() {
            legacy_fallback = Some(number);
        }
    }
    legacy_fallback
}

fn canary_health(data: &Value, history: Option<&Value>, id: &str, model_name: &str) -> Option<f64> {
    let suite_health = history
        .and_then(|h| axis_value(h, "correctness", "canary"))
        .map(|v| (v * 100.0).clamp(0.0, 100.0));
    let incident_health = canary_drift_health(data, id, model_name);

    match (suite_health, incident_health) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(v), None) | (None, Some(v)) => Some(v),
        (None, None) => None,
    }
}

fn canary_drift_health(data: &Value, id: &str, model_name: &str) -> Option<f64> {
    let incidents = data.get("driftIncidents").and_then(Value::as_array)?;
    let mut worst_health: Option<f64> = None;
    for incident in incidents {
        let resolved = incident
            .get("resolvedAt")
            .is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
        if resolved {
            continue;
        }
        let method = incident
            .get("metadata")
            .and_then(|m| m.get("detectionMethod"))
            .and_then(Value::as_str);
        if method != Some("canary_drift") {
            continue;
        }
        let matches_id = value_to_string_opt(incident.get("modelId")).as_deref() == Some(id);
        let matches_name = incident
            .get("modelName")
            .and_then(Value::as_str)
            .is_some_and(|name| name.eq_ignore_ascii_case(model_name));
        if !matches_id && !matches_name {
            continue;
        }
        let Some(drop_percent) = incident
            .get("metadata")
            .and_then(|m| m.get("dropPercent"))
            .and_then(|v| value_to_f64(Some(v)))
        else {
            continue;
        };
        let health = (100.0 - drop_percent).clamp(0.0, 100.0);
        worst_health = Some(worst_health.map_or(health, |prev| prev.min(health)));
    }
    worst_health
}

fn value_to_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn value_to_string_opt(value: Option<&Value>) -> Option<String> {
    value.map(value_to_string)
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
    fn parse_rows_keeps_axes_on_their_upstream_suite() {
        let payload = json!({
            "data": {
                "modelScores": [
                    { "id": "1", "name": "suite-aware-model", "vendor": "v", "currentScore": 50.0 }
                ],
                "historyMap": {
                    "1": [
                        {
                            "timestamp": "2026-04-26T03:30:00Z",
                            "score": 12.0,
                            "suite": "canary",
                            "axes": {
                                "correctness": 0.12,
                                "latency": 15,
                                "tasksCompleted": 1,
                                "totalTasks": 2
                            }
                        },
                        {
                            "timestamp": "2026-04-26T03:00:00Z",
                            "score": 70.0,
                            "suite": "tooling",
                            "axes": {
                                "toolSelection": 0.70,
                                "parameterAccuracy": 0.71,
                                "taskCompletion": 0.72,
                                "efficiency": 0.90,
                                "contextAwareness": 0.73,
                                "safetyCompliance": 0.74
                            }
                        },
                        {
                            "timestamp": "2026-04-26T02:00:00Z",
                            "score": 80.0,
                            "suite": "hourly",
                            "axes": {
                                "correctness": 0.80,
                                "format": 0.81,
                                "codeQuality": 0.82,
                                "efficiency": 0.83,
                                "stability": 0.84,
                                "safety": 0.85,
                                "debugging": 0.86,
                                "complexity": 0.87,
                                "edgeCases": 0.88
                            }
                        },
                        {
                            "timestamp": "2026-04-26T01:00:00Z",
                            "score": 60.0,
                            "suite": "deep",
                            "axes": {
                                "hallucinationRate": 0.60,
                                "planCoherence": 0.61,
                                "memoryRetention": 0.62,
                                "correctness": 0.63
                            }
                        }
                    ]
                }
            }
        });
        let rows = parse_rows(&payload).expect("should parse");
        let row = &rows[0];

        assert_eq!(
            row.fields.get("AI_correctness").and_then(Value::as_f64),
            Some(80.0)
        );
        assert_eq!(
            row.fields.get("AI_efficiency").and_then(Value::as_f64),
            Some(83.0)
        );
        assert_eq!(
            row.fields
                .get("AI_hallucination_resistance")
                .and_then(Value::as_f64),
            Some(60.0)
        );
        assert_eq!(
            row.fields.get("AI_tool_selection").and_then(Value::as_f64),
            Some(70.0)
        );
    }

    #[test]
    fn parse_rows_extracts_dedicated_canary_health_signal() {
        let payload = json!({
            "data": {
                "modelScores": [
                    { "id": "1", "name": "canary-model", "vendor": "v", "currentScore": 80.0 }
                ],
                "historyMap": {
                    "1": [
                        {
                            "timestamp": "2026-04-26T03:30:00Z",
                            "score": 35.0,
                            "suite": "canary",
                            "axes": {
                                "correctness": 0.35,
                                "latency": 15,
                                "tasksCompleted": 1,
                                "totalTasks": 2
                            }
                        },
                        {
                            "timestamp": "2026-04-26T03:00:00Z",
                            "score": 80.0,
                            "suite": "hourly",
                            "axes": {
                                "correctness": 0.80,
                                "format": 0.81,
                                "codeQuality": 0.82
                            }
                        }
                    ]
                },
                "driftIncidents": [
                    {
                        "modelId": 1,
                        "modelName": "canary-model",
                        "resolvedAt": null,
                        "metadata": {
                            "dropPercent": "40.0",
                            "detectionMethod": "canary_drift"
                        }
                    }
                ]
            }
        });
        let rows = parse_rows(&payload).expect("should parse");
        let row = &rows[0];

        assert_eq!(
            row.fields.get("AI_correctness").and_then(Value::as_f64),
            Some(80.0)
        );
        assert_eq!(
            row.fields.get("AI_canary_health").and_then(Value::as_f64),
            Some(35.0)
        );
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
