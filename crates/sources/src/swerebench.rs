//! SWE-rebench source — continuously-refreshed agentic SWE leaderboard.
//!
//! SWE-rebench publishes a rolling-window benchmark sourced from real GitHub
//! pull requests filed *after* each model's release date, which removes the
//! contamination concerns that plague static SWE-bench Verified.
//!
//! The site at <https://swe-rebench.com> renders in the browser with Next.js;
//! the leaderboard payload is server-rendered into the HTML as a JSON-encoded
//! React Server Component blob. We locate the `"items":[…]` array, unescape
//! the embedded JSON, and pick out each model's resolved rate over its full
//! observation window. We prefer the `tools` agent variant (agentic execution)
//! and fall back to `text` if a model only ships in the non-agentic harness.
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
    let mut by_model: BTreeMap<String, (i32, f64)> = BTreeMap::new();
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
        let Some(rate) = headline_rate(item) else {
            continue;
        };
        by_model
            .entry(name.to_string())
            .and_modify(|slot| {
                if priority > slot.0 {
                    *slot = (priority, rate);
                }
            })
            .or_insert((priority, rate));
    }

    if by_model.is_empty() {
        return Err(SourceError::Parse(
            "SWE-rebench items array yielded no models with resolved rates".into(),
        ));
    }

    Ok(by_model
        .into_iter()
        .map(|(model_name, (_, rate))| {
            let mut fields = BTreeMap::new();
            fields.insert("SWERebench".to_string(), Value::from(rate));
            RawRow {
                source_id: SOURCE_ID.to_string(),
                model_name,
                vendor_hint: None,
                fields,
                synthesized_from: None,
            }
        })
        .collect())
}

fn headline_rate(item: &Value) -> Option<f64> {
    let trt = item.get("taskRangeTimestamp")?;
    let from = trt.get("from").and_then(Value::as_i64)?;
    let to = trt.get("to").and_then(Value::as_i64)?;
    let key = format!("{from}:{to}");
    let stats = item.get("rangeStats")?.get(&key)?;
    let rate = stats.get("resolvedRate").and_then(Value::as_f64)?;
    if rate.is_finite() { Some(rate) } else { None }
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
    }

    #[test]
    fn missing_anchor_errors() {
        let err = parse_rows("<html>nothing here</html>").unwrap_err();
        assert!(matches!(err, SourceError::Parse(_)));
    }
}
