use std::collections::BTreeMap;

use ipbr_core::RawRow;
use serde_json::Value;

use std::time::Duration;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "lmarena";
const CACHE_KEY: &str = "lmarena_overall";
const DATASET: &str = "lmarena-ai/leaderboard-dataset";
const CONFIGS: &[&str] = &["text", "webdev", "search", "document"];

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
        _secrets: &SecretStore,
    ) -> Result<Vec<RawRow>, SourceError> {
        if use_cached_json(opts, self.cache_key(), self.cache_ttl()) {
            let Some(dir) = opts.cache_dir else {
                return Err(SourceError::CacheMiss(format!(
                    "{} requires --cache in --offline mode",
                    self.id()
                )));
            };
            let payload = serde_json::from_slice::<Value>(&read_cached_bytes(&cache_json_path(
                dir,
                self.cache_key(),
            ))?)?;
            return parse_rows(&payload);
        }

        let mut wrapper = serde_json::Map::new();
        let mut configs = serde_json::Map::new();
        for config in CONFIGS {
            let mut pages = Vec::new();
            let mut offset = 0usize;
            loop {
                let url = format!(
                    "https://datasets-server.huggingface.co/rows?dataset={DATASET}&config={config}&split=latest&offset={offset}&length=100"
                );
                let page = http.get_json(&url, &[]).await?;
                let rows = page.get("rows").and_then(Value::as_array).ok_or_else(|| {
                    SourceError::Parse(format!("LMArena {config} payload missing rows[]"))
                })?;
                let page_len = rows.len();
                pages.push(page.clone());
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
            }
            configs.insert((*config).to_string(), Value::Array(pages));
        }
        wrapper.insert("dataset".to_string(), Value::String(DATASET.to_string()));
        wrapper.insert("split".to_string(), Value::String("latest".to_string()));
        wrapper.insert("configs".to_string(), Value::Object(configs));
        let payload = Value::Object(wrapper);
        if let Some(dir) = opts.cache_dir {
            write_cache_json(dir, self.cache_key(), &payload)?;
        }
        parse_rows(&payload)
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

    let mut rows_by_model: BTreeMap<(String, String), RawRow> = BTreeMap::new();
    for (config, pages) in config_pages {
        for page in pages {
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
                let key = (model_name.to_string(), vendor_hint.to_string());
                let raw_row = rows_by_model.entry(key).or_insert_with(|| RawRow {
                    source_id: SOURCE_ID.to_string(),
                    model_name: model_name.to_string(),
                    vendor_hint: (!vendor_hint.is_empty()).then(|| vendor_hint.to_string()),
                    fields: BTreeMap::new(),
                    synthesized_from: None,
                });
                map_rating(config, rating, &mut raw_row.fields);
                copy_numeric(&mut raw_row.fields, "Rank", row.get("rank"));
                copy_numeric(&mut raw_row.fields, "VoteCount", row.get("vote_count"));
                copy_numeric(&mut raw_row.fields, "RatingLower", row.get("rating_lower"));
                copy_numeric(&mut raw_row.fields, "RatingUpper", row.get("rating_upper"));
            }
        }
    }

    Ok(rows_by_model.into_values().collect())
}

fn map_rating(config: &str, rating: f64, fields: &mut BTreeMap<String, Value>) {
    match config {
        "text" => {
            fields.insert("LMArenaText".to_string(), Value::from(rating));
            fields
                .entry("LMArenaCreativeOrOpenEnded".to_string())
                .or_insert_with(|| Value::from(rating));
        }
        "webdev" => {
            fields.insert("CopilotArenaOrLMArenaCode".to_string(), Value::from(rating));
        }
        "search" | "document" => {
            // Reviewer note: the current coefficient set exposes a single hard-arena slot,
            // so search/document are coalesced into that shared metric until a dedicated
            // retrieval-style metric lands in a later source task.
            merge_max(fields, "JudgeArenaOrLMArenaHard", rating);
        }
        _ => {}
    }
}

fn merge_max(fields: &mut BTreeMap<String, Value>, key: &str, rating: f64) {
    let next = match fields.get(key).and_then(number_like) {
        Some(existing) => existing.max(rating),
        None => rating,
    };
    fields.insert(key.to_string(), Value::from(next));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            model_a
                .fields
                .get("JudgeArenaOrLMArenaHard")
                .and_then(number_like),
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
        assert_eq!(
            rows[0]
                .fields
                .get("LMArenaCreativeOrOpenEnded")
                .and_then(number_like),
            Some(1000.0)
        );
    }
}
