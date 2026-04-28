#![allow(dead_code)]

//! Round-trip regression for the documented 1.1.0 scoreboard schema.
//!
//! Builds a `Scoreboard` containing both real and synthesized cells,
//! serializes it via `ipbr_render::toml_output::write_scoreboard`, and
//! re-parses the rendered TOML through a fresh `serde::Deserialize`
//! struct that mirrors `docs/output-schema.md`. Every documented field
//! must round-trip. A second deserializer that only knows about the
//! 1.0.0 shape — i.e. one that ignores `synthesized` /
//! `synthesis_dominant` — must still parse the same output unchanged,
//! verifying the bump is additive and downstream consumers gating on
//! the major version stay compatible.

use std::collections::BTreeMap;

use ipbr_core::{
    Coefficients, MissingInfo, ModelRecord, SynthesisProvenance, Vendor, compute_scores_with,
};
use ipbr_render::{Scoreboard, toml_output::write_scoreboard};
use serde::Deserialize;
use tempfile::tempdir;

#[derive(Deserialize)]
struct Schema11 {
    schema_version: String,
    generated_at: String,
    generator: String,
    methodology: String,
    #[serde(default)]
    sources: BTreeMap<String, SourceTable>,
    models: Vec<Model11>,
}

#[derive(Deserialize)]
struct SourceTable {
    status: String,
    n_rows_ingested: usize,
    n_rows_matched: usize,
    n_rows_unmatched: usize,
}

#[derive(Deserialize)]
struct Model11 {
    canonical_id: String,
    display_name: String,
    vendor: String,
    thinking_effort: String,
    aliases: Vec<String>,
    sources: Vec<String>,
    scores: Scores,
    groups: BTreeMap<String, f64>,
    metrics: BTreeMap<String, f64>,
    missing: Missing11,
    #[serde(default)]
    synthesized: BTreeMap<String, Provenance>,
}

#[derive(Deserialize)]
struct Scores {
    i_raw: f64,
    p_raw: f64,
    b_raw: f64,
    r: f64,
    i_adj: f64,
    p_adj: f64,
    b_adj: f64,
}

#[derive(Deserialize)]
struct Missing11 {
    metrics: Vec<String>,
    groups_shrunk: Vec<String>,
    synthesis_dominant: bool,
}

#[derive(Deserialize, PartialEq, Debug)]
struct Provenance {
    source: String,
    from: String,
}

/// 1.0.0-shape consumer: matches a `1.x.x` major version, ignores
/// 1.1.0-specific fields. Used to assert the bump is additive.
#[derive(Deserialize)]
struct Schema10 {
    schema_version: String,
    models: Vec<Model10>,
}

#[derive(Deserialize)]
struct Model10 {
    canonical_id: String,
    metrics: BTreeMap<String, f64>,
    missing: Missing10,
}

#[derive(Deserialize)]
struct Missing10 {
    metrics: Vec<String>,
    groups_shrunk: Vec<String>,
}

