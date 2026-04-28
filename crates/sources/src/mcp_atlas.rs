//! MCP-Atlas source — Scale's tool-use leaderboard.
//!
//! MCP-Atlas evaluates how well models orchestrate real Model Context Protocol
//! tools across 36 servers / 220 tools / 1000 tasks. Each task asks the agent
//! to identify the right servers, sequence 3-6 tool calls, and return a
//! correct end-state — the closest public proxy for "real Claude Code / Codex
//! style work" that we can ingest.
//!
//! The leaderboard at <https://labs.scale.com/leaderboard/mcp_atlas> is a
//! Next.js page that streams its data via React Server Component pushes
//! (`self.__next_f.push([1, "<JSON-with-escaped-quotes>"])`). The headline
//! pass-rate per model is embedded as `"model":"<name>","...","score":<N>`
//! within those streamed chunks. We don't need to reconstruct the full RSC
//! tree — scanning for the `\"model\":\"…\"` anchor and pulling the nearest
//! `\"score\":` value within the same JSON object is enough.
//!
//! Fragility: depends on the JSON-shape Scale embeds in the streamed RSC.
//! If the field names change (`model` → `name`, `score` → `passRate`), the
//! parser will need updating.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_html_path,
    read_cached_string, use_cached_html, write_cache_html,
};

const SOURCE_ID: &str = "mcp_atlas";
const CACHE_KEY: &str = "mcp_atlas";
const URL: &str = "https://labs.scale.com/leaderboard/mcp_atlas";

#[derive(Debug, Default, Clone, Copy)]
pub struct McpAtlasSource;

#[async_trait::async_trait]
impl Source for McpAtlasSource {
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
        // Scale refreshes the public leaderboard on the order of weeks.
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
        parse_rows(&html, "MCPAtlas", SOURCE_ID)
    }
}

/// Shared parser for Scale's RSC-embedded leaderboards. Walks the document
/// looking for `\"model\":\"…\"` anchors and pulls the nearest `\"score\":N`
/// within the same JSON object (closing `}` or 400 bytes — whichever first).
/// Used by both `mcp_atlas` and `swebench_pro` since they share the embedding.
pub(crate) fn parse_rows(
    html: &str,
    metric: &str,
    source_id: &str,
) -> Result<Vec<RawRow>, SourceError> {
    const MODEL_ANCHOR: &str = r#"\"model\":\""#;
    const SCORE_ANCHOR: &str = r#"\"score\":"#;
    const WINDOW: usize = 600;

    let mut rows: Vec<RawRow> = Vec::new();
    let mut seen: BTreeMap<String, f64> = BTreeMap::new();
    let bytes = html.as_bytes();
    let mut cursor = 0usize;

    while let Some(rel) = html[cursor..].find(MODEL_ANCHOR) {
        let name_start = cursor + rel + MODEL_ANCHOR.len();
        let Some(name_end_rel) = find_escaped_quote(&bytes[name_start..]) else {
            break;
        };
        let name_end = name_start + name_end_rel;
        let model_name = html[name_start..name_end].trim();
        cursor = name_end + 2; // skip past `\"`

        if model_name.is_empty() {
            continue;
        }

        // Search forward for the nearest `\"score\":N`, but stop at a `}` that
        // closes the current JSON object so we don't grab a sibling row's score.
        let window_end = bytes.len().min(name_end + WINDOW);
        let window = &html[name_end..window_end];
        let Some(score_pos) = window.find(SCORE_ANCHOR) else {
            continue;
        };
        // Reject if a closing brace appears before the score anchor.
        if let Some(brace_pos) = find_object_close(&window[..score_pos])
            && brace_pos < score_pos
        {
            continue;
        }
        let num_start = score_pos + SCORE_ANCHOR.len();
        let mut num_end = num_start;
        while num_end < window.len() {
            let b = window.as_bytes()[num_end];
            if b.is_ascii_digit() || b == b'.' || b == b'-' {
                num_end += 1;
            } else {
                break;
            }
        }
        if num_end == num_start {
            continue;
        }
        let Ok(score) = window[num_start..num_end].parse::<f64>() else {
            continue;
        };
        if !score.is_finite() {
            continue;
        }

        // The same model can appear multiple times in the streamed chunks
        // (header card + table row). Keep the first occurrence to lock onto
        // the headline value rather than chasing a sub-score.
        if seen.contains_key(model_name) {
            continue;
        }
        seen.insert(model_name.to_string(), score);

        let mut fields = BTreeMap::new();
        fields.insert(metric.to_string(), Value::from(score));
        rows.push(RawRow {
            source_id: source_id.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
        });
    }

    if rows.is_empty() {
        return Err(SourceError::Parse(format!(
            "{source_id} HTML yielded no model rows"
        )));
    }
    Ok(rows)
}

/// Finds the index of an escaped closing quote `\"` in the byte slice.
fn find_escaped_quote(bytes: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1] == b'"' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Returns the byte offset of an unescaped `}` in `slice`, or `None`. Used
/// to detect the close of a JSON object before reaching the score field.
fn find_object_close(slice: &str) -> Option<usize> {
    slice.bytes().position(|b| b == b'}')
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"<html>self.__next_f.push([1,"junk \"company\":\"anthropic\",\"model\":\"Claude Opus 4.7 (max)\",\"score\":79.1,\"order\":1}, {\"company\":\"google\",\"model\":\"Gemini 3.1 Pro\",\"score\":78.2}, {\"model\":\"Bad Entry\",\"other\":42}, {\"company\":\"zai\",\"model\":\"glm-5p1\",\"score\":75.6}"])</html>"#;

    #[test]
    fn extracts_model_score_pairs() {
        let rows = parse_rows(FIXTURE, "MCPAtlas", "mcp_atlas").expect("fixture should parse");
        let by: BTreeMap<_, _> = rows
            .iter()
            .map(|r| {
                (
                    r.model_name.as_str(),
                    r.fields["MCPAtlas"].as_f64().unwrap(),
                )
            })
            .collect();
        assert_eq!(by["Claude Opus 4.7 (max)"], 79.1);
        assert_eq!(by["Gemini 3.1 Pro"], 78.2);
        assert_eq!(by["glm-5p1"], 75.6);
        assert!(!by.contains_key("Bad Entry"));
    }
}
