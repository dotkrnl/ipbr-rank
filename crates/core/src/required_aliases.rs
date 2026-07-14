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
            let vendor = Vendor::from_label(&entry.vendor);
            let display_name = derive_display_name(&canonical_id);
            let mut r = ModelRecord::new(canonical_id, display_name, vendor);
            r.aliases.extend(entry.aliases);
            r
        })
        .collect();
    records.sort_by(|a, b| a.canonical_id.cmp(&b.canonical_id));
    Ok(records)
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
    use crate::alias::{compact_key, normalize_name};

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
        assert!(matches!(Vendor::from_label("OpenAI"), Vendor::Openai));
        assert!(matches!(Vendor::from_label("zai"), Vendor::Zai));
        assert!(matches!(Vendor::from_label("z-ai"), Vendor::Zai));
    }

    #[test]
    fn embedded_alias_keys_do_not_collide_across_models() {
        let records = load_embedded().expect("required_aliases.toml must parse");
        let mut normalized = BTreeMap::<String, String>::new();
        let mut compact = BTreeMap::<String, String>::new();

        for record in &records {
            let keys = std::iter::once(record.canonical_id.as_str())
                .chain(std::iter::once(record.display_name.as_str()))
                .chain(record.aliases.iter().map(String::as_str));
            for key in keys {
                for (kind, candidate, seen) in [
                    ("normalized", normalize_name(key), &mut normalized),
                    ("compact", compact_key(key), &mut compact),
                ] {
                    if candidate.is_empty() {
                        continue;
                    }
                    if let Some(existing) =
                        seen.insert(candidate.clone(), record.canonical_id.clone())
                    {
                        assert_eq!(
                            existing, record.canonical_id,
                            "{kind} alias key {candidate:?} is shared by {existing} and {}",
                            record.canonical_id,
                        );
                    }
                }
            }
        }
    }
}
