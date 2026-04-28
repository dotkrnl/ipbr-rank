//! Triage subcommand — surfaces unmatched rows and synthesis gaps.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::Context;
use ipbr_core::{
    AliasIndex, ModelRecord, RawRow, SynthesisConfig, load_embedded_pairs, normalize_name,
    normalize_vendor_hint, synthesize_rows,
};
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
    gaps: BTreeMap<String, GapEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct Summary {
    total_unmatched_groups: usize,
    total_synthesis_gaps: usize,
    total_cap_blocked: usize,
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

#[derive(Debug, Clone, Serialize)]
struct GapEntry {
    display_name: String,
    missing_at: Vec<MissingAt>,
}

#[derive(Debug, Clone, Serialize)]
struct MissingAt {
    source: String,
    cap_blocked: bool,
    candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Serialize)]
struct Candidate {
    canonical_id: String,
    overlap: f64,
    present_at_source: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn cmd_triage(
    http: &dyn Http,
    cache_dir: &Path,
    out_dir: &Path,
    sources: &[Box<dyn Source>],
    records: Vec<ModelRecord>,
    synthesis_cfg: &SynthesisConfig,
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
    let synthesis_pairs =
        load_embedded_pairs().context("failed parsing embedded synthesis aliases")?;

    let mut unmatched_groups: BTreeMap<UnmatchedKey, UnmatchedGroup> = BTreeMap::new();

    for (source_id, rows) in &all_rows {
        for row in rows {
            if index
                .match_record(&row.model_name, row.vendor_hint.as_deref())
                .is_some()
            {
                continue;
            }
            let vendor_norm = normalize_vendor_hint(row.vendor_hint.as_deref().unwrap_or(""));
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

    let mut rows_for_synthesis = all_rows.clone();
    let _stats = synthesize_rows(
        &mut rows_for_synthesis,
        &synthesis_pairs,
        &records,
        synthesis_cfg,
    );

    let mut matched_by_source: BTreeMap<String, Vec<(RawRow, usize)>> = BTreeMap::new();
    let mut matched_counts: BTreeMap<String, usize> = BTreeMap::new();

    for (source_id, rows) in &rows_for_synthesis {
        for row in rows {
            if let Some(record_idx) =
                index.match_record(&row.model_name, row.vendor_hint.as_deref())
            {
                matched_by_source
                    .entry(source_id.clone())
                    .or_default()
                    .push((row.clone(), record_idx));
                *matched_counts.entry(source_id.clone()).or_default() += 1;
            }
        }
    }

    let uncapped_cfg = SynthesisConfig {
        per_source_cap: 1.0,
        per_model_cap: synthesis_cfg.per_model_cap,
    };
    let mut rows_uncapped = all_rows.clone();
    let _uncapped_stats = synthesize_rows(
        &mut rows_uncapped,
        &synthesis_pairs,
        &records,
        &uncapped_cfg,
    );

    let post_synth_coverage = compute_coverage(&rows_for_synthesis, &records);
    let uncapped_coverage = compute_coverage(&rows_uncapped, &records);

    let mut gaps: BTreeMap<String, GapEntry> = BTreeMap::new();
    for record in &records {
        let canonical_id = &record.canonical_id;
        let mut missing_at_list: Vec<MissingAt> = Vec::new();

        for source_id in all_rows.keys() {
            let has_coverage = post_synth_coverage
                .get(canonical_id)
                .is_some_and(|sources| sources.contains(source_id));
            if has_coverage {
                continue;
            }

            let would_have_uncapped = uncapped_coverage
                .get(canonical_id)
                .is_some_and(|sources| sources.contains(source_id));
            let cap_blocked = would_have_uncapped && !has_coverage;

            let candidates = find_sibling_candidates(
                canonical_id,
                &record.vendor,
                source_id,
                &matched_by_source,
                &records,
            );

            missing_at_list.push(MissingAt {
                source: source_id.clone(),
                cap_blocked,
                candidates,
            });
        }

        missing_at_list.sort_by(|a, b| a.source.cmp(&b.source));

        if !missing_at_list.is_empty() {
            gaps.insert(
                canonical_id.clone(),
                GapEntry {
                    display_name: record.display_name.clone(),
                    missing_at: missing_at_list,
                },
            );
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
    let total_synthesis_gaps = gaps.len();
    let total_cap_blocked: usize = gaps
        .values()
        .flat_map(|g| &g.missing_at)
        .filter(|m| m.cap_blocked)
        .count();

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
            total_synthesis_gaps,
            total_cap_blocked,
        },
        sources: source_reports,
        gaps,
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

fn compute_coverage(
    rows_by_source: &BTreeMap<String, Vec<RawRow>>,
    records: &[ModelRecord],
) -> BTreeMap<String, BTreeSet<String>> {
    let index = AliasIndex::build(records);
    let mut coverage: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for (source_id, rows) in rows_by_source {
        for row in rows {
            if let Some(idx) = index.match_record(&row.model_name, row.vendor_hint.as_deref()) {
                let canonical_id = &records[idx].canonical_id;
                coverage
                    .entry(canonical_id.clone())
                    .or_default()
                    .insert(source_id.clone());
            }
        }
    }

    coverage
}

fn find_sibling_candidates(
    target_canonical_id: &str,
    target_vendor: &ipbr_core::Vendor,
    source_id: &str,
    matched_by_source: &BTreeMap<String, Vec<(RawRow, usize)>>,
    records: &[ModelRecord],
) -> Vec<Candidate> {
    let Some(source_matched) = matched_by_source.get(source_id) else {
        return Vec::new();
    };

    let target_vendor_str = target_vendor.as_str().to_lowercase();
    let target_vendor_prefix = target_canonical_id
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let target_name_norm = normalize_name(
        target_canonical_id
            .split('/')
            .nth(1)
            .unwrap_or(target_canonical_id),
    );

    let mut candidates: Vec<(f64, String, bool)> = Vec::new();

    for (row, record_idx) in source_matched.iter() {
        let sibling_record = &records[*record_idx];
        let sibling_canonical_id = &sibling_record.canonical_id;

        if sibling_canonical_id == target_canonical_id {
            continue;
        }

        let sibling_vendor_str = sibling_record.vendor.as_str().to_lowercase();
        let sibling_vendor_prefix = sibling_canonical_id
            .split('/')
            .next()
            .unwrap_or("")
            .to_lowercase();

        let row_vendor_hint = row
            .vendor_hint
            .as_deref()
            .map(|h| h.to_lowercase())
            .unwrap_or_default();

        let vendor_match = if !row_vendor_hint.is_empty() {
            let row_vendor_norm = normalize_vendor_hint(&row_vendor_hint);
            row_vendor_norm == normalize_vendor_hint(&target_vendor_str)
        } else {
            sibling_vendor_str == target_vendor_str || sibling_vendor_prefix == target_vendor_prefix
        };

        if !vendor_match {
            continue;
        }

        let sibling_name_norm = normalize_name(
            sibling_canonical_id
                .split('/')
                .nth(1)
                .unwrap_or(sibling_canonical_id),
        );

        let overlap = token_overlap(&target_name_norm, &sibling_name_norm);
        if overlap >= 0.6 {
            candidates.push((overlap, sibling_canonical_id.clone(), true));
        }
    }

    candidates.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
    });

    candidates
        .into_iter()
        .take(3)
        .map(|(overlap, canonical_id, present)| Candidate {
            canonical_id,
            overlap: (overlap * 100.0).round() / 100.0,
            present_at_source: present,
        })
        .collect()
}

fn token_overlap(a: &str, b: &str) -> f64 {
    let tokens_a: BTreeSet<&str> = a.split_whitespace().collect();
    let tokens_b: BTreeSet<&str> = b.split_whitespace().collect();

    if tokens_a.is_empty() && tokens_b.is_empty() {
        return 1.0;
    }
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }

    let intersection = tokens_a.intersection(&tokens_b).count();
    let union = tokens_a.union(&tokens_b).count();

    intersection as f64 / union as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use ipbr_sources::{SweRebenchSource, cache_html_path, cache_json_path};

    #[test]
    fn token_overlap_identical() {
        assert!((token_overlap("gpt 5.5", "gpt 5.5") - 1.0).abs() < 0.001);
    }

    #[test]
    fn token_overlap_partial() {
        let overlap = token_overlap("gpt 5.5 turbo", "gpt 5.5");
        assert!((0.6..1.0).contains(&overlap));
    }

    #[test]
    fn token_overlap_disjoint() {
        assert!(token_overlap("abc", "xyz") < 0.1);
    }

    #[test]
    fn normalize_vendor_hint_lowercases_and_normalizes() {
        assert_eq!(normalize_vendor_hint("OpenAI"), "openai");
        assert_eq!(normalize_vendor_hint("Moonshot AI"), "moonshot");
    }

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
