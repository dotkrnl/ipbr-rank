//! DeepSWE v1.1 source.
//!
//! DeepSWE evaluates long-horizon repository work with a fixed
//! `mini-swe-agent` harness. The public artifact groups repeated rollouts by
//! model and reasoning effort and reports pass@1 together with a run-to-run
//! 95% confidence interval.
//!
//! The ranking compares models at their strongest published effort. We pick
//! one complete configuration per canonical model here (max, then xhigh,
//! high, thinking/adaptive, medium, default, low) instead of letting ingestion
//! choose every numeric field independently. That keeps the headline score,
//! confidence interval, and configuration provenance attached to the same
//! observation.

use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, alias_dedupe_key,
    cache_json_path, embedded_alias_records, read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "deep_swe_v1_1";
const CACHE_KEY: &str = "deep_swe_v1_1";
const URL: &str = "https://deepswe.datacurve.ai/artifacts/v1.1/leaderboard-live.json";
const FIXED_HARNESS: &str = "mini-swe-agent";
const MIN_TASKS_IN_SET: u64 = 100;
const MIN_MODEL_COHORT: usize = 5;

#[derive(Debug, Default, Clone, Copy)]
pub struct DeepSweV11Source;

#[async_trait::async_trait]
impl Source for DeepSweV11Source {
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

#[derive(Debug)]
struct Candidate {
    effort_priority: u8,
    attempted: u64,
    score: f64,
    row: RawRow,
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let tasks_in_set = payload
        .get("n_tasks_in_set")
        .and_then(Value::as_u64)
        .ok_or_else(|| SourceError::Parse("DeepSWE v1.1 payload missing n_tasks_in_set".into()))?;
    if tasks_in_set < MIN_TASKS_IN_SET {
        return Err(SourceError::Parse(format!(
            "DeepSWE v1.1 task set shrank to {tasks_in_set}; expected at least {MIN_TASKS_IN_SET}"
        )));
    }

    let entries = payload
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("DeepSWE v1.1 payload missing rows[]".into()))?;

    let alias_records = embedded_alias_records();
    let alias_index = ipbr_core::AliasIndex::build(&alias_records);
    let mut best_by_model: BTreeMap<String, Candidate> = BTreeMap::new();

    for entry in entries {
        let harness = entry.get("harness").and_then(Value::as_str).unwrap_or("");
        if !harness.eq_ignore_ascii_case(FIXED_HARNESS) {
            continue;
        }

        let Some(model_name) = entry
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let Some(score_fraction) = entry
            .get("pass_at_1")
            .or_else(|| entry.get("pass_rate"))
            .and_then(valid_fraction)
        else {
            continue;
        };

        let effort = entry
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|effort| !effort.is_empty())
            .unwrap_or("default");
        let priority = effort_priority(effort);
        let attempted = entry
            .get("n_attempted")
            .and_then(Value::as_u64)
            .unwrap_or_default();

        let mut fields = BTreeMap::new();
        fields.insert("DeepSWE".to_string(), Value::from(score_fraction * 100.0));
        fields.insert("DeepSWEReasoningEffort".to_string(), Value::from(effort));
        fields.insert("DeepSWEHarness".to_string(), Value::from(harness));

        copy_string(entry, "config", "DeepSWEConfig", &mut fields);
        copy_string(entry, "source", "DeepSWEUpstreamSource", &mut fields);
        copy_string(entry, "ci_method", "DeepSWECIMethod", &mut fields);
        copy_fraction(entry, "pass_at_4", "DeepSWEPassAt4", &mut fields);
        copy_fraction(entry, "ci_lo", "DeepSWECILow", &mut fields);
        copy_fraction(entry, "ci_hi", "DeepSWECIHigh", &mut fields);
        copy_u64(entry, "n_attempted", "DeepSWEAttempts", &mut fields);
        copy_u64(
            entry,
            "n_tasks_attempted",
            "DeepSWETasksAttempted",
            &mut fields,
        );
        copy_u64(entry, "n_runs", "DeepSWERuns", &mut fields);

        let row = RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
            synthesis_category: None,
        };
        let key = alias_dedupe_key(&alias_records, &alias_index, model_name, None);
        let candidate = Candidate {
            effort_priority: priority,
            attempted,
            score: score_fraction,
            row,
        };

        match best_by_model.get(&key) {
            Some(existing) if !prefer_candidate(&candidate, existing) => {}
            _ => {
                best_by_model.insert(key, candidate);
            }
        }
    }

    if best_by_model.len() < MIN_MODEL_COHORT {
        return Err(SourceError::Parse(format!(
            "DeepSWE v1.1 payload contained only {} usable fixed-harness models; expected at least {MIN_MODEL_COHORT}",
            best_by_model.len()
        )));
    }
    Ok(best_by_model
        .into_values()
        .map(|candidate| candidate.row)
        .collect())
}

fn prefer_candidate(incoming: &Candidate, existing: &Candidate) -> bool {
    incoming.effort_priority < existing.effort_priority
        || (incoming.effort_priority == existing.effort_priority
            && (incoming.attempted > existing.attempted
                || (incoming.attempted == existing.attempted && incoming.score > existing.score)))
}

