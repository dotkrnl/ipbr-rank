use crate::model::{ModelRecord, Vendor};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct AliasEntry {
    vendor: String,
    aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AliasFile {
    models: BTreeMap<String, AliasEntry>,
}

const EMBEDDED: &str = include_str!("../../../data/required_aliases.toml");

pub fn load_embedded() -> Result<Vec<ModelRecord>, toml::de::Error> {
    load_from_str(EMBEDDED)
}

pub fn load_from_str(s: &str) -> Result<Vec<ModelRecord>, toml::de::Error> {
    let file: AliasFile = toml::from_str(s)?;
    let mut records: Vec<ModelRecord> = file
        .models
        .into_iter()
        .map(|(canonical_id, entry)| {
            let vendor = parse_vendor(&entry.vendor);
            let display_name = derive_display_name(&canonical_id);
            let mut r = ModelRecord::new(canonical_id, display_name, vendor);
            r.aliases.extend(entry.aliases);
            r
        })
        .collect();
    records.sort_by(|a, b| a.canonical_id.cmp(&b.canonical_id));
    Ok(records)
}

fn parse_vendor(s: &str) -> Vendor {
    match s.to_lowercase().as_str() {
        "openai" => Vendor::Openai,
        "anthropic" => Vendor::Anthropic,
        "google" => Vendor::Google,
        "moonshot" | "moonshotai" => Vendor::Moonshot,
        "zai" | "z-ai" | "z.ai" => Vendor::Zai,
        "xai" => Vendor::Xai,
        "alibaba" => Vendor::Alibaba,
        "deepseek" => Vendor::Deepseek,
        "mistral" => Vendor::Mistral,
        "meta" => Vendor::Meta,
        "minimax" => Vendor::Minimax,
        "nvidia" => Vendor::Nvidia,
        "baidu" => Vendor::Baidu,
        "tencent" => Vendor::Tencent,
        "inclusionai" => Vendor::Inclusionai,
        "xiaomi" => Vendor::Xiaomi,
        other => Vendor::Other(other.to_string()),
    }
}

fn derive_display_name(canonical_id: &str) -> String {
    canonical_id
        .split('/')
        .next_back()
        .unwrap_or(canonical_id)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_aliases_load() {
        let records = load_embedded().expect("required_aliases.toml must parse");
        assert!(records.len() >= 13);
        assert!(
            records
                .iter()
                .any(|r| r.canonical_id == "anthropic/claude-opus-4.7")
        );
    }

    #[test]
    fn vendor_parsing_is_case_insensitive() {
        assert!(matches!(parse_vendor("OpenAI"), Vendor::Openai));
        assert!(matches!(parse_vendor("zai"), Vendor::Zai));
        assert!(matches!(parse_vendor("z-ai"), Vendor::Zai));
    }
}
