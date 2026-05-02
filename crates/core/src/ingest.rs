use crate::alias::{AliasIndex, normalize_name};
use crate::model::{ModelRecord, RawRow, SourceId};
use std::collections::{BTreeMap, BTreeSet};

const NON_SYNTHESIZED_METRICS: &[&str] = &["AI_canary_health"];

#[derive(Debug, Default, Clone)]
pub struct IngestStats {
    pub matched: usize,
    pub unmatched: Vec<RawRow>,
}

pub fn ingest_rows(records: &mut [ModelRecord], rows: Vec<RawRow>) -> IngestStats {
    let mut stats = IngestStats::default();
    let snapshot: Vec<ModelRecord> = records.to_vec();
    let index = AliasIndex::build(&snapshot);
    let mut real_metric_choices: BTreeMap<(usize, String), EffortPreference> = BTreeMap::new();

    let (real_rows, synthesized_rows): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .partition(|row| row.synthesized_from.is_none());

    for row in real_rows {
        ingest_real_row(records, &index, row, &mut stats, &mut real_metric_choices);
    }
    for row in synthesized_rows {
        ingest_synthesized_row(records, &index, row, &mut stats);
    }

    stats
}

/// Identify override entries that are also supplied by a non-override source
/// for the same (model, metric). When a public leaderboard catches up, the
/// override gets clobbered by the real value during ingest precedence — but
/// the entry sits in `data/score_overrides.toml` indefinitely. Surfacing it
/// keeps the file from bloating with retired hand-curations.
///
/// Stderr only (no logging dependency); the returned list lets tests assert
/// on which entries were flagged.
pub fn warn_stale_overrides(
    rows_by_source: &BTreeMap<SourceId, Vec<RawRow>>,
    records: &[ModelRecord],
) -> Vec<(String, String, Vec<String>)> {
    let index = AliasIndex::build(records);
    let mut by_pair: BTreeMap<(usize, String), BTreeSet<String>> = BTreeMap::new();
    for (source_id, rows) in rows_by_source {
        for row in rows {
            if row.synthesized_from.is_some() {
                continue;
            }
            let Some(i) = index.match_record(&row.model_name, row.vendor_hint.as_deref()) else {
                continue;
            };
            for key in row.fields.keys() {
                by_pair
                    .entry((i, key.clone()))
                    .or_default()
                    .insert(source_id.clone());
            }
        }
    }
    let mut stale = Vec::new();
    for ((i, metric), sources) in by_pair {
        if !sources.contains("overrides") {
            continue;
        }
        let other: Vec<String> = sources
            .iter()
            .filter(|s| s.as_str() != "overrides")
            .cloned()
            .collect();
        if other.is_empty() {
            continue;
        }
        let canonical = records[i].canonical_id.clone();
        eprintln!(
            "warning: override for {canonical}/{metric} is duplicated by {other:?}; consider removing it from data/score_overrides.toml"
        );
        stale.push((canonical, metric, other));
    }
    stale
}

pub fn mark_synthesis_dominant(records: &mut [ModelRecord], per_model_cap: f64) {
    for record in records {
        let total_cells = record.raw_metrics.len();
        let synthesized_cells = record.synthesized.len();
        record.missing.synthesis_dominant =
            total_cells > 0 && (synthesized_cells as f64 / total_cells as f64) > per_model_cap;
    }
}

