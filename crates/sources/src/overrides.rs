//! Manual overrides source — vendor-published metric values.
//!
//! Some metrics on some models are absent from public leaderboards (because
//! the model launched after the leaderboard's last refresh, the vendor never
//! submitted, or the upstream simply rejects the entry). We fill those gaps
//! by hand from vendor system cards, launch posts, or other authoritative
//! secondary sources, and load them through this source so they flow through
//! the same ingest path as everything else.
//!
//! Provenance is preserved: every override row carries `source_id =
//! "overrides"`, so its origin is visible in the scoreboard's source summary
//! and per-metric provenance trails.
//!
//! Schema (`data/score_overrides.toml`):
//!
//! ```toml
//! [[entries]]
//! canonical_id = "anthropic/claude-opus-4.7"
//! metric       = "SWEBenchVerified"
//! value        = 87.6
//! note         = "Anthropic Opus 4.7 launch announcement, 2026-04-16"
//! ```
//!
//! Each entry is keyed by canonical_id (the alias matcher will recognize it
//! by definition) and emits a single-field `RawRow` for the named metric.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ipbr_core::RawRow;
use serde::Deserialize;
use serde_json::Value;

use crate::{FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus};

const SOURCE_ID: &str = "overrides";
const EMBEDDED: &str = include_str!("../../../data/score_overrides.toml");

#[derive(Debug, Clone, Default)]
pub struct OverridesSource {
    /// Optional override path — when `None`, the source uses the embedded
    /// copy of `data/score_overrides.toml` baked in at build time. Tests
    /// pass an explicit path to exercise a custom file.
    file_path: Option<PathBuf>,
}

impl OverridesSource {
    pub fn from_file(path: PathBuf) -> Self {
        Self {
            file_path: Some(path),
        }
    }
}

#[async_trait::async_trait]
impl Source for OverridesSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn cache_paths(&self, _cache_dir: &Path) -> Vec<PathBuf> {
        // No on-disk cache — the override file is checked into the repo and
        // baked into the binary, so cache TTL semantics don't apply.
        Vec::new()
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Verified
    }

    fn required_secret(&self) -> Option<crate::SecretRef> {
        None
    }

    async fn fetch(
        &self,
        _http: &dyn Http,
        _opts: FetchOptions<'_>,
        _secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        let raw = match &self.file_path {
            Some(path) => std::fs::read_to_string(path).map_err(|err| {
                SourceError::Parse(format!(
                    "overrides file {} unreadable: {err}",
                    path.display()
                ))
            })?,
            None => EMBEDDED.to_string(),
        };
        parse_rows(&raw)
    }
}

#[derive(Debug, Deserialize)]
struct OverrideFile {
    #[serde(default)]
    entries: Vec<OverrideEntry>,
}

#[derive(Debug, Deserialize)]
struct OverrideEntry {
    canonical_id: String,
    metric: String,
    value: f64,
    /// Free-form provenance string. Captured at parse time as a sanity check
    /// that authors record where each number came from.
    #[serde(default)]
    #[allow(dead_code)]
    note: String,
}

fn parse_rows(toml_text: &str) -> Result<Vec<RawRow>, SourceError> {
    let parsed: OverrideFile = toml::from_str(toml_text)
        .map_err(|err| SourceError::Parse(format!("overrides TOML invalid: {err}")))?;

    let mut rows = Vec::with_capacity(parsed.entries.len());
    for entry in parsed.entries {
        if entry.metric.trim().is_empty() {
            return Err(SourceError::Parse(format!(
                "overrides entry for {} has empty metric",
                entry.canonical_id
            )));
        }
        if entry.note.trim().is_empty() {
            return Err(SourceError::Parse(format!(
                "overrides entry {}.{} is missing a note",
                entry.canonical_id, entry.metric
            )));
        }
        if !entry.value.is_finite() {
            return Err(SourceError::Parse(format!(
                "overrides entry {}.{} has non-finite value",
                entry.canonical_id, entry.metric
            )));
        }
        let mut fields = BTreeMap::new();
        fields.insert(entry.metric.clone(), Value::from(entry.value));
        fields.insert("Note".to_string(), Value::from(entry.note.clone()));
        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            // The alias matcher takes any registered alias, and every
            // canonical_id is itself a registered alias by construction.
            model_name: entry.canonical_id.clone(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
[[entries]]
canonical_id = "anthropic/claude-opus-4.7"
metric = "SWEBenchVerified"
value = 87.6
note = "Anthropic launch, 2026-04-16"

[[entries]]
canonical_id = "openai/gpt-5.5"
metric = "TerminalBench"
value = 82.7
note = "OpenAI launch, 2026-04-23"
"#;

    #[test]
    fn parses_entries_into_rows() {
        let rows = parse_rows(FIXTURE).expect("fixture should parse");
        assert_eq!(rows.len(), 2);
        let by_model: BTreeMap<_, _> = rows
            .iter()
            .map(|r| (r.model_name.as_str(), &r.fields))
            .collect();
        assert_eq!(
            by_model["anthropic/claude-opus-4.7"]
                .get("SWEBenchVerified")
                .and_then(Value::as_f64),
            Some(87.6)
        );
        assert_eq!(
            by_model["anthropic/claude-opus-4.7"]
                .get("Note")
                .and_then(Value::as_str),
            Some("Anthropic launch, 2026-04-16")
        );
        assert_eq!(
            by_model["openai/gpt-5.5"]
                .get("TerminalBench")
                .and_then(Value::as_f64),
            Some(82.7)
        );
    }

    #[test]
    fn empty_metric_is_rejected() {
        let bad = r#"
[[entries]]
canonical_id = "x"
metric = ""
value = 1.0
"#;
        assert!(parse_rows(bad).is_err());
    }

    #[test]
    fn missing_note_is_rejected() {
        let bad = r#"
[[entries]]
canonical_id = "x"
metric = "TerminalBench"
value = 1.0
"#;
        assert!(parse_rows(bad).is_err());
    }

    #[test]
    fn empty_file_is_ok() {
        let rows = parse_rows("").expect("empty TOML should parse");
        assert!(rows.is_empty());
    }
}
