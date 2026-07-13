use crate::alias::{AliasIndex, normalize_name};
use crate::model::{ModelRecord, RawRow, SourceId, Vendor};
use std::collections::{BTreeMap, BTreeSet};

const NON_SYNTHESIZED_METRICS: &[&str] = &[
    "AI_canary_health",
    // Launch-card reported-only metrics are specific measured rows, not
    // sibling priors. Keep them from propagating through synthesis.
    "KimiCodeBenchV2",
    "ProgramBench",
    "MLSBenchLite",
    "KimiClaw247Bench",
    "MCPMarkVerified",
    // Observation-specific uncertainty metadata must stay attached to the
    // measured row and never transfer to a sibling model.
    "TerminalBenchUncertainty",
    "TerminalBench21Uncertainty",
    "SWERebenchSEM",
    "EQBenchJudgemarkCILow",
    "EQBenchJudgemarkCIHigh",
    "DeepSWECILow",
    "DeepSWECIHigh",
    "DeepSWEPassAt4",
    "DeepSWEAttempts",
    "DeepSWETasksAttempted",
    "DeepSWERuns",
    "FactoryCodeReviewF1Stdev",
    "GDPvalAA2CILow",
    "GDPvalAA2CIHigh",
    "GDPvalAA2HybridFallback",
    "GDPvalAA2HybridFallbackCILow",
    "GDPvalAA2HybridFallbackCIHigh",
    "CritPtHybridFallback",
    "AAOmniscienceIndexHybridFallback",
    "AAOmniscienceAccuracyHybridFallback",
    "AAOmniscienceNonHallucinationHybridFallback",
    "EnterpriseOpsGymAAHybridFallback",
    "AutomationBenchAAHybridFallback",
];

const EVIDENCE_NOTE_SUFFIX: &str = "__evidence_note";

#[derive(Debug, Default, Clone)]
pub struct IngestStats {
    pub matched: usize,
    pub unmatched: Vec<RawRow>,
}

pub fn ingest_rows(records: &mut [ModelRecord], rows: Vec<RawRow>) -> IngestStats {
    ingest_rows_with_policy(records, rows, &crate::coefficients::EffortPolicy::default())
}

