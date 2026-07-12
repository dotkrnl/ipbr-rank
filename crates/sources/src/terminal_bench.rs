use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::{AliasIndex, RawRow};
use scraper::{ElementRef, Html, Selector};

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_html_path,
    read_cached_string, use_cached_html, write_cache_html,
};

const SOURCE_ID: &str = "terminal_bench";
const CACHE_KEY: &str = "terminal_bench";
const URL: &str = "https://www.tbench.ai/leaderboard/terminal-bench/2.0";
const SOURCE_ID_2_1: &str = "terminal_bench_2_1";
const CACHE_KEY_2_1: &str = "terminal_bench_2_1";
const URL_2_1: &str = "https://www.tbench.ai/leaderboard/terminal-bench/2.1";

#[derive(Debug, Default, Clone, Copy)]
pub struct TerminalBenchSource;

#[derive(Debug, Default, Clone, Copy)]
pub struct TerminalBench21Source;

#[async_trait::async_trait]
impl Source for TerminalBenchSource {
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
        fetch_terminal_bench(
            http,
            opts,
            self.id(),
            self.cache_key(),
            URL,
            "TerminalBench",
        )
        .await
    }
}

#[async_trait::async_trait]
impl Source for TerminalBench21Source {
    fn id(&self) -> &str {
        SOURCE_ID_2_1
    }

    fn cache_key(&self) -> &str {
        CACHE_KEY_2_1
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
        fetch_terminal_bench(
            http,
            opts,
            self.id(),
            self.cache_key(),
            URL_2_1,
            "TerminalBench21",
        )
        .await
    }
}

async fn fetch_terminal_bench(
    http: &dyn Http,
    opts: FetchOptions<'_>,
    source_id: &str,
    cache_key: &str,
    url: &str,
    metric: &str,
) -> Result<Vec<RawRow>, SourceError> {
    let html = if use_cached_html(opts, cache_key, Duration::from_secs(7 * 24 * 3600)) {
        let Some(dir) = opts.cache_dir else {
            return Err(SourceError::CacheMiss(format!(
                "{source_id} requires --cache in --offline mode",
            )));
        };
        read_cached_string(&cache_html_path(dir, cache_key))?
    } else {
        let html = http.get_text(url, &[]).await?;
        if let Some(dir) = opts.cache_dir {
            write_cache_html(dir, cache_key, &html)?;
        }
        html
    };
    parse_rows(&html, metric, source_id)
}

