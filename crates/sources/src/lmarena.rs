use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ipbr_core::{AliasIndex, RawRow, normalize_name};
use serde_json::{Map, Value};

use std::time::Duration;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "lmarena";
const CACHE_KEY: &str = "lmarena_overall";
const DATASET: &str = "lmarena-ai/leaderboard-dataset";
const CONFIGS: &[&str] = &["text", "webdev", "search", "document"];
const WEB_PAGES: &[(&str, &str)] = &[
    ("text", "https://lmarena.ai/leaderboard/text"),
    ("webdev", "https://lmarena.ai/leaderboard/code"),
];
const PAGE_DELAY: Duration = Duration::from_secs(5);
const PAGE_ORIGIN_FIELD: &str = "_ipbr_origin";
const LIVE_WEB_ORIGIN: &str = "live_web";

#[derive(Debug, Default, Clone, Copy)]
pub struct LmArenaSource;

#[async_trait::async_trait]
impl Source for LmArenaSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn cache_key(&self) -> &str {
        CACHE_KEY
    }

    fn status(&self) -> VerificationStatus {
        VerificationStatus::Verified
    }

    fn required_secret(&self) -> Option<crate::SecretRef> {
        None
    }

    fn cache_ttl(&self) -> Duration {
        Duration::from_secs(24 * 3600)
    }

    async fn fetch(
        &self,
        http: &dyn Http,
        opts: FetchOptions<'_>,
        secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        if use_cached_json(opts, self.cache_key(), self.cache_ttl()) {
            let Some(dir) = opts.cache_dir else {
                return Err(SourceError::CacheMiss(format!(
                    "{} requires --cache in --offline mode",
                    self.id()
                )));
            };
            let mut payload = serde_json::from_slice::<Value>(&read_cached_bytes(
                &cache_json_path(dir, self.cache_key()),
            )?)?;
            if !opts.offline {
                append_live_web_pages_to_payload(http, &mut payload).await;
            }
            return parse_rows(&payload);
        }

        let auth_header = secrets
            .get(crate::SecretRef::HfToken)
            .map(|token| format!("Bearer {token}"));
        let headers = auth_header
            .as_ref()
            .map(|value| vec![("Authorization", value.as_str())])
            .unwrap_or_default();
        let partial_cache = opts
            .cache_dir
            .map(|dir| partial_cache_path(dir, self.cache_key()));
        let (mut payload, refresh_cache) =
            match fetch_live_payload(http, &headers, partial_cache.as_deref(), PAGE_DELAY).await {
                Ok(payload) => (payload, true),
                Err(err) => {
                    let Some(dir) = opts.cache_dir else {
                        return Err(err);
                    };
                    let path = cache_json_path(dir, self.cache_key());
                    if !path.exists() {
                        return Err(err);
                    }
                    eprintln!(
                        "warning: {} live fetch failed ({err}); using stale cache {}",
                        self.id(),
                        path.display()
                    );
                    (
                        serde_json::from_slice::<Value>(&read_cached_bytes(&path)?)?,
                        false,
                    )
                }
            };
        if !refresh_cache {
            append_live_web_pages_to_payload(http, &mut payload).await;
        }
        if let (true, Some(dir)) = (refresh_cache, opts.cache_dir) {
            write_cache_json(dir, self.cache_key(), &payload)?;
        }
        if let (true, Some(path)) = (refresh_cache, partial_cache.as_deref()) {
            remove_partial_cache(path)?;
        }
        parse_rows(&payload)
    }
}

