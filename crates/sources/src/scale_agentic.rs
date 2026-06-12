//! Scale agentic leaderboards beyond SWE Atlas.
//!
//! HiL-Bench uses the same Next.js/RSC embedding pattern as MCP Atlas and SWE
//! Atlas. It measures selective escalation: whether an agent recognizes when
//! missing or contradictory information requires asking a targeted human
//! question instead of guessing through an under-specified task.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::RawRow;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_html_path,
    mcp_atlas::parse_rows_with_model_map, read_cached_string, use_cached_html, write_cache_html,
};

const HIL_SOURCE_ID: &str = "hil_bench";
const HIL_CACHE_KEY: &str = "hil_bench";
const HIL_URL: &str = "https://labs.scale.com/leaderboard/hil";

#[derive(Debug, Default, Clone, Copy)]
pub struct HilBenchSource;

#[async_trait::async_trait]
impl Source for HilBenchSource {
    fn id(&self) -> &str {
        HIL_SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        HIL_CACHE_KEY
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
            let html = http
                .get_text(HIL_URL, &[("User-Agent", "ipbr-rank")])
                .await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_html(dir, self.cache_key(), &html)?;
            }
            html
        };
        parse_rows_with_model_map(&html, "HiLBench", self.id(), clean_scale_agentic_model_name)
    }
}

fn clean_scale_agentic_model_name(model_name: &str) -> String {
    model_name.trim().trim_end_matches('*').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipbr_core::alias::AliasIndex;
    use ipbr_core::required_aliases::load_embedded;

    #[test]
    fn parses_hil_fixture() {
        let html = include_str!("../../../data/fixtures/hil_bench.html");
        let rows = crate::mcp_atlas::parse_rows_with_model_map(
            html,
            "HiLBench",
            HIL_SOURCE_ID,
            clean_scale_agentic_model_name,
        )
        .expect("fixture should parse");
        assert!(rows.len() >= 10, "expected >=10 rows, got {}", rows.len());
        assert!(rows.iter().all(|r| r.fields.contains_key("HiLBench")));

        let records = load_embedded().expect("required_aliases.toml must parse");
        let idx = AliasIndex::build(&records);
        let matched = rows
            .iter()
            .filter(|r| idx.match_record(&r.model_name, None).is_some())
            .count();
        assert!(
            matched >= 10,
            "expected broad HiL-Bench alias coverage, got {matched}"
        );
    }
}
