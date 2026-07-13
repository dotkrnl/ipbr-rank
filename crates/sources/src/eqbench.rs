//! EQ-Bench Creative Writing v3 and Judgemark v4 leaderboards.
//!
//! EQ-Bench publishes both current leaderboards as CSV strings embedded in
//! JavaScript assets. Creative Writing v3 evaluates model-authored prose and
//! exposes a Glicko-derived Elo. Judgemark v4 evaluates a model's ability to
//! discriminate writing quality and publishes a normalized separability score
//! with a prompt-bootstrap 95% confidence interval.
//!
//! The adapters deliberately retain one best row per canonical model. The
//! primary Judgemark score is scaled from the upstream 0–1 fraction to 0–100;
//! its confidence bounds are retained as auxiliary, unscored raw fields.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::{AliasIndex, RawRow};
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_js_path,
    read_cached_string, use_cached_js, write_cache_js,
};

const CREATIVE_SOURCE_ID: &str = "eqbench_creative_writing";
const CREATIVE_CACHE_KEY: &str = "eqbench_creative_writing_v3";
const CREATIVE_URL: &str = "https://eqbench.com/creative_writing.js?v=1.0.91";
const CREATIVE_VARIABLE: &str = "leaderboardDataCreativeWritingV3";

const JUDGEMARK_SOURCE_ID: &str = "eqbench_judgemark";
const JUDGEMARK_CACHE_KEY: &str = "eqbench_judgemark_v4";
const JUDGEMARK_URL: &str = "https://eqbench.com/judgemark-v4.js?v=1.1";
const JUDGEMARK_VARIABLE: &str = "leaderboardDataJudgemarkV4";

const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Default, Clone, Copy)]
pub struct EqBenchCreativeWritingSource;

#[derive(Debug, Default, Clone, Copy)]
pub struct EqBenchJudgemarkSource;

#[async_trait::async_trait]
impl Source for EqBenchCreativeWritingSource {
    fn id(&self) -> &str {
        CREATIVE_SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        CREATIVE_CACHE_KEY
    }

    fn cache_paths(&self, cache_dir: &Path) -> Vec<PathBuf> {
        vec![cache_js_path(cache_dir, self.cache_key())]
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Verified
    }

    fn required_secret(&self) -> Option<crate::SecretRef> {
        None
    }

    fn cache_ttl(&self) -> Duration {
        CACHE_TTL
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        _secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        let js = fetch_js(
            http,
            opts,
            self.id(),
            self.cache_key(),
            self.cache_ttl(),
            CREATIVE_URL,
        )
        .await?;
        parse_creative_rows(&js)
    }
}

#[async_trait::async_trait]
impl Source for EqBenchJudgemarkSource {
    fn id(&self) -> &str {
        JUDGEMARK_SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        JUDGEMARK_CACHE_KEY
    }

    fn cache_paths(&self, cache_dir: &Path) -> Vec<PathBuf> {
        vec![cache_js_path(cache_dir, self.cache_key())]
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Verified
    }

    fn required_secret(&self) -> Option<crate::SecretRef> {
        None
    }

    fn cache_ttl(&self) -> Duration {
        CACHE_TTL
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        _secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        let js = fetch_js(
            http,
            opts,
            self.id(),
            self.cache_key(),
            self.cache_ttl(),
            JUDGEMARK_URL,
        )
        .await?;
        parse_judgemark_rows(&js)
    }
}

async fn fetch_js(
    http: &dyn Http,
    opts: FetchOptions<'_>,
    source_id: &str,
    cache_key: &str,
    ttl: Duration,
    url: &str,
) -> Result<String, SourceError> {
    if use_cached_js(opts, cache_key, ttl) {
        let Some(dir) = opts.cache_dir else {
            return Err(SourceError::CacheMiss(format!(
                "{source_id} requires --cache in --offline mode"
            )));
        };
        return read_cached_string(&cache_js_path(dir, cache_key));
    }

    let js = http.get_text(url, &[("User-Agent", "ipbr-rank")]).await?;
    if let Some(dir) = opts.cache_dir {
        write_cache_js(dir, cache_key, &js)?;
    }
    Ok(js)
}

