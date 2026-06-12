//! Berkeley Function Calling Leaderboard (BFCL) V4.
//!
//! The public leaderboard page loads `data_overall.csv` at runtime. We use
//! the overall accuracy column as a PLAN/BUILD tool-calling signal and clean
//! `(FC)` / `(Prompt)` display suffixes before alias matching.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::{AliasIndex, RawRow};
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_csv_path,
    read_cached_string, use_cached_csv, write_cache_csv,
};

const SOURCE_ID: &str = "bfcl";
const CACHE_KEY: &str = "bfcl";
const URL: &str = "https://gorilla.cs.berkeley.edu/data_overall.csv";

#[derive(Debug, Default, Clone, Copy)]
pub struct BfclSource;

#[async_trait::async_trait]
impl Source for BfclSource {
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
        return Err(SourceError::Parse("BFCL CSV is empty".into()));
    };
    let model_idx = find_column(header, "Model")?;
    let score_idx = find_column(header, "Overall Acc")?;
    let org_idx = header.iter().position(|name| name == "Organization");

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut best_by_model: BTreeMap<String, (f64, RawRow)> = BTreeMap::new();
    for record in table.iter().skip(1) {
        let Some(model_name) = record.get(model_idx) else {
            continue;
        };
        let Some(score_raw) = record.get(score_idx) else {
            continue;
        };
        let Some(score) = parse_percent(score_raw) else {
            continue;
        };
        let model_name = clean_bfcl_model_name(model_name);
        if model_name.is_empty() {
            continue;
        }
        let vendor_hint = org_idx
            .and_then(|i| record.get(i))
            .and_then(|value| vendor_slug(value));

        let mut fields = BTreeMap::new();
        fields.insert("BFCL".to_string(), Value::from(score));
        let row = RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_name.clone(),
            vendor_hint: vendor_hint.clone(),
            fields,
            synthesized_from: None,
            synthesis_category: None,
        };
        let key = crate::alias_dedupe_key(
            &alias_records,
            &alias_index,
            &model_name,
            vendor_hint.as_deref(),
        );
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
        return Err(SourceError::Parse("BFCL CSV yielded no model rows".into()));
    }
    Ok(rows)
}

fn find_column(header: &[String], name: &str) -> Result<usize, SourceError> {
    header
        .iter()
        .position(|value| value == name)
        .ok_or_else(|| SourceError::Parse(format!("BFCL CSV missing {name} column")))
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
            "BFCL CSV has an unterminated quote".into(),
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

fn parse_percent(value: &str) -> Option<f64> {
    value
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn clean_bfcl_model_name(model_name: &str) -> String {
    let mut text = model_name.trim().to_string();
    for suffix in [
        " (FC thinking)",
        " (FC Thinking)",
        " (Prompt thinking)",
        " (Prompt Thinking)",
        " (FC)",
        " (Prompt)",
    ] {
        if let Some(stripped) = text.strip_suffix(suffix) {
            text = stripped.trim().to_string();
            break;
        }
    }
    text
}

fn vendor_slug(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let slug = match normalized.as_str() {
        "alibaba" | "qwen" => "alibaba",
        "anthropic" => "anthropic",
        "deepseek" => "deepseek",
        "google" => "google",
        "meta" => "meta",
        "minimax" => "minimax",
        "moonshot" | "moonshot ai" => "moonshot",
        "openai" => "openai",
        "xai" | "x.ai" => "xai",
        "zhipu ai" | "z.ai" | "zai" => "zai",
        _ => return None,
    };
    Some(slug.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_overall_accuracy_csv() {
        let csv = "Rank,Overall Acc,Model,Organization\n1,77.47%,Claude-Opus-4-5-20251101 (FC),Anthropic\n2,72.51%,Gemini-3-Pro-Preview (Prompt),Google\n";
        let rows = parse_rows(csv).expect("CSV should parse");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model_name, "Claude-Opus-4-5-20251101");
        assert_eq!(rows[0].vendor_hint.as_deref(), Some("anthropic"));
        assert_eq!(rows[0].fields["BFCL"].as_f64(), Some(77.47));
        assert_eq!(rows[1].model_name, "Gemini-3-Pro-Preview");
        assert_eq!(rows[1].vendor_hint.as_deref(), Some("google"));
    }

    #[test]
    fn csv_parser_handles_quoted_commas() {
        let rows = parse_csv("Model,Note\n\"a,b\",\"quoted \"\"value\"\"\"\n").unwrap();
        assert_eq!(rows[1][0], "a,b");
        assert_eq!(rows[1][1], "quoted \"value\"");
    }

    #[test]
    fn keeps_best_mode_per_canonical_model() {
        let csv = "Rank,Overall Acc,Model,Organization\n1,77.47%,Claude-Opus-4-5-20251101 (FC),Anthropic\n2,33.47%,Claude-Opus-4-5-20251101 (Prompt),Anthropic\n";
        let rows = parse_rows(csv).expect("CSV should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].fields.get("BFCL").and_then(Value::as_f64),
            Some(77.47)
        );
    }
}
