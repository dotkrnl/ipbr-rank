//! Factory's open code-review benchmark.
//!
//! Factory evaluates models on 50 real pull requests with human-curated bug
//! sets, three repeated runs, and a cross-validated semantic judge. The
//! benchmark is unusually close to the repository's REVIEW construct, but its
//! model cohort is still small and every model was run at a fixed "high"
//! effort. We therefore ingest its quality fields as diagnostics and leave
//! weighting to the scoring configuration once coverage improves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::RawRow;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_html_path,
    read_cached_string, use_cached_html, write_cache_html,
};

const SOURCE_ID: &str = "factory_code_review";
const CACHE_KEY: &str = "factory_code_review";
const URL: &str = "https://factory.ai/news/code-review-benchmark";

#[derive(Debug, Default, Clone, Copy)]
pub struct FactoryCodeReviewSource;

#[async_trait::async_trait]
impl Source for FactoryCodeReviewSource {
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
        // The official blog is authoritative, but its presentation HTML is a
        // less stable transport than an API or standalone data file.
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
    let document = Html::parse_document(html);
    let table_selector = Selector::parse("table").expect("static selector");
    let header_selector = Selector::parse("thead th").expect("static selector");
    let row_selector = Selector::parse("tbody tr").expect("static selector");
    let cell_selector = Selector::parse("td").expect("static selector");

    for table in document.select(&table_selector) {
        let headers: Vec<String> = table
            .select(&header_selector)
            .map(normalized_text)
            .collect();
        let Some(model_idx) = find_header(&headers, "Model") else {
            continue;
        };
        let Some(f1_idx) = find_header_prefix(&headers, "Mean F1") else {
            continue;
        };
        let (Some(precision_idx), Some(recall_idx)) = (
            find_header_prefix(&headers, "Precision"),
            find_header_prefix(&headers, "Recall"),
        ) else {
            // The page also has a cost table with Mean F1. Requiring both
            // quality components selects the actual model-ranking table.
            continue;
        };
        let stdev_idx = find_header_prefix(&headers, "Stdev");

        let mut rows = Vec::new();
        for row in table.select(&row_selector) {
            let cells: Vec<String> = row.select(&cell_selector).map(normalized_text).collect();
            let Some(model_name) = cells.get(model_idx).filter(|name| !name.is_empty()) else {
                continue;
            };
            let Some(f1) = cells.get(f1_idx).and_then(|raw| parse_number(raw)) else {
                continue;
            };
            let Some(precision) = cells.get(precision_idx).and_then(|raw| parse_number(raw)) else {
                continue;
            };
            let Some(recall) = cells.get(recall_idx).and_then(|raw| parse_number(raw)) else {
                continue;
            };

            let mut fields = BTreeMap::new();
            fields.insert("FactoryCodeReviewF1".to_string(), Value::from(f1));
            fields.insert(
                "FactoryCodeReviewPrecision".to_string(),
                Value::from(precision),
            );
            fields.insert("FactoryCodeReviewRecall".to_string(), Value::from(recall));
            if let Some(stdev) = stdev_idx
                .and_then(|idx| cells.get(idx))
                .and_then(|raw| parse_number(raw))
            {
                fields.insert("FactoryCodeReviewF1Stdev".to_string(), Value::from(stdev));
            }
            rows.push(RawRow {
                source_id: SOURCE_ID.to_string(),
                model_name: model_name.to_string(),
                vendor_hint: vendor_hint(model_name).map(str::to_string),
                fields,
            });
        }

        if rows.len() < 5 {
            return Err(SourceError::Parse(format!(
                "Factory code-review ranking yielded only {} rows",
                rows.len()
            )));
        }
        return Ok(rows);
    }

    Err(SourceError::Parse(
        "Factory page missing the Model/Mean F1/Precision/Recall table".into(),
    ))
}

fn normalized_text(element: ElementRef<'_>) -> String {
    element
        .text()
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

fn find_header(headers: &[String], expected: &str) -> Option<usize> {
    headers
        .iter()
        .position(|header| header.eq_ignore_ascii_case(expected))
}

fn find_header_prefix(headers: &[String], expected: &str) -> Option<usize> {
    let expected = expected.to_ascii_lowercase();
    headers
        .iter()
        .position(|header| header.to_ascii_lowercase().starts_with(&expected))
}

fn parse_number(raw: &str) -> Option<f64> {
    let trimmed = raw
        .trim()
        .trim_start_matches(['±', '+'])
        .trim_end_matches('%')
        .trim();
    trimmed
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 100.0)
}

fn vendor_hint(model: &str) -> Option<&'static str> {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("gpt") {
        Some("openai")
    } else if lower.starts_with("opus") || lower.starts_with("sonnet") {
        Some("anthropic")
    } else if lower.starts_with("gemini") {
        Some("google")
    } else if lower.starts_with("glm") {
        Some("zai")
    } else if lower.starts_with("kimi") {
        Some("moonshot")
    } else if lower.starts_with("minimax") {
        Some("minimax")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quality_table_and_ignores_cost_table() {
        let html = r#"
            <table>
              <thead><tr><th>Model</th><th>Mean F1▼</th><th>Stdev▼</th><th>Precision▼</th><th>Recall▼</th></tr></thead>
              <tbody>
                <tr><td>GPT-5.2</td><td>60.5<!-- -->%</td><td>±3</td><td>65%</td><td>57.6%</td></tr>
                <tr><td>Opus 4.6</td><td>59.8%</td><td>±2.1</td><td>58.1%</td><td>61.8%</td></tr>
                <tr><td>Sonnet 4.6</td><td>57.4%</td><td>±4.9</td><td>62.6%</td><td>47.3%</td></tr>
                <tr><td>GLM-5.1</td><td>55.8%</td><td>±2.8</td><td>63.5%</td><td>50.7%</td></tr>
                <tr><td>GPT-5.5</td><td>47.9%</td><td>±1.9</td><td>47.5%</td><td>48.4%</td></tr>
              </tbody>
            </table>
            <table>
              <thead><tr><th>Model</th><th>Mean F1</th><th>Cost/PR</th></tr></thead>
              <tbody><tr><td>GPT-5.2</td><td>60.5%</td><td>$1.25</td></tr></tbody>
            </table>
        "#;
        let rows = parse_rows(html).expect("quality table should parse");
        assert_eq!(rows.len(), 5);
        let gpt = rows
            .iter()
            .find(|row| row.model_name == "GPT-5.2")
            .expect("GPT row");
        assert_eq!(
            gpt.fields
                .get("FactoryCodeReviewF1")
                .and_then(Value::as_f64),
            Some(60.5)
        );
        assert_eq!(
            gpt.fields
                .get("FactoryCodeReviewF1Stdev")
                .and_then(Value::as_f64),
            Some(3.0)
        );
        assert!(!gpt.fields.keys().any(|key| key.contains("Cost")));
    }

    #[test]
    fn rejects_an_unrelated_mean_f1_table() {
        let html =
            "<table><thead><tr><th>Model</th><th>Mean F1</th><th>Cost/PR</th></tr></thead></table>";
        assert!(parse_rows(html).is_err());
    }
}
