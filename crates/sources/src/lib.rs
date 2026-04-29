use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ipbr_core::RawRow;

pub mod aistupidlevel;
pub mod arc_agi;
pub mod artificial_analysis;
pub mod gso;
pub mod http;
pub mod livecodebench;
pub mod lmarena;
pub mod mcp_atlas;
pub mod openrouter;
pub mod overrides;
pub mod registry;
pub mod sonar;
pub mod swebench;
pub mod swebench_pro;
pub mod swerebench;
pub mod terminal_bench;

pub use aistupidlevel::AiStupidLevelSource;
pub use arc_agi::ArcAgiSource;
pub use artificial_analysis::ArtificialAnalysisSource;
pub use gso::GsoSource;
pub use http::ReqwestHttp;
pub use livecodebench::LiveCodeBenchSource;
pub use lmarena::LmArenaSource;
pub use mcp_atlas::McpAtlasSource;
pub use openrouter::OpenRouterSource;
pub use overrides::OverridesSource;
pub use sonar::SonarSource;
pub use swebench::SweBenchSource;
pub use swebench_pro::SweBenchProSource;
pub use swerebench::SweRebenchSource;
pub use terminal_bench::TerminalBenchSource;

#[derive(Debug, Clone, Copy)]
pub struct FetchOptions<'a> {
    pub cache_dir: Option<&'a Path>,
    pub offline: bool,
}

#[derive(Debug, Default, Clone)]
pub struct SecretStore {
    aa_api_key: Option<String>,
    openrouter_api_key: Option<String>,
    hf_token: Option<String>,
}

impl SecretStore {
    pub fn new(
        aa_api_key: Option<String>,
        openrouter_api_key: Option<String>,
        hf_token: Option<String>,
    ) -> Self {
        Self {
            aa_api_key,
            openrouter_api_key,
            hf_token,
        }
    }

    pub fn get(&self, secret: SecretRef) -> Option<&str> {
        match secret {
            SecretRef::AaApiKey => self.aa_api_key.as_deref(),
            SecretRef::OpenRouterApiKey => self.openrouter_api_key.as_deref(),
            SecretRef::HfToken => self.hf_token.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    Verified,
    Experimental,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretRef {
    AaApiKey,
    OpenRouterApiKey,
    HfToken,
}

#[async_trait::async_trait]
pub trait Http: Send + Sync {
    async fn get_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<serde_json::Value, SourceError>;

    async fn get_text(&self, url: &str, headers: &[(&str, &str)]) -> Result<String, SourceError>;
}

#[async_trait::async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> &str;
    fn cache_key(&self) -> &str {
        self.id()
    }
    fn status(&self) -> VerificationStatus;
    fn required_secret(&self) -> Option<SecretRef>;
    /// How long a cached payload remains fresh. When `--cache` is set and
    /// the cached file's age is under this duration, the source skips the
    /// network call. `--offline` always uses cache regardless. Default 24h.
    fn cache_ttl(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }
    /// Candidate on-disk cache paths the source may read from.
    /// Non-JSON sources must override this so provenance reflects the payload
    /// `fetch` actually consumes when multiple sibling extensions exist.
    fn cache_paths(&self, cache_dir: &Path) -> Vec<PathBuf> {
        let key = self.cache_key();
        vec![cache_json_path(cache_dir, key)]
    }
    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError>;
}

pub(crate) fn use_cached_json(opts: FetchOptions<'_>, key: &str, ttl: Duration) -> bool {
    if opts.offline {
        return true;
    }
    opts.cache_dir
        .map(|d| cache_is_fresh(&cache_json_path(d, key), ttl))
        .unwrap_or(false)
}

pub(crate) fn use_cached_html(opts: FetchOptions<'_>, key: &str, ttl: Duration) -> bool {
    if opts.offline {
        return true;
    }
    opts.cache_dir
        .map(|d| cache_is_fresh(&cache_html_path(d, key), ttl))
        .unwrap_or(false)
}

pub fn cache_is_fresh(path: &Path, ttl: Duration) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age < ttl)
        .unwrap_or(false)
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("cache miss: {0}")]
    CacheMiss(String),
    #[error("missing secret: {0}")]
    MissingSecret(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn cache_json_path(cache: &Path, key: &str) -> PathBuf {
    cache.join(format!("{key}.json"))
}

pub fn cache_csv_path(cache: &Path, key: &str) -> PathBuf {
    cache.join(format!("{key}.csv"))
}

pub fn cache_html_path(cache: &Path, key: &str) -> PathBuf {
    cache.join(format!("{key}.html"))
}

pub(crate) fn read_cached_bytes(path: &Path) -> Result<Vec<u8>, SourceError> {
    if !path.exists() {
        return Err(SourceError::CacheMiss(path.display().to_string()));
    }
    Ok(std::fs::read(path)?)
}

pub(crate) fn read_cached_string(path: &Path) -> Result<String, SourceError> {
    if !path.exists() {
        return Err(SourceError::CacheMiss(path.display().to_string()));
    }
    Ok(std::fs::read_to_string(path)?)
}

pub(crate) fn write_cache_json(
    cache_dir: &Path,
    key: &str,
    payload: &serde_json::Value,
) -> Result<(), SourceError> {
    std::fs::create_dir_all(cache_dir)?;
    let bytes = serde_json::to_vec_pretty(payload)?;
    std::fs::write(cache_json_path(cache_dir, key), bytes)?;
    Ok(())
}

pub(crate) fn write_cache_html(
    cache_dir: &Path,
    key: &str,
    payload: &str,
) -> Result<(), SourceError> {
    std::fs::create_dir_all(cache_dir)?;
    std::fs::write(cache_html_path(cache_dir, key), payload)?;
    Ok(())
}
