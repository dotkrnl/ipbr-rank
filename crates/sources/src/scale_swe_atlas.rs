//! Scale SWE Atlas leaderboards.
//!
//! SWE Atlas is split into three public Scale Labs leaderboards:
//! codebase Q&A, test writing, and refactoring. They share the same
//! Next.js/RSC embedding shape as MCP-Atlas and SWE-Bench Pro, so this
//! source reuses the RSC scanner while stripping harness labels like
//! `(Codex)` / `(Claude Code)` from the model name before alias matching.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ipbr_core::RawRow;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_html_path,
    mcp_atlas::parse_rows_with_model_map, read_cached_string, use_cached_html, write_cache_html,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct SweAtlasQnaSource;

#[derive(Debug, Default, Clone, Copy)]
pub struct SweAtlasTestWritingSource;

#[derive(Debug, Default, Clone, Copy)]
pub struct SweAtlasRefactoringSource;

#[async_trait::async_trait]
impl Source for SweAtlasQnaSource {
    fn id(&self) -> &str {
        "sweatlas_qna"
    }

    fn cache_key(&self) -> &str {
        "sweatlas_qna"
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
        fetch_scale_leaderboard(
            http,
            opts,
            self.id(),
            self.cache_key(),
            "https://labs.scale.com/leaderboard/sweatlas-qna",
            "SWEAtlasQnA",
        )
        .await
    }
}

#[async_trait::async_trait]
impl Source for SweAtlasTestWritingSource {
    fn id(&self) -> &str {
        "sweatlas_test_writing"
    }

    fn cache_key(&self) -> &str {
        "sweatlas_test_writing"
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
        fetch_scale_leaderboard(
            http,
            opts,
            self.id(),
            self.cache_key(),
            "https://labs.scale.com/leaderboard/sweatlas-tw",
            "SWEAtlasTestWriting",
        )
        .await
    }
}

#[async_trait::async_trait]
impl Source for SweAtlasRefactoringSource {
    fn id(&self) -> &str {
        "sweatlas_refactoring"
    }

    fn cache_key(&self) -> &str {
        "sweatlas_refactoring"
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
        fetch_scale_leaderboard(
            http,
            opts,
            self.id(),
            self.cache_key(),
            "https://labs.scale.com/leaderboard/sweatlas-refactoring",
            "SWEAtlasRefactoring",
        )
        .await
    }
}

async fn fetch_scale_leaderboard(
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
        let html = http.get_text(url, &[("User-Agent", "ipbr-rank")]).await?;
        if let Some(dir) = opts.cache_dir {
            write_cache_html(dir, cache_key, &html)?;
        }
        html
    };
    parse_rows_with_model_map(&html, metric, source_id, clean_swe_atlas_model_name)
}

fn clean_swe_atlas_model_name(model_name: &str) -> String {
    let mut text = model_name.trim().to_string();
    for suffix in [
        " (Codex CLI)",
        " (Codex)",
        " (Claude Code)",
        " (Gemini CLI)",
        " (Mini-SWE-Agent)",
        " (Mini-SWE)",
        " (OpenHands)",
    ] {
        if let Some(stripped) = text.strip_suffix(suffix) {
            text = stripped.trim().to_string();
            break;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn cleans_harness_suffixes_before_matching() {
        assert_eq!(clean_swe_atlas_model_name("GPT 5.5 (Codex)"), "GPT 5.5");
        assert_eq!(
            clean_swe_atlas_model_name("Gpt-5.4-xHigh (Codex CLI)"),
            "Gpt-5.4-xHigh"
        );
        assert_eq!(
            clean_swe_atlas_model_name("Opus-4.7 (Claude Code)"),
            "Opus-4.7"
        );
        assert_eq!(
            clean_swe_atlas_model_name("Gemini-3.1-Pro (Gemini CLI)"),
            "Gemini-3.1-Pro"
        );
    }

    #[test]
    fn parses_scale_rsc_fixture() {
        let html = r#"<html>self.__next_f.push([1,"{\"model\":\"GPT 5.5 (Codex)\",\"score\":45.43},{\"model\":\"Opus-4.7 (Claude Code)\",\"score\":48.57}"])</html>"#;
        let rows = parse_rows_with_model_map(
            html,
            "SWEAtlasQnA",
            "sweatlas_qna",
            clean_swe_atlas_model_name,
        )
        .expect("fixture should parse");
        let by: BTreeMap<_, _> = rows
            .iter()
            .map(|row| {
                (
                    row.model_name.as_str(),
                    row.fields["SWEAtlasQnA"].as_f64().unwrap(),
                )
            })
            .collect();
        assert_eq!(by["GPT 5.5"], 45.43);
        assert_eq!(by["Opus-4.7"], 48.57);
    }
}