async fn fetch_live_payload(
    http: &dyn Http,
    headers: &[(&str, &str)],
    partial_cache: Option<&Path>,
    page_delay: Duration,
) -> Result<Value, SourceError> {
    let mut configs = load_partial_configs(partial_cache)?;
    for config in CONFIGS {
        let mut pages = configs
            .remove(*config)
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        let mut offset = count_rows(&pages)?;
        if config_is_complete(&pages, offset)? {
            configs.insert((*config).to_string(), Value::Array(pages));
            continue;
        }

        loop {
            let url = format!(
                "https://datasets-server.huggingface.co/rows?dataset={DATASET}&config={config}&split=latest&offset={offset}&length=100"
            );
            let page = match http.get_json(&url, headers).await {
                Ok(page) => page,
                Err(err) if offset == 0 && is_locked_dataset_error(&err) => {
                    let fallback_url = format!(
                        "https://datasets-server.huggingface.co/first-rows?dataset={DATASET}&config={config}&split=latest"
                    );
                    let page = http.get_json(&fallback_url, headers).await?;
                    pages.push(page);
                    write_partial_cache(partial_cache, &configs, config, &pages)?;
                    break;
                }
                Err(err) => return Err(err),
            };
            let rows = page.get("rows").and_then(Value::as_array).ok_or_else(|| {
                SourceError::Parse(format!("LMArena {config} payload missing rows[]"))
            })?;
            let page_len = rows.len();
            pages.push(page.clone());
            write_partial_cache(partial_cache, &configs, config, &pages)?;
            let total = page
                .get("num_rows_total")
                .and_then(Value::as_u64)
                .or_else(|| page.get("num_rows").and_then(Value::as_u64))
                .unwrap_or(page_len as u64);
            if page_len == 0 {
                break;
            }
            offset += page_len;
            if offset as u64 >= total {
                break;
            }
            sleep_between_pages(page_delay).await;
        }
        configs.insert((*config).to_string(), Value::Array(pages));
    }

    let mut wrapper = Map::new();
    wrapper.insert("dataset".to_string(), Value::String(DATASET.to_string()));
    wrapper.insert("split".to_string(), Value::String("latest".to_string()));
    append_live_web_pages(http, &mut configs).await;
    wrapper.insert("configs".to_string(), Value::Object(configs));
    Ok(Value::Object(wrapper))
}

async fn append_live_web_pages_to_payload(http: &dyn Http, payload: &mut Value) {
    if let Some(configs) = payload.get_mut("configs").and_then(Value::as_object_mut) {
        append_live_web_pages(http, configs).await;
    }
}

async fn append_live_web_pages(http: &dyn Http, configs: &mut Map<String, Value>) {
    for (config, url) in WEB_PAGES {
        match http.get_text(url, &[("User-Agent", "ipbr-rank")]).await {
            Ok(html) => match parse_web_page(config, &html) {
                Ok(Some(page)) => configs
                    .entry((*config).to_string())
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .expect("LMArena configs are arrays")
                    .push(page),
                Ok(None) => {}
                Err(err) => eprintln!("warning: LMArena live {config} parse failed: {err}"),
            },
            Err(err) => eprintln!("warning: LMArena live {config} fetch failed: {err}"),
        }
    }
}

fn parse_web_page(config: &str, html: &str) -> Result<Option<Value>, SourceError> {
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_anchor) = html[cursor..].find(r#"\"modelKey\":\""#) {
        let anchor = cursor + rel_anchor;
        let Some(start) = html[..anchor].rfind('{') else {
            cursor = anchor + 1;
            continue;
        };
        let Some(end_rel) = html[anchor..].find('}') else {
            cursor = anchor + 1;
            continue;
        };
        let end = anchor + end_rel;
        cursor = end + 1;

        let object_text = unescape_jsx_string(&html[start..=end]);
        let item: Value = serde_json::from_str(&object_text).map_err(|err| {
            SourceError::Parse(format!("LMArena {config} live row failed to parse: {err}"))
        })?;
        let Some(model_name) = item.get("modelDisplayName").and_then(Value::as_str) else {
            continue;
        };
        let Some(rating) = item.get("rating").and_then(number_like) else {
            continue;
        };
        let mut row = Map::new();
        row.insert("category".to_string(), Value::String("overall".to_string()));
        row.insert(
            "model_name".to_string(),
            Value::String(model_name.to_string()),
        );
        row.insert("rating".to_string(), Value::from(rating));
        if let Some(org) = item.get("modelOrganization").and_then(Value::as_str) {
            row.insert("organization".to_string(), Value::String(org.to_string()));
        }
        copy_web_numeric(&mut row, "rank", item.get("rank"));
        copy_web_numeric(&mut row, "vote_count", item.get("votes"));
        copy_web_numeric(&mut row, "rating_lower", item.get("ratingLower"));
        copy_web_numeric(&mut row, "rating_upper", item.get("ratingUpper"));

        let mut entry = Map::new();
        entry.insert("row".to_string(), Value::Object(row));
        rows.push(Value::Object(entry));
    }

    if rows.is_empty() {
        return Ok(None);
    }
    let mut page = Map::new();
    page.insert("rows".to_string(), Value::Array(rows));
    // The API dataset can lag the live leaderboard. Retain transport
    // provenance in the cached wrapper so parsing can deterministically give
    // the current website row precedence even when page order changes.
    page.insert(
        PAGE_ORIGIN_FIELD.to_string(),
        Value::String(LIVE_WEB_ORIGIN.to_string()),
    );
    Ok(Some(Value::Object(page)))
}

