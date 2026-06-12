use crate::alias::AliasIndex;
use crate::coefficients::SynthesisConfig;
use crate::model::{ModelRecord, RawRow, SourceId, SynthesisCategory};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SynthesisStats {
    pub per_source: BTreeMap<SourceId, usize>,
    pub capped_sources: Vec<SourceId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SynthesisPair {
    pub target: String,
    pub from: String,
    #[serde(default)]
    pub category: SynthesisCategory,
    #[serde(default)]
    pub sources: Vec<String>,
}

impl SynthesisPair {
    pub fn conservative(target: impl Into<String>, from: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            from: from.into(),
            category: SynthesisCategory::Conservative,
            sources: Vec::new(),
        }
    }

    pub fn same_series_forward(target: impl Into<String>, from: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            from: from.into(),
            category: SynthesisCategory::SameSeriesForward,
            sources: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PairFile {
    pair: Vec<SynthesisPair>,
}

const EMBEDDED_PAIRS: &str = include_str!("../../../data/synthesis_aliases.toml");

pub fn load_embedded_pairs() -> Result<Vec<SynthesisPair>, toml::de::Error> {
    load_pairs_from_str(EMBEDDED_PAIRS)
}

pub fn load_pairs_from_str(raw: &str) -> Result<Vec<SynthesisPair>, toml::de::Error> {
    let file: PairFile = toml::from_str(raw)?;
    Ok(file.pair)
}

/// After ingestion, identify synthesis pairs that contributed zero fields
/// to their target. A pair is "stale" when the target now has direct
/// upstream coverage for every field the donor would have supplied — the
/// classic case is a freshly released model that finally lands on the
/// rolling-window benchmark we'd been borrowing a sibling's row for.
///
/// Warnings go to stderr (no logging dependency in `ipbr-core`); the
/// returned vec lets tests assert on which pairs were flagged.
pub fn warn_stale_synthesis_pairs(
    records: &[ModelRecord],
    pairs: &[SynthesisPair],
) -> Vec<(String, String)> {
    let mut stale = Vec::new();
    for pair in pairs {
        let Some(record) = records.iter().find(|r| r.canonical_id == pair.target) else {
            // Target isn't registered at all — separate hygiene issue,
            // surfaced elsewhere via required_aliases validation.
            continue;
        };
        let useful = record
            .synthesized
            .values()
            .any(|prov| prov.from == pair.from);
        if !useful {
            let target = &pair.target;
            let from = &pair.from;
            eprintln!(
                "warning: synthesis pair {target} <- {from} contributed no fields after ingestion; consider removing it from data/synthesis_aliases.toml"
            );
            stale.push((pair.target.clone(), pair.from.clone()));
        }
    }
    stale
}

pub fn synthesize_rows(
    rows_by_source: &mut BTreeMap<SourceId, Vec<RawRow>>,
    pairs: &[SynthesisPair],
    records: &[ModelRecord],
    cfg: &SynthesisConfig,
) -> SynthesisStats {
    let index = AliasIndex::build(records);
    let resolve_canonical = |row: &RawRow| -> Option<&str> {
        index
            .match_record(&row.model_name, row.vendor_hint.as_deref())
            .map(|idx| records[idx].canonical_id.as_str())
    };
    let display_name_for = |canonical_id: &str| -> Option<&str> {
        records
            .iter()
            .find(|record| record.canonical_id == canonical_id)
            .map(|record| record.display_name.as_str())
    };

    let mut stats = SynthesisStats::default();

    for (source_id, rows) in rows_by_source.iter_mut() {
        let real_count = rows.len();
        let mut row_indices_by_canonical: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (idx, row) in rows.iter().enumerate() {
            if let Some(canonical_id) = resolve_canonical(row) {
                row_indices_by_canonical
                    .entry(canonical_id.to_string())
                    .or_default()
                    .push(idx);
            }
        }
        // Cap counts unique synthesis pairs that emitted (matching the
        // original semantic: "how many target models use this source via
        // synthesis"). Stats track total cloned rows for downstream debug.
        let mut synth_pair_count = 0usize;
        let mut synth_row_count = 0usize;

        for pair in pairs {
            if !pair.sources.is_empty() && !pair.sources.iter().any(|source| source == source_id) {
                continue;
            }
            let target_id = &pair.target;
            let from_id = &pair.from;
            if real_count > 0
                && synth_pair_count > 0
                && (synth_pair_count as f64 / (real_count + synth_pair_count) as f64)
                    > cfg.per_source_cap
            {
                // REVIEWER: the spec calls for a warning here, but `ipbr-core` deliberately avoids
                // adding a logging dependency; `eprintln!` keeps the runtime signal without widening
                // the crate surface just for this one cap notification.
                eprintln!(
                    "warning: synthesis per-source cap reached for {source_id}; remaining pairs skipped"
                );
                stats.capped_sources.push(source_id.clone());
                break;
            }

            // Emit only when the donor row has at least one field that the
            // target does not already have in this source. The ingest layer
            // still applies the authoritative per-field filtering, but this
            // pre-filter keeps sparse-source caps from being spent on rows
            // that can only become no-ops.
            //
            // Some sources emit multiple rows per model (e.g. swebench has a
            // separate row for each leaderboard, and the overrides source
            // emits one row per metric). Clone *every* matching donor row so
            // each one gets a chance to fill a different field on the target.
            let donor_indices = row_indices_by_canonical
                .get(from_id)
                .cloned()
                .unwrap_or_default();

            if donor_indices.is_empty() {
                continue;
            }

            let Some(display_name) = display_name_for(target_id) else {
                continue;
            };

            let mut target_fields =
                current_fields_for_target(rows, &row_indices_by_canonical, target_id);
            let mut emitted_for_pair = false;
            for donor_idx in donor_indices {
                let donor = rows[donor_idx].clone();
                if !has_fillable_field(&donor, &target_fields) {
                    continue;
                }
                let donor_fields: Vec<String> = donor.fields.keys().cloned().collect();
                let donor_model_name = donor.model_name.clone();
                let category = donor
                    .synthesis_category
                    .unwrap_or(SynthesisCategory::SameSeriesForward)
                    .chain(pair.category);
                let mut synthesized = donor;
                synthesized.fields.insert(
                    "SynthesizedFromModelName".to_string(),
                    Value::from(donor_model_name),
                );
                synthesized.model_name = display_name.to_string();
                synthesized.synthesized_from = Some(from_id.clone());
                synthesized.synthesis_category = Some(category);
                let synthesized_idx = rows.len();
                rows.push(synthesized);
                row_indices_by_canonical
                    .entry(target_id.clone())
                    .or_default()
                    .push(synthesized_idx);
                for field in donor_fields {
                    target_fields.insert(field);
                }
                synth_row_count += 1;
                emitted_for_pair = true;
            }
            if emitted_for_pair {
                synth_pair_count += 1;
            }
        }

        stats.per_source.insert(source_id.clone(), synth_row_count);
    }

    stats
}

fn current_fields_for_target(
    rows: &[RawRow],
    row_indices_by_canonical: &BTreeMap<String, Vec<usize>>,
    target_id: &str,
) -> BTreeSet<String> {
    row_indices_by_canonical
        .get(target_id)
        .into_iter()
        .flatten()
        .flat_map(|idx| rows[*idx].fields.keys().cloned())
        .collect()
}

fn has_fillable_field(donor: &RawRow, target_fields: &BTreeSet<String>) -> bool {
    donor
        .fields
        .keys()
        .any(|field| field != "SynthesizedFromModelName" && !target_fields.contains(field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{ingest_rows, mark_synthesis_dominant};
    use crate::model::{SynthesisProvenance, Vendor};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn record(canonical_id: &str, display_name: &str, aliases: &[&str]) -> ModelRecord {
        let mut record = ModelRecord::new(
            canonical_id.to_string(),
            display_name.to_string(),
            Vendor::Other("test".to_string()),
        );
        record.aliases = aliases.iter().map(|alias| (*alias).to_string()).collect();
        record
    }

    fn raw(
        source_id: &str,
        model_name: &str,
        synthesized_from: Option<&str>,
        fields: &[(&str, serde_json::Value)],
    ) -> RawRow {
        let mut map = BTreeMap::new();
        for (key, value) in fields {
            map.insert((*key).to_string(), value.clone());
        }
        RawRow {
            source_id: source_id.to_string(),
            model_name: model_name.to_string(),
            vendor_hint: None,
            fields: map,
            synthesized_from: synthesized_from.map(str::to_string),
            synthesis_category: None,
        }
    }

    fn rows_by_source(rows: Vec<RawRow>) -> BTreeMap<SourceId, Vec<RawRow>> {
        let mut out = BTreeMap::new();
        for row in rows {
            out.entry(row.source_id.clone())
                .or_insert_with(Vec::new)
                .push(row);
        }
        out
    }

    fn cfg(per_source_cap: f64, per_model_cap: f64) -> SynthesisConfig {
        SynthesisConfig {
            per_source_cap,
            per_model_cap,
        }
    }

    #[test]
    fn synthesize_emits_donor_row_even_when_target_has_partial_coverage() {
        // Synthesis runs at the row level, but we only need to emit when
        // the donor has at least one field the target lacks. The ingest layer
        // still performs authoritative per-field arbitration later.
        let records = vec![
            record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]),
            record("openai/gpt-5.4", "gpt-5.4", &["gpt-5.4"]),
        ];
        let mut rows = rows_by_source(vec![
            raw("lmarena", "gpt-5.5", None, &[("score", json!(91.0))]),
            raw(
                "lmarena",
                "gpt-5.4",
                None,
                &[("score", json!(88.0)), ("tail", json!(77.0))],
            ),
        ]);

        let stats = synthesize_rows(
            &mut rows,
            &[SynthesisPair::same_series_forward(
                "openai/gpt-5.5",
                "openai/gpt-5.4",
            )],
            &records,
            &cfg(0.50, 0.50),
        );

        assert_eq!(stats.per_source.get("lmarena"), Some(&1));
        assert_eq!(rows["lmarena"].len(), 3);
        let synth: Vec<_> = rows["lmarena"]
            .iter()
            .filter(|row| row.synthesized_from.is_some())
            .collect();
        assert_eq!(synth.len(), 1);
        assert_eq!(synth[0].synthesized_from.as_deref(), Some("openai/gpt-5.4"));
        assert_eq!(
            synth[0].synthesis_category,
            Some(SynthesisCategory::SameSeriesForward)
        );
        assert_eq!(synth[0].model_name, "gpt-5.5");
    }

    #[test]
    fn synthesize_skips_noop_when_target_already_has_donor_fields() {
        let records = vec![
            record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]),
            record("openai/gpt-5.4", "gpt-5.4", &["gpt-5.4"]),
        ];
        let mut rows = rows_by_source(vec![
            raw("lmarena", "gpt-5.5", None, &[("score", json!(91.0))]),
            raw("lmarena", "gpt-5.4", None, &[("score", json!(88.0))]),
        ]);

        let stats = synthesize_rows(
            &mut rows,
            &[SynthesisPair::same_series_forward(
                "openai/gpt-5.5",
                "openai/gpt-5.4",
            )],
            &records,
            &cfg(0.50, 0.50),
        );

        assert_eq!(stats.per_source.get("lmarena"), Some(&0));
        assert_eq!(rows["lmarena"].len(), 2);
        assert!(
            rows["lmarena"]
                .iter()
                .all(|row| row.synthesized_from.is_none())
        );
    }

    #[test]
    fn synthesize_respects_source_scoped_pairs() {
        let records = vec![
            record("openai/gpt-5.5-pro", "gpt-5.5-pro", &["gpt-5.5-pro"]),
            record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]),
        ];
        let mut rows = rows_by_source(vec![
            raw(
                "terminal_bench_2_1",
                "gpt-5.5",
                None,
                &[("TerminalBench21", json!(52.4))],
            ),
            raw(
                "swerebench",
                "gpt-5.5",
                None,
                &[("SWERebench", json!(71.0))],
            ),
        ]);

        synthesize_rows(
            &mut rows,
            &[SynthesisPair {
                target: "openai/gpt-5.5-pro".to_string(),
                from: "openai/gpt-5.5".to_string(),
                category: SynthesisCategory::Conservative,
                sources: vec!["terminal_bench_2_1".to_string()],
            }],
            &records,
            &cfg(0.80, 0.80),
        );

        assert_eq!(
            rows["terminal_bench_2_1"]
                .iter()
                .filter(|row| row.synthesized_from.is_some())
                .count(),
            1
        );
        assert!(
            rows["swerebench"]
                .iter()
                .all(|row| row.synthesized_from.is_none())
        );
    }

    #[test]
    fn synthesize_clones_every_donor_row_for_multi_row_sources() {
        // swebench emits one row per (model, leaderboard); the overrides
        // source emits one row per metric. Synthesis must clone *every*
        // donor row so each one can fill a different field on the target,
        // not just the first match.
        let records = vec![
            record(
                "anthropic/claude-sonnet-4",
                "claude-sonnet-4",
                &["claude-sonnet-4"],
            ),
            record(
                "anthropic/claude-sonnet-4.5",
                "claude-sonnet-4.5",
                &["claude-sonnet-4.5"],
            ),
        ];
        let mut rows = rows_by_source(vec![
            raw(
                "swebench",
                "claude-sonnet-4.5",
                None,
                &[("SWEBenchVerified", json!(72.0))],
            ),
            raw(
                "swebench",
                "claude-sonnet-4.5",
                None,
                &[("SWEBenchMultilingual", json!(67.0))],
            ),
        ]);

        synthesize_rows(
            &mut rows,
            &[SynthesisPair::conservative(
                "anthropic/claude-sonnet-4",
                "anthropic/claude-sonnet-4.5",
            )],
            &records,
            &cfg(0.80, 0.80),
        );

        let synth: Vec<_> = rows["swebench"]
            .iter()
            .filter(|row| row.synthesized_from.is_some())
            .collect();
        assert_eq!(synth.len(), 2, "both donor rows should be cloned");
        let metrics: std::collections::BTreeSet<_> = synth
            .iter()
            .flat_map(|r| r.fields.keys().filter(|k| k.starts_with("SWEBench")))
            .map(String::as_str)
            .collect();
        assert!(metrics.contains("SWEBenchVerified"));
        assert!(metrics.contains("SWEBenchMultilingual"));
        assert!(
            synth
                .iter()
                .all(|row| row.synthesis_category == Some(SynthesisCategory::Conservative))
        );
    }

    #[test]
    fn synthesize_chains_keep_conservative_category_sticky() {
        let records = vec![
            record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]),
            record("openai/gpt-5.4", "gpt-5.4", &["gpt-5.4"]),
            record("openai/gpt-5.3-codex", "gpt-5.3-codex", &["gpt-5.3-codex"]),
        ];
        let mut rows = rows_by_source(vec![raw(
            "swerebench",
            "gpt-5.3-codex",
            None,
            &[("SWERebench", json!(72.0))],
        )]);

        synthesize_rows(
            &mut rows,
            &[
                SynthesisPair::conservative("openai/gpt-5.4", "openai/gpt-5.3-codex"),
                SynthesisPair::same_series_forward("openai/gpt-5.5", "openai/gpt-5.4"),
            ],
            &records,
            &cfg(0.90, 0.90),
        );

        let synth_55 = rows["swerebench"]
            .iter()
            .find(|row| row.model_name == "gpt-5.5")
            .expect("chained synthesized GPT-5.5 row should exist");
        assert_eq!(
            synth_55.synthesis_category,
            Some(SynthesisCategory::Conservative),
            "a conservative donor row should remain conservative through a same-series forward hop"
        );
    }

    #[test]
    fn warn_stale_synthesis_pairs_flags_pair_with_zero_contributions() {
        // Set up two pairs:
        //   useful_pair: target has one synthesized field from the donor
        //   stale_pair:  target has zero synthesized fields with provenance.from
        //                matching the declared donor (target's data came from
        //                a real source, not from this pair)
        let mut useful_target = ModelRecord::new(
            "openai/gpt-5.5".into(),
            "gpt-5.5".into(),
            Vendor::Other("test".into()),
        );
        useful_target.synthesized.insert(
            "SWERebench".to_string(),
            crate::model::SynthesisProvenance {
                source_id: "swerebench".into(),
                from: "openai/gpt-5.4".into(),
                category: SynthesisCategory::SameSeriesForward,
            },
        );
        let stale_target = ModelRecord::new(
            "anthropic/claude-opus-4.7".into(),
            "claude-opus-4.7".into(),
            Vendor::Other("test".into()),
        );
        // No `synthesized` entries — every metric came from a real source.
        let records = vec![useful_target, stale_target];
        let pairs = vec![
            SynthesisPair::same_series_forward("openai/gpt-5.5", "openai/gpt-5.4"),
            SynthesisPair::same_series_forward(
                "anthropic/claude-opus-4.7",
                "anthropic/claude-opus-4.6",
            ),
        ];
        let stale = warn_stale_synthesis_pairs(&records, &pairs);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].0, "anthropic/claude-opus-4.7");
        assert_eq!(stale[0].1, "anthropic/claude-opus-4.6");
    }

    #[test]
    fn synthesized_rows_preserve_donor_name_for_effort_filtering() {
        let records = vec![
            record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]),
            record("openai/gpt-5.4", "gpt-5.4", &["gpt-5.4", "gpt-5-4-high"]),
        ];
        let mut rows = rows_by_source(vec![raw(
            "swebench_pro",
            "gpt-5-4-high",
            None,
            &[("SWEBenchPro", json!(88.0))],
        )]);

        synthesize_rows(
            &mut rows,
            &[SynthesisPair::same_series_forward(
                "openai/gpt-5.5",
                "openai/gpt-5.4",
            )],
            &records,
            &cfg(0.50, 0.50),
        );

        let mut scored = records;
        ingest_rows(&mut scored, rows.remove("swebench_pro").unwrap());

        assert!(
            !scored[0].raw_metrics.contains_key("SWEBenchPro"),
            "synthesized high-effort donor row should not score as target default"
        );
    }

    #[test]
    fn synthesize_skips_target_when_sibling_absent_at_source() {
        let records = vec![
            record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]),
            record("openai/gpt-5.4", "gpt-5.4", &["gpt-5.4"]),
        ];
        let mut rows = rows_by_source(vec![raw(
            "lmarena",
            "some-other-model",
            None,
            &[("score", json!(70.0))],
        )]);

        let stats = synthesize_rows(
            &mut rows,
            &[SynthesisPair::same_series_forward(
                "openai/gpt-5.5",
                "openai/gpt-5.4",
            )],
            &records,
            &cfg(0.30, 0.50),
        );

        assert_eq!(stats.per_source.get("lmarena"), Some(&0));
        assert_eq!(rows["lmarena"].len(), 1);
    }

    #[test]
    fn synthesize_per_source_cap_drops_trailing_pairs_deterministically() {
        let records = vec![
            record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]),
            record("openai/gpt-5.4", "gpt-5.4", &["gpt-5.4"]),
            record(
                "google/gemini-3.1-pro-preview",
                "gemini-3.1-pro-preview",
                &["gemini-3.1-pro-preview"],
            ),
            record("google/gemini-3-pro", "gemini-3-pro", &["gemini-3-pro"]),
        ];
        let mut rows = rows_by_source(vec![
            raw("lmarena", "gpt-5.4", None, &[("score", json!(88.0))]),
            raw("lmarena", "gemini-3-pro", None, &[("score", json!(81.0))]),
        ]);

        let stats = synthesize_rows(
            &mut rows,
            &[
                SynthesisPair::same_series_forward("openai/gpt-5.5", "openai/gpt-5.4"),
                SynthesisPair::same_series_forward(
                    "google/gemini-3.1-pro-preview",
                    "google/gemini-3-pro",
                ),
            ],
            &records,
            &cfg(0.30, 0.50),
        );

        let lmarena_rows = &rows["lmarena"];
        assert_eq!(stats.per_source.get("lmarena"), Some(&1));
        assert_eq!(stats.capped_sources, vec!["lmarena".to_string()]);
        assert_eq!(lmarena_rows.len(), 3);
        assert_eq!(lmarena_rows[2].model_name, "gpt-5.5");
        assert_eq!(
            lmarena_rows[2].synthesized_from.as_deref(),
            Some("openai/gpt-5.4")
        );
    }

    #[test]
    fn synthesize_per_source_cap_does_not_cap_at_exact_boundary() {
        let records = vec![
            record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]),
            record("openai/gpt-5.4", "gpt-5.4", &["gpt-5.4"]),
            record(
                "google/gemini-3.1-pro-preview",
                "gemini-3.1-pro-preview",
                &["gemini-3.1-pro-preview"],
            ),
            record("google/gemini-3-pro", "gemini-3-pro", &["gemini-3-pro"]),
            record("anthropic/claude-opus-4.7", "opus-4.7", &["opus-4.7"]),
        ];
        let mut rows = rows_by_source(vec![
            raw("lmarena", "gpt-5.4", None, &[("score", json!(88.0))]),
            raw("lmarena", "gemini-3-pro", None, &[("score", json!(81.0))]),
        ]);

        let stats = synthesize_rows(
            &mut rows,
            &[
                SynthesisPair::same_series_forward("openai/gpt-5.5", "openai/gpt-5.4"),
                SynthesisPair::same_series_forward(
                    "google/gemini-3.1-pro-preview",
                    "google/gemini-3-pro",
                ),
                SynthesisPair::same_series_forward(
                    "anthropic/claude-opus-4.7",
                    "anthropic/claude-opus-4.6",
                ),
            ],
            &records,
            &cfg(0.50, 0.50),
        );

        let lmarena_rows = &rows["lmarena"];
        assert_eq!(stats.per_source.get("lmarena"), Some(&2));
        assert!(stats.capped_sources.is_empty());
        assert_eq!(lmarena_rows.len(), 4);
    }

    #[test]
    fn synthesize_ingest_real_override_is_order_independent() {
        let synth_then_real = vec![
            raw(
                "openrouter",
                "gpt-5.5",
                Some("openai/gpt-5.4"),
                &[("OutputSpeed", json!(75.0))],
            ),
            raw(
                "openrouter",
                "gpt-5.5",
                None,
                &[("OutputSpeed", json!(90.0))],
            ),
        ];
        let real_then_synth = vec![
            raw(
                "openrouter",
                "gpt-5.5",
                None,
                &[("OutputSpeed", json!(90.0))],
            ),
            raw(
                "openrouter",
                "gpt-5.5",
                Some("openai/gpt-5.4"),
                &[("OutputSpeed", json!(75.0))],
            ),
        ];

        for rows in [synth_then_real, real_then_synth] {
            let mut records = vec![record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"])];
            let stats = ingest_rows(&mut records, rows);
            assert_eq!(stats.matched, 2);
            assert_eq!(
                records[0].raw_metrics.get("OutputSpeed"),
                Some(&90.0),
                "real row should win regardless of input ordering"
            );
            assert!(records[0].synthesized.is_empty());
        }
    }

    #[test]
    fn synthesize_ingest_marks_provenance_without_adding_source() {
        let mut records = vec![record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"])];
        let stats = ingest_rows(
            &mut records,
            vec![raw(
                "openrouter",
                "gpt-5.5",
                Some("openai/gpt-5.4"),
                &[("OutputSpeed", json!(75.0))],
            )],
        );

        assert_eq!(stats.matched, 1);
        assert_eq!(records[0].raw_metrics.get("OutputSpeed"), Some(&75.0));
        assert_eq!(
            records[0].synthesized.get("OutputSpeed"),
            Some(&SynthesisProvenance {
                source_id: "openrouter".to_string(),
                from: "openai/gpt-5.4".to_string(),
                category: SynthesisCategory::Conservative,
            })
        );
        assert!(records[0].sources.is_empty());
    }

    #[test]
    fn synthesize_per_model_cap_marks_synthesis_dominant() {
        let mut record = record("openai/gpt-5.5", "gpt-5.5", &["gpt-5.5"]);
        record.raw_metrics = [
            ("AI_correctness".to_string(), 80.0),
            ("AI_code".to_string(), 70.0),
        ]
        .into_iter()
        .collect();
        record.synthesized = [(
            "AI_correctness".to_string(),
            SynthesisProvenance {
                source_id: "openrouter".to_string(),
                from: "openai/gpt-5.4".to_string(),
                category: SynthesisCategory::Conservative,
            },
        )]
        .into_iter()
        .collect();
        let mut records = vec![record];

        mark_synthesis_dominant(&mut records, 0.40);
        assert!(records[0].missing.synthesis_dominant);

        mark_synthesis_dominant(&mut records, 0.60);
        assert!(!records[0].missing.synthesis_dominant);
    }

    #[test]
    fn synthesize_pair_loader_preserves_declared_order() {
        let pairs = load_pairs_from_str(
            r#"
                [[pair]]
                target = "first/target"
                from = "first/from"
                category = "same_series_forward"

                [[pair]]
                target = "second/target"
                from = "second/from"
            "#,
        )
        .expect("pair file should parse");

        assert_eq!(
            pairs,
            vec![
                SynthesisPair::same_series_forward("first/target", "first/from"),
                SynthesisPair::conservative("second/target", "second/from"),
            ]
        );
    }
}
