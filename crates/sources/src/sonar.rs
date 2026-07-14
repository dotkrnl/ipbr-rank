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
//! The page is a client-rendered SPA. We fetch a model registry and then one
//! metrics file per model:
//!
//!   `…/leaderboard/data/models.json`
//!   `…/leaderboard/data/<org>/<model>-metrics.json`
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
//! We emit four metrics:
//!   * `SonarFunctionalSkill` — pass-rate-ish (higher is better)
//!   * `SonarIssueDensity` — issues per kLOC (lower is better; metric def
//!     in coefficients sets `higher_better = false`)
//!   * `SonarBugDensity` — bugs per kLOC (lower is better)
//!   * `SonarVulnerabilityDensity` — vulnerabilities per kLOC (lower is
//!     better).

use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::{AliasIndex, RawRow};
use serde_json::{Map, Value, json};

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "sonar";
const CACHE_KEY: &str = "sonar";
const DATA_BASE_URL: &str =
    "https://www.sonarsource.com/the-coding-personalities-of-leading-llms/leaderboard/data/";
const REGISTRY_URL: &str = "https://www.sonarsource.com/the-coding-personalities-of-leading-llms/leaderboard/data/models.json";
const LANG_PRIORITY: &[&str] = &["java", "typescript", "python"];

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
            let payload = fetch_live_payload(http).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_json(dir, self.cache_key(), &payload)?;
            }
            payload
        };
        parse_rows(&payload)
    }
}

async fn fetch_live_payload(http: &dyn Http) -> Result<Value, SourceError> {
    let registry = http
        .get_json(REGISTRY_URL, &[("User-Agent", "ipbr-rank")])
        .await?;

    let registry_obj = registry
        .as_object()
        .ok_or_else(|| SourceError::Parse("Sonar registry payload is not an object".into()))?;

    let mut models = Vec::with_capacity(registry_obj.len());
    for (model_id, entry) in registry_obj {
        if entry
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }

        let meta = entry.get("meta").and_then(Value::as_object);
        let name = meta
            .and_then(|m| m.get("name"))
            .and_then(Value::as_str)
            .unwrap_or(model_id);
        let organization = meta
            .and_then(|m| m.get("organization"))
            .and_then(Value::as_str)
            .unwrap_or_default();

        let Some(metrics_path) = choose_metrics_path(entry) else {
            continue;
        };
        let metrics_url =
            if metrics_path.starts_with("http://") || metrics_path.starts_with("https://") {
                metrics_path.to_string()
            } else {
                format!("{}{}", DATA_BASE_URL, metrics_path.trim_start_matches('/'))
            };
        let raw_metrics = http
            .get_json(&metrics_url, &[("User-Agent", "ipbr-rank")])
            .await?;
        if let Some(flat) = flatten_registry_metrics(name, organization, &raw_metrics) {
            models.push(flat);
        }
    }

    if models.is_empty() {
        return Err(SourceError::Parse(
            "Sonar registry yielded no model metrics".into(),
        ));
    }

    Ok(json!({ "models": models }))
}

fn choose_metrics_path(entry: &Value) -> Option<&str> {
    let files = entry.get("files").and_then(Value::as_object)?;
    for lang in LANG_PRIORITY {
        if let Some(path) = files
            .get(*lang)
            .and_then(|lang_entry| lang_entry.get("metrics"))
            .and_then(Value::as_str)
        {
            return Some(path);
        }
    }
    files
        .values()
        .find_map(|lang_entry| lang_entry.get("metrics").and_then(Value::as_str))
}

fn flatten_registry_metrics(name: &str, organization: &str, raw: &Value) -> Option<Value> {
    let models = raw.get("models").and_then(Value::as_object)?;
    let entry = models.get(name).or_else(|| models.values().next())?;
    let metrics = entry.get("metrics").and_then(Value::as_object)?;

    let mut item = Map::new();
    item.insert("name".to_string(), Value::String(name.to_string()));
    if !organization.trim().is_empty() {
        item.insert(
            "organization".to_string(),
            Value::String(organization.trim().to_string()),
        );
    }

    insert_metric(&mut item, metrics, "functionalSkill", "passing_tests_pct");
    insert_metric(&mut item, metrics, "issueDensity", "issues_per_kloc");
    insert_metric(&mut item, metrics, "bugDensityPerKloc", "bugs_per_kloc");
    insert_metric(
        &mut item,
        metrics,
        "vulnerabilityDensityPerKloc",
        "vulnerabilities_per_kloc",
    );

    Some(Value::Object(item))
}

