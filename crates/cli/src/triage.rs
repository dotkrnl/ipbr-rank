//! Triage subcommand — surfaces unmatched leaderboard rows for alias curation.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Context;
use ipbr_core::{AliasIndex, ModelRecord, RawRow, normalize_name};
use ipbr_sources::{FetchOptions, Http, SecretStore, Source, SourceError};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct UnmatchedKey {
    source_id: String,
    vendor_norm: String,
    name_norm: String,
}

#[derive(Debug, Clone)]
struct UnmatchedGroup {
    normalized: String,
    example_name: String,
    vendor_hint: String,
    count: usize,
    sample_fields: Vec<SampleField>,
}

#[derive(Debug, Clone, Serialize)]
struct SampleField {
    key: String,
    value: String,
}

#[derive(Debug, Clone, Serialize)]
struct TriageReport {
    generated_at: String,
    generator: String,
    provenance: BTreeMap<String, String>,
    summary: Summary,
    sources: BTreeMap<String, SourceReport>,
}

#[derive(Debug, Clone, Serialize)]
struct Summary {
    total_unmatched_groups: usize,
}

#[derive(Debug, Clone, Serialize)]
struct SourceReport {
    ingested: usize,
    matched: usize,
    unmatched_groups: usize,
    unmatched: Vec<UnmatchedEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct UnmatchedEntry {
    normalized: String,
    example_name: String,
    vendor_hint: String,
    count: usize,
    sample_fields: Vec<SampleField>,
}

pub async fn cmd_triage(
    http: &dyn Http,
    cache_dir: &Path,
    out_dir: &Path,
    sources: &[Box<dyn Source>],
    records: Vec<ModelRecord>,
    min_count: usize,
    secrets: &SecretStore,
) -> anyhow::Result<()> {
    let fetch_opts = FetchOptions {
        cache_dir: Some(cache_dir),
        offline: true,
    };

    let mut provenance: BTreeMap<String, String> = BTreeMap::new();
    let mut provenance_times: BTreeMap<String, OffsetDateTime> = BTreeMap::new();
    let mut all_rows: BTreeMap<String, Vec<RawRow>> = BTreeMap::new();
    let mut ingested_counts: BTreeMap<String, usize> = BTreeMap::new();

    for source in sources {
        match source.as_ref().fetch(http, fetch_opts, secrets).await {
            Ok(rows) => {
                let mtime = get_cache_mtime(cache_dir, source.as_ref())?;
                let formatted = mtime
                    .format(&Rfc3339)
                    .context("failed formatting cache mtime")?;
                provenance.insert(source.id().to_string(), formatted);
                provenance_times.insert(source.id().to_string(), mtime);
                ingested_counts.insert(source.id().to_string(), rows.len());
                all_rows.insert(source.id().to_string(), rows);
            }
            Err(SourceError::CacheMiss(msg)) => {
                eprintln!("triage: cache miss for {} — {}", source.id(), msg);
            }
            Err(e) => {
                return Err(e).context(format!("triage fetch failed for {}", source.id()));
            }
        }
    }

    let index = AliasIndex::build(&records);

    let mut unmatched_groups: BTreeMap<UnmatchedKey, UnmatchedGroup> = BTreeMap::new();
    let mut matched_counts: BTreeMap<String, usize> = BTreeMap::new();

    for (source_id, rows) in &all_rows {
        for row in rows {
            if index
                .match_record(&row.model_name, row.vendor_hint.as_deref())
                .is_some()
            {
                *matched_counts.entry(source_id.clone()).or_default() += 1;
                continue;
            }
            let vendor_norm = normalize_name(row.vendor_hint.as_deref().unwrap_or(""));
            let name_norm = normalize_name(&row.model_name);
            let key = UnmatchedKey {
                source_id: source_id.clone(),
                vendor_norm: vendor_norm.clone(),
                name_norm: name_norm.clone(),
            };
            let group = unmatched_groups.entry(key).or_insert_with(|| {
                let sample_fields = extract_sample_fields(&row.fields);
                UnmatchedGroup {
                    normalized: name_norm.clone(),
                    example_name: row.model_name.clone(),
                    vendor_hint: row.vendor_hint.clone().unwrap_or_default(),
                    count: 0,
                    sample_fields,
                }
            });
            group.count += 1;
        }
    }

    let mut source_reports: BTreeMap<String, SourceReport> = BTreeMap::new();
    for source_id in all_rows.keys() {
        let ingested = ingested_counts.get(source_id).copied().unwrap_or(0);
        let matched = matched_counts.get(source_id).copied().unwrap_or(0);

        let mut source_unmatched: Vec<_> = unmatched_groups
            .iter()
            .filter(|(k, _)| &k.source_id == source_id)
            .filter(|(_, g)| g.count >= min_count)
            .map(|(_, g)| UnmatchedEntry {
                normalized: g.normalized.clone(),
                example_name: g.example_name.clone(),
                vendor_hint: g.vendor_hint.clone(),
                count: g.count,
                sample_fields: g.sample_fields.clone(),
            })
            .collect();

        source_unmatched.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.normalized.cmp(&b.normalized))
        });

        source_reports.insert(
            source_id.clone(),
            SourceReport {
                ingested,
                matched,
                unmatched_groups: source_unmatched.len(),
                unmatched: source_unmatched,
            },
        );
    }

    let total_unmatched_groups: usize = source_reports.values().map(|s| s.unmatched_groups).sum();

    let generated_at = match provenance_times.values().max() {
        Some(latest) => latest
            .format(&Rfc3339)
            .context("failed formatting generated_at")?,
        None => "1970-01-01T00:00:00Z".to_string(),
    };

    let report = TriageReport {
        generated_at,
        generator: format!("ipbr-rank {} triage", env!("CARGO_PKG_VERSION")),
        provenance,
        summary: Summary {
            total_unmatched_groups,
        },
        sources: source_reports,
    };

    fs::create_dir_all(out_dir)?;
    let output_path = out_dir.join("triage.toml");
    let toml_str = toml::to_string_pretty(&report).context("failed serializing triage report")?;
    fs::write(&output_path, toml_str)?;

    Ok(())
}

