#![allow(dead_code)]

//! Strict round-trip regression for the public scoreboard schema v2.1.

use std::collections::BTreeMap;

use ipbr_core::{
    Coefficients, ModelRecord, SourceSummary, ThinkingEffort, Vendor, compute_scores_with,
};
use ipbr_render::{Scoreboard, toml_output::write_scoreboard};
use serde::Deserialize;
use tempfile::tempdir;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Schema21 {
    schema_version: String,
    generated_at: String,
    generator: String,
    methodology: String,
    configuration_policy: String,
    sources: BTreeMap<String, SourceTable>,
    models: Vec<Model21>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceTable {
    status: String,
    n_rows_ingested: usize,
    n_rows_matched: usize,
    n_rows_unmatched: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Model21 {
    canonical_id: String,
    display_name: String,
    vendor: String,
    thinking_effort: String,
    aliases: Vec<String>,
    sources: Vec<String>,
    scores: Scores21,
    groups: BTreeMap<String, f64>,
    metrics: BTreeMap<String, f64>,
    raw_metrics: BTreeMap<String, f64>,
    metric_evidence: BTreeMap<String, MetricEvidence>,
    evidence: EvidenceTables,
    missing: Missing21,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scores21 {
    i_raw: f64,
    p_raw: f64,
    b_raw: f64,
    r: f64,
    i_status: String,
    p_status: String,
    b_status: String,
    r_status: String,
    balanced_status: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricEvidence {
    class: String,
    source: Option<String>,
    citation: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceTables {
    groups: BTreeMap<String, Coverage>,
    roles: BTreeMap<String, Coverage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    direct: f64,
    reported: f64,
    missing: f64,
    effective: f64,
    family_count: usize,
    direct_families: Vec<String>,
    #[serde(default)]
    core_direct: Option<f64>,
    #[serde(default)]
    core_family_count: Option<usize>,
    #[serde(default)]
    core_direct_families: Vec<String>,
    #[serde(default)]
    historical_family_count: Option<usize>,
    #[serde(default)]
    historical_direct_families: Vec<String>,
    #[serde(default)]
    qualification_path: Option<String>,
    #[serde(default)]
    provisional: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Missing21 {
    metrics: Vec<String>,
    groups_shrunk: Vec<String>,
}

#[test]
fn rendered_scoreboard_round_trips_through_schema_v2_1() {
    let coefficients = Coefficients::load_embedded().expect("embedded coefficients should parse");

    let mut anthropic = ModelRecord::new(
        "anthropic/claude-opus-4.7".to_string(),
        "Claude Opus 4.7".to_string(),
        Vendor::Anthropic,
    );
    anthropic.aliases.insert("claude-opus-4-7".to_string());
    anthropic.sources.extend([
        "lmarena".to_string(),
        "overrides".to_string(),
        "terminal_bench".to_string(),
    ]);
    anthropic
        .raw_metrics
        .insert("LMArenaText".to_string(), 1450.0);
    anthropic
        .metric_sources
        .insert("LMArenaText".to_string(), "lmarena".to_string());
    anthropic
        .raw_metrics
        .insert("SWEBenchVerified".to_string(), 80.0);
    anthropic
        .curated_overrides
        .insert("SWEBenchVerified".to_string());
    anthropic
        .metric_sources
        .insert("SWEBenchVerified".to_string(), "overrides".to_string());
    anthropic.override_notes.insert(
        "SWEBenchVerified".to_string(),
        "Anthropic system card, table 8".to_string(),
    );
    anthropic
        .raw_metrics
        .insert("TerminalBench".to_string(), 75.0);
    anthropic
        .metric_sources
        .insert("TerminalBench".to_string(), "terminal_bench".to_string());
    anthropic
        .raw_metrics
        .insert("TerminalBenchUncertainty".to_string(), 1.25);
    anthropic.metric_sources.insert(
        "TerminalBenchUncertainty".to_string(),
        "terminal_bench".to_string(),
    );

    let mut openai = ModelRecord::new(
        "openai/gpt-5.5".to_string(),
        "GPT-5.5".to_string(),
        Vendor::Openai,
    );
    openai.thinking_effort = Some(ThinkingEffort::Medium);
    openai.aliases.insert("gpt-5-5".to_string());
    openai.sources.insert("lmarena".to_string());
    openai.raw_metrics.insert("LMArenaText".to_string(), 1500.0);
    openai
        .metric_sources
        .insert("LMArenaText".to_string(), "lmarena".to_string());

    let mut models = vec![openai, anthropic];
    compute_scores_with(&mut models, &coefficients);

    let scoreboard = Scoreboard {
        models,
        coefficients,
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        generator: "ipbr-rank 0.1.0".to_string(),
        methodology: "v3".to_string(),
        source_summary: [(
            "lmarena".to_string(),
            SourceSummary {
                status: "verified".to_string(),
                rows: 2,
                matched: 2,
                unmatched: 0,
            },
        )]
        .into_iter()
        .collect(),
        prev_scores: None,
    };

    let tmp = tempdir().expect("tempdir should be created");
    write_scoreboard(&scoreboard, tmp.path()).expect("scoreboard should render");
    let rendered = std::fs::read_to_string(tmp.path().join("scoreboard.toml"))
        .expect("scoreboard.toml should be written");

    let parsed: Schema21 =
        toml::from_str(&rendered).expect("rendered TOML must match schema v2.1 exactly");
    assert_eq!(parsed.schema_version, "2.1.0");
    assert_eq!(parsed.generated_at, "2026-01-01T00:00:00Z");
    assert_eq!(parsed.generator, "ipbr-rank 0.1.0");
    assert_eq!(parsed.methodology, "v3");
    assert_eq!(parsed.configuration_policy, "best_available_max_effort");
    assert_eq!(parsed.sources["lmarena"].n_rows_matched, 2);
    assert_eq!(parsed.models.len(), 2);

    let anthropic = parsed
        .models
        .iter()
        .find(|model| model.canonical_id == "anthropic/claude-opus-4.7")
        .expect("Anthropic model present");
    assert_eq!(anthropic.display_name, "Claude Opus 4.7");
    assert_eq!(anthropic.vendor, "anthropic");
    assert_eq!(anthropic.thinking_effort, "best_available");
    assert_eq!(anthropic.aliases, vec!["claude-opus-4-7".to_string()]);
    assert_eq!(anthropic.raw_metrics["SWEBenchVerified"], 80.0);
    assert_eq!(anthropic.raw_metrics["TerminalBenchUncertainty"], 1.25);

    let direct = &anthropic.metric_evidence["LMArenaText"];
    assert_eq!(direct.class, "direct");
    assert_eq!(direct.source.as_deref(), Some("lmarena"));

    let curated = &anthropic.metric_evidence["SWEBenchVerified"];
    assert_eq!(curated.class, "direct");
    assert_eq!(curated.source.as_deref(), Some("overrides"));
    assert_eq!(
        curated.citation.as_deref(),
        Some("Anthropic system card, table 8")
    );

    let terminal = &anthropic.metric_evidence["TerminalBench"];
    assert_eq!(terminal.class, "direct");
    assert_eq!(terminal.source.as_deref(), Some("terminal_bench"));

    let uncertainty = &anthropic.metric_evidence["TerminalBenchUncertainty"];
    assert_eq!(uncertainty.class, "direct");
    assert_eq!(uncertainty.source.as_deref(), Some("terminal_bench"));

    assert!(!anthropic.evidence.groups.is_empty());
    assert_eq!(anthropic.evidence.roles.len(), 4);
    let idea_evidence = &anthropic.evidence.roles["I_raw"];
    assert!(idea_evidence.core_direct.is_some());
    assert!(idea_evidence.core_family_count.is_some());
    assert!(idea_evidence.qualification_path.is_some());
    assert_eq!(
        idea_evidence.core_family_count,
        Some(idea_evidence.core_direct_families.len())
    );
    assert_eq!(
        idea_evidence.historical_family_count,
        Some(idea_evidence.historical_direct_families.len())
    );
    assert_eq!(
        anthropic.scores.i_status,
        if anthropic.evidence.roles["I_raw"].provisional == Some(true) {
            "provisional"
        } else {
            "ranked"
        }
    );
    assert!(matches!(
        anthropic.scores.balanced_status.as_str(),
        "ranked" | "provisional"
    ));
    assert!(
        anthropic
            .missing
            .groups_shrunk
            .windows(2)
            .all(|pair| pair[0] <= pair[1])
    );

    let openai = parsed
        .models
        .iter()
        .find(|model| model.canonical_id == "openai/gpt-5.5")
        .expect("OpenAI model present");
    assert_eq!(openai.thinking_effort, "medium");
}