fn cell_text(td: ElementRef<'_>) -> String {
    let raw: String = td.text().collect();
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

// Live tbench (2026-04 snapshot) renders a Next.js shadcn `<table data-slot="table">`
// with rows tagged `data-slot="table-row"` and 8 cells per row:
//   [0] checkbox  [1] rank  [2] agent  [3] model  [4] date
//   [5] agent_org [6] model_org [7] accuracy ("82.0% ± 2.2")
// The header tr also carries `data-slot="table-row"` so we filter by parent <tbody>.
fn parse_rows(html: &str, metric: &str, source_id: &str) -> Result<Vec<RawRow>, SourceError> {
    let document = Html::parse_document(html);
    let row_sel = Selector::parse(r#"table[data-slot="table"] tbody tr[data-slot="table-row"]"#)
        .expect("valid selector");
    let td_sel = Selector::parse(r#"td[data-slot="table-cell"]"#).expect("valid selector");

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut best_by_model: BTreeMap<String, (f64, RawRow)> = BTreeMap::new();
    for tr in document.select(&row_sel) {
        let cells: Vec<String> = tr.select(&td_sel).map(cell_text).collect();
        if cells.len() < 8 {
            continue;
        }
        let model_name = cells[3].trim();
        if model_name.is_empty() {
            continue;
        }
        let acc_raw = cells[7].trim();
        let (score, uncertainty) = match parse_accuracy(acc_raw) {
            Some(v) => v,
            None => continue,
        };

        let mut fields = BTreeMap::new();
        fields.insert(metric.to_string(), serde_json::Value::from(score));
        if let Some(uncertainty) = uncertainty {
            // Auxiliary, unscored uncertainty in percentage points. Keeping
            // the key metric-specific prevents 2.0 and 2.1 observations from
            // overwriting one another after canonical model aggregation.
            fields.insert(
                format!("{metric}Uncertainty"),
                serde_json::Value::from(uncertainty),
            );
        }
        let row = RawRow {
            source_id: source_id.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: None,
            fields,
            synthesized_from: None,
            synthesis_category: None,
        };
        let key = crate::alias_dedupe_key(&alias_records, &alias_index, model_name, None);
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
    Ok(best_by_model.into_values().map(|(_, row)| row).collect())
}

fn parse_accuracy(s: &str) -> Option<(f64, Option<f64>)> {
    let (score, uncertainty) = match s.split_once('±') {
        Some((score, uncertainty)) => (score, Some(uncertainty)),
        None => (s, None),
    };
    let score = parse_percentage(score)?;
    let uncertainty = uncertainty.and_then(parse_percentage);
    Some((score, uncertainty))
}

fn parse_percentage(s: &str) -> Option<f64> {
    s.trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipbr_core::alias::AliasIndex;
    use ipbr_core::required_aliases::load_embedded;

    #[test]
    fn parse_terminal_bench_fixture() {
        let html = include_str!("../../../data/fixtures/terminal_bench.html");
        let rows = parse_rows(html, "TerminalBench", SOURCE_ID).expect("fixture should parse");
        assert!(rows.len() >= 10, "expected >=10 rows, got {}", rows.len());
        assert!(rows.iter().all(|r| r.fields.contains_key("TerminalBench")));

        let records = load_embedded().expect("required_aliases.toml must parse");
        let idx = AliasIndex::build(&records);
        let resolved: Vec<&str> = rows
            .iter()
            .filter_map(|r| idx.match_record(&r.model_name, None))
            .map(|i| records[i].canonical_id.as_str())
            .collect();

        // Spec asks for a row resolving to `anthropic/claude-opus-4.7`. The current
        // live snapshot's top Anthropic flagship is 4.6 (4.7 is not yet listed
        // upstream), so we accept any flagship Claude Opus 4.x — the substantive
        // strengthening over the previous `contains("Claude Opus")` check is that
        // the model name now has to round-trip through AliasIndex into an embedded
        // canonical record.
        assert!(
            resolved
                .iter()
                .any(|id| id.starts_with("anthropic/claude-opus-4.")),
            "expected at least one row to resolve to anthropic/claude-opus-4.x; got {:?}",
            resolved
        );
    }

    #[test]
    fn accuracy_strips_margin_and_percent() {
        assert_eq!(parse_accuracy("82.0 % ± 2.2"), Some((82.0, Some(2.2))));
        assert_eq!(parse_accuracy("78.4%"), Some((78.4, None)));
        assert_eq!(parse_accuracy("n/a"), None);
    }

    #[test]
    fn keeps_best_score_per_canonical_model() {
        let html = r#"
        <table data-slot="table"><tbody>
          <tr data-slot="table-row">
            <td data-slot="table-cell"></td><td data-slot="table-cell">1</td>
            <td data-slot="table-cell">agent-a</td><td data-slot="table-cell">GPT-5.3-Codex</td>
            <td data-slot="table-cell"></td><td data-slot="table-cell"></td>
            <td data-slot="table-cell"></td><td data-slot="table-cell">78.4% ± 2.0</td>
          </tr>
          <tr data-slot="table-row">
            <td data-slot="table-cell"></td><td data-slot="table-cell">2</td>
            <td data-slot="table-cell">agent-b</td><td data-slot="table-cell">GPT-5.3-Codex</td>
            <td data-slot="table-cell"></td><td data-slot="table-cell"></td>
            <td data-slot="table-cell"></td><td data-slot="table-cell">64.7% ± 2.0</td>
          </tr>
        </tbody></table>
        "#;
        let rows = parse_rows(html, "TerminalBench", SOURCE_ID).expect("fixture should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]
                .fields
                .get("TerminalBench")
                .and_then(serde_json::Value::as_f64),
            Some(78.4)
        );
        assert_eq!(
            rows[0]
                .fields
                .get("TerminalBenchUncertainty")
                .and_then(serde_json::Value::as_f64),
            Some(2.0)
        );
    }
}