fn get_cache_mtime(cache_dir: &Path, source: &dyn Source) -> anyhow::Result<OffsetDateTime> {
    let candidates = source.cache_paths(cache_dir);
    let path = candidates
        .iter()
        .find(|p| p.exists())
        .ok_or_else(|| anyhow::anyhow!("no cache file found for {}", source.id()))?;

    let meta = fs::metadata(path)?;
    let mtime = meta.modified()?;
    Ok(OffsetDateTime::from(mtime))
}

fn extract_sample_fields(fields: &BTreeMap<String, serde_json::Value>) -> Vec<SampleField> {
    let mut sorted: Vec<_> = fields.iter().collect();
    sorted.sort_by_key(|(k, _)| *k);

    sorted
        .into_iter()
        .take(3)
        .map(|(key, value)| SampleField {
            key: key.clone(),
            value: format!("{}", value),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use ipbr_sources::{SweRebenchSource, cache_html_path, cache_json_path};

    #[test]
    fn cache_mtime_uses_consumed_payload_extension() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        fs::write(cache_json_path(tmp.path(), "swerebench"), "{}")
            .expect("json sibling should be written");
        std::thread::sleep(Duration::from_millis(25));
        fs::write(
            cache_html_path(tmp.path(), "swerebench"),
            "<html><body>fixture</body></html>",
        )
        .expect("html cache should be written");

        let expected = OffsetDateTime::from(
            fs::metadata(cache_html_path(tmp.path(), "swerebench"))
                .expect("html metadata should exist")
                .modified()
                .expect("html mtime should exist"),
        );
        let actual = get_cache_mtime(tmp.path(), &SweRebenchSource).expect("mtime should resolve");

        assert_eq!(actual, expected);
    }
}
