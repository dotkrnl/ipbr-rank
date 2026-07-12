//! AGC-Bench — a broad, open artificial-general-creativity meta-benchmark.
//!
//! The official Hugging Face dataset publishes a canonical machine-readable
//! leaderboard covering 83 models and 67 primary datasets across six domains.
//! Its composite is a mean z-score, so negative values are valid. The metric
//! remains diagnostic while the very new benchmark is validated alongside the
//! existing EQ-Bench creative-writing signal.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_csv_path,
    read_cached_string, use_cached_csv, write_cache_csv,
};

const SOURCE_ID: &str = "agc_bench";
const CACHE_KEY: &str = "agc_bench";
const URL: &str = "https://huggingface.co/datasets/agcbench-2026/AGC-Bench/resolve/main/release_data/leaderboard.csv";

#[derive(Debug, Default, Clone, Copy)]
pub struct AgcBenchSource;

#[async_trait::async_trait]
impl Source for AgcBenchSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        CACHE_KEY
    }

    fn cache_paths(&self, cache_dir: &Path) -> Vec<PathBuf> {
        vec![cache_csv_path(cache_dir, self.cache_key())]
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Experimental
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
        let csv = if use_cached_csv(opts, self.cache_key(), self.cache_ttl()) {
            let Some(dir) = opts.cache_dir else {
                return Err(SourceError::CacheMiss(format!(
                    "{} requires --cache in --offline mode",
                    self.id()
                )));
            };
            read_cached_string(&cache_csv_path(dir, self.cache_key()))?
        } else {
            let csv = http.get_text(URL, &[("User-Agent", "ipbr-rank")]).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_csv(dir, self.cache_key(), &csv)?;
            }
            csv
        };
        parse_rows(&csv)
    }
}

fn parse_rows(csv: &str) -> Result<Vec<RawRow>, SourceError> {
    let table = parse_csv(csv)?;
    let Some(header) = table.first() else {
        return Err(SourceError::Parse("AGC-Bench CSV is empty".into()));
    };
    let model_idx = find_column(header, "model")?;
    let score_idx = find_column(header, "mean_z")?;
    let datasets_idx = find_column(header, "datasets")?;

    let mut rows = Vec::new();
    for record in table.iter().skip(1) {
        let Some(model_name) = record.get(model_idx).filter(|name| !name.is_empty()) else {
            continue;
        };
        let Some(score) = record
            .get(score_idx)
            .and_then(|raw| raw.parse::<f64>().ok())
            .filter(|value| value.is_finite())
        else {
            continue;
        };
        let Some(_) = record
            .get(datasets_idx)
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|count| *count > 0)
        else {
            continue;
        };

        let mut fields = BTreeMap::new();
        fields.insert("AGCBench".to_string(), Value::from(score));
        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: model_name
                .split_once('/')
                .map(|(vendor, _)| vendor.to_string()),
            fields,
            synthesized_from: None,
            synthesis_category: None,
        });
    }

    if rows.len() < 20 {
        return Err(SourceError::Parse(format!(
            "AGC-Bench leaderboard yielded only {} model rows",
            rows.len()
        )));
    }
    Ok(rows)
}

fn find_column(header: &[String], expected: &str) -> Result<usize, SourceError> {
    header
        .iter()
        .position(|value| value == expected)
        .ok_or_else(|| SourceError::Parse(format!("AGC-Bench CSV missing {expected} column")))
}

fn parse_csv(input: &str) -> Result<Vec<Vec<String>>, SourceError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = input.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                row.push(field.trim().to_string());
                field.clear();
            }
            '\n' if !in_quotes => {
                row.push(field.trim_end_matches('\r').trim().to_string());
                field.clear();
                if row.iter().any(|value| !value.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            _ => field.push(ch),
        }
    }
    if in_quotes {
        return Err(SourceError::Parse(
            "AGC-Bench CSV has an unterminated quote".into(),
        ));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field.trim_end_matches('\r').trim().to_string());
        if row.iter().any(|value| !value.is_empty()) {
            rows.push(row);
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_positive_and_negative_z_scores() {
        let mut csv = String::from(
            "model,datasets,n_metric_obs,mean_z,median_z,n_jrt_cells,rank,release_model\n",
        );
        for i in 0..20 {
            let score = if i == 19 {
                -1.565
            } else {
                0.8 - i as f64 / 20.0
            };
            csv.push_str(&format!(
                "vendor/model-{i},67,67,{score},0.0,24,{},yes\n",
                i + 1
            ));
        }
        let rows = parse_rows(&csv).expect("leaderboard should parse");
        assert_eq!(rows.len(), 20);
        assert_eq!(rows[0].vendor_hint.as_deref(), Some("vendor"));
        assert_eq!(
            rows[19].fields.get("AGCBench").and_then(Value::as_f64),
            Some(-1.565)
        );
        assert_eq!(rows[0].fields.len(), 1);
    }

    #[test]
    fn requires_canonical_schema_and_nontrivial_population() {
        assert!(parse_rows("model,score\nopenai/gpt,1.0\n").is_err());
    }
}
