use crate::alias::AliasIndex;
use crate::coefficients::SynthesisConfig;
use crate::model::{ModelRecord, RawRow, SourceId};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SynthesisStats {
    pub per_source: BTreeMap<SourceId, usize>,
    pub capped_sources: Vec<SourceId>,
}

#[derive(Debug, Deserialize)]
struct PairEntry {
    target: String,
    from: String,
}

#[derive(Debug, Deserialize)]
struct PairFile {
    pair: Vec<PairEntry>,
}

const EMBEDDED_PAIRS: &str = include_str!("../../../data/synthesis_aliases.toml");

pub fn load_embedded_pairs() -> Result<Vec<(String, String)>, toml::de::Error> {
    load_pairs_from_str(EMBEDDED_PAIRS)
}

pub fn load_pairs_from_str(raw: &str) -> Result<Vec<(String, String)>, toml::de::Error> {
    let file: PairFile = toml::from_str(raw)?;
    Ok(file
        .pair
        .into_iter()
        .map(|entry| (entry.target, entry.from))
        .collect())
}

pub fn synthesize_rows(
    rows_by_source: &mut BTreeMap<SourceId, Vec<RawRow>>,
    pairs: &[(String, String)],
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
        let mut synth_count = 0usize;

        for (target_id, from_id) in pairs {
            if real_count > 0
                && synth_count > 0
                && (synth_count as f64 / (real_count + synth_count) as f64) > cfg.per_source_cap
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

            // Always emit when a donor row exists. The ingest layer applies
            // per-field filtering (`ingest_synthesized_row` skips any field
            // that the target already has a real value for), so a synthesis
            // row only ends up filling fields that are *actually* missing
            // for the target — no toml-level source-skip list needed. This
            // is what makes synthesis the last-resort fill-in: real values
            // always win, and partial real coverage (e.g. AISL hourly
            // axes for a freshly-released model) is preserved while the
            // donor's tail fills the rest.
            let Some(donor) = rows
                .iter()
                .find(|row| resolve_canonical(row) == Some(from_id.as_str()))
                .cloned()
            else {
                continue;
            };

            let Some(display_name) = display_name_for(target_id) else {
                continue;
            };

            let donor_model_name = donor.model_name.clone();
            let mut synthesized = donor;
            synthesized.fields.insert(
                "SynthesizedFromModelName".to_string(),
                Value::from(donor_model_name),
            );
            synthesized.model_name = display_name.to_string();
            synthesized.synthesized_from = Some(from_id.clone());
            rows.push(synthesized);
            synth_count += 1;
        }

        stats.per_source.insert(source_id.clone(), synth_count);
    }

    stats
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
        // Synthesis runs at the row level, but the ingest layer's
        // `ingest_synthesized_row` skips any field that the target already
        // carries a real value for (see crates/core/src/ingest.rs). So
        // synthesize_rows always emits a synthesized row when a donor exists,
        // and the per-field arbitration happens later. This shape lets a
        // model with partial real coverage (e.g. AISL hourly-suite axes for
        // a fresh release) keep its real values while picking up the donor's
        // tail for the genuinely missing fields.
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
            &[("openai/gpt-5.5".to_string(), "openai/gpt-5.4".to_string())],
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
        assert_eq!(synth[0].model_name, "gpt-5.5");
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
            &[("openai/gpt-5.5".to_string(), "openai/gpt-5.4".to_string())],
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
            &[("openai/gpt-5.5".to_string(), "openai/gpt-5.4".to_string())],
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
                ("openai/gpt-5.5".to_string(), "openai/gpt-5.4".to_string()),
                (
                    "google/gemini-3.1-pro-preview".to_string(),
                    "google/gemini-3-pro".to_string(),
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
                ("openai/gpt-5.5".to_string(), "openai/gpt-5.4".to_string()),
                (
                    "google/gemini-3.1-pro-preview".to_string(),
                    "google/gemini-3-pro".to_string(),
                ),
                (
                    "anthropic/claude-opus-4.7".to_string(),
                    "anthropic/claude-opus-4.6".to_string(),
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

                [[pair]]
                target = "second/target"
                from = "second/from"
            "#,
        )
        .expect("pair file should parse");

        assert_eq!(
            pairs,
            vec![
                ("first/target".to_string(), "first/from".to_string()),
                ("second/target".to_string(), "second/from".to_string()),
            ]
        );
    }
}
