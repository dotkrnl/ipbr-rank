//! Opt-in recovery for scheduled refreshes. Source parsers stay strict and only
//! cache validated, nonempty live responses, so a failed refresh can reuse the
//! last successful response without marking it fresh.

use ipbr_core::RawRow;
use std::time::Duration;

use crate::{FetchOptions, Http, SecretStore, Source, SourceError, cache_is_fresh, require_rows};

pub struct RefreshResult {
    pub rows: Vec<RawRow>,
    pub stale: bool,
}

pub async fn fetch_with_fallback(
    source: &dyn Source,
    http: &dyn Http,
    opts: FetchOptions<'_>,
    secrets: &SecretStore,
) -> Result<RefreshResult, SourceError> {
    fetch_with_timeout(source, http, opts, secrets, Duration::from_secs(120)).await
}

async fn fetch_with_timeout(
    source: &dyn Source,
    http: &dyn Http,
    opts: FetchOptions<'_>,
    secrets: &SecretStore,
    timeout: Duration,
) -> Result<RefreshResult, SourceError> {
    // Offline callers must still see missing or corrupt cache errors.
    let fetch = source.fetch(http, opts, secrets);
    let result = if opts.offline {
        fetch.await
    } else {
        // Includes retries and pagination. Sources that support resuming keep
        // their partial downloads; a slow upstream cannot monopolize the job.
        tokio::time::timeout(timeout, fetch)
            .await
            .unwrap_or_else(|_| {
                Err(SourceError::Http(format!(
                    "source refresh timed out after {} seconds",
                    timeout.as_secs()
                )))
            })
    }
    .and_then(require_rows);
    match result {
        Ok(rows) => {
            // LMArena can already recover internally from a failed refresh.
            // Its old cache timestamp lets us expose that recovery as stale.
            let stale = !opts.offline
                && opts.cache_dir.is_some_and(|dir| {
                    let paths = source.cache_paths(dir);
                    !paths.is_empty()
                        && paths
                            .iter()
                            .all(|path| !cache_is_fresh(path, source.cache_ttl()))
                });
            Ok(RefreshResult { rows, stale })
        }
        Err(error) if !opts.offline && opts.cache_dir.is_some() => {
            match source
                .fetch(
                    http,
                    FetchOptions {
                        offline: true,
                        ..opts
                    },
                    secrets,
                )
                .await
                .and_then(require_rows)
            {
                Ok(rows) => {
                    eprintln!(
                        "warning: {} refresh failed ({error}); using stale cache",
                        source.id()
                    );
                    Ok(RefreshResult { rows, stale: true })
                }
                Err(cache_error) => {
                    eprintln!("warning: {} cache unavailable: {cache_error}", source.id());
                    Err(error)
                }
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenRouterSource;
    use std::path::Path;
    use std::time::SystemTime;

    struct HangingHttp;

    #[async_trait::async_trait]
    impl Http for HangingHttp {
        async fn get_json(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<serde_json::Value, SourceError> {
            std::future::pending().await
        }

        async fn get_text(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, SourceError> {
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn timeout_reuses_stale_data_but_fails_without_cache() {
        let dir = tempfile::tempdir().unwrap();
        let opts = FetchOptions {
            cache_dir: Some(dir.path()),
            offline: false,
        };
        let secrets = SecretStore::default();
        let error = fetch_with_timeout(
            &OpenRouterSource,
            &HangingHttp,
            opts,
            &secrets,
            Duration::from_millis(1),
        )
        .await
        .err()
        .expect("empty cache must fail");
        assert!(error.to_string().contains("timed out"));

        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/fixtures/openrouter_models.json");
        let path = dir.path().join("openrouter_models.json");
        std::fs::copy(fixture, &path).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        let result = fetch_with_timeout(
            &OpenRouterSource,
            &HangingHttp,
            opts,
            &secrets,
            Duration::from_millis(1),
        )
        .await
        .unwrap();
        assert!(result.stale);
        assert!(!result.rows.is_empty());
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            SystemTime::UNIX_EPOCH
        );
    }
}
