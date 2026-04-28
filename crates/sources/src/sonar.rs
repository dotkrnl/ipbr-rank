//! Sonar Code Quality leaderboard source.
//!
//! Sonar publishes an LLM code-quality leaderboard at
//! <https://www.sonarsource.com/the-coding-personalities-of-leading-llms/leaderboard/>
//! that goes beyond pass-rate to measure properties of the *code itself*:
//! issue density (issues per kLOC), vulnerability density, bug density, and
//! cognitive/cyclomatic complexity. This is the rare benchmark that tries
//! to capture "is the generated code actually well-written" instead of
//! just "does it pass tests."
//!
//! The page is a client-rendered SPA that fetches its data from a static
//! JSON file:
//!
//!   `…/leaderboard/data.json`
//!
//! Schema (relevant fields):
//!
//! ```json
//! {
//!   "models": [
//!     {
//!       "name": "Claude Opus 4.7 Thinking",
//!       "organization": "Anthropic",
//!       "functionalSkill": 82.52,
//!       "issueDensity": 24.10,
//!       "vulnerabilityDensityPerKloc": 0.29,
//!       "bugDensityPerKloc": 0.8,
//!       "codeSmellDensityPerKloc": 23.01,
//!       …
//!     }
//!   ]
//! }
//! ```
//!
//! We emit two metrics:
//!   * `SonarFunctionalSkill` — pass-rate-ish (higher is better)
//!   * `SonarIssueDensity` — issues per kLOC (lower is better; metric def
//!     in coefficients sets `higher_better = false`).

use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "sonar";
const CACHE_KEY: &str = "sonar";
const URL: &str = "https://www.sonarsource.com/the-coding-personalities-of-leading-llms/leaderboard/data.json";

#[derive(Debug, Default, Clone, Copy)]
pub struct SonarSource;

#[async_trait::async_trait]
impl Source for SonarSource {
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
        // Sonar refreshes "regularly" but in practice a few times a month.
        // 7 days mirrors the other slow-changing leaderboards.
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
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("Sonar payload missing models[]".into()))?;

    let mut rows = Vec::with_capacity(models.len());
    for item in models {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut fields = BTreeMap::new();
        if let Some(skill) = item.get("functionalSkill").and_then(number_like)
            && skill.is_finite()
        {
            fields.insert("SonarFunctionalSkill".to_string(), Value::from(skill));
        }
        if let Some(issue_density) = item.get("issueDensity").and_then(number_like)
            && issue_density.is_finite()
            && issue_density > 0.0
        {
            // Lower is better — coefficients flip the direction via
            // `higher_better = false`, so we emit the raw rate here.
            fields.insert("SonarIssueDensity".to_string(), Value::from(issue_density));
        }
        if fields.is_empty() {
            continue;
        }
        let vendor_hint = item
            .get("organization")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: trimmed.to_string(),
            vendor_hint,
            fields,
            synthesized_from: None,
        });
    }

    if rows.is_empty() {
        return Err(SourceError::Parse(
            "Sonar payload yielded no model rows".into(),
        ));
    }
    Ok(rows)
}

fn number_like(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_two_metrics_per_model() {
        let payload = json!({
            "title": "Sonar LLM Leaderboard",
            "models": [
                {
                    "name": "Claude Opus 4.7 Thinking",
                    "organization": "Anthropic",
                    "functionalSkill": 82.52,
                    "issueDensity": 24.10
                },
                {
                    "name": "GPT-5.5 Medium",
                    "organization": "OpenAI",
                    "functionalSkill": 78.67,
                    "issueDensity": 17.72
                }
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 2);
        let opus = rows.iter().find(|r| r.model_name.contains("Opus")).unwrap();
        assert_eq!(
            opus.fields
                .get("SonarFunctionalSkill")
                .and_then(Value::as_f64),
            Some(82.52)
        );
        assert_eq!(
            opus.fields.get("SonarIssueDensity").and_then(Value::as_f64),
            Some(24.10)
        );
    }

    #[test]
    fn skips_models_without_either_metric() {
        let payload = json!({
            "models": [
                { "name": "Foo", "organization": "x" },
                {
                    "name": "Bar",
                    "organization": "y",
                    "functionalSkill": 50.0
                }
            ]
        });
        let rows = parse_rows(&payload).expect("should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "Bar");
    }
}
