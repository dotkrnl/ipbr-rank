use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ipbr_sources::{
    AaOmniscienceSource, ArtificialAnalysisSource, FetchOptions, Http, OpenRouterSource,
    SecretStore, Source, SourceError, refresh::fetch_with_fallback, registry::registry,
};
use serde_json::{Value, json};

struct BrokenUpstream;

#[async_trait::async_trait]
impl Http for BrokenUpstream {
    async fn get_json(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<Value, SourceError> {
        // Structurally valid JSON with no model rows; exercises both parser
        // failures and successful-but-empty parsing (e.g. OpenRouter).
        Ok(json!({"data": []}))
    }

    async fn get_text(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<String, SourceError> {
        Ok("<html>Leaderboard moved</html>".into())
    }
}

struct OfflineOnly;

#[async_trait::async_trait]
impl Http for OfflineOnly {
    async fn get_json(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<Value, SourceError> {
        panic!("offline/cache-only fetch attempted network access")
    }

    async fn get_text(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<String, SourceError> {
        panic!("offline/cache-only fetch attempted network access")
    }
}

struct JsonUpstream(Result<Value, &'static str>);

#[async_trait::async_trait]
impl Http for JsonUpstream {
    async fn get_json(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<Value, SourceError> {
        self.0
            .clone()
            .map_err(|message| SourceError::Http(message.into()))
    }

    async fn get_text(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<String, SourceError> {
        panic!("OpenRouter should only request JSON")
    }
}

#[tokio::test]
async fn successful_live_refresh_replaces_stale_cache_and_network_failure_reuses_it() {
    let dir = tempfile::tempdir().unwrap();
    let original = seed_cache(&OpenRouterSource, dir.path());
    let payload =
        json!({"data": [{"id": "openai/gpt-5.5", "name": "GPT-5.5", "context_length": 123456}]});
    let opts = FetchOptions {
        cache_dir: Some(dir.path()),
        offline: false,
    };
    let result = fetch_with_fallback(
        &OpenRouterSource,
        &JsonUpstream(Ok(payload.clone())),
        opts,
        &SecretStore::default(),
    )
    .await
    .unwrap();
    assert!(!result.stale);
    assert_eq!(result.rows.len(), 1);
    let path = &original[0].0;
    let stored: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(stored, payload);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH)
        .unwrap();

    let result = fetch_with_fallback(
        &OpenRouterSource,
        &JsonUpstream(Err("503 unavailable")),
        opts,
        &SecretStore::default(),
    )
    .await
    .unwrap();
    assert!(result.stale);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        std::fs::metadata(path).unwrap().modified().unwrap(),
        SystemTime::UNIX_EPOCH
    );
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures")
}

fn seed_cache(source: &dyn Source, dir: &Path) -> Vec<(PathBuf, Vec<u8>, SystemTime)> {
    source
        .cache_paths(&fixtures())
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| {
            let destination = dir.join(path.file_name().unwrap());
            std::fs::copy(&path, &destination).unwrap();
            let expired = SystemTime::now() - source.cache_ttl() - Duration::from_secs(60);
            std::fs::File::options()
                .write(true)
                .open(&destination)
                .unwrap()
                .set_modified(expired)
                .unwrap();
            let modified = std::fs::metadata(&destination).unwrap().modified().unwrap();
            let bytes = std::fs::read(&destination).unwrap();
            (destination, bytes, modified)
        })
        .collect()
}

#[tokio::test]
async fn every_network_source_preserves_and_reuses_valid_cache_on_upstream_drift() {
    let secrets = SecretStore::new(Some("test-key".into()), None, None);
    for source in registry() {
        if source.id() == "overrides" {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let original = seed_cache(source.as_ref(), dir.path());
        assert!(!original.is_empty(), "{} must have a fixture", source.id());
        let opts = FetchOptions {
            cache_dir: Some(dir.path()),
            offline: false,
        };
        let expected = source
            .fetch(
                &OfflineOnly,
                FetchOptions {
                    offline: true,
                    ..opts
                },
                &secrets,
            )
            .await
            .unwrap();

        // Repeat to catch a response that poisons the fallback on the next run.
        for _ in 0..2 {
            let result = fetch_with_fallback(source.as_ref(), &BrokenUpstream, opts, &secrets)
                .await
                .unwrap_or_else(|error| panic!("{}: {error}", source.id()));
            assert!(result.stale, "{} should report stale data", source.id());
            assert_eq!(
                result.rows.len(),
                expected.len(),
                "{} lost cached rows",
                source.id()
            );
            for (path, bytes, modified) in &original {
                assert_eq!(
                    &std::fs::read(path).unwrap(),
                    bytes,
                    "{} replaced valid cache",
                    source.id()
                );
                assert_eq!(
                    std::fs::metadata(path).unwrap().modified().unwrap(),
                    *modified,
                    "{} renewed the TTL of a failed response",
                    source.id()
                );
            }
        }
    }
}

#[tokio::test]
async fn missing_or_corrupt_cache_cannot_rescue_a_failed_source() {
    let dir = tempfile::tempdir().unwrap();
    let opts = FetchOptions {
        cache_dir: Some(dir.path()),
        offline: false,
    };
    for cached in [None, Some("<html>broken cache</html>")] {
        if let Some(cached) = cached {
            std::fs::write(dir.path().join("aa_omniscience.html"), cached).unwrap();
        }
        assert!(
            fetch_with_fallback(
                &AaOmniscienceSource,
                &BrokenUpstream,
                opts,
                &SecretStore::default()
            )
            .await
            .is_err()
        );
    }
    assert!(
        fetch_with_fallback(
            &AaOmniscienceSource,
            &BrokenUpstream,
            FetchOptions {
                cache_dir: None,
                offline: false
            },
            &SecretStore::default()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn offline_cache_miss_stays_an_error_without_network_access() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        fetch_with_fallback(
            &OpenRouterSource,
            &OfflineOnly,
            FetchOptions {
                cache_dir: Some(dir.path()),
                offline: true
            },
            &SecretStore::default()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn missing_credentials_can_still_use_valid_stale_cache() {
    let dir = tempfile::tempdir().unwrap();
    seed_cache(&ArtificialAnalysisSource, dir.path());
    let result = fetch_with_fallback(
        &ArtificialAnalysisSource,
        &OfflineOnly,
        FetchOptions {
            cache_dir: Some(dir.path()),
            offline: false,
        },
        &SecretStore::default(),
    )
    .await
    .unwrap();
    assert!(result.stale);
    assert!(!result.rows.is_empty());
}

#[tokio::test]
async fn fresh_cache_needs_no_network_and_is_not_stale() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixtures().join("openrouter_models.json"),
        dir.path().join("openrouter_models.json"),
    )
    .unwrap();
    let result = fetch_with_fallback(
        &OpenRouterSource,
        &OfflineOnly,
        FetchOptions {
            cache_dir: Some(dir.path()),
            offline: false,
        },
        &SecretStore::default(),
    )
    .await
    .unwrap();
    assert!(!result.stale);
    assert!(!result.rows.is_empty());
}