pub fn ingest_rows_with_policy(
    records: &mut [ModelRecord],
    rows: Vec<RawRow>,
    effort_policy: &crate::coefficients::EffortPolicy,
) -> IngestStats {
    let mut stats = IngestStats::default();
    let snapshot: Vec<ModelRecord> = records.to_vec();
    let index = AliasIndex::build(&snapshot);
    let mut real_metric_choices: BTreeMap<(usize, String), EffortPreference> = BTreeMap::new();

    let (real_rows, synthesized_rows): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .partition(|row| row.synthesized_from.is_none());

    for row in real_rows {
        ingest_real_row(
            records,
            &index,
            row,
            &mut stats,
            &mut real_metric_choices,
            effort_policy,
        );
    }
    for row in synthesized_rows {
        ingest_synthesized_row(records, &index, row, &mut stats, effort_policy);
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
                if is_evidence_note_key(key) {
                    continue;
                }
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
    let coefficients = crate::coefficients::Coefficients::load_embedded()
        .expect("embedded coefficients are valid");
    mark_synthesis_dominant_with_coefficients(records, per_model_cap, &coefficients);
}

/// Preliminary pre-score synthesis diagnostic using only leaf metrics that
/// can reach a configured role. `compute_scores_with` replaces this with the
/// authoritative family-capped role-weight calculation after scoring.
pub fn mark_synthesis_dominant_with_coefficients(
    records: &mut [ModelRecord],
    per_model_cap: f64,
    coefficients: &crate::coefficients::Coefficients,
) {
    let scored_metrics = scored_leaf_metrics(coefficients);
    for record in records {
        let total_cells = record
            .raw_metrics
            .keys()
            .filter(|metric| scored_metrics.contains(*metric))
            .count();
        let synthesized_cells = record
            .synthesized
            .keys()
            .filter(|metric| scored_metrics.contains(*metric))
            .count();
        record.missing.synthesis_dominant =
            total_cells > 0 && (synthesized_cells as f64 / total_cells as f64) > per_model_cap;
    }
}

fn scored_leaf_metrics(coefficients: &crate::coefficients::Coefficients) -> BTreeSet<String> {
    fn expand(
        metric: &str,
        coefficients: &crate::coefficients::Coefficients,
        out: &mut BTreeSet<String>,
        visiting: &mut BTreeSet<String>,
    ) {
        let Some(inputs) = coefficients.composite_metrics.get(metric) else {
            if coefficients.metrics.contains_key(metric) {
                out.insert(metric.to_string());
            }
            return;
        };
        if !visiting.insert(metric.to_string()) {
            return;
        }
        for input in inputs.keys() {
            expand(input, coefficients, out, visiting);
        }
        visiting.remove(metric);
    }

    let mut out = BTreeSet::new();
    for groups in coefficients.final_score_weights.values() {
        for group in groups.keys() {
            if let Some(metrics) = coefficients.group_weights.get(group) {
                for metric in metrics.keys() {
                    expand(metric, coefficients, &mut out, &mut BTreeSet::new());
                }
            }
        }
    }
    out
}

fn ingest_real_row(
    records: &mut [ModelRecord],
    index: &AliasIndex<'_>,
    row: RawRow,
    stats: &mut IngestStats,
    metric_choices: &mut BTreeMap<(usize, String), EffortPreference>,
    effort_policy: &crate::coefficients::EffortPolicy,
) {
    match index.match_record(&row.model_name, row.vendor_hint.as_deref()) {
        Some(i) => {
            let record = &mut records[i];
            let is_override = row.source_id == "overrides";
            let preference = EffortPreference::from_row(&row);
            let evidence_notes: BTreeMap<String, String> = row
                .fields
                .iter()
                .filter_map(|(key, value)| {
                    evidence_note_metric(key).and_then(|metric| {
                        value
                            .as_str()
                            .map(|note| (metric.to_string(), note.to_string()))
                    })
                })
                .collect();
            // Capture before `row.source_id` is moved into `record.sources`.
            let source_id = row.source_id.clone();
            let canonical_id = record.canonical_id.clone();
            let vendor = record.vendor.clone();
            record.sources.insert(row.source_id);
            for (key, value) in row.fields {
                if is_evidence_note_key(&key) {
                    continue;
                }
                if let Some(num) = json_to_f64(&value) {
                    if !is_scoring_allowed_for(
                        preference,
                        &source_id,
                        &canonical_id,
                        &vendor,
                        effort_policy,
                    ) {
                        continue;
                    }
                    let choice_key = (i, key.clone());
                    let incoming_evidence = if is_override {
                        EvidencePriority::CuratedDirect
                    } else {
                        EvidencePriority::Direct
                    };
                    let existing_evidence = evidence_priority(record, &key);
                    if let Some(existing_evidence) = existing_evidence {
                        if existing_evidence > incoming_evidence {
                            continue;
                        }
                        if existing_evidence == incoming_evidence {
                            let compare_value = match metric_choices.get(&choice_key) {
                                // A stronger-effort row already won.
                                Some(existing) if *existing < preference => continue,
                                // Incoming effort is stronger: it wins even
                                // if sampling noise made its headline lower.
                                Some(existing) if *existing > preference => false,
                                // Same or unknown effort: use metric direction.
                                _ => true,
                            };
                            if compare_value
                                && let Some(existing) = record.raw_metrics.get(&key)
                                && !should_replace_metric_value(&key, *existing, num)
                            {
                                continue;
                            }
                        }
                    }
                    metric_choices.insert(choice_key, preference);
                    record.raw_metrics.insert(key.clone(), num);
                    record.synthesized.remove(&key);
                    record.metric_sources.insert(key.clone(), source_id.clone());
                    if is_override {
                        record.curated_overrides.insert(key.clone());
                        if let Some(note) = evidence_notes.get(&key) {
                            record.override_notes.insert(key, note.clone());
                        } else {
                            record.override_notes.remove(&key);
                        }
                    } else {
                        record.curated_overrides.remove(&key);
                        record.override_notes.remove(&key);
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
    Max = 0,
    High = 1,
    Thinking = 2,
    Medium = 3,
    Default = 4,
    Other = 5,
    Low = 6,
    NonReasoning = 7,
}

impl EffortPreference {
    fn from_row(row: &RawRow) -> Self {
        let mut text = row.model_name.clone();
        for (key, value) in &row.fields {
            if is_evidence_note_key(key) {
                continue;
            }
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
        } else if contains("non reasoning") {
            Self::NonReasoning
        } else if contains("low") {
            Self::Low
        } else if contains("max") || contains("xhigh") {
            Self::Max
        } else if contains("high") {
            Self::High
        } else if contains("thinking") || contains("reasoning") || contains("adaptive") {
            Self::Thinking
        } else if contains("medium") {
            Self::Medium
        } else {
            Self::Other
        }
    }

    fn is_scoring_allowed(self) -> bool {
        matches!(
            self,
            Self::Default | Self::Medium | Self::Thinking | Self::High | Self::Max
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EvidencePriority {
    Synthesized,
    CuratedDirect,
    Direct,
}

fn evidence_priority(record: &ModelRecord, metric: &str) -> Option<EvidencePriority> {
    record.raw_metrics.contains_key(metric).then(|| {
        if record.synthesized.contains_key(metric) {
            EvidencePriority::Synthesized
        } else if record.curated_overrides.contains(metric) {
            EvidencePriority::CuratedDirect
        } else {
            EvidencePriority::Direct
        }
    })
}

pub(crate) fn is_evidence_note_key(key: &str) -> bool {
    evidence_note_metric(key).is_some()
}

pub(crate) fn is_synthesizable_field(key: &str) -> bool {
    key != "SynthesizedFromModelName"
        && !is_evidence_note_key(key)
        && !NON_SYNTHESIZED_METRICS.contains(&key)
}

fn evidence_note_metric(key: &str) -> Option<&str> {
    key.strip_suffix(EVIDENCE_NOTE_SUFFIX)
        .filter(|metric| !metric.is_empty())
}

/// Variant policy driven by `[effort_policy]` in `coefficients.toml`. The
/// default scoring set is `default | medium | thinking | high | max/xhigh`.
/// Exceptions remain for intentionally blocked variants, currently low and
/// non-reasoning rows.
fn is_scoring_allowed_for(
    preference: EffortPreference,
    source_id: &str,
    canonical_id: &str,
    vendor: &Vendor,
    effort_policy: &crate::coefficients::EffortPolicy,
) -> bool {
    if preference.is_scoring_allowed() {
        return true;
    }
    let effort_name = match preference {
        EffortPreference::High => "high",
        EffortPreference::Max => "max",
        EffortPreference::Thinking => "thinking",
        EffortPreference::Low => "low",
        EffortPreference::NonReasoning => "non reasoning",
        EffortPreference::Medium => "medium",
        EffortPreference::Default => "default",
        EffortPreference::Other => "other",
    };
    effort_policy.allows(effort_name, source_id, vendor.as_str(), canonical_id)
}

fn contains_phrase(normalized_text: &str, phrase: &str) -> bool {
    let haystack = format!(" {normalized_text} ");
    let needle = format!(" {phrase} ");
    haystack.contains(&needle)
}

fn should_replace_metric_value(metric: &str, existing: f64, incoming: f64) -> bool {
    match metric {
        "BlendedCost" | "PromptPricePerMillion" | "CompletionPricePerMillion" | "TTFT" => {
            incoming.is_finite()
                && incoming > 0.0
                && (!existing.is_finite() || existing <= 0.0 || incoming < existing)
        }
        "ContextWindow"
        | "MaxCompletionTokens"
        | "SupportedParametersCount"
        | "SupportsTools"
        | "SupportsStructuredOutputs"
        | "SupportsReasoning" => {
            incoming.is_finite() && (!existing.is_finite() || incoming > existing)
        }
        _ => true,
    }
}

fn ingest_synthesized_row(
    records: &mut [ModelRecord],
    index: &AliasIndex<'_>,
    row: RawRow,
    stats: &mut IngestStats,
    effort_policy: &crate::coefficients::EffortPolicy,
) {
    match index.match_record(&row.model_name, row.vendor_hint.as_deref()) {
        Some(i) => {
            let record = &mut records[i];
            let from = row
                .synthesized_from
                .clone()
                .expect("synthesized rows must carry synthesized_from");
            let category = row.synthesis_category.unwrap_or_default();
            let preference = EffortPreference::from_row(&row);
            let source_id = row.source_id.clone();
            let canonical_id = record.canonical_id.clone();
            let vendor = record.vendor.clone();
            for (key, value) in row.fields {
                if is_evidence_note_key(&key) {
                    continue;
                }
                if !is_synthesizable_field(&key) {
                    continue;
                }
                if !is_scoring_allowed_for(
                    preference,
                    &source_id,
                    &canonical_id,
                    &vendor,
                    effort_policy,
                ) {
                    continue;
                }
                if record.raw_metrics.contains_key(&key) {
                    continue;
                }
                if let Some(num) = json_to_f64(&value) {
                    record.raw_metrics.insert(key.clone(), num);
                    record
                        .metric_sources
                        .insert(key.clone(), row.source_id.clone());
                    record.synthesized.insert(
                        key,
                        crate::model::SynthesisProvenance {
                            source_id: row.source_id.clone(),
                            from: from.clone(),
                            category,
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
            synthesis_category: None,
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
        assert_eq!(
            records[0]
                .metric_sources
                .get("OutputSpeed")
                .map(String::as_str),
            Some("openrouter")
        );
    }

    #[test]
    fn direct_source_beats_override_across_ingest_order_and_clears_note() {
        for direct_first in [false, true] {
            let mut record =
                ModelRecord::new("openai/gpt-5.5".into(), "gpt-5.5".into(), Vendor::Openai);
            record.aliases.insert("gpt-5.5".into());
            let mut records = vec![record];
            let direct = raw(
                "terminal_bench",
                "gpt-5.5",
                &[("TerminalBench", json!(80.0))],
            );
            let curated = raw(
                "overrides",
                "gpt-5.5",
                &[
                    ("TerminalBench", json!(99.0)),
                    (
                        "TerminalBench__evidence_note",
                        json!("vendor xHigh launch-card result"),
                    ),
                ],
            );
            let ordered = if direct_first {
                vec![direct, curated]
            } else {
                vec![curated, direct]
            };
            // Separate calls exercise precedence across the CLI's per-source
            // ingestion boundary, not only within one input vector.
            for row in ordered {
                ingest_rows(&mut records, vec![row]);
            }
            assert_eq!(records[0].raw_metrics.get("TerminalBench"), Some(&80.0));
            assert_eq!(
                records[0]
                    .metric_sources
                    .get("TerminalBench")
                    .map(String::as_str),
                Some("terminal_bench")
            );
            assert!(!records[0].curated_overrides.contains("TerminalBench"));
            assert!(!records[0].override_notes.contains_key("TerminalBench"));
        }
    }

    #[test]
    fn winning_override_captures_note_without_note_affecting_effort() {
        let mut record =
            ModelRecord::new("openai/gpt-5.5".into(), "gpt-5.5".into(), Vendor::Openai);
        record.aliases.insert("gpt-5.5".into());
        let mut records = vec![record];
        ingest_rows(
            &mut records,
            vec![raw(
                "overrides",
                "gpt-5.5",
                &[
                    ("TerminalBench", json!(80.0)),
                    (
                        "TerminalBench__evidence_note",
                        json!("reported at prohibited low effort, xHigh comparison"),
                    ),
                ],
            )],
        );
        assert_eq!(records[0].raw_metrics.get("TerminalBench"), Some(&80.0));
        assert_eq!(
            records[0]
                .override_notes
                .get("TerminalBench")
                .map(String::as_str),
            Some("reported at prohibited low effort, xHigh comparison")
        );
        assert_eq!(
            records[0]
                .metric_sources
                .get("TerminalBench")
                .map(String::as_str),
            Some("overrides")
        );
        assert!(records[0].curated_overrides.contains("TerminalBench"));
        assert!(
            !records[0]
                .raw_metrics
                .contains_key("TerminalBench__evidence_note")
        );
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
    fn synthesized_rows_skip_non_transferable_signals() {
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
                ("KimiCodeBenchV2", json!(62.0)),
                ("TerminalBenchUncertainty", json!(1.2)),
                ("SWERebenchSEM", json!(0.7)),
                ("AI_correctness", json!(80.0)),
            ],
        );
        row.synthesized_from = Some("openai/gpt-5.4".to_string());

        let stats = ingest_rows(&mut records, vec![row]);

        assert_eq!(stats.matched, 1);
        assert!(!records[0].raw_metrics.contains_key("AI_canary_health"));
        assert!(!records[0].synthesized.contains_key("AI_canary_health"));
        assert!(!records[0].raw_metrics.contains_key("KimiCodeBenchV2"));
        assert!(!records[0].synthesized.contains_key("KimiCodeBenchV2"));
        assert!(
            !records[0]
                .raw_metrics
                .contains_key("TerminalBenchUncertainty")
        );
        assert!(!records[0].raw_metrics.contains_key("SWERebenchSEM"));
        assert_eq!(records[0].raw_metrics.get("AI_correctness"), Some(&80.0));
        assert!(records[0].synthesized.contains_key("AI_correctness"));
    }

    #[test]
    fn synthesized_rows_never_carry_curated_override_flag() {
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
            !records[0].curated_overrides.contains("TerminalBench"),
            "synthesized rows must not be marked as curated overrides"
        );
    }

    #[test]
    fn real_rows_prefer_thinking_variant_over_default() {
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
        assert_eq!(records[0].raw_metrics.get("LMArenaText"), Some(&99.0));
    }

    #[test]
    fn real_rows_prefer_stronger_effort_even_when_its_score_is_lower() {
        for max_first in [false, true] {
            let mut record = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            record.aliases.insert("gpt-5-5".to_string());
            record.aliases.insert("gpt-5-5-max".to_string());
            let default = raw("benchmark", "gpt-5-5", &[("TerminalBench", json!(90.0))]);
            let max = raw(
                "benchmark",
                "gpt-5-5-max",
                &[("TerminalBench", json!(80.0))],
            );
            let rows = if max_first {
                vec![max, default]
            } else {
                vec![default, max]
            };
            let mut records = vec![record];

            ingest_rows(&mut records, rows);

            assert_eq!(
                records[0].raw_metrics.get("TerminalBench"),
                Some(&80.0),
                "max effort must win independently of row order"
            );
        }
    }

    #[test]
    fn real_rows_keep_lower_blended_cost_across_source_ingests() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            r.aliases.insert("gpt-5.5".to_string());
            r
        }];

        let stats = ingest_rows(
            &mut records,
            vec![raw(
                "openrouter",
                "gpt-5.5",
                &[("BlendedCost", json!(1.25))],
            )],
        );
        assert_eq!(stats.matched, 1);

        let stats = ingest_rows(
            &mut records,
            vec![raw(
                "artificial_analysis",
                "gpt-5.5",
                &[("BlendedCost", json!(1.75))],
            )],
        );
        assert_eq!(stats.matched, 1);
        assert_eq!(records[0].raw_metrics.get("BlendedCost"), Some(&1.25));

        let stats = ingest_rows(
            &mut records,
            vec![raw(
                "artificial_analysis",
                "gpt-5.5",
                &[("BlendedCost", json!(0.95))],
            )],
        );
        assert_eq!(stats.matched, 1);
        assert_eq!(records[0].raw_metrics.get("BlendedCost"), Some(&0.95));
    }

    #[test]
    fn real_rows_replace_synthesized_operational_values() {
        let mut records = vec![{
            let mut r = ModelRecord::new(
                "openai/gpt-5.5".to_string(),
                "gpt-5.5".to_string(),
                Vendor::Openai,
            );
            r.aliases.insert("gpt-5.5".to_string());
            r
        }];
        let mut synthesized = raw("openrouter", "gpt-5.5", &[("BlendedCost", json!(0.50))]);
        synthesized.synthesized_from = Some("openai/gpt-5.4".to_string());

        let stats = ingest_rows(&mut records, vec![synthesized]);
        assert_eq!(stats.matched, 1);
        assert_eq!(records[0].raw_metrics.get("BlendedCost"), Some(&0.50));
        assert!(records[0].synthesized.contains_key("BlendedCost"));

        let stats = ingest_rows(
            &mut records,
            vec![raw(
                "artificial_analysis",
                "gpt-5.5",
                &[("BlendedCost", json!(0.90))],
            )],
        );

        assert_eq!(stats.matched, 1);
        assert_eq!(records[0].raw_metrics.get("BlendedCost"), Some(&0.90));
        assert!(!records[0].synthesized.contains_key("BlendedCost"));
    }

    #[test]
    fn real_rows_prefer_high_variant_when_default_is_absent() {
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
            Some(&99.0)
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
            Some(&99.0)
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
    fn real_rows_allow_high_effort_when_no_default_medium_or_thinking_exists() {
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
        assert_eq!(
            records[0].raw_metrics.get("ArtificialAnalysisIntelligence"),
            Some(&99.0)
        );
    }

    #[test]
    fn qwen_max_product_tier_is_not_treated_as_effort() {
        for (canonical, display, preview_alias) in [
            (
                "qwen/qwen3.6-max-preview",
                "qwen3.6-max-preview",
                "qwen3.6-max",
            ),
            ("qwen/qwen3.7-max", "qwen3.7-max", "qwen3.7-max-preview"),
        ] {
            let mut records = vec![{
                let mut r =
                    ModelRecord::new(canonical.to_string(), display.to_string(), Vendor::Alibaba);
                r.aliases.insert(canonical.to_string());
                r.aliases.insert(preview_alias.to_string());
                r.aliases.insert(display.to_string());
                r
            }];
            let mut synthesized = raw("swerebench", display, &[("SWERebench", json!(72.0))]);
            synthesized.synthesized_from = Some("qwen/qwen3.6-plus".to_string());
            let rows = vec![
                raw("overrides", canonical, &[("SWEBenchPro", json!(60.6))]),
                raw("lmarena", preview_alias, &[("LMArenaText", json!(91.0))]),
                synthesized,
            ];

            let stats = ingest_rows_with_policy(
                &mut records,
                rows,
                &crate::Coefficients::load_embedded().unwrap().effort_policy,
            );

            assert_eq!(stats.matched, 3);
            assert_eq!(records[0].raw_metrics.get("SWEBenchPro"), Some(&60.6));
            assert_eq!(records[0].raw_metrics.get("LMArenaText"), Some(&91.0));
            assert_eq!(records[0].raw_metrics.get("SWERebench"), Some(&72.0));
            assert!(records[0].curated_overrides.contains("SWEBenchPro"));
            assert!(records[0].synthesized.contains_key("SWERebench"));
        }
    }

    #[test]
    fn synthesized_rows_allow_high_effort_values() {
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
                ("DisplayName", json!("Claude Opus 4.6 (High Effort)")),
                ("ArtificialAnalysisIntelligence", json!(80.0)),
            ],
        );
        row.synthesized_from = Some("anthropic/claude-opus-4.6".to_string());

        let stats = ingest_rows(&mut records, vec![row]);

        assert_eq!(stats.matched, 1);
        assert_eq!(
            records[0].raw_metrics.get("ArtificialAnalysisIntelligence"),
            Some(&80.0)
        );
        assert!(
            records[0]
                .synthesized
                .contains_key("ArtificialAnalysisIntelligence")
        );
    }
}
