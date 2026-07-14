use std::collections::BTreeMap;

use ipbr_core::{Coefficients, ModelRecord, Vendor, compute_scores_with};
use ipbr_render::{
    Scoreboard,
    toml_output::{write_coefficients, write_missing, write_scoreboard},
};
use tempfile::tempdir;

#[test]
fn writes_valid_nested_scoreboard_toml() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");

    write_scoreboard(&scoreboard, tmp.path()).expect("scoreboard should render");
    let rendered = std::fs::read_to_string(tmp.path().join("scoreboard.toml"))
        .expect("scoreboard.toml should exist");

    assert!(rendered.contains("[[models]]"));
    assert!(rendered.contains("[models.scores]"));
    assert!(rendered.contains("[models.groups]"));
    assert!(rendered.contains("[models.metrics]"));
    assert!(rendered.contains("[models.raw_metrics]"));
    assert!(rendered.contains("[models.metric_evidence."));
    assert!(rendered.contains("[models.evidence.groups."));
    assert!(rendered.contains("[models.evidence.roles."));
    assert!(rendered.contains("[models.missing]"));

    let parsed: toml::Value = toml::from_str(&rendered).expect("rendered TOML should parse");
    assert_eq!(parsed["schema_version"].as_str(), Some("2.1.0"));
    assert_eq!(parsed["methodology"].as_str(), Some("v3"));
    assert_eq!(
        parsed["configuration_policy"].as_str(),
        Some("best_available_max_effort")
    );
    assert_eq!(
        parsed["generated_at"].as_str(),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(parsed["models"].as_array().map(std::vec::Vec::len), Some(2));

    let models = parsed["models"].as_array().unwrap();
    let anthropic = models
        .iter()
        .find(|model| model["canonical_id"].as_str() == Some("anthropic/claude-opus-4.7"))
        .unwrap();
    assert_eq!(
        anthropic["thinking_effort"].as_str(),
        Some("best_available")
    );
    assert_eq!(
        anthropic["scores"]["i_status"].as_str(),
        Some("provisional")
    );
    assert!(matches!(
        anthropic["scores"]["balanced_status"].as_str(),
        Some("ranked" | "provisional")
    ));
    assert!(anthropic["scores"].get("i_adj").is_none());
    assert_eq!(
        anthropic["raw_metrics"]["SWEBenchVerified"].as_float(),
        Some(80.0)
    );
    assert_eq!(
        anthropic["metric_evidence"]["SWEBenchVerified"]["class"].as_str(),
        Some("direct")
    );
    assert_eq!(
        anthropic["metric_evidence"]["SWEBenchVerified"]["citation"].as_str(),
        Some("Anthropic system card")
    );
    assert_eq!(
        anthropic["metric_evidence"]["TerminalBench"]["class"].as_str(),
        Some("direct")
    );
    assert!(anthropic["evidence"]["groups"].as_table().is_some());
    assert!(anthropic["evidence"]["roles"].as_table().is_some());
    let idea_evidence = &anthropic["evidence"]["roles"]["I_raw"];
    assert!(idea_evidence["core_direct"].as_float().is_some());
    assert!(idea_evidence["core_family_count"].as_integer().is_some());
    assert!(idea_evidence["core_direct_families"].as_array().is_some());
    assert!(
        idea_evidence["historical_family_count"]
            .as_integer()
            .is_some()
    );
    assert!(
        idea_evidence["historical_direct_families"]
            .as_array()
            .is_some()
    );
    assert!(idea_evidence["qualification_path"].as_str().is_some());

    let openai = models
        .iter()
        .find(|model| model["canonical_id"].as_str() == Some("openai/gpt-5.5"))
        .unwrap();
    assert_eq!(openai["thinking_effort"].as_str(), Some("best_available"));
    assert_eq!(
        openai["raw_metrics"]["TerminalBenchUncertainty"].as_float(),
        Some(1.25)
    );
    assert_eq!(
        openai["metric_evidence"]["TerminalBenchUncertainty"]["source"].as_str(),
        Some("terminal_bench")
    );
}