/// Unwinds the JSON escaping used inside Next.js' streamed RSC payload.
fn unescape_jsx_string(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    let bytes = escaped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'\\' {
            match bytes[i + 1] {
                b'"' => {
                    out.push('"');
                    i += 2;
                    continue;
                }
                b'\\' => {
                    out.push('\\');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn partial_cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(format!("{key}.partial.json"))
}

fn load_partial_configs(partial_cache: Option<&Path>) -> Result<Map<String, Value>, SourceError> {
    let Some(path) = partial_cache else {
        return Ok(Map::new());
    };
    if !path.exists() {
        return Ok(Map::new());
    }
    let payload = serde_json::from_slice::<Value>(&read_cached_bytes(path)?)?;
    Ok(payload
        .get("configs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default())
}

fn write_partial_cache(
    partial_cache: Option<&Path>,
    completed_configs: &Map<String, Value>,
    active_config: &str,
    active_pages: &[Value],
) -> Result<(), SourceError> {
    let Some(path) = partial_cache else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut configs = completed_configs.clone();
    configs.insert(
        active_config.to_string(),
        Value::Array(active_pages.to_vec()),
    );

    let mut wrapper = Map::new();
    wrapper.insert("dataset".to_string(), Value::String(DATASET.to_string()));
    wrapper.insert("split".to_string(), Value::String("latest".to_string()));
    wrapper.insert("configs".to_string(), Value::Object(configs));

    let bytes = serde_json::to_vec_pretty(&Value::Object(wrapper))?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn remove_partial_cache(path: &Path) -> Result<(), SourceError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(SourceError::Io(err)),
    }
}

fn count_rows(pages: &[Value]) -> Result<usize, SourceError> {
    let mut total = 0usize;
    for page in pages {
        total += page
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| SourceError::Parse("LMArena partial page missing rows[]".into()))?
            .len();
    }
    Ok(total)
}

fn config_is_complete(pages: &[Value], offset: usize) -> Result<bool, SourceError> {
    let Some(last) = pages.last() else {
        return Ok(false);
    };
    let rows = last
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("LMArena partial page missing rows[]".into()))?;
    if rows.is_empty()
        || last
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Ok(true);
    }
    let total = last
        .get("num_rows_total")
        .and_then(Value::as_u64)
        .or_else(|| last.get("num_rows").and_then(Value::as_u64));
    Ok(total.is_some_and(|total| offset as u64 >= total))
}

async fn sleep_between_pages(delay: Duration) {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let config_pages = if let Some(configs) = payload.get("configs").and_then(Value::as_object) {
        let mut out = Vec::new();
        for (config, pages) in configs {
            let pages = pages.as_array().ok_or_else(|| {
                SourceError::Parse(format!("LMArena config {config} pages must be an array"))
            })?;
            out.push((config.as_str(), pages.clone()));
        }
        out
    } else if payload.get("rows").is_some() {
        vec![("text", vec![payload.clone()])]
    } else {
        return Err(SourceError::Parse(
            "LMArena payload must be a rows page or a config wrapper".into(),
        ));
    };

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut rows_by_model: BTreeMap<(String, LmArenaVariant), AccumulatedRow> = BTreeMap::new();
    for (config, pages) in config_pages {
        for page in pages {
            let origin = PageOrigin::from_page(&page);
            let rows = page.get("rows").and_then(Value::as_array).ok_or_else(|| {
                SourceError::Parse(format!("LMArena {config} page missing rows[]"))
            })?;
            for entry in rows {
                let row = entry.get("row").unwrap_or(entry);
                let model_name = row
                    .get("model_name")
                    .or_else(|| row.get("model"))
                    .or_else(|| row.get("name"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        SourceError::Parse(format!("LMArena {config} row missing model name"))
                    })?;
                let vendor_hint = row
                    .get("organization")
                    .or_else(|| row.get("creator"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let category = row
                    .get("category")
                    .and_then(Value::as_str)
                    .unwrap_or("overall");
                if category != "overall" {
                    continue;
                }
                let rating = row.get("rating").and_then(number_like).ok_or_else(|| {
                    SourceError::Parse(format!("LMArena {config} row missing numeric rating"))
                })?;
                let identity = crate::alias_dedupe_key(
                    &alias_records,
                    &alias_index,
                    model_name,
                    (!vendor_hint.is_empty()).then_some(vendor_hint),
                );
                let key = (identity, LmArenaVariant::from_label(model_name));
                let accumulated = rows_by_model.entry(key).or_insert_with(|| AccumulatedRow {
                    row: RawRow {
                        source_id: SOURCE_ID.to_string(),
                        model_name: model_name.to_string(),
                        vendor_hint: (!vendor_hint.is_empty()).then(|| vendor_hint.to_string()),
                        fields: BTreeMap::new(),
                        synthesized_from: None,
                        synthesis_category: None,
                    },
                    field_origins: BTreeMap::new(),
                    label_origin: origin,
                });
                accumulated.prefer_label(model_name, vendor_hint, origin);

                let mut fields = BTreeMap::new();
                map_rating(config, rating, &mut fields);
                copy_numeric(&mut fields, "Rank", row.get("rank"));
                copy_numeric(&mut fields, "VoteCount", row.get("vote_count"));
                copy_numeric(&mut fields, "RatingLower", row.get("rating_lower"));
                copy_numeric(&mut fields, "RatingUpper", row.get("rating_upper"));
                accumulated.merge_fields(fields, origin);
            }
        }
    }

    Ok(rows_by_model
        .into_values()
        .map(|accumulated| accumulated.row)
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PageOrigin {
    Dataset,
    LiveWeb,
}

impl PageOrigin {
    fn from_page(page: &Value) -> Self {
        if page.get(PAGE_ORIGIN_FIELD).and_then(Value::as_str) == Some(LIVE_WEB_ORIGIN) {
            Self::LiveWeb
        } else {
            Self::Dataset
        }
    }
}

/// Keep effort/configuration variants separate while collapsing spelling and
/// organization-casing differences to one model identity. The ranking layer
/// still needs distinct rows so it can apply its configured effort policy per
/// metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LmArenaVariant {
    Max,
    XHigh,
    High,
    Thinking,
    Medium,
    Default,
    Low,
    NonReasoning,
}

impl LmArenaVariant {
    fn from_label(label: &str) -> Self {
        let normalized = normalize_name(label);
        let contains = |phrase: &str| normalized.split_whitespace().any(|token| token == phrase);
        if normalized.contains("non reasoning") {
            Self::NonReasoning
        } else if contains("instant") || contains("minimal") || contains("low") {
            Self::Low
        } else if contains("max") {
            Self::Max
        } else if contains("xhigh") {
            Self::XHigh
        } else if contains("high") {
            Self::High
        } else if contains("medium") {
            Self::Medium
        } else if contains("thinking") || contains("reasoning") || contains("adaptive") {
            Self::Thinking
        } else {
            Self::Default
        }
    }
}

struct AccumulatedRow {
    row: RawRow,
    field_origins: BTreeMap<String, PageOrigin>,
    label_origin: PageOrigin,
}

impl AccumulatedRow {
    fn prefer_label(&mut self, model_name: &str, vendor_hint: &str, origin: PageOrigin) {
        if origin > self.label_origin {
            self.row.model_name = model_name.to_string();
            self.row.vendor_hint = (!vendor_hint.is_empty()).then(|| vendor_hint.to_string());
            self.label_origin = origin;
        }
    }

    fn merge_fields(&mut self, fields: BTreeMap<String, Value>, origin: PageOrigin) {
        for (key, value) in fields {
            if self
                .field_origins
                .get(&key)
                .is_none_or(|existing| origin >= *existing)
            {
                self.row.fields.insert(key.clone(), value);
                self.field_origins.insert(key, origin);
            }
        }
    }
}

fn is_locked_dataset_error(err: &SourceError) -> bool {
    match err {
        SourceError::Http(message) => {
            message.contains("LockedDatasetTimeoutError")
                || message.contains("dataset is currently locked")
                || message.contains("501 Not Implemented")
        }
        _ => false,
    }
}

fn map_rating(config: &str, rating: f64, fields: &mut BTreeMap<String, Value>) {
    match config {
        "text" => {
            fields.insert("LMArenaText".to_string(), Value::from(rating));
        }
        "webdev" => {
            fields.insert("CopilotArenaOrLMArenaCode".to_string(), Value::from(rating));
        }
        "search" => {
            fields.insert("LMArenaSearch".to_string(), Value::from(rating));
        }
        "document" => {
            fields.insert("LMArenaDocument".to_string(), Value::from(rating));
        }
        _ => {}
    }
}

fn number_like(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn copy_numeric(fields: &mut BTreeMap<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value.and_then(number_like) {
        fields.insert(key.to_string(), Value::from(value));
    }
}

fn copy_web_numeric(fields: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value.and_then(number_like) {
        fields.insert(key.to_string(), Value::from(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct HeaderCheckingHttp {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Http for HeaderCheckingHttp {
        async fn get_json(
            &self,
            _url: &str,
            headers: &[(&str, &str)],
        ) -> Result<Value, SourceError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert_eq!(headers, &[("Authorization", "Bearer hf_test_token")]);
            Ok(json!({
                "rows": [],
                "num_rows_total": 0
            }))
        }

        async fn get_text(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, SourceError> {
            Ok(String::new())
        }
    }

    #[tokio::test]
    async fn fetch_sends_optional_hf_bearer_token() {
        let http = HeaderCheckingHttp {
            calls: AtomicUsize::new(0),
        };
        let secrets = SecretStore::new(None, None, Some("hf_test_token".to_string()));

        let rows = LmArenaSource
            .fetch(
                &http,
                FetchOptions {
                    cache_dir: None,
                    offline: false,
                },
                &secrets,
            )
            .await
            .expect("empty HF pages still parse");

        assert!(rows.is_empty());
        assert_eq!(http.calls.load(Ordering::Relaxed), CONFIGS.len());
    }

    struct LockedRowsHttp;

    #[async_trait::async_trait]
    impl Http for LockedRowsHttp {
        async fn get_json(
            &self,
            url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<Value, SourceError> {
            if url.contains("/rows?") && url.contains("config=text") {
                return Err(SourceError::Http(
                    "HTTP status server error (501 Not Implemented); LockedDatasetTimeoutError"
                        .to_string(),
                ));
            }
            if url.contains("/first-rows?") && url.contains("config=text") {
                return Ok(json!({
                    "rows": [
                        {"row_idx": 0, "row": {
                            "model_name": "claude-opus-4-6-thinking",
                            "organization": "anthropic",
                            "rating": 1499.4,
                            "category": "overall"
                        }}
                    ],
                    "truncated": true
                }));
            }
            Ok(json!({
                "rows": [],
                "num_rows_total": 0
            }))
        }

        async fn get_text(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, SourceError> {
            Ok(String::new())
        }
    }

    #[tokio::test]
    async fn fetch_falls_back_to_first_rows_when_rows_endpoint_is_locked() {
        let rows = LmArenaSource
            .fetch(
                &LockedRowsHttp,
                FetchOptions {
                    cache_dir: None,
                    offline: false,
                },
                &SecretStore::default(),
            )
            .await
            .expect("locked datasets-server rows endpoint should use first-rows fallback");

        let row = rows
            .iter()
            .find(|row| row.model_name == "claude-opus-4-6-thinking")
            .expect("fallback row should be parsed");
        assert_eq!(
            row.fields.get("LMArenaText").and_then(number_like),
            Some(1499.4)
        );
    }

    struct FailingRowsHttp;

    #[async_trait::async_trait]
    impl Http for FailingRowsHttp {
        async fn get_json(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<Value, SourceError> {
            Err(SourceError::Http(
                "HTTP status server error (500 Internal Server Error)".to_string(),
            ))
        }

        async fn get_text(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, SourceError> {
            Ok(String::new())
        }
    }

    #[tokio::test]
    async fn online_fetch_uses_stale_cache_when_upstream_fails() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let payload = json!({
            "configs": {
                "text": [{
                    "rows": [
                        {"row": {
                            "model_name": "cached-model",
                            "organization": "cached-vendor",
                            "rating": 1234.0,
                            "category": "overall"
                        }}
                    ],
                    "num_rows_total": 1
                }]
            }
        });
        write_cache_json(tmp.path(), CACHE_KEY, &payload).expect("cache should write");
        let cache_path = cache_json_path(tmp.path(), CACHE_KEY);
        let status = std::process::Command::new("touch")
            .args(["-t", "202001010000"])
            .arg(&cache_path)
            .status()
            .expect("touch should run");
        assert!(status.success(), "touch should mark fixture cache stale");

        let rows = LmArenaSource
            .fetch(
                &FailingRowsHttp,
                FetchOptions {
                    cache_dir: Some(tmp.path()),
                    offline: false,
                },
                &SecretStore::default(),
            )
            .await
            .expect("stale cache should be used when online refresh fails");

        let row = rows
            .iter()
            .find(|row| row.model_name == "cached-model")
            .expect("cached row should be parsed");
        assert_eq!(
            row.fields.get("LMArenaText").and_then(number_like),
            Some(1234.0)
        );
        assert!(
            !crate::cache_is_fresh(&cache_path, LmArenaSource.cache_ttl()),
            "stale fallback should not refresh cache metadata"
        );
    }

    struct RecordingResumeHttp {
        urls: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Http for RecordingResumeHttp {
        async fn get_json(
            &self,
            url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<Value, SourceError> {
            self.urls.lock().expect("urls lock").push(url.to_string());
            if url.contains("config=text") && url.contains("offset=1") {
                return Ok(json!({
                    "rows": [
                        {"row": {
                            "model_name": "model-b",
                            "organization": "anthropic",
                            "rating": 1010.0,
                            "category": "overall"
                        }}
                    ],
                    "num_rows_total": 2
                }));
            }
            Ok(json!({
                "rows": [],
                "num_rows_total": 0
            }))
        }

        async fn get_text(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, SourceError> {
            Ok(String::new())
        }
    }

    #[tokio::test]
    async fn live_fetch_resumes_existing_partial_cache() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let partial_path = partial_cache_path(tmp.path(), CACHE_KEY);
        let partial = json!({
            "dataset": DATASET,
            "split": "latest",
            "configs": {
                "text": [{
                    "rows": [
                        {"row": {
                            "model_name": "model-a",
                            "organization": "openai",
                            "rating": 1000.0,
                            "category": "overall"
                        }}
                    ],
                    "num_rows_total": 2
                }]
            }
        });
        std::fs::write(&partial_path, serde_json::to_vec_pretty(&partial).unwrap())
            .expect("partial cache should write");

        let http = RecordingResumeHttp {
            urls: Mutex::new(Vec::new()),
        };
        let payload = fetch_live_payload(&http, &[], Some(&partial_path), Duration::ZERO)
            .await
            .expect("partial cache should resume");
        let rows = parse_rows(&payload).expect("resumed payload should parse");

        assert_eq!(rows.len(), 2);
        let urls = http.urls.lock().expect("urls lock");
        assert!(
            urls.iter()
                .any(|url| url.contains("config=text") && url.contains("offset=1")),
            "text config should resume at offset 1, got {urls:?}"
        );
        assert!(
            !urls
                .iter()
                .any(|url| url.contains("config=text") && url.contains("offset=0")),
            "text config should not refetch offset 0 when partial cache is present"
        );
    }

    struct FailsAfterFirstPageHttp;

    #[async_trait::async_trait]
    impl Http for FailsAfterFirstPageHttp {
        async fn get_json(
            &self,
            url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<Value, SourceError> {
            if url.contains("config=text") && url.contains("offset=0") {
                return Ok(json!({
                    "rows": [
                        {"row": {
                            "model_name": "partial-model",
                            "organization": "openai",
                            "rating": 1000.0,
                            "category": "overall"
                        }}
                    ],
                    "num_rows_total": 2
                }));
            }
            Err(SourceError::Http(
                "HTTP status client error (429 Too Many Requests)".to_string(),
            ))
        }

        async fn get_text(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<String, SourceError> {
            Ok(String::new())
        }
    }

    #[tokio::test]
    async fn interrupted_empty_cache_refresh_preserves_partial_progress() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let partial_path = partial_cache_path(tmp.path(), CACHE_KEY);

        let err = LmArenaSource
            .fetch(
                &FailsAfterFirstPageHttp,
                FetchOptions {
                    cache_dir: Some(tmp.path()),
                    offline: false,
                },
                &SecretStore::default(),
            )
            .await
            .expect_err("empty-cache interrupted refresh should still fail");
        assert!(
            err.to_string().contains("429"),
            "expected the upstream 429 to surface, got {err}"
        );

        let partial = serde_json::from_slice::<Value>(
            &read_cached_bytes(&partial_path).expect("partial cache should exist"),
        )
        .expect("partial cache should parse");
        let text_pages = partial
            .get("configs")
            .and_then(Value::as_object)
            .and_then(|configs| configs.get("text"))
            .and_then(Value::as_array)
            .expect("partial cache should contain text pages");
        assert_eq!(text_pages.len(), 1);
        assert_eq!(
            count_rows(text_pages).expect("partial rows should count"),
            1
        );
        assert!(
            !cache_json_path(tmp.path(), CACHE_KEY).exists(),
            "interrupted empty-cache refresh must not publish an incomplete full cache"
        );
    }

    #[tokio::test]
    async fn successful_fetch_removes_partial_cache() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let partial_path = partial_cache_path(tmp.path(), CACHE_KEY);
        std::fs::write(
            &partial_path,
            serde_json::to_vec_pretty(&json!({
                "dataset": DATASET,
                "split": "latest",
                "configs": {
                    "text": [{
                        "rows": [],
                        "num_rows_total": 0
                    }]
                }
            }))
            .unwrap(),
        )
        .expect("partial cache should write");

        let http = HeaderCheckingHttp {
            calls: AtomicUsize::new(0),
        };
        let rows = LmArenaSource
            .fetch(
                &http,
                FetchOptions {
                    cache_dir: Some(tmp.path()),
                    offline: false,
                },
                &SecretStore::new(None, None, Some("hf_test_token".to_string())),
            )
            .await
            .expect("successful fetch should parse");

        assert!(rows.is_empty());
        assert!(
            !partial_path.exists(),
            "successful full refresh should remove partial cache"
        );
        assert!(
            cache_json_path(tmp.path(), CACHE_KEY).exists(),
            "successful full refresh should publish full cache"
        );
    }

    #[test]
    fn parse_wrapper_maps_all_configs_and_pages() {
        let payload = json!({
            "configs": {
                "text": [{
                    "rows": [
                        {"row": {"model_name": "model-a", "organization": "openai", "rating": 1000.0, "category": "overall"}}
                    ],
                    "num_rows_total": 2
                }, {
                    "rows": [
                        {"row": {"model_name": "model-b", "organization": "anthropic", "rating": 1010.0, "category": "overall"}}
                    ],
                    "num_rows_total": 2
                }],
                "webdev": [{
                    "rows": [
                        {"row": {"model_name": "model-a", "organization": "openai", "rating": 990.0, "category": "overall"}}
                    ],
                    "num_rows_total": 1
                }],
                "search": [{
                    "rows": [
                        {"row": {"model_name": "model-a", "organization": "openai", "rating": 980.0, "category": "overall"}}
                    ],
                    "num_rows_total": 1
                }],
                "document": [{
                    "rows": [
                        {"row": {"model_name": "model-a", "organization": "openai", "rating": 995.0, "category": "overall"}}
                    ],
                    "num_rows_total": 1
                }]
            }
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 2);
        let model_a = rows.iter().find(|row| row.model_name == "model-a").unwrap();
        assert_eq!(model_a.vendor_hint.as_deref(), Some("openai"));
        assert_eq!(
            model_a.fields.get("LMArenaText").and_then(number_like),
            Some(1000.0)
        );
        assert_eq!(
            model_a
                .fields
                .get("CopilotArenaOrLMArenaCode")
                .and_then(number_like),
            Some(990.0)
        );
        assert_eq!(
            model_a.fields.get("LMArenaSearch").and_then(number_like),
            Some(980.0)
        );
        assert_eq!(
            model_a.fields.get("LMArenaDocument").and_then(number_like),
            Some(995.0)
        );
    }

    #[test]
    fn single_page_fixture_defaults_to_text_mapping() {
        let payload = json!({
            "rows": [
                {"row": {"model_name": "model-a", "organization": "openai", "rating": 1000.0, "category": "overall"}}
            ],
            "num_rows_total": 1
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(
            rows[0].fields.get("LMArenaText").and_then(number_like),
            Some(1000.0)
        );
        assert!(!rows[0].fields.contains_key("LMArenaCreativeOrOpenEnded"));
    }

    #[test]
    fn parse_web_page_extracts_live_leaderboard_rows() {
        let html = r#"<script>self.__next_f.push([1,"{\"rank\":2,\"rankUpper\":2,\"rankLower\":2,\"modelKey\":\"glm-5.2-code\",\"modelDisplayName\":\"glm-5.2 (max)\",\"rating\":1595.19,\"ratingUpper\":1611.46,\"ratingLower\":1578.92,\"votes\":1641,\"modelOrganization\":\"Z.ai\",\"modelUrl\":\"https://huggingface.co/zai-org/GLM-5.2\",\"license\":\"MIT\"}"])</script>"#;
        let page = parse_web_page("webdev", html)
            .expect("web page should parse")
            .expect("web page should yield rows");
        let payload = json!({
            "configs": {
                "webdev": [page]
            }
        });
        let rows = parse_rows(&payload).expect("web page rows should map through normal parser");
        let row = rows
            .iter()
            .find(|row| row.model_name == "glm-5.2 (max)")
            .expect("GLM-5.2 max row should be present");

        assert_eq!(
            row.fields
                .get("CopilotArenaOrLMArenaCode")
                .and_then(number_like),
            Some(1595.19)
        );
        assert_eq!(
            row.fields.get("VoteCount").and_then(number_like),
            Some(1641.0)
        );
        assert_eq!(row.vendor_hint.as_deref(), Some("Z.ai"));
        assert_eq!(
            page.get(PAGE_ORIGIN_FIELD).and_then(Value::as_str),
            Some(LIVE_WEB_ORIGIN)
        );
    }

    #[test]
    fn dedupes_vendor_case_by_normalized_model_identity() {
        let payload = json!({
            "configs": {
                "text": [{
                    "rows": [{"row": {
                        "model_name": "case-model",
                        "organization": "Anthropic",
                        "rating": 1400.0,
                        "category": "overall"
                    }}]
                }],
                "webdev": [{
                    "rows": [{"row": {
                        "model_name": "Case Model",
                        "organization": "anthropic",
                        "rating": 1410.0,
                        "category": "overall"
                    }}]
                }]
            }
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1, "vendor casing must not create a second row");
        assert_eq!(
            rows[0].fields.get("LMArenaText").and_then(number_like),
            Some(1400.0)
        );
        assert_eq!(
            rows[0]
                .fields
                .get("CopilotArenaOrLMArenaCode")
                .and_then(number_like),
            Some(1410.0)
        );
    }

    #[test]
    fn live_web_row_wins_even_when_dataset_page_comes_later() {
        let payload = json!({
            "configs": {
                "text": [{
                    PAGE_ORIGIN_FIELD: LIVE_WEB_ORIGIN,
                    "rows": [{"row": {
                        "model_name": "Live Model",
                        "organization": "OpenAI",
                        "rating": 1508.62,
                        "rank": 3,
                        "vote_count": 5000,
                        "category": "overall"
                    }}]
                }, {
                    "rows": [{"row": {
                        "model_name": "live-model",
                        "organization": "openai",
                        "rating": 1495.08,
                        "rank": 7,
                        "vote_count": 4000,
                        "category": "overall"
                    }}]
                }]
            }
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.model_name, "Live Model");
        assert_eq!(row.vendor_hint.as_deref(), Some("OpenAI"));
        assert_eq!(
            row.fields.get("LMArenaText").and_then(number_like),
            Some(1508.62)
        );
        assert_eq!(row.fields.get("Rank").and_then(number_like), Some(3.0));
        assert_eq!(
            row.fields.get("VoteCount").and_then(number_like),
            Some(5000.0)
        );
    }

    #[test]
    fn instant_and_default_rows_remain_distinct_before_effort_filtering() {
        let payload = json!({
            "configs": {
                "text": [{
                    "rows": [
                        {"row": {
                            "model_name": "kimi-k2.5",
                            "organization": "Moonshot AI",
                            "rating": 1400.0,
                            "category": "overall"
                        }},
                        {"row": {
                            "model_name": "kimi-k2.5-instant",
                            "organization": "moonshot ai",
                            "rating": 1300.0,
                            "category": "overall"
                        }}
                    ]
                }]
            }
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 2, "instant is a distinct low-effort row");
        assert!(rows.iter().any(|row| row.model_name == "kimi-k2.5"));
        assert!(rows.iter().any(|row| row.model_name == "kimi-k2.5-instant"));
    }
}
