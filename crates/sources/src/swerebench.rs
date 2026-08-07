//! SWE-rebench source — continuously-refreshed agentic SWE leaderboard.
//!
//! SWE-rebench publishes a rolling-window benchmark sourced from real GitHub
//! pull requests filed *after* each model's release date, which removes the
//! contamination concerns that plague static SWE-bench Verified.
//!
//! The site at <https://swe-rebench.com> renders in the browser with Next.js;
//! the leaderboard payload is server-rendered into the HTML as a JSON-encoded
//! React Server Component blob. We locate the `"items":[…]` array, unescape
//! the embedded JSON, and pick out each model's resolved rate over its latest
//! full post-release observation window. This release-date check is important:
//! the upstream payload also includes ranges that the site marks as potentially
//! contaminated because their tasks predate the model. We prefer the `tools`
//! agent variant (agentic execution) and fall back to `text` if a model only
//! ships in the non-agentic harness.
//!
//! The HTML embedding is the most fragile part of this source — if the site
//! switches to client-side hydration or renames the keys, we'll need to
//! adjust. The fetch logic itself is otherwise simple and dependency-free
//! (we lean on serde_json once the JSON has been unescaped).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_html_path,
    read_cached_string, use_cached_html, write_cache_html,
};

const SOURCE_ID: &str = "swerebench";
const CACHE_KEY: &str = "swerebench";
const URL: &str = "https://swe-rebench.com";
const ITEMS_ANCHOR: &str = r#"\"items\":["#;

#[derive(Debug, Default, Clone, Copy)]
pub struct SweRebenchSource;

#[async_trait::async_trait]
impl Source for SweRebenchSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        CACHE_KEY
    }

    fn cache_paths(&self, cache_dir: &Path) -> Vec<PathBuf> {
        vec![cache_html_path(cache_dir, self.cache_key())]
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
        let html = if use_cached_html(opts, self.cache_key(), self.cache_ttl()) {
            let Some(dir) = opts.cache_dir else {
                return Err(SourceError::CacheMiss(format!(
                    "{} requires --cache in --offline mode",
                    self.id()
                )));
            };
            read_cached_string(&cache_html_path(dir, self.cache_key()))?
        } else {
            let html = http.get_text(URL, &[("User-Agent", "ipbr-rank")]).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_html(dir, self.cache_key(), &html)?;
            }
            html
        };
        parse_rows(&html)
    }
}

fn parse_rows(html: &str) -> Result<Vec<RawRow>, SourceError> {
    let array_text = extract_items_array(html)?;
    let json = unescape_jsx_string(&array_text);
    let items: Vec<Value> = serde_json::from_str(&json).map_err(|err| {
        SourceError::Parse(format!("SWE-rebench items array failed to parse: {err}"))
    })?;

    // Prefer the `tools` (agentic) variant per model, fall back to `text`.
    let mut by_model: BTreeMap<String, (i32, f64, Option<f64>)> = BTreeMap::new();
    for item in &items {
        let Some(name) = item.get("modelName").and_then(Value::as_str) else {
            continue;
        };
        let agent = item
            .get("agentVersion")
            .and_then(Value::as_str)
            .unwrap_or("");
        let priority = match agent {
            "tools" => 2,
            "text" => 1,
            _ => 0,
        };
        let Some((rate, sem)) = headline_stats(item) else {
            continue;
        };
        by_model
            .entry(name.to_string())
            .and_modify(|slot| {
                if priority > slot.0 {
                    *slot = (priority, rate, sem);
                }
            })
            .or_insert((priority, rate, sem));
    }

    if by_model.is_empty() {
        return Err(SourceError::Parse(
            "SWE-rebench items array yielded no models with resolved rates".into(),
        ));
    }

    Ok(by_model
        .into_iter()
        .map(|(model_name, (_, rate, sem))| {
            let mut fields = BTreeMap::new();
            fields.insert("SWERebench".to_string(), Value::from(rate));
            if let Some(sem) = sem {
                // Auxiliary, unscored standard error for the selected agent
                // variant and task window.
                fields.insert("SWERebenchSEM".to_string(), Value::from(sem));
            }
            RawRow {
                source_id: SOURCE_ID.to_string(),
                model_name,
                vendor_hint: None,
                fields,
            }
        })
        .collect())
}

/// Resolves the `"from:to" -> stats` map for an item. The current upstream
/// payload nests ranges under per-language segments (`all`, `go`, `java`,
/// `python`, `rust`, `typescript`); `all` is the cross-language aggregate the
/// leaderboard headlines. Older payloads placed ranges flat under `rangeStats`
/// directly, which we keep as a fallback so a partial upstream change still
/// parses. Timestamps shifted from seconds to milliseconds in the same revision,
/// but that is transparent here — `release` and the range keys share a unit, so
/// the `from < release` contamination check is unaffected.
fn range_map(item: &Value) -> Option<&serde_json::Map<String, Value>> {
    let rs = item.get("rangeStats")?.as_object()?;
    if let Some(all) = rs.get("all").and_then(Value::as_object) {
        return Some(all);
    }
    Some(rs)
}