fn parse_creative_rows(js: &str) -> Result<Vec<RawRow>, SourceError> {
    let csv = extract_embedded_csv(js, CREATIVE_VARIABLE)?;
    let table = parse_csv(csv)?;
    let header = table
        .first()
        .ok_or_else(|| SourceError::Parse("EQ-Bench Creative Writing CSV is empty".into()))?;
    let model_idx = find_column(header, "model_name", "Creative Writing")?;
    let score_idx = find_column(header, "elo_score", "Creative Writing")?;

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut best_by_model: BTreeMap<String, (f64, RawRow)> = BTreeMap::new();

    for record in table.iter().skip(1) {
        let Some(model_name) = record.get(model_idx).map(|name| clean_model_name(name)) else {
            continue;
        };
        if model_name.is_empty() || model_name == "__metadata__" {
            continue;
        }
        let Some(score) = record.get(score_idx).and_then(|raw| parse_number(raw)) else {
            continue;
        };
        let mut fields = BTreeMap::new();
        fields.insert("EQBenchCreativeWriting".to_string(), Value::from(score));
        let row = RawRow {
            source_id: CREATIVE_SOURCE_ID.to_string(),
            model_name: model_name.clone(),
            vendor_hint: None,
            fields,
        };
        keep_best_row(
            &mut best_by_model,
            &alias_records,
            &alias_index,
            model_name,
            score,
            row,
        );
    }

    finish_rows(best_by_model, "EQ-Bench Creative Writing")
}

fn parse_judgemark_rows(js: &str) -> Result<Vec<RawRow>, SourceError> {
    let csv = extract_embedded_csv(js, JUDGEMARK_VARIABLE)?;
    let table = parse_csv(csv)?;
    let header = table
        .first()
        .ok_or_else(|| SourceError::Parse("EQ-Bench Judgemark CSV is empty".into()))?;
    let model_idx = find_column(header, "model", "Judgemark")?;
    let score_idx = find_column(header, "score", "Judgemark")?;
    let ci_low_idx = header.iter().position(|column| column == "ci_low");
    let ci_high_idx = header.iter().position(|column| column == "ci_high");

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut best_by_model: BTreeMap<String, (f64, RawRow)> = BTreeMap::new();

    for record in table.iter().skip(1) {
        let Some(model_name) = record.get(model_idx).map(|name| clean_model_name(name)) else {
            continue;
        };
        if model_name.is_empty() || model_name == "__metadata__" {
            continue;
        }
        let Some(score) = record
            .get(score_idx)
            .and_then(|raw| parse_number(raw))
            .map(|score| score * 100.0)
        else {
            continue;
        };

        let mut fields = BTreeMap::new();
        fields.insert("EQBenchJudgemark".to_string(), Value::from(score));
        if let Some(ci_low) = ci_low_idx
            .and_then(|idx| record.get(idx))
            .and_then(|raw| parse_number(raw))
        {
            fields.insert(
                "EQBenchJudgemarkCILow".to_string(),
                Value::from(ci_low * 100.0),
            );
        }
        if let Some(ci_high) = ci_high_idx
            .and_then(|idx| record.get(idx))
            .and_then(|raw| parse_number(raw))
        {
            fields.insert(
                "EQBenchJudgemarkCIHigh".to_string(),
                Value::from(ci_high * 100.0),
            );
        }

        let row = RawRow {
            source_id: JUDGEMARK_SOURCE_ID.to_string(),
            model_name: model_name.clone(),
            vendor_hint: None,
            fields,
        };
        keep_best_row(
            &mut best_by_model,
            &alias_records,
            &alias_index,
            model_name,
            score,
            row,
        );
    }

    finish_rows(best_by_model, "EQ-Bench Judgemark")
}

