use crate::alias::{AliasIndex, normalize_name};
use crate::model::{ModelRecord, RawRow, SourceId, Vendor};
use std::collections::{BTreeMap, BTreeSet};

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

    for row in rows {
        ingest_real_row(
            records,
            &index,
            row,
            &mut stats,
            &mut real_metric_choices,
            effort_policy,
        );
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

/// Report rows that matched a canonical model only through fuzzy substring
/// matching (i.e. `lookup_exact` missed but `match_record` succeeded). Exact
/// and suffix-stripped matches are deterministic and trusted; fuzzy matches are
/// where a silent mis-attribution would hide, so they are logged for manual
/// audit. Returns `(source, input_name, canonical_id)` per fuzzy match and
/// writes one line each to stderr.
pub fn audit_fuzzy_matches(
    rows_by_source: &BTreeMap<SourceId, Vec<RawRow>>,
    records: &[ModelRecord],
) -> Vec<(String, String, String)> {
    let index = AliasIndex::build(records);
    let mut audited = Vec::new();
    let mut seen: BTreeSet<(String, String, String)> = BTreeSet::new();
    for (source_id, rows) in rows_by_source {
        for row in rows {
            let vendor = row.vendor_hint.as_deref();
            if index.lookup_exact(&row.model_name, vendor).is_some() {
                continue;
            }
            let Some(idx) = index.match_record(&row.model_name, vendor) else {
                continue;
            };
            let canonical = records[idx].canonical_id.clone();
            let entry = (source_id.clone(), row.model_name.clone(), canonical.clone());
            if !seen.insert(entry.clone()) {
                continue;
            }
            eprintln!(
                "audit: fuzzy alias match {source_id}: {:?} -> {canonical} (no exact/suffix-stripped key); verify in data/required_aliases.toml",
                row.model_name
            );
            audited.push(entry);
        }
    }
    audited
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
                    record.metric_sources.insert(key.clone(), source_id.clone());
                    if is_override {
                        record.curated_overrides.insert(key.clone());
                    } else {
                        record.curated_overrides.remove(&key);
                    }
                    if let Some(note) = evidence_notes.get(&key) {
                        record.metric_citations.insert(key, note.clone());
                    } else {
                        record.metric_citations.remove(&key);
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
    XHigh = 1,
    High = 2,
    Thinking = 3,
    Medium = 4,
    Default = 5,
    Other = 6,
    Low = 7,
    NonReasoning = 8,
}

/// String fields that legitimately encode reasoning effort. Effort detection
/// reads only these plus the model name — NOT arbitrary string fields.
/// Scanning every string value let unrelated descriptive text drive effort:
/// a "low-latency" tagline classified the row as Low and silently dropped it,
/// and "high-throughput"/"max context" mislabeled default rows as high/max.
const EFFORT_FIELDS: &[&str] = &[
    "DisplayName",
    "display_name",
    "variant",
    "Variant",
    "effort",
    "Effort",
    "reasoning_effort",
    "ReasoningEffort",
];

impl EffortPreference {
    fn from_row(row: &RawRow) -> Self {
        let mut text = row.model_name.clone();
        for (key, value) in &row.fields {
            if !EFFORT_FIELDS.contains(&key.as_str()) {
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
            "minimal",
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
        let known_low_instant_endpoint = ["kimi k2.5 instant", "kimi k2 5 instant"]
            .iter()
            .any(|phrase| normalized.contains(phrase));
        let has_effort_marker = has_effort_marker || known_low_instant_endpoint;

        if contains("default") || !has_effort_marker {
            Self::Default
        } else if contains("non reasoning") {
            Self::NonReasoning
        } else if contains("minimal") || contains("low") || known_low_instant_endpoint {
            // `thinking-minimal` and Kimi K2.5's audited `-instant` endpoint
            // are explicitly below the default reasoning tier. `Instant` is
            // otherwise identity-bearing (for example GPT-5.5 Instant), so
            // do not interpret it as a global effort token.
            Self::Low
        } else if contains("max") {
            Self::Max
        } else if contains("xhigh") {
            Self::XHigh
        } else if contains("high") {
            Self::High
        } else if contains("medium") {
            // Explicit tiers take precedence over generic reasoning-mode
            // words, e.g. "Adaptive Reasoning, Medium Effort".
            Self::Medium
        } else if contains("thinking") || contains("reasoning") || contains("adaptive") {
            Self::Thinking
        } else {
            Self::Other
        }
    }

    /// `None` for efforts in the default scoring set, which always score.
    /// `Some(policy name)` for blocked variants, which score only when an
    /// `[effort_policy]` exception admits them.
    fn blocked_effort_name(self) -> Option<&'static str> {
        match self {
            Self::Default
            | Self::Medium
            | Self::Thinking
            | Self::High
            | Self::XHigh
            | Self::Max => None,
            Self::Low => Some("low"),
            Self::NonReasoning => Some("non reasoning"),
            Self::Other => Some("other"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EvidencePriority {
    CuratedDirect,
    Direct,
}

fn evidence_priority(record: &ModelRecord, metric: &str) -> Option<EvidencePriority> {
    record.raw_metrics.contains_key(metric).then(|| {
        if record.curated_overrides.contains(metric) {
            EvidencePriority::CuratedDirect
        } else {
            EvidencePriority::Direct
        }
    })
}

pub(crate) fn is_evidence_note_key(key: &str) -> bool {
    evidence_note_metric(key).is_some()
}

fn evidence_note_metric(key: &str) -> Option<&str> {
    key.strip_suffix(EVIDENCE_NOTE_SUFFIX)
        .filter(|metric| !metric.is_empty())
}

/// Variant policy driven by `[effort_policy]` in `coefficients.toml`. Efforts
/// in the default scoring set (`default | medium | thinking | high | xhigh |
/// max`) always score; the rest are dropped unless an exception admits them.
fn is_scoring_allowed_for(
    preference: EffortPreference,
    source_id: &str,
    canonical_id: &str,
    vendor: &Vendor,
    effort_policy: &crate::coefficients::EffortPolicy,
) -> bool {
    let Some(effort_name) = preference.blocked_effort_name() else {
        return true;
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
            assert!(!records[0].metric_citations.contains_key("TerminalBench"));
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
                .metric_citations
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
    fn winning_native_observation_retains_provenance_note() {
        let mut record = ModelRecord::new(
            "anthropic/claude-fable-5".into(),
            "claude-fable-5".into(),
            Vendor::Anthropic,
        );
        record.aliases.insert("claude-fable-5".into());
        let mut records = vec![record];
        ingest_rows(
            &mut records,
            vec![raw(
                "artificial_analysis",
                "claude-fable-5",
                &[
                    ("GPQA", json!(92.6)),
                    (
                        "GPQA__evidence_note",
                        json!("served product with automatic fallback"),
                    ),
                ],
            )],
        );
        assert_eq!(records[0].raw_metrics.get("GPQA"), Some(&92.6));
        assert_eq!(
            records[0].metric_citations.get("GPQA").map(String::as_str),
            Some("served product with automatic fallback")
        );
        assert!(!records[0].curated_overrides.contains("GPQA"));
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
    fn audit_fuzzy_matches_reports_only_non_exact_resolutions() {
        let mut record = ModelRecord::new(
            "example/mystery-preview".into(),
            "mystery-preview".into(),
            Vendor::Other("example".into()),
        );
        record.aliases.insert("mystery preview".into());
        let records = vec![record];

        let mut rows_by_source = BTreeMap::new();
        rows_by_source.insert(
            "board".to_string(),
            vec![
                // Exact/alias hit — trusted, must not be audited.
                raw("board", "mystery-preview", &[("LMArenaText", json!(80.0))]),
                // Only a fuzzy substring match — must be surfaced.
                raw(
                    "board",
                    "acme-mystery-preview",
                    &[("LMArenaText", json!(81.0))],
                ),
            ],
        );

        let audited = audit_fuzzy_matches(&rows_by_source, &records);
        assert_eq!(audited.len(), 1, "{audited:?}");
        assert_eq!(audited[0].1, "acme-mystery-preview");
        assert_eq!(audited[0].2, "example/mystery-preview");
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
    fn effort_classifier_keeps_audited_low_endpoints_out_of_max_effort_scoring() {
        for label in ["gemini-3-flash (thinking-minimal)", "kimi-k2.5-instant"] {
            let preference = EffortPreference::from_text(label);
            assert_eq!(preference, EffortPreference::Low, "label={label:?}");
            assert_eq!(
                preference.blocked_effort_name(),
                Some("low"),
                "label={label:?}"
            );
        }
    }

    #[test]
    fn effort_detection_ignores_non_effort_string_fields() {
        // A descriptive field ("low-latency ...") must not classify the row as
        // Low and get it silently dropped from scoring. Only the model name and
        // the allowlisted effort fields drive effort.
        let mut record =
            ModelRecord::new("openai/gpt-5.5".into(), "gpt-5.5".into(), Vendor::Openai);
        record.aliases.insert("gpt-5.5".into());
        let mut records = vec![record];
        let stats = ingest_rows(
            &mut records,
            vec![raw(
                "benchmark",
                "gpt-5.5",
                &[
                    ("tagline", json!("low-latency high-throughput flagship")),
                    ("TerminalBench", json!(88.0)),
                ],
            )],
        );
        assert_eq!(stats.matched, 1);
        assert_eq!(
            records[0].raw_metrics.get("TerminalBench"),
            Some(&88.0),
            "a descriptive 'low-latency' field must not drop the row as Low effort"
        );
    }

    #[test]
    fn instant_product_names_are_not_globally_treated_as_low_effort() {
        for label in ["GPT-5.5 Instant (June 2026)", "Claude Instant"] {
            let preference = EffortPreference::from_text(label);
            assert_eq!(preference, EffortPreference::Default, "label={label:?}");
            assert_eq!(preference.blocked_effort_name(), None, "label={label:?}");
        }
    }

    #[test]
    fn thinking_minimal_cannot_override_an_eligible_default_row() {
        let mut record = ModelRecord::new(
            "google/gemini-3-flash".to_string(),
            "gemini-3-flash".to_string(),
            Vendor::Google,
        );
        record.aliases.insert("gemini-3-flash".to_string());
        record
            .aliases
            .insert("gemini-3-flash-thinking-minimal".to_string());
        let mut records = vec![record];

        let stats = ingest_rows(
            &mut records,
            vec![
                raw(
                    "lmarena",
                    "gemini-3-flash-thinking-minimal",
                    &[("LMArenaText", json!(80.0))],
                ),
                raw("lmarena", "gemini-3-flash", &[("LMArenaText", json!(90.0))]),
            ],
        );

        assert_eq!(stats.matched, 2);
        assert_eq!(records[0].raw_metrics.get("LMArenaText"), Some(&90.0));
    }

    #[test]
    fn effort_classifier_prefers_explicit_medium_over_generic_reasoning_words() {
        assert_eq!(
            EffortPreference::from_text("Claude Sonnet 5 (Adaptive Reasoning, Medium Effort)"),
            EffortPreference::Medium
        );
    }

    #[test]
    fn real_rows_prefer_explicit_max_over_xhigh_regardless_of_score_or_order() {
        for max_first in [false, true] {
            let mut record = ModelRecord::new(
                "openai/gpt-5.6-sol".to_string(),
                "gpt-5.6-sol".to_string(),
                Vendor::Openai,
            );
            record.aliases.insert("gpt-5-6-sol-max".to_string());
            record.aliases.insert("gpt-5-6-sol-xhigh".to_string());
            let max = raw(
                "benchmark",
                "gpt-5-6-sol-max",
                &[("TerminalBench", json!(80.0))],
            );
            let xhigh = raw(
                "benchmark",
                "gpt-5-6-sol-xhigh",
                &[("TerminalBench", json!(90.0))],
            );
            let rows = if max_first {
                vec![max, xhigh]
            } else {
                vec![xhigh, max]
            };
            let mut records = vec![record];

            ingest_rows(&mut records, rows);

            assert_eq!(
                records[0].raw_metrics.get("TerminalBench"),
                Some(&80.0),
                "explicit max must beat a numerically higher xhigh row independently of order"
            );
        }
    }

    #[test]
    fn unlabeled_default_rows_remain_eligible() {
        let mut record = ModelRecord::new(
            "openai/gpt-5.6-sol".to_string(),
            "gpt-5.6-sol".to_string(),
            Vendor::Openai,
        );
        record.aliases.insert("gpt-5-6-sol".to_string());
        let mut records = vec![record];

        let stats = ingest_rows(
            &mut records,
            vec![raw(
                "benchmark-without-effort-metadata",
                "gpt-5-6-sol",
                &[("TerminalBench", json!(88.0))],
            )],
        );

        assert_eq!(stats.matched, 1);
        assert_eq!(records[0].raw_metrics.get("TerminalBench"), Some(&88.0));
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
            let rows = vec![
                raw("overrides", canonical, &[("SWEBenchPro", json!(60.6))]),
                raw("lmarena", preview_alias, &[("LMArenaText", json!(91.0))]),
                raw("swerebench", display, &[("SWERebench", json!(72.0))]),
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
        }
    }
}