fn ingest_real_row(
    records: &mut [ModelRecord],
    index: &AliasIndex<'_>,
    row: RawRow,
    stats: &mut IngestStats,
    metric_choices: &mut BTreeMap<(usize, String), EffortPreference>,
) {
    match index.match_record(&row.model_name, row.vendor_hint.as_deref()) {
        Some(i) => {
            let record = &mut records[i];
            let is_override = row.source_id == "overrides";
            let preference = EffortPreference::from_row(&row);
            record.sources.insert(row.source_id);
            for (key, value) in row.fields {
                if let Some(num) = json_to_f64(&value) {
                    if !preference.is_scoring_allowed() {
                        continue;
                    }
                    let choice_key = (i, key.clone());
                    if metric_choices
                        .get(&choice_key)
                        .is_some_and(|existing| *existing < preference)
                    {
                        continue;
                    }
                    metric_choices.insert(choice_key, preference);
                    record.raw_metrics.insert(key.clone(), num);
                    record.synthesized.remove(&key);
                    if is_override {
                        record.override_reported.insert(key);
                    } else {
                        record.override_reported.remove(&key);
                    }
                }
            }
            stats.matched += 1;
        }
        None => stats.unmatched.push(row),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EffortPreference {
    Default = 0,
    Medium = 1,
    Thinking = 2,
    NonReasoning = 3,
    Low = 4,
    High = 5,
    Max = 6,
    Other = 7,
}

impl EffortPreference {
    fn from_row(row: &RawRow) -> Self {
        let mut text = row.model_name.clone();
        for value in row.fields.values() {
            if let Some(s) = value.as_str() {
                text.push(' ');
                text.push_str(s);
            }
        }
        Self::from_text(&text)
    }

    fn from_text(text: &str) -> Self {
        let normalized = normalize_name(text);
        let contains = |phrase: &str| contains_phrase(&normalized, phrase);
        let has_effort_marker = [
            "default",
            "medium",
            "non reasoning",
            "low",
            "high",
            "thinking",
            "reasoning",
            "adaptive",
            "max",
            "xhigh",
        ]
        .iter()
        .any(|phrase| contains(phrase));

        if contains("default") || !has_effort_marker {
            Self::Default
        } else if contains("medium") {
            Self::Medium
        } else if contains("non reasoning") {
            Self::NonReasoning
        } else if contains("low") {
            Self::Low
        } else if contains("thinking") || contains("reasoning") || contains("adaptive") {
            Self::Thinking
        } else if contains("high") {
            Self::High
        } else if contains("max") || contains("xhigh") {
            Self::Max
        } else {
            Self::Other
        }
    }

    fn is_scoring_allowed(self) -> bool {
        matches!(self, Self::Default | Self::Medium | Self::Thinking)
    }
}

fn contains_phrase(normalized_text: &str, phrase: &str) -> bool {
    let haystack = format!(" {normalized_text} ");
    let needle = format!(" {phrase} ");
    haystack.contains(&needle)
}

fn ingest_synthesized_row(
    records: &mut [ModelRecord],
    index: &AliasIndex<'_>,
    row: RawRow,
    stats: &mut IngestStats,
) {
    match index.match_record(&row.model_name, row.vendor_hint.as_deref()) {
        Some(i) => {
            let record = &mut records[i];
            let from = row
                .synthesized_from
                .clone()
                .expect("synthesized rows must carry synthesized_from");
            let preference = EffortPreference::from_row(&row);
            for (key, value) in row.fields {
                if NON_SYNTHESIZED_METRICS.contains(&key.as_str()) {
                    continue;
                }
                if !preference.is_scoring_allowed() {
                    continue;
                }
                if record.raw_metrics.contains_key(&key) {
                    continue;
                }
                if let Some(num) = json_to_f64(&value) {
                    record.raw_metrics.insert(key.clone(), num);
                    record.synthesized.insert(
                        key,
                        crate::model::SynthesisProvenance {
                            source_id: row.source_id.clone(),
                            from: from.clone(),
                        },
                    );
                }
            }
            stats.matched += 1;
        }
        None => stats.unmatched.push(row),
    }
}

fn json_to_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Vendor;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn raw(source: &str, name: &str, fields: &[(&str, serde_json::Value)]) -> RawRow {
        let mut map = BTreeMap::new();
        for (k, v) in fields {
            map.insert(k.to_string(), v.clone());
        }
        RawRow {
            source_id: source.to_string(),
            model_name: name.to_string(),
            vendor_hint: None,
            fields: map,
            synthesized_from: None,
        }
    }

    #[test]
    fn matched_row_populates_raw_metrics() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            r.aliases.insert("gpt-5.5".to_string());
            r
        }];
        let rows = vec![raw(
            "openrouter",
            "gpt-5.5",
            &[
                ("ContextWindow", json!(128000)),
                ("OutputSpeed", json!(75.5)),
            ],
        )];
        let stats = ingest_rows(&mut records, rows);
        assert_eq!(stats.matched, 1);
        assert!(stats.unmatched.is_empty());
        assert_eq!(records[0].raw_metrics.get("ContextWindow"), Some(&128000.0));
        assert_eq!(records[0].raw_metrics.get("OutputSpeed"), Some(&75.5));
        assert!(records[0].sources.contains("openrouter"));
    }

    #[test]
    fn warn_stale_overrides_flags_metric_provided_by_real_source() {
        let mut record = ModelRecord::new(
            "anthropic/claude-opus-4.7".into(),
            "claude-opus-4.7".into(),
            Vendor::Anthropic,
        );
        record.aliases.insert("claude-opus-4.7".into());
        let records = vec![record];

        // overrides has SWEBenchVerified for the same model that lmarena
        // also reports — the override is now redundant. Note: we set up a
        // *second* override metric that isn't in any other source so we
        // can confirm we don't false-positive on still-useful overrides.
        let mut rows_by_source = BTreeMap::new();
        rows_by_source.insert(
            "overrides".to_string(),
            vec![
                raw(
                    "overrides",
                    "claude-opus-4.7",
                    &[("SWEBenchVerified", json!(87.6))],
                ),
                raw(
                    "overrides",
                    "claude-opus-4.7",
                    &[("SWEBenchPro", json!(64.3))],
                ),
            ],
        );
        rows_by_source.insert(
            "swebench".to_string(),
            vec![raw(
                "swebench",
                "claude-opus-4.7",
                &[("SWEBenchVerified", json!(85.2))],
            )],
        );

        let stale = warn_stale_overrides(&rows_by_source, &records);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].0, "anthropic/claude-opus-4.7");
        assert_eq!(stale[0].1, "SWEBenchVerified");
        assert_eq!(stale[0].2, vec!["swebench"]);
    }

    #[test]
    fn unmatched_row_collected_for_review() {
        let mut records: Vec<ModelRecord> = vec![];
        let rows = vec![raw("foo", "totally-unknown-model-zzz", &[])];
        let stats = ingest_rows(&mut records, rows);
        assert_eq!(stats.matched, 0);
        assert_eq!(stats.unmatched.len(), 1);
    }

    #[test]
    fn synthesized_rows_skip_canary_health_signal() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            r.aliases.insert("gpt-5.5".to_string());
            r
        }];
        let mut row = raw(
            "aistupidlevel",
            "gpt-5.5",
            &[
                ("AI_canary_health", json!(42.0)),
                ("AI_correctness", json!(80.0)),
            ],
        );
        row.synthesized_from = Some("openai/gpt-5.4".to_string());

        let stats = ingest_rows(&mut records, vec![row]);

        assert_eq!(stats.matched, 1);
        assert!(!records[0].raw_metrics.contains_key("AI_canary_health"));
        assert!(!records[0].synthesized.contains_key("AI_canary_health"));
        assert_eq!(records[0].raw_metrics.get("AI_correctness"), Some(&80.0));
        assert!(records[0].synthesized.contains_key("AI_correctness"));
    }

    #[test]
    fn synthesized_rows_never_carry_override_flag() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            r.aliases.insert("gpt-5.5".to_string());
            r
        }];
        let mut row = raw("overrides", "gpt-5.5", &[("TerminalBench", json!(80.0))]);
        row.synthesized_from = Some("openai/gpt-5.4".to_string());

        let stats = ingest_rows(&mut records, vec![row]);

        assert_eq!(stats.matched, 1);
        assert_eq!(records[0].raw_metrics.get("TerminalBench"), Some(&80.0));
        assert!(
            records[0].synthesized.contains_key("TerminalBench"),
            "synthesized flag should be set"
        );
        assert!(
            !records[0].override_reported.contains("TerminalBench"),
            "synthesized rows must not be marked override_reported"
        );
    }

    #[test]
    fn real_rows_prefer_default_variant_over_thinking() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "anthropic/claude-opus-4.7".to_string(),
                "claude-opus-4.7".to_string(),
                Vendor::Anthropic,
            );
            r.aliases.insert("claude-opus-4-7".to_string());
            r.aliases.insert("claude-opus-4-7-thinking".to_string());
            r
        }];
        let rows = vec![
            raw(
                "lmarena",
                "claude-opus-4-7",
                &[("LMArenaText", json!(80.0))],
            ),
            raw(
                "lmarena",
                "claude-opus-4-7-thinking",
                &[("LMArenaText", json!(99.0))],
            ),
        ];

        let stats = ingest_rows(&mut records, rows);

        assert_eq!(stats.matched, 2);
        assert_eq!(records[0].raw_metrics.get("LMArenaText"), Some(&80.0));
    }

    #[test]
    fn real_rows_prefer_medium_variant_when_default_is_absent() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            r.aliases.insert("gpt-5-5-high".to_string());
            r.aliases.insert("gpt-5-5-medium".to_string());
            r
        }];
        let rows = vec![
            raw(
                "artificial_analysis",
                "gpt-5-5-medium",
                &[("ArtificialAnalysisIntelligence", json!(70.0))],
            ),
            raw(
                "artificial_analysis",
                "gpt-5-5-high",
                &[("ArtificialAnalysisIntelligence", json!(99.0))],
            ),
        ];

        let stats = ingest_rows(&mut records, rows);

        assert_eq!(stats.matched, 2);
        assert_eq!(
            records[0].raw_metrics.get("ArtificialAnalysisIntelligence"),
            Some(&70.0)
        );
    }

    #[test]
    fn real_rows_use_string_fields_when_detecting_effort() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "anthropic/claude-opus-4.7".to_string(),
                "claude-opus-4.7".to_string(),
                Vendor::Anthropic,
            );
            r.aliases.insert("claude-opus-4-7".to_string());
            r.aliases.insert("claude-opus-4-7-medium".to_string());
            r
        }];
        let rows = vec![
            raw(
                "artificial_analysis",
                "claude-opus-4-7-medium",
                &[
                    ("DisplayName", json!("Claude Opus 4.7 Medium")),
                    ("ArtificialAnalysisIntelligence", json!(70.0)),
                ],
            ),
            raw(
                "artificial_analysis",
                "claude-opus-4-7",
                &[
                    (
                        "DisplayName",
                        json!("Claude Opus 4.7 (Adaptive Reasoning, Max Effort)"),
                    ),
                    ("ArtificialAnalysisIntelligence", json!(99.0)),
                ],
            ),
        ];

        let stats = ingest_rows(&mut records, rows);

        assert_eq!(stats.matched, 2);
        assert_eq!(
            records[0].raw_metrics.get("ArtificialAnalysisIntelligence"),
            Some(&70.0)
        );
    }

    #[test]
    fn real_rows_use_thinking_as_medium_when_default_and_literal_medium_are_absent() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "anthropic/claude-opus-4.7".to_string(),
                "claude-opus-4.7".to_string(),
                Vendor::Anthropic,
            );
            r.aliases.insert("claude-opus-4-7".to_string());
            r
        }];
        let rows = vec![raw(
            "artificial_analysis",
            "claude-opus-4-7",
            &[
                (
                    "DisplayName",
                    json!("Claude Opus 4.7 (Adaptive Reasoning, Max Effort)"),
                ),
                ("ArtificialAnalysisIntelligence", json!(99.0)),
            ],
        )];

        let stats = ingest_rows(&mut records, rows);

        assert_eq!(stats.matched, 1);
        assert_eq!(
            records[0].raw_metrics.get("ArtificialAnalysisIntelligence"),
            Some(&99.0)
        );
    }

    #[test]
    fn real_rows_skip_high_effort_when_no_default_medium_or_thinking_exists() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            r.aliases.insert("gpt-5-5-high".to_string());
            r
        }];
        let rows = vec![raw(
            "artificial_analysis",
            "gpt-5-5-high",
            &[("ArtificialAnalysisIntelligence", json!(99.0))],
        )];

        let stats = ingest_rows(&mut records, rows);

        assert_eq!(stats.matched, 1);
        assert!(
            !records[0]
                .raw_metrics
                .contains_key("ArtificialAnalysisIntelligence")
        );
    }

    #[test]
    fn synthesized_rows_skip_high_effort_values() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "anthropic/claude-opus-4.7".to_string(),
                "claude-opus-4.7".to_string(),
                Vendor::Anthropic,
            );
            r.aliases.insert("claude-opus-4-7".to_string());
            r
        }];
        let mut row = raw(
            "artificial_analysis",
            "claude-opus-4.7",
            &[
                (
                    "DisplayName",
                    json!("Claude Opus 4.6 (Non-reasoning, High Effort)"),
                ),
                ("ArtificialAnalysisIntelligence", json!(80.0)),
            ],
        );
        row.synthesized_from = Some("anthropic/claude-opus-4.6".to_string());

        let stats = ingest_rows(&mut records, vec![row]);

        assert_eq!(stats.matched, 1);
        assert!(
            !records[0]
                .raw_metrics
                .contains_key("ArtificialAnalysisIntelligence")
        );
    }
}