fn keep_best_row(
    best_by_model: &mut BTreeMap<String, (f64, RawRow)>,
    alias_records: &[ipbr_core::ModelRecord],
    alias_index: &AliasIndex<'_>,
    model_name: String,
    score: f64,
    row: RawRow,
) {
    let key = crate::alias_dedupe_key(alias_records, alias_index, &model_name, None);
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

fn finish_rows(
    best_by_model: BTreeMap<String, (f64, RawRow)>,
    label: &str,
) -> Result<Vec<RawRow>, SourceError> {
    let rows: Vec<_> = best_by_model.into_values().map(|(_, row)| row).collect();
    if rows.is_empty() {
        return Err(SourceError::Parse(format!(
            "{label} CSV yielded no model rows"
        )));
    }
    Ok(rows)
}

fn clean_model_name(raw: &str) -> String {
    raw.trim().trim_start_matches('*').trim().to_string()
}

fn parse_number(raw: &str) -> Option<f64> {
    raw.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn find_column(header: &[String], name: &str, label: &str) -> Result<usize, SourceError> {
    header
        .iter()
        .position(|column| column == name)
        .ok_or_else(|| SourceError::Parse(format!("EQ-Bench {label} CSV missing {name} column")))
}

fn extract_embedded_csv<'a>(js: &'a str, variable: &str) -> Result<&'a str, SourceError> {
    let declaration = js.find(variable).ok_or_else(|| {
        SourceError::Parse(format!(
            "EQ-Bench JavaScript missing {variable} declaration"
        ))
    })?;
    let after_declaration = &js[declaration + variable.len()..];
    let opening = after_declaration.find('`').ok_or_else(|| {
        SourceError::Parse(format!("EQ-Bench {variable} template string never opens"))
    })?;
    let body = &after_declaration[opening + 1..];
    let closing = body.find('`').ok_or_else(|| {
        SourceError::Parse(format!("EQ-Bench {variable} template string never closes"))
    })?;
    Ok(body[..closing].trim())
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
            "EQ-Bench CSV has an unterminated quote".into(),
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

    const CREATIVE_FIXTURE: &str =
        include_str!("../../../data/fixtures/eqbench_creative_writing_v3.js");
    const JUDGEMARK_FIXTURE: &str = include_str!("../../../data/fixtures/eqbench_judgemark_v4.js");

    #[test]
    fn parses_current_creative_writing_fixture() {
        let rows = parse_creative_rows(CREATIVE_FIXTURE).expect("creative fixture should parse");
        assert!(
            rows.len() >= 100,
            "expected broad coverage, got {}",
            rows.len()
        );
        let fable = rows
            .iter()
            .find(|row| row.model_name == "claude-fable-5")
            .expect("leading source marker should be stripped from Fable 5");
        assert_eq!(
            fable
                .fields
                .get("EQBenchCreativeWriting")
                .and_then(Value::as_f64),
            Some(2229.6)
        );
        assert!(rows.iter().all(|row| row.model_name != "__metadata__"));
    }

    #[test]
    fn parses_current_judgemark_fixture_and_scales_confidence_interval() {
        let rows = parse_judgemark_rows(JUDGEMARK_FIXTURE).expect("judgemark fixture should parse");
        assert!(
            rows.len() >= 30,
            "expected broad coverage, got {}",
            rows.len()
        );
        let gpt = rows
            .iter()
            .find(|row| row.model_name == "gpt-5.5")
            .expect("GPT-5.5 should be present");
        assert_eq!(
            gpt.fields.get("EQBenchJudgemark").and_then(Value::as_f64),
            Some(87.8134)
        );
        assert_eq!(
            gpt.fields
                .get("EQBenchJudgemarkCILow")
                .and_then(Value::as_f64),
            Some(85.3332)
        );
        assert_eq!(
            gpt.fields
                .get("EQBenchJudgemarkCIHigh")
                .and_then(Value::as_f64),
            Some(92.4747)
        );
    }

    #[test]
    fn keeps_best_alias_row_per_canonical_model() {
        let js = r#"
            const leaderboardDataJudgemarkV4 = `
            model,score,ci_low,ci_high
            gpt-5.5,0.70,0.60,0.80
            gpt-5-5,0.90,0.85,0.95
            `;
        "#;
        let rows = parse_judgemark_rows(js).expect("duplicate aliases should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "gpt-5-5");
        assert_eq!(
            rows[0]
                .fields
                .get("EQBenchJudgemark")
                .and_then(Value::as_f64),
            Some(90.0)
        );
    }

    #[test]
    fn csv_parser_handles_quoted_commas() {
        let rows = parse_csv("model,score\n\"Model, Inc.\",1.0\n").expect("CSV should parse");
        assert_eq!(rows[1][0], "Model, Inc.");
        assert_eq!(rows[1][1], "1.0");
    }

    #[test]
    fn missing_named_payload_is_rejected() {
        let err = parse_creative_rows("const somethingElse = `model,score`; ")
            .expect_err("wrong variable must not be accepted");
        assert!(matches!(err, SourceError::Parse(_)));
    }
}