fn headline_stats(item: &Value) -> Option<(f64, Option<f64>)> {
    let release = item
        .get("release")?
        .get("timestamp")
        .and_then(Value::as_i64)?;
    let ranges = range_map(item)?;

    // The page's selected `taskRangeTimestamp` may begin before the model was
    // released. The site flags such rows as potentially contaminated, but that
    // warning is presentation metadata rather than part of the selected stats.
    // Pick the most recent ending range and, among ranges ending together, the
    // widest one that starts on or after release. If no such range exists, the
    // model has no uncontaminated SWE-rebench observation yet and is omitted.
    let mut best: Option<(i64, i64, f64, Option<f64>)> = None;
    for (key, stats) in ranges {
        let Some((from, to)) = key.split_once(':') else {
            continue;
        };
        let (Ok(from), Ok(to)) = (from.parse::<i64>(), to.parse::<i64>()) else {
            continue;
        };
        if from < release || to <= from {
            continue;
        }
        let Some(rate) = stats.get("resolvedRate").and_then(Value::as_f64) else {
            continue;
        };
        if !rate.is_finite() {
            continue;
        }
        let sem = stats
            .get("sem")
            .and_then(Value::as_f64)
            .filter(|sem| sem.is_finite() && *sem >= 0.0);
        let replace = best.as_ref().is_none_or(|(best_from, best_to, _, _)| {
            to > *best_to || (to == *best_to && from < *best_from)
        });
        if replace {
            best = Some((from, to, rate, sem));
        }
    }

    best.map(|(_, _, rate, sem)| (rate, sem))
}

