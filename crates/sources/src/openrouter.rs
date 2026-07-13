use std::collections::BTreeMap;
use std::time::Duration;

use ipbr_core::{AliasIndex, ModelRecord, RawRow, normalize_name};
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

    let alias_records = crate::embedded_alias_records();
    let alias_index = AliasIndex::build(&alias_records);
    let mut rows_by_model: BTreeMap<String, RawRow> = BTreeMap::new();
    for item in data {
        let public_id = item.get("id").and_then(Value::as_str);
        let canonical_slug = item.get("canonical_slug").and_then(Value::as_str);
        let display_name = item.get("name").and_then(Value::as_str);
        if public_id.is_none() && canonical_slug.is_none() && display_name.is_none() {
            return Err(SourceError::Parse("OpenRouter row missing id/name".into()));
        }
        let Some(identity) = select_openrouter_identity(
            public_id,
            canonical_slug,
            display_name,
            &alias_records,
            &alias_index,
        ) else {
            continue;
        };

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
                    fields.insert("BlendedCost".to_string(), Value::from(blended));
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

        let row = rows_by_model.entry(identity.key).or_insert_with(|| RawRow {
            source_id: SOURCE_ID.to_string(),
            model_name: identity.output_name,
            vendor_hint: identity.vendor_hint,
            fields: BTreeMap::new(),
        });
        for (key, value) in fields {
            merge_openrouter_field(&mut row.fields, key, value);
        }
    }
    Ok(rows_by_model.into_values().collect())
}

struct OpenRouterIdentity {
    key: String,
    output_name: String,
    vendor_hint: Option<String>,
}

fn select_openrouter_identity(
    public_id: Option<&str>,
    canonical_slug: Option<&str>,
    display_name: Option<&str>,
    alias_records: &[ModelRecord],
    alias_index: &AliasIndex<'_>,
) -> Option<OpenRouterIdentity> {
    let validated_slug =
        canonical_slug.filter(|slug| public_id.is_none_or(|id| provider_prefixes_match(id, slug)));
    let preferred = validated_slug.or(public_id).or(display_name)?;
    let vendor_hint = provider_prefix(preferred)
        .or_else(|| public_id.and_then(provider_prefix))
        .map(ToOwned::to_owned);

    if let Some(index) = alias_index.lookup_exact(preferred, vendor_hint.as_deref()) {
        let canonical = alias_records[index].canonical_id.clone();
        return Some(OpenRouterIdentity {
            key: format!("canonical:{canonical}"),
            output_name: canonical,
            vendor_hint,
        });
    }

    if let Some(index) = alias_index.match_record(preferred, vendor_hint.as_deref()) {
        let canonical = &alias_records[index].canonical_id;
        if explicit_versions_compatible(preferred, canonical) {
            return Some(OpenRouterIdentity {
                key: format!("canonical:{canonical}"),
                output_name: canonical.clone(),
                vendor_hint,
            });
        }

        // A rolling public ID can point at an older canonical version on
        // OpenRouter (for example deepseek/deepseek-chat -> ...-chat-v3).
        // Prefer a version-bearing display label when it does not resolve back
        // to the conflicting ranked model; this keeps the historical row
        // visible for triage without letting core fuzzy matching merge it.
        if let Some(display_name) = display_name
            && alias_index
                .match_record(display_name, vendor_hint.as_deref())
                .is_none_or(|display_index| display_index != index)
        {
            return Some(OpenRouterIdentity {
                key: format!("raw:{}", normalize_name(preferred)),
                output_name: display_name.to_string(),
                vendor_hint,
            });
        }

        // If no safe display identity exists, omit the conflicting operational
        // diagnostic rather than attach it to the wrong ranked product.
        return None;
    }

    // Versioned canonical slugs often add a release date that intentionally
    // fails the generic fuzzy matcher. In that case the public ID may still
    // identify the ranked family, but only accept it when the explicit model
    // generation agrees with the canonical slug.
    if validated_slug.is_some()
        && let Some(public_id) = public_id
        && let Some(index) = alias_index
            .lookup_exact(public_id, vendor_hint.as_deref())
            .or_else(|| alias_index.match_record(public_id, vendor_hint.as_deref()))
    {
        let canonical = &alias_records[index].canonical_id;
        if explicit_versions_compatible(preferred, canonical) {
            return Some(OpenRouterIdentity {
                key: format!("canonical:{canonical}"),
                output_name: canonical.clone(),
                vendor_hint,
            });
        }
        if let Some(display_name) = display_name
            && alias_index
                .match_record(display_name, vendor_hint.as_deref())
                .is_none_or(|display_index| display_index != index)
        {
            return Some(OpenRouterIdentity {
                key: format!("raw:{}", normalize_name(preferred)),
                output_name: display_name.to_string(),
                vendor_hint,
            });
        }
        return None;
    }

    Some(OpenRouterIdentity {
        key: format!("raw:{}", normalize_name(preferred)),
        output_name: preferred.to_string(),
        vendor_hint,
    })
}

