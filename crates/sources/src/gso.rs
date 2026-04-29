use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "gso";
const CACHE_KEY: &str = "gso";
/// Discovered from `https://livecodebench.github.io/gso.html`: the SPA fetches
/// `https://gso-bench.github.io/assets/leaderboard.json`. GSO is the
/// "Generalized Software Optimization" replacement track for the now-frozen
/// LiveCodeBench leaderboard — currently active and accepting frontier
/// submissions where LiveCodeBench has not since mid-2025.
const URL: &str = "https://gso-bench.github.io/assets/leaderboard.json";

#[derive(Debug, Default, Clone, Copy)]
pub struct GsoSource;

#[async_trait::async_trait]
impl Source for GsoSource {
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
        // GSO refreshes every few weeks; 2 days matches LiveCodeBench's old
        // contour and gives daily refresh.sh runs a free pass without round-
        // tripping for unchanged data.
        Duration::from_secs(2 * 24 * 3600)
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
            let payload = http.get_json(URL, &[]).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_json(dir, self.cache_key(), &payload)?;
            }
            payload
        };
        parse_rows(&payload)
    }
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("GSO payload missing models[]".into()))?;

    // GSO publishes one row per (model, scaffold, setting, reasoning_effort).
    // We only consume Opt@1 rows; among multiple Opt@1 rows for the same model
    // we prefer the lowest reasoning_effort (`high` over `xhigh`, unspecified
    // over `high`). This honors the project variant policy (medium/thinking/
    // adaptive only) where possible while still ingesting the frontier rows
    // GSO publishes only at -high (e.g. Claude Opus 4.7 high), per the
    // explicit per-source carve-out documented in CLAUDE.md/memory.
    //
    // We use `score_hack_control` (the contamination-resistant variant) over
    // raw `score`: GSO added the hack-control column specifically to penalize
    // deceptive optimizations, mirroring the rationale that already drove our
    // SWEComposite to favor SWERebench/Pro over Verified.
    let mut best: BTreeMap<String, (i64, f64)> = BTreeMap::new();
    for entry in models {
        let setting = entry.get("setting").and_then(Value::as_str).unwrap_or("");
        if !setting.eq_ignore_ascii_case("Opt@1") {
            continue;
        }
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(score) = entry
            .get("score_hack_control")
            .or_else(|| entry.get("score"))
            .and_then(Value::as_f64)
        else {
            continue;
        };
        if !score.is_finite() {
            continue;
        }
        let effort = entry
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .unwrap_or("");
        let priority = effort_priority(effort);
        let key = name.trim().to_string();
        match best.get(&key) {
            Some(&(existing_priority, _)) if existing_priority <= priority => {}
            _ => {
                best.insert(key, (priority, score));
            }
        }
    }

    let mut rows = Vec::with_capacity(best.len());
    for (name, (_priority, score)) in best {
        let mut fields = BTreeMap::new();
        fields.insert("GSO".to_string(), Value::from(score));
        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: name,
            vendor_hint: None,
            fields,
            synthesized_from: None,
        });
    }
    Ok(rows)
}

/// Lower priority wins when picking between Opt@1 rows for the same model.
/// Unspecified efforts beat -high beats -xhigh beats -max.
fn effort_priority(effort: &str) -> i64 {
    match effort.to_ascii_lowercase().as_str() {
        "" | "?" | "default" | "medium" | "adaptive" | "thinking" => 0,
        "low" => 1,
        "high" => 2,
        "xhigh" | "x-high" => 3,
        "max" | "pro" => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gso_fixture() {
        let payload: Value = serde_json::from_str(include_str!("../../../data/fixtures/gso.json"))
            .expect("fixture should parse as JSON");
        let rows = parse_rows(&payload).expect("fixture should parse");
        assert!(!rows.is_empty(), "expected GSO rows, got 0");
        for row in &rows {
            assert!(row.fields.contains_key("GSO"));
            let v = row.fields.get("GSO").and_then(Value::as_f64).unwrap();
            assert!(v.is_finite() && v >= 0.0);
        }
        // Frontier sanity: Claude Opus 4.7 should be present and score above
        // GPT 5.4 (per the 2026-03 leaderboard snapshot).
        let opus = rows
            .iter()
            .find(|r| r.model_name == "Claude Opus 4.7")
            .expect("Claude Opus 4.7 row must exist in fixture");
        let gpt54 = rows
            .iter()
            .find(|r| r.model_name == "GPT 5.4")
            .expect("GPT 5.4 row must exist in fixture");
        let opus_score = opus.fields.get("GSO").and_then(Value::as_f64).unwrap();
        let gpt54_score = gpt54.fields.get("GSO").and_then(Value::as_f64).unwrap();
        assert!(
            opus_score > gpt54_score,
            "Opus 4.7 ({opus_score}) should outscore GPT 5.4 ({gpt54_score}) in fixture"
        );
    }

    #[test]
    fn prefers_lower_reasoning_effort_when_duplicated() {
        // GPT 5.4 in the fixture has both `high` and `xhigh` Opt@1 rows.
        // The parser must keep `high` (lower-priority effort).
        let payload: Value = serde_json::from_str(include_str!("../../../data/fixtures/gso.json"))
            .expect("fixture should parse as JSON");
        let rows = parse_rows(&payload).expect("fixture should parse");
        let gpt54 = rows
            .iter()
            .find(|r| r.model_name == "GPT 5.4")
            .expect("GPT 5.4 row must exist");
        let score = gpt54.fields.get("GSO").and_then(Value::as_f64).unwrap();
        // The `high` row has score_hack_control=22.55; the `xhigh` row has 30.39.
        // We prefer `high`.
        assert!(
            (score - 22.55).abs() < 0.01,
            "expected GPT 5.4 to keep the `high` row score (22.55), got {score}"
        );
    }

    #[test]
    fn skips_non_opt_at_1_rows() {
        // Sonnet 3.5 V2 has both Opt@1 and Opt@10 rows in the fixture.
        let payload: Value = serde_json::from_str(include_str!("../../../data/fixtures/gso.json"))
            .expect("fixture should parse as JSON");
        let rows = parse_rows(&payload).expect("fixture should parse");
        let sonnet = rows.iter().find(|r| r.model_name == "Claude Sonnet 3.5 V2");
        if let Some(row) = sonnet {
            let score = row.fields.get("GSO").and_then(Value::as_f64).unwrap();
            // Opt@1 score is 4.6; Opt@10 is 15.7. Must be the Opt@1 value.
            assert!(
                (score - 4.6).abs() < 0.01,
                "expected Opt@1 value 4.6 (hack-control), got {score}"
            );
        }
    }
}