#[test]
fn renders_missing_and_coefficients_toml() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");

    write_missing(&scoreboard, tmp.path()).expect("missing output should render");
    write_coefficients(&scoreboard.coefficients, tmp.path()).expect("coefficients should render");

    let missing = std::fs::read_to_string(tmp.path().join("missing.toml"))
        .expect("missing.toml should exist");
    let coefficients = std::fs::read_to_string(tmp.path().join("coefficients.toml"))
        .expect("coefficients.toml should exist");

    let missing_value: toml::Value = toml::from_str(&missing).expect("missing TOML should parse");
    assert!(
        missing_value["models"]
            .as_table()
            .is_some_and(|models| models.contains_key("anthropic/claude-opus-4.7"))
    );

    let coefficients_value: toml::Value =
        toml::from_str(&coefficients).expect("coefficients TOML should parse");
    assert!(
        coefficients_value["metrics"]
            .as_table()
            .is_some_and(|m| !m.is_empty()),
        "expected metric definitions in rendered coefficients"
    );
}

#[test]
fn missing_output_uses_current_core_missing_diagnostics() {
    let coefficients = Coefficients::load_embedded().expect("embedded coefficients should parse");
    let mut model = ModelRecord::new(
        "test/model".to_string(),
        "Test Model".to_string(),
        Vendor::Other("test".to_string()),
    );
    model.raw_metrics.insert("LMArenaText".to_string(), 1400.0);
    model
        .metric_sources
        .insert("LMArenaText".to_string(), "lmarena".to_string());
    compute_scores_with(std::slice::from_mut(&mut model), &coefficients);
    let scoreboard = Scoreboard {
        models: vec![model],
        coefficients,
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        generator: "ipbr-rank 0.1.0".to_string(),
        methodology: "v3".to_string(),
        source_summary: BTreeMap::new(),
        prev_scores: None,
    };
    let tmp = tempdir().expect("tempdir should be created");

    write_missing(&scoreboard, tmp.path()).expect("missing output should render");

    let missing = std::fs::read_to_string(tmp.path().join("missing.toml"))
        .expect("missing.toml should exist");
    let missing_value: toml::Value = toml::from_str(&missing).expect("missing TOML should parse");
    let groups = missing_value["models"]["test/model"]["groups_shrunk"]
        .as_array()
        .expect("groups_shrunk should be an array");
    assert!(
        groups.iter().any(|group| group.as_str() == Some("CRE")),
        "core evidence marks the partially observed CRE group: {missing}"
    );
}

fn sample_scoreboard() -> Scoreboard {
    let coefficients = Coefficients::load_embedded().expect("embedded coefficients should parse");
    let mut model_b = ModelRecord::new(
        "openai/gpt-5.5".to_string(),
        "GPT-5.5".to_string(),
        Vendor::Openai,
    );
    model_b.aliases.insert("gpt-5-5".to_string());
    model_b.aliases.insert("gpt-5.5".to_string());
    model_b.sources.insert("openrouter".to_string());
    model_b
        .raw_metrics
        .insert("LMArenaText".to_string(), 1450.0);
    model_b
        .metric_sources
        .insert("LMArenaText".to_string(), "lmarena".to_string());
    model_b
        .raw_metrics
        .insert("TerminalBenchUncertainty".to_string(), 1.25);
    model_b.metric_sources.insert(
        "TerminalBenchUncertainty".to_string(),
        "terminal_bench".to_string(),
    );

    let mut model_a = ModelRecord::new(
        "anthropic/claude-opus-4.7".to_string(),
        "Claude Opus 4.7".to_string(),
        Vendor::Anthropic,
    );
    model_a.aliases.insert("claude-opus-4-7".to_string());
    model_a.aliases.insert("opus 4.7".to_string());
    model_a.sources.insert("lmarena".to_string());
    model_a
        .raw_metrics
        .insert("LMArenaText".to_string(), 1400.0);
    model_a
        .metric_sources
        .insert("LMArenaText".to_string(), "lmarena".to_string());
    model_a
        .raw_metrics
        .insert("SWEBenchVerified".to_string(), 80.0);
    model_a
        .curated_overrides
        .insert("SWEBenchVerified".to_string());
    model_a
        .metric_sources
        .insert("SWEBenchVerified".to_string(), "overrides".to_string());
    model_a.metric_citations.insert(
        "SWEBenchVerified".to_string(),
        "Anthropic system card".to_string(),
    );
    model_a
        .raw_metrics
        .insert("TerminalBench".to_string(), 75.0);
    model_a
        .metric_sources
        .insert("TerminalBench".to_string(), "terminal_bench".to_string());

    let mut models = vec![model_b, model_a];
    compute_scores_with(&mut models, &coefficients);

    Scoreboard {
        models,
        coefficients,
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        generator: "ipbr-rank 0.1.0".to_string(),
        methodology: "v3".to_string(),
        source_summary: BTreeMap::new(),
        prev_scores: None,
    }
}