fn provider_prefix(value: &str) -> Option<&str> {
    value
        .split_once('/')
        .map(|(provider, _)| provider)
        .filter(|provider| !provider.is_empty())
}

fn provider_prefixes_match(public_id: &str, canonical_slug: &str) -> bool {
    match (provider_prefix(public_id), provider_prefix(canonical_slug)) {
        (Some(public), Some(canonical)) => normalize_name(public) == normalize_name(canonical),
        _ => true,
    }
}

fn explicit_versions_compatible(source_identity: &str, canonical_id: &str) -> bool {
    match (
        explicit_v_major(source_identity),
        explicit_v_major(canonical_id),
    ) {
        (Some(source), Some(canonical)) => source == canonical,
        _ => true,
    }
}

fn explicit_v_major(value: &str) -> Option<u32> {
    normalize_name(value)
        .split_whitespace()
        .filter_map(|token| token.strip_prefix('v'))
        .filter_map(|version| {
            let major: String = version.chars().take_while(char::is_ascii_digit).collect();
            (!major.is_empty())
                .then(|| major.parse::<u32>().ok())
                .flatten()
        })
        .next()
}

fn merge_openrouter_field(fields: &mut BTreeMap<String, Value>, key: String, value: Value) {
    let Some(existing) = fields.get(&key) else {
        fields.insert(key, value);
        return;
    };
    let replace = match key.as_str() {
        "BlendedCost" | "PromptPricePerMillion" | "CompletionPricePerMillion" => {
            numeric_value(&value)
                .zip(numeric_value(existing))
                .is_some_and(|(new, old)| {
                    new.is_finite() && old.is_finite() && new > 0.0 && new < old
                })
        }
        "ContextWindow"
        | "MaxCompletionTokens"
        | "SupportedParametersCount"
        | "SupportsTools"
        | "SupportsStructuredOutputs"
        | "SupportsReasoning" => numeric_value(&value)
            .zip(numeric_value(existing))
            .is_some_and(|(new, old)| new.is_finite() && old.is_finite() && new > old),
        _ => false,
    };
    if replace {
        fields.insert(key, value);
    }
}

fn numeric_value(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        _ => number_like(value),
    }
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
        assert!(row.fields.contains_key("BlendedCost"));
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

    #[test]
    fn canonical_slug_keeps_deepseek_v3_separate_from_v4_flash() {
        let payload = json!({
            "data": [
                {
                    "id": "deepseek/deepseek-chat",
                    "canonical_slug": "deepseek/deepseek-chat-v3",
                    "name": "DeepSeek: DeepSeek V3",
                    "context_length": 2000000,
                    "pricing": { "prompt": "0.00000025", "completion": "0.0000011" }
                },
                {
                    "id": "deepseek/deepseek-v4-flash",
                    "canonical_slug": "deepseek/deepseek-v4-flash-20260423",
                    "name": "DeepSeek: DeepSeek V4 Flash",
                    "context_length": 1048576,
                    "pricing": { "prompt": "0.00000007", "completion": "0.00000035" }
                }
            ]
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 2, "V3 and V4 are distinct upstream models");
        let v4 = rows
            .iter()
            .find(|row| row.model_name == "deepseek/deepseek-v4-flash")
            .expect("canonical row should be present");
        assert_eq!(
            v4.fields.get("ContextWindow").and_then(number_like),
            Some(1048576.0)
        );
        let blended = v4
            .fields
            .get("BlendedCost")
            .and_then(number_like)
            .expect("blended cost should be present");
        assert!((blended - 0.14).abs() < 1e-9, "blended={blended}");

        let v3 = rows
            .iter()
            .find(|row| {
                row.fields.get("CanonicalSlug").and_then(Value::as_str)
                    == Some("deepseek/deepseek-chat-v3")
            })
            .expect("historical V3 row should remain visible");
        assert_eq!(v3.model_name, "DeepSeek: DeepSeek V3");

        let mut records = crate::embedded_alias_records();
        ipbr_core::ingest_rows(&mut records, rows);
        let ranked_v4 = records
            .iter()
            .find(|record| record.canonical_id == "deepseek/deepseek-v4-flash")
            .expect("ranked V4 record");
        assert_eq!(
            ranked_v4.raw_metrics.get("ContextWindow").copied(),
            Some(1048576.0),
            "the larger V3 context must not leak into V4"
        );
    }

    #[test]
    fn rejects_cross_provider_canonical_slug_for_identity() {
        let payload = json!({
            "data": [{
                "id": "openai/gpt-5.5",
                "canonical_slug": "anthropic/claude-opus-4.8",
                "name": "OpenAI: GPT-5.5",
                "context_length": 400000
            }]
        });

        let rows = parse_rows(&payload).expect("payload should parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_name, "openai/gpt-5.5");
        assert_eq!(rows[0].vendor_hint.as_deref(), Some("openai"));
    }
}