fn insert_metric(
    item: &mut Map<String, Value>,
    metrics: &Map<String, Value>,
    to: &str,
    from: &str,
) {
    if let Some(value) = metrics.get(from).and_then(number_like)
        && value.is_finite()
    {
        item.insert(to.to_string(), Value::from(value));
    }
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("Sonar payload missing models[]".into()))?;

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut best_by_model: BTreeMap<String, (f64, RawRow)> = BTreeMap::new();
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
            && issue_density >= 0.0
        {
            // Lower is better — coefficients flip the direction via
            // `higher_better = false`, so we emit the raw rate here.
            fields.insert("SonarIssueDensity".to_string(), Value::from(issue_density));
        }
        if let Some(bug_density) = item.get("bugDensityPerKloc").and_then(number_like)
            && bug_density.is_finite()
            && bug_density >= 0.0
        {
            fields.insert("SonarBugDensity".to_string(), Value::from(bug_density));
        }
        if let Some(vulnerability_density) = item
            .get("vulnerabilityDensityPerKloc")
            .and_then(number_like)
            && vulnerability_density.is_finite()
            && vulnerability_density >= 0.0
        {
            fields.insert(
                "SonarVulnerabilityDensity".to_string(),
                Value::from(vulnerability_density),
            );
        }
        if fields.is_empty() {
            continue;
        }
        let vendor_hint = item
            .get("organization")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        let row = RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: trimmed.to_string(),
            vendor_hint: vendor_hint.clone(),
            fields,
        };
        let key = crate::alias_dedupe_key(
            &alias_records,
            &alias_index,
            trimmed,
            vendor_hint.as_deref(),
        );
        let score = sonar_row_priority(&row);
        match best_by_model.get_mut(&key) {
            Some((best_score, best_row)) if score > *best_score => {
                *best_score = score;
                *best_row = row;
            }
            Some(_) => {}
            None => {
                best_by_model.insert(key, (score, row));
            }
        }
    }

    let rows: Vec<RawRow> = best_by_model.into_values().map(|(_, row)| row).collect();
    if rows.is_empty() {
        return Err(SourceError::Parse(
            "Sonar payload yielded no model rows".into(),
        ));
    }
    Ok(rows)
}

fn sonar_row_priority(row: &RawRow) -> f64 {
    row.fields
        .get("SonarFunctionalSkill")
        .and_then(number_like)
        .or_else(|| {
            row.fields
                .get("SonarIssueDensity")
                .and_then(number_like)
                .map(|value| -value)
        })
        .unwrap_or(f64::NEG_INFINITY)
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
                    "issueDensity": 24.10,
                    "bugDensityPerKloc": 0.8,
                    "vulnerabilityDensityPerKloc": 0.29
                },
                {
                    "name": "GPT-5.5 Medium",
                    "organization": "OpenAI",
                    "functionalSkill": 78.67,
                    "issueDensity": 17.72,
                    "bugDensityPerKloc": 0.3,
                    "vulnerabilityDensityPerKloc": 0.11
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
        assert_eq!(
            opus.fields.get("SonarBugDensity").and_then(Value::as_f64),
            Some(0.8)
        );
        assert_eq!(
            opus.fields
                .get("SonarVulnerabilityDensity")
                .and_then(Value::as_f64),
            Some(0.29)
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

    #[test]
    fn accepts_zero_issue_density_as_a_valid_measurement() {
        let payload = json!({
            "models": [{
                "name": "Zero-Issue Model",
                "organization": "Example",
                "issueDensity": 0.0
            }]
        });
        let rows = parse_rows(&payload).expect("zero is a valid lower-better density");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]
                .fields
                .get("SonarIssueDensity")
                .and_then(Value::as_f64),
            Some(0.0)
        );
    }

    #[test]
    fn flattens_registry_metrics_shape() {
        let raw = json!({
            "models": {
                "GPT-Codex 5.3 High": {
                    "metrics": {
                        "passing_tests_pct": 78.4,
                        "issues_per_kloc": 18.2,
                        "bugs_per_kloc": 0.3,
                        "vulnerabilities_per_kloc": 0.1
                    }
                }
            }
        });

        let flat = flatten_registry_metrics("GPT-Codex 5.3 High", "OpenAI", &raw)
            .expect("registry metrics should flatten");
        let rows = parse_rows(&json!({ "models": [flat] })).expect("flat payload should parse");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "GPT-Codex 5.3 High");
        assert_eq!(rows[0].vendor_hint.as_deref(), Some("openai"));
        assert_eq!(
            rows[0]
                .fields
                .get("SonarFunctionalSkill")
                .and_then(Value::as_f64),
            Some(78.4)
        );
        assert_eq!(
            rows[0]
                .fields
                .get("SonarVulnerabilityDensity")
                .and_then(Value::as_f64),
            Some(0.1)
        );
    }

    #[test]
    fn keeps_best_functional_skill_per_canonical_model() {
        let payload = json!({
            "models": [
                {
                    "name": "Claude Sonnet 4.5",
                    "organization": "Anthropic",
                    "functionalSkill": 76.29,
                    "issueDensity": 20.09
                },
                {
                    "name": "Claude Sonnet 4.5 Thinking",
                    "organization": "Anthropic",
                    "functionalSkill": 80.53,
                    "issueDensity": 19.25
                }
            ]
        });
        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]
                .fields
                .get("SonarFunctionalSkill")
                .and_then(Value::as_f64),
            Some(80.53)
        );
    }
}