/// Locates the JSON-array body of the embedded `"items":[…]` payload. The
/// HTML stream uses `\"` for `"` and balances brackets in the escaped form,
/// so we mirror that during the bracket walk.
fn extract_items_array(html: &str) -> Result<String, SourceError> {
    let anchor_pos = html
        .find(ITEMS_ANCHOR)
        .ok_or_else(|| SourceError::Parse("SWE-rebench HTML missing items[] anchor".into()))?;
    let start = anchor_pos + ITEMS_ANCHOR.len() - 1; // includes opening '['
    let bytes = html.as_bytes();
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut i = start;
    while i < bytes.len() {
        // `\"` in the byte stream toggles a string boundary.
        if i + 1 < bytes.len() && bytes[i] == b'\\' && bytes[i + 1] == b'"' {
            in_string = !in_string;
            i += 2;
            continue;
        }
        if !in_string {
            match bytes[i] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(html[start..=i].to_string());
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    Err(SourceError::Parse(
        "SWE-rebench items[] array did not close cleanly".into(),
    ))
}

/// Unwinds one layer of JSON escaping: `\\\\` → `\\`, `\\"` → `"`. Other
/// escapes (`\\u…`, `\\n`) round-trip through serde_json's parser fine, so we
/// leave them alone.
fn unescape_jsx_string(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    let bytes = escaped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'\\' {
            match bytes[i + 1] {
                b'"' => {
                    out.push('"');
                    i += 2;
                    continue;
                }
                b'\\' => {
                    out.push('\\');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mimics the structure of the real HTML payload: an `\"items\":[…]`
    /// fragment with two items at the same task range, one in `tools` mode
    /// and one in `text` mode. The parser should prefer `tools`.
    const FIXTURE: &str = r#"<html><body>some streaming junk here \"items\":[{\"modelId\":\"opus__tools\",\"modelName\":\"Claude Opus 4.7\",\"release\":{\"timestamp\":1,\"date\":\"2026-01-01\"},\"taskRangeTimestamp\":{\"from\":100,\"to\":200},\"agentVersion\":\"tools\",\"rangeStats\":{\"100:200\":{\"resolvedRate\":61.5,\"sem\":0.4,\"passN\":80.0,\"instanceCosts\":1.5,\"totalTokenUsage\":42}}},{\"modelId\":\"opus__text\",\"modelName\":\"Claude Opus 4.7\",\"release\":{\"timestamp\":1,\"date\":\"2026-01-01\"},\"taskRangeTimestamp\":{\"from\":100,\"to\":200},\"agentVersion\":\"text\",\"rangeStats\":{\"100:200\":{\"resolvedRate\":40.0,\"sem\":0.5,\"passN\":50.0,\"instanceCosts\":0.7,\"totalTokenUsage\":21}}},{\"modelId\":\"glm__text\",\"modelName\":\"GLM-5.1\",\"release\":{\"timestamp\":2,\"date\":\"2026-02-01\"},\"taskRangeTimestamp\":{\"from\":100,\"to\":200},\"agentVersion\":\"text\",\"rangeStats\":{\"100:200\":{\"resolvedRate\":33.3,\"sem\":0.6,\"passN\":42.0,\"instanceCosts\":0.4,\"totalTokenUsage\":15}}}],\"otherKey\":\"...\"}</body></html>"#;

    #[test]
    fn prefers_tools_variant_and_collapses_per_model() {
        let rows = parse_rows(FIXTURE).expect("fixture should parse");
        let by: BTreeMap<_, _> = rows
            .iter()
            .map(|r| (r.model_name.as_str(), r.fields.get("SWERebench").unwrap()))
            .collect();
        assert_eq!(by.len(), 2);
        assert_eq!(by["Claude Opus 4.7"], &Value::from(61.5));
        assert_eq!(by["GLM-5.1"], &Value::from(33.3));

        let opus = rows
            .iter()
            .find(|row| row.model_name == "Claude Opus 4.7")
            .expect("tools row should be retained");
        assert_eq!(opus.fields.get("SWERebenchSEM"), Some(&Value::from(0.4)));
        let glm = rows
            .iter()
            .find(|row| row.model_name == "GLM-5.1")
            .expect("text fallback should be retained");
        assert_eq!(glm.fields.get("SWERebenchSEM"), Some(&Value::from(0.6)));
    }

    #[test]
    fn selects_latest_full_post_release_range() {
        let item = serde_json::json!({
            "release": { "timestamp": 150 },
            "taskRangeTimestamp": { "from": 100, "to": 400 },
            "rangeStats": {
                "100:400": { "resolvedRate": 99.0, "sem": 9.9 },
                "200:300": { "resolvedRate": 41.0, "sem": 0.8 },
                "250:400": { "resolvedRate": 50.0, "sem": 0.7 },
                "200:400": { "resolvedRate": 61.0, "sem": 0.6 },
                "300:400": { "resolvedRate": 70.0, "sem": 0.5 },
                "400:400": { "resolvedRate": 100.0, "sem": 0.0 }
            }
        });

        assert_eq!(headline_stats(&item), Some((61.0, Some(0.6))));
    }

    #[test]
    fn omits_model_without_post_release_range() {
        let item = serde_json::json!({
            "release": { "timestamp": 300 },
            "taskRangeTimestamp": { "from": 100, "to": 250 },
            "rangeStats": {
                "100:250": { "resolvedRate": 80.0, "sem": 0.5 },
                "200:300": { "resolvedRate": 90.0, "sem": 0.4 }
            }
        });

        assert_eq!(headline_stats(&item), None);
    }

    /// Current upstream payload: `rangeStats` nests ranges under per-language
    /// segments, `all` being the aggregate, and every timestamp is in
    /// milliseconds. We must descend into `all`, ignore the per-language
    /// segments, and still drop ranges that begin before release.
    #[test]
    fn descends_into_all_segment_of_nested_rangestats() {
        let item = serde_json::json!({
            "release": { "timestamp": 1744588800000i64 },
            "taskRangeTimestamp": { "from": 1735689600000i64, "to": 1754006400000i64 },
            "agentVersion": "tools",
            "rangeStats": {
                // Pre-release range (from < release) must be skipped even though
                // it has a higher headline rate.
                "1735689600000:1736899200000": { "resolvedRate": 99.0, "sem": 9.9 },
                // First full post-release range.
                "1744848000000:1748736000000": { "resolvedRate": 45.0, "sem": 0.5 },
                // Later, wider post-release range — latest end wins.
                "1744848000000:1754006400000": { "resolvedRate": 52.0, "sem": 0.4 },
                // Per-language segments must not be read as ranges.
                "python": { "1744848000000:1754006400000": { "resolvedRate": 12.0 } }
            }
        });

        assert_eq!(headline_stats(&item), Some((52.0, Some(0.4))));
    }

    /// End-to-end check that a realistic nested HTML payload (as emitted today)
    /// produces rows rather than the "no models with resolved rates" parse error.
    #[test]
    fn parses_current_nested_payload_shape() {
        let html = r#"<html>streaming junk here \"items\":[{\"modelId\":\"opus__tools\",\"modelName\":\"Claude Opus 4.7\",\"release\":{\"timestamp\":1744588800000,\"date\":\"2025-04-14\"},\"taskRangeTimestamp\":{\"from\":1735689600000,\"to\":1754006400000},\"agentVersion\":\"tools\",\"rangeStats\":{\"all\":{\"1735689600000:1736899200000\":{\"resolvedRate\":99.0,\"sem\":9.9},\"1744848000000:1754006400000\":{\"resolvedRate\":57.5,\"sem\":0.4}}}}],\"otherKey\":\"...\"}</html>"#;
        let rows = parse_rows(html).expect("nested payload should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "Claude Opus 4.7");
        assert_eq!(rows[0].fields.get("SWERebench"), Some(&Value::from(57.5)));
        assert_eq!(rows[0].fields.get("SWERebenchSEM"), Some(&Value::from(0.4)));
    }

    #[test]
    fn missing_anchor_errors() {
        let err = parse_rows("<html>nothing here</html>").unwrap_err();
        assert!(matches!(err, SourceError::Parse(_)));
    }
}
