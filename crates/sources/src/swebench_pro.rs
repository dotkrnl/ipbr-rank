//! SWE-Bench Pro source — Scale's harder, multi-file SWE-bench variant.
//!
//! Where SWE-bench Verified saturates near 90% on the easiest issues,
//! SWE-Bench Pro draws from 1,865 tasks across 41 actively-maintained
//! repositories (Python, Go, TypeScript, JavaScript) requiring substantial
//! multi-file edits (avg 107 LOC across 4.1 files). Frontier models top out
//! around 60-65%, so it differentiates better at the top of the leaderboard.
//!
//! It joins `SWERebench`, `SWEBenchVerified`, and `SWEBenchMultilingual` in
//! the SWEComposite — four complementary views of agentic SWE-bench
//! performance, normalized into a single BUILD-group input so we don't pile
//! up overlapping signals as independent weights.
//!
//! The leaderboard at <https://labs.scale.com/leaderboard/swe_bench_pro_public>
//! shares the same RSC embedding pattern as `mcp_atlas` (Scale uses one
//! Next.js stack across both), so we reuse the parser.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::RawRow;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_html_path,
    mcp_atlas::parse_rows, read_cached_string, use_cached_html, write_cache_html,
};

const SOURCE_ID: &str = "swebench_pro";
const CACHE_KEY: &str = "swebench_pro";
const URL: &str = "https://labs.scale.com/leaderboard/swe_bench_pro_public";

#[derive(Debug, Default, Clone, Copy)]
pub struct SweBenchProSource;

#[async_trait::async_trait]
impl Source for SweBenchProSource {
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
        parse_rows(&html, "SWEBenchPro", SOURCE_ID)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipbr_core::alias::AliasIndex;
    use ipbr_core::required_aliases::load_embedded;

    #[test]
    fn parses_swebench_pro_fixture() {
        let html = include_str!("../../../data/fixtures/swebench_pro.html");
        let rows = parse_rows(html, "SWEBenchPro", SOURCE_ID).expect("fixture should parse");
        assert!(rows.len() >= 15, "expected ≥15 rows, got {}", rows.len());
        assert!(rows.iter().all(|r| r.fields.contains_key("SWEBenchPro")));

        let records = load_embedded().expect("required_aliases.toml must parse");
        let idx = AliasIndex::build(&records);
        let resolved: Vec<&str> = rows
            .iter()
            .filter_map(|r| idx.match_record(&r.model_name, None))
            .map(|i| records[i].canonical_id.as_str())
            .collect();
        assert!(
            resolved
                .iter()
                .any(|id| id.starts_with("anthropic/claude-opus-4.")),
            "expected at least one Claude Opus 4.x match: {resolved:?}"
        );
    }
}
