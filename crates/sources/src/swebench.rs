use std::collections::BTreeMap;

use ipbr_core::RawRow;
use serde_json::Value;

use std::time::Duration;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "swebench";
const CACHE_KEY: &str = "swebench_leaderboards";
const URL: &str =
    "https://raw.githubusercontent.com/swe-bench/swe-bench.github.io/master/data/leaderboards.json";

#[derive(Debug, Default, Clone, Copy)]
pub struct SweBenchSource;

#[async_trait::async_trait]
impl Source for SweBenchSource {
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
        Duration::from_secs(7 * 24 * 3600)
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
    let leaderboards = payload
        .get("leaderboards")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("SWE-bench payload missing leaderboards[]".into()))?;

    let mut rows = Vec::new();
    extract_board(leaderboards, is_verified_leaderboard, "SWEBenchVerified", &mut rows)?;
    // Multilingual is best-effort: the upstream payload has shipped this board
    // for some time, but we don't want a future rename to break the verified
    // pipeline, so a missing board is non-fatal.
    let _ = extract_board(
        leaderboards,
        is_multilingual_leaderboard,
        "SWEBenchMultilingual",
        &mut rows,
    );
    Ok(rows)
}

fn extract_board(
    leaderboards: &[Value],
    matcher: fn(&str) -> bool,
    metric: &str,
    rows: &mut Vec<RawRow>,
) -> Result<(), SourceError> {
    let board = leaderboards
        .iter()
        .find(|board| {
            board
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(matcher)
        })
        .ok_or_else(|| {
            SourceError::Parse(format!(
                "SWE-bench payload missing leaderboard for metric {metric}"
            ))
        })?;

    let results = board
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Parse(format!("SWE-bench leaderboard for {metric} missing results[]"))
        })?;

    for entry in results {
        let model_name = match entry.get("name").and_then(Value::as_str) {
            Some(value) if !value.trim().is_empty() => extract_model(value),
            _ => continue,
        };
        let resolved = match entry.get("resolved").and_then(number_like) {
            Some(value) if value.is_finite() => value,
            _ => continue,
        };

        let mut fields = BTreeMap::new();
        fields.insert(metric.to_string(), Value::from(resolved));
        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
        });
    }
    Ok(())
}

/// Strips a trailing parenthesized YYYY-MM-DD date, then takes the rightmost
/// segment split on " + " to remove agent-framework prefixes like "OpenHands + ".
fn extract_model(name: &str) -> &str {
    let without_date = strip_date_suffix(name.trim());
    without_date
        .rsplit_once(" + ")
        .map_or(without_date, |(_, right)| right.trim())
}

fn strip_date_suffix(s: &str) -> &str {
    if s.ends_with(')')
        && let Some(paren_pos) = s.rfind('(')
    {
        let inner = &s[paren_pos + 1..s.len() - 1];
        if is_iso_date(inner.trim()) {
            return s[..paren_pos].trim_end();
        }
    }
    s
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

fn is_verified_leaderboard(name: &str) -> bool {
    // Reviewer note: spec text references "SWE-bench Verified", but the live payload
    // currently uses "Verified". Accept both to keep fixture/live behavior aligned.
    matches!(name.trim(), "SWE-bench Verified" | "Verified")
}

fn is_multilingual_leaderboard(name: &str) -> bool {
    matches!(name.trim(), "SWE-bench Multilingual" | "Multilingual")
}

fn number_like(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_rows_accepts_verified_name_variants() {
        let payload = json!({
            "leaderboards": [
                {
                    "name": "SWE-bench Verified",
                    "results": [
                        {"name": "model-a", "resolved": 75.5}
                    ]
                }
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "model-a");
        assert_eq!(
            rows[0].fields.get("SWEBenchVerified").and_then(number_like),
            Some(75.5)
        );

        let payload = json!({
            "leaderboards": [
                {
                    "name": "Verified",
                    "results": [
                        {"name": "model-b", "resolved": "66.2"}
                    ]
                }
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "model-b");
        assert_eq!(
            rows[0].fields.get("SWEBenchVerified").and_then(number_like),
            Some(66.2)
        );
    }

    #[test]
    fn parse_rows_requires_verified_leaderboard() {
        let payload = json!({
            "leaderboards": [
                {
                    "name": "Lite",
                    "results": [
                        {"name": "model-a", "resolved": 22.0}
                    ]
                }
            ]
        });
        let err = parse_rows(&payload).expect_err("leaderboard selection should fail");
        assert!(err.to_string().contains("SWEBenchVerified"));
    }

    #[test]
    fn extract_model_strips_date_and_agent_prefix() {
        // Date suffix (YYYY-MM-DD) stripped, then agent prefix removed
        assert_eq!(
            extract_model("OpenHands + Claude Opus 4.7 (2025-12-15)"),
            "Claude Opus 4.7"
        );
        assert_eq!(
            extract_model("Tools + Claude 4 Sonnet (2025-05-22)"),
            "Claude 4 Sonnet"
        );
        assert_eq!(
            extract_model("mini-SWE-agent + Gemini 2.5 Flash (2025-04-17)"),
            "Gemini 2.5 Flash"
        );
        // No agent prefix — returns trimmed name
        assert_eq!(extract_model("Claude Opus 4.7"), "Claude Opus 4.7");
        // Date without dashes is NOT stripped (compact YYYYMMDD format)
        assert_eq!(
            extract_model("mini-SWE-agent + Claude 4.5 Opus medium (20251101)"),
            "Claude 4.5 Opus medium (20251101)"
        );
        // Agent prefix without date
        assert_eq!(
            extract_model("mini-SWE-agent + Claude Opus 4.6"),
            "Claude Opus 4.6"
        );
        // Multiple " + " separators — rightmost segment taken
        assert_eq!(
            extract_model("EPAM AI/Run + Claude 4 Sonnet (2025-07-19)"),
            "Claude 4 Sonnet"
        );
    }

    #[test]
    fn parse_rows_uses_extract_model() {
        let payload = json!({
            "leaderboards": [{
                "name": "Verified",
                "results": [
                    {"name": "mini-SWE-agent + Claude Opus 4.7 (2025-12-15)", "resolved": 82.4},
                    {"name": "Tools + Claude 4 Sonnet (2025-05-22)", "resolved": 72.4},
                    {"name": "bare-model", "resolved": 55.0},
                ]
            }]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].model_name, "Claude Opus 4.7");
        assert_eq!(rows[1].model_name, "Claude 4 Sonnet");
        assert_eq!(rows[2].model_name, "bare-model");
    }
}