#[test]
fn rendered_scoreboard_round_trips_through_documented_schema() {
    let coefficients = Coefficients::load_embedded().expect("embedded coefficients should parse");

    let mut real = ModelRecord::new(
        "anthropic/claude-opus-4.7".to_string(),
        "Claude Opus 4.7".to_string(),
        Vendor::Anthropic,
    );
    real.aliases.insert("claude-opus-4-7".to_string());
    real.sources.insert("lmarena".to_string());
    real.metrics.insert("LMArenaText".to_string(), 81.0);
    real.metrics.insert("SWEBenchVerified".to_string(), 76.5);
    real.groups.insert("CRE".to_string(), 80.0);
    real.missing = MissingInfo {
        metrics: ["NoveltyBench".to_string()].into_iter().collect(),
        groups_shrunk: ["LM_ARENA_REVIEW_PROXY".to_string()].into_iter().collect(),
        synthesis_dominant: false,
    };

    let mut synth = ModelRecord::new(
        "openai/gpt-5.5".to_string(),
        "GPT-5.5".to_string(),
        Vendor::Openai,
    );
    synth.aliases.insert("gpt-5-5".to_string());
    synth.sources.insert("openrouter".to_string());
    synth.metrics.insert("LMArenaText".to_string(), 79.5);
    synth.metrics.insert("SWEBenchVerified".to_string(), 70.25);
    synth.groups.insert("CRE".to_string(), 78.0);
    synth.synthesized.insert(
        "SWEBenchVerified".to_string(),
        SynthesisProvenance {
            source_id: "openrouter".to_string(),
            from: "openai/gpt-5.4".to_string(),
        },
    );
    synth.missing = MissingInfo {
        metrics: Default::default(),
        groups_shrunk: Default::default(),
        synthesis_dominant: true,
    };

    let mut models = vec![real, synth];
    compute_scores_with(&mut models, &coefficients);

    let scoreboard = Scoreboard {
        models,
        coefficients,
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        generator: "ipbr-rank 0.1.0".to_string(),
        methodology: "v1".to_string(),
        source_summary: BTreeMap::new(),
    };

    let tmp = tempdir().expect("tempdir should be created");
    write_scoreboard(&scoreboard, tmp.path()).expect("scoreboard should render");
    let rendered = std::fs::read_to_string(tmp.path().join("scoreboard.toml"))
        .expect("scoreboard.toml should be written");

    let parsed: Schema11 =
        toml::from_str(&rendered).expect("rendered TOML must match documented 1.1.0 shape");

    assert_eq!(parsed.schema_version, "1.1.0");
    assert_eq!(parsed.generated_at, "2026-01-01T00:00:00Z");
    assert_eq!(parsed.generator, "ipbr-rank 0.1.0");
    assert_eq!(parsed.methodology, "v1");
    assert!(parsed.sources.is_empty());
    assert_eq!(parsed.models.len(), 2);

    let real = parsed
        .models
        .iter()
        .find(|m| m.canonical_id == "anthropic/claude-opus-4.7")
        .expect("real model present");
    assert_eq!(real.display_name, "Claude Opus 4.7");
    assert_eq!(real.vendor, "anthropic");
    assert_eq!(real.thinking_effort, "default");
    assert_eq!(real.aliases, vec!["claude-opus-4-7".to_string()]);
    assert_eq!(real.sources, vec!["lmarena".to_string()]);
    assert!((real.metrics["LMArenaText"] - 81.0).abs() < 1e-9);
    assert!((real.metrics["SWEBenchVerified"] - 76.5).abs() < 1e-9);
    assert_eq!(real.missing.metrics, vec!["NoveltyBench".to_string()]);
    // `groups_shrunk` is recomputed by the renderer from coefficient
    // weights, so we only assert it round-trips as a sorted string array.
    let mut shrunk = real.missing.groups_shrunk.clone();
    let was_sorted = shrunk.windows(2).all(|w| w[0] <= w[1]);
    shrunk.sort();
    shrunk.dedup();
    assert_eq!(shrunk.len(), real.missing.groups_shrunk.len());
    assert!(was_sorted, "groups_shrunk must be emitted sorted");
    assert!(!real.missing.synthesis_dominant);
    assert!(
        real.synthesized.is_empty(),
        "real model has no synthesized entries"
    );
    // Sanity-check role scores are real f64s (deserialized at all).
    let _ = real.scores.i_raw + real.scores.b_adj + real.scores.r;

    let synth = parsed
        .models
        .iter()
        .find(|m| m.canonical_id == "openai/gpt-5.5")
        .expect("synth model present");
    assert!(synth.missing.synthesis_dominant);
    assert_eq!(synth.synthesized.len(), 1);
    assert_eq!(
        synth.synthesized.get("SWEBenchVerified"),
        Some(&Provenance {
            source: "openrouter".to_string(),
            from: "openai/gpt-5.4".to_string(),
        })
    );
    assert!(
        !synth.metrics.contains_key("Synthesized"),
        "synthesized values stay in [models.metrics] under their real key"
    );

    let legacy: Schema10 =
        toml::from_str(&rendered).expect("1.0.0-shape consumer must parse 1.1.0 output unchanged");
    assert!(
        legacy.schema_version.starts_with("1."),
        "consumers gating on major version 1.x.x parse 1.1.0"
    );
    assert_eq!(legacy.models.len(), 2);
    let legacy_synth = legacy
        .models
        .iter()
        .find(|m| m.canonical_id == "openai/gpt-5.5")
        .expect("synth model present in legacy view");
    assert_eq!(
        legacy_synth.metrics.get("SWEBenchVerified").copied(),
        Some(70.25)
    );
    assert!(legacy_synth.missing.metrics.is_empty());
}
