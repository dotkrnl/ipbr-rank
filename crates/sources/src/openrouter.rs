use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::RawRow;
use serde_json::Value;

use crate::{
    FetchOptions, Http, SecretStore, Source, SourceError, VerificationStatus, cache_json_path,
    read_cached_bytes, use_cached_json, write_cache_json,
};

const SOURCE_ID: &str = "openrouter";
const CACHE_KEY: &str = "openrouter_models";
const URL: &str = "https://openrouter.ai/api/v1/models";

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenRouterSource;

#[async_trait::async_trait]
impl Source for OpenRouterSource {
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
        let payload = if use_cached_json(opts, self.cache_key(), self.cache_ttl()) {
            let Some(dir) = opts.cache_dir else {
                return Err(SourceError::CacheMiss(format!(
                    "{} requires --cache in --offline mode",
                    self.id()
                )));
            };
            serde_json::from_slice::<Value>(&read_cached_bytes(&cache_json_path(
                dir,
                self.cache_key(),
            ))?)?
        } else {
            let payload = http.get_json(URL, &[]).await?;
            if let Some(dir) = opts.cache_dir {
                write_cache_json(dir, self.cache_key(), &payload)?;
            }
            payload
        };
        parse_rows(&payload)
    }
}

fn parse_rows(payload: &Value) -> Result<Vec<RawRow>, SourceError> {
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("OpenRouter payload missing data[]".into()))?;

    let mut rows = Vec::with_capacity(data.len());
    for item in data {
        let model_id = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.get("canonical_slug").and_then(Value::as_str))
            .or_else(|| item.get("name").and_then(Value::as_str))
            .ok_or_else(|| SourceError::Parse("OpenRouter row missing id/name".into()))?;
        let vendor_hint = model_id.split('/').next().filter(|s| !s.is_empty());

        let mut fields = BTreeMap::new();
        copy_if_present(&mut fields, "ModelId", item.get("id"));
        copy_if_present(&mut fields, "CanonicalSlug", item.get("canonical_slug"));
        copy_if_present(&mut fields, "DisplayName", item.get("name"));
        copy_if_present(&mut fields, "Created", item.get("created"));

        let context_length = item
            .get("context_length")
            .and_then(number_like)
            .or_else(|| {
                item.get("top_provider")
                    .and_then(|value| value.get("context_length"))
                    .and_then(number_like)
            });
        if let Some(context_length) = context_length {
            fields.insert("ContextWindow".to_string(), Value::from(context_length));
        }

        if let Some(top_provider) = item.get("top_provider") {
            copy_numeric(
                &mut fields,
                "MaxCompletionTokens",
                top_provider.get("max_completion_tokens"),
            );
        }

        if let Some(pricing) = item.get("pricing") {
            let prompt = pricing.get("prompt").and_then(number_like);
            let completion = pricing.get("completion").and_then(number_like);
            if let Some(prompt) = prompt {
                fields.insert(
                    "PromptPricePerMillion".to_string(),
                    Value::from(prompt * 1_000_000.0),
                );
            }
            if let Some(completion) = completion {
                fields.insert(
                    "CompletionPricePerMillion".to_string(),
                    Value::from(completion * 1_000_000.0),
                );
            }
            if let (Some(prompt), Some(completion)) = (prompt, completion) {
                let blended = (0.75 * prompt + 0.25 * completion) * 1_000_000.0;
                if blended.is_finite() && blended > 0.0 {
                    fields.insert("InverseCost".to_string(), Value::from(blended));
                }
            }
        }

        if let Some(supported) = item.get("supported_parameters").and_then(Value::as_array) {
            let set: std::collections::BTreeSet<&str> =
                supported.iter().filter_map(Value::as_str).collect();
            fields.insert(
                "SupportedParametersCount".to_string(),
                Value::from(supported.len() as u64),
            );
            fields.insert(
                "SupportsTools".to_string(),
                Value::from(set.contains("tools") || set.contains("tool_choice")),
            );
            fields.insert(
                "SupportsStructuredOutputs".to_string(),
                Value::from(set.contains("structured_outputs") || set.contains("response_format")),
            );
            fields.insert(
                "SupportsReasoning".to_string(),
                Value::from(set.contains("reasoning") || set.contains("include_reasoning")),
            );
        }

        rows.push(RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: model_id.to_string(),
            vendor_hint: vendor_hint.map(ToOwned::to_owned),
            fields,
            synthesized_from: None,
        });
    }
    Ok(rows)
}

fn number_like(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn copy_if_present(fields: &mut BTreeMap<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value {
        fields.insert(key.to_string(), value.clone());
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
    fn parse_rows_extracts_metric_keys() {
        let payload = json!({
            "data": [{
                "id": "openai/gpt-5.5",
                "canonical_slug": "openai/gpt-5.5",
                "name": "OpenAI: GPT-5.5",
                "context_length": 400000,
                "pricing": { "prompt": "0.000001", "completion": "0.000004" },
                "top_provider": { "max_completion_tokens": 32768 },
                "supported_parameters": ["tools", "response_format", "reasoning"]
            }]
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        let row = &rows[0];
        assert_eq!(row.model_name, "openai/gpt-5.5");
        assert_eq!(row.vendor_hint.as_deref(), Some("openai"));
        assert_eq!(
            row.fields.get("ContextWindow").and_then(number_like),
            Some(400000.0)
        );
        assert!(row.fields.contains_key("InverseCost"));
        assert_eq!(row.fields.get("SupportsTools"), Some(&Value::Bool(true)));
        assert_eq!(
            row.fields.get("SupportsStructuredOutputs"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            row.fields.get("SupportsReasoning"),
            Some(&Value::Bool(true))
        );
    }
}