/// Lower is stronger. Unknown future effort labels remain ingestible but sort
/// below known variants; retaining their raw label lets the core effort policy
/// decide whether they may score.
fn effort_priority(effort: &str) -> u8 {
    match effort
        .to_ascii_lowercase()
        .replace(['_', '-'], " ")
        .as_str()
    {
        "max" | "pro" => 0,
        "xhigh" | "x high" => 1,
        "high" => 2,
        "adaptive" | "thinking" => 3,
        "medium" => 4,
        "default" | "" => 5,
        "low" => 6,
        "non reasoning" | "none" => 7,
        _ => 8,
    }
}

fn valid_fraction(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
}

fn copy_fraction(entry: &Value, source: &str, target: &str, fields: &mut BTreeMap<String, Value>) {
    if let Some(value) = entry.get(source).and_then(valid_fraction) {
        fields.insert(target.to_string(), Value::from(value * 100.0));
    }
}

fn copy_u64(entry: &Value, source: &str, target: &str, fields: &mut BTreeMap<String, Value>) {
    if let Some(value) = entry.get(source).and_then(Value::as_u64) {
        fields.insert(target.to_string(), Value::from(value));
    }
}

fn copy_string(entry: &Value, source: &str, target: &str, fields: &mut BTreeMap<String, Value>) {
    if let Some(value) = entry
        .get(source)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        fields.insert(target.to_string(), Value::from(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medium_effort_precedes_default() {
        assert!(effort_priority("medium") < effort_priority("default"));
        assert!(effort_priority("default") < effort_priority("low"));
    }

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../data/fixtures/deep_swe_v1_1.json"))
            .expect("fixture should parse as JSON")
    }

    fn row<'a>(rows: &'a [RawRow], model: &str) -> &'a RawRow {
        rows.iter()
            .find(|row| row.model_name == model)
            .unwrap_or_else(|| panic!("missing {model}"))
    }

    #[test]
    fn parses_fixed_harness_fixture_and_scales_intervals() {
        let rows = parse_rows(&fixture()).expect("fixture should parse");
        assert_eq!(rows.len(), 5);

        let opus = row(&rows, "claude-opus-4-8");
        assert_eq!(opus.source_id, SOURCE_ID);
        assert_eq!(
            opus.fields.get("DeepSWEReasoningEffort"),
            Some(&Value::from("max"))
        );
        assert_eq!(
            opus.fields.get("DeepSWEHarness"),
            Some(&Value::from(FIXED_HARNESS))
        );
        assert_eq!(opus.fields.get("DeepSWERuns"), Some(&Value::from(4)));
        assert_eq!(
            opus.fields.get("DeepSWETasksAttempted"),
            Some(&Value::from(111))
        );
        assert_eq!(
            opus.fields.get("DeepSWEConfig"),
            Some(&Value::from("mini_swe_agent_claude_opus_4_8_max"))
        );

        let score = opus.fields["DeepSWE"].as_f64().unwrap();
        let pass_at_4 = opus.fields["DeepSWEPassAt4"].as_f64().unwrap();
        let ci_low = opus.fields["DeepSWECILow"].as_f64().unwrap();
        let ci_high = opus.fields["DeepSWECIHigh"].as_f64().unwrap();
        assert!((score - 58.974_358_974).abs() < 1e-8);
        assert!((pass_at_4 - 79.279_279_279).abs() < 1e-8);
        assert!((ci_low - 57.209_543_627).abs() < 1e-8);
        assert!((ci_high - 60.739_174_318).abs() < 1e-8);
        assert!(ci_low <= score && score <= ci_high);
    }

    #[test]
    fn prefers_max_effort_over_a_higher_scoring_xhigh_row() {
        let rows = parse_rows(&fixture()).expect("fixture should parse");
        let fable = row(&rows, "claude-fable-5");
        assert_eq!(
            fable.fields.get("DeepSWEReasoningEffort"),
            Some(&Value::from("max"))
        );
        let score = fable.fields["DeepSWE"].as_f64().unwrap();
        assert!((score - 69.724_770_642).abs() < 1e-8);
    }

    #[test]
    fn falls_back_to_xhigh_then_to_unspecified_default() {
        let rows = parse_rows(&fixture()).expect("fixture should parse");
        let gpt = row(&rows, "gpt-5-5");
        assert_eq!(
            gpt.fields.get("DeepSWEReasoningEffort"),
            Some(&Value::from("xhigh"))
        );
        let kimi = row(&rows, "kimi-k2-7-code");
        assert_eq!(
            kimi.fields.get("DeepSWEReasoningEffort"),
            Some(&Value::from("default"))
        );
    }

    #[test]
    fn rejects_payload_without_usable_fixed_harness_rows() {
        let payload = serde_json::json!({
            "n_tasks_in_set": 113,
            "rows": [{
                "model": "gpt-5-5",
                "harness": "different-agent",
                "pass_at_1": 0.99
            }]
        });
        let error = parse_rows(&payload).expect_err("wrong harness must not be accepted");
        assert!(
            error
                .to_string()
                .contains("only 0 usable fixed-harness models")
        );
    }

    #[test]
    fn rejects_small_task_set_and_partial_model_cohort() {
        let mut small_tasks = fixture();
        small_tasks["n_tasks_in_set"] = Value::from(99);
        let error = parse_rows(&small_tasks).expect_err("small task set must fail");
        assert!(error.to_string().contains("task set shrank"));

        let mut partial = fixture();
        partial["rows"] = Value::Array(
            partial["rows"]
                .as_array()
                .expect("fixture rows")
                .iter()
                .take(2)
                .cloned()
                .collect(),
        );
        let error = parse_rows(&partial).expect_err("partial cohort must fail");
        assert!(error.to_string().contains("usable fixed-harness models"));
    }
}
