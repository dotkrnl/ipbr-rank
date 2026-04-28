use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ipbr_core::{
    Coefficients, ModelRecord, RawRow, SourceSummary, ThinkingEffort, Vendor, compute_scores_with,
    ingest_rows, load_embedded_pairs, required_aliases, synthesize_rows,
};
use ipbr_render::{
    Scoreboard,
    toml_output::{write_coefficients, write_missing, write_scoreboard},
};
use ipbr_sources::{Http, SourceError, registry::registry};
use tempfile::tempdir;

struct OfflineOnlyHttp;

#[async_trait::async_trait]
impl Http for OfflineOnlyHttp {
    async fn get_json(
        &self,
        _url: &str,
        _headers: &[(&str, &str)],
    ) -> Result<serde_json::Value, SourceError> {
        panic!("offline fixture tests must not hit the network")
    }

    async fn get_text(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<String, SourceError> {
        panic!("offline fixture tests must not hit the network")
    }
}

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
    assert!(rendered.contains("[models.missing]"));

    let parsed: toml::Value = toml::from_str(&rendered).expect("rendered TOML should parse");
    assert_eq!(parsed["schema_version"].as_str(), Some("1.1.0"));
    assert_eq!(
        parsed["generated_at"].as_str(),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(parsed["models"].as_array().map(std::vec::Vec::len), Some(2));
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
fn missing_output_marks_groups_shrunk_at_actual_threshold() {
    let coefficients = Coefficients::load_embedded().expect("embedded coefficients should parse");
    let mut model = ModelRecord::new(
        "test/model".to_string(),
        "Test Model".to_string(),
        Vendor::Other("test".to_string()),
    );
    model
        .metrics
        .insert("LMArenaCreativeOrOpenEnded".to_string(), 80.0);
    let scoreboard = Scoreboard {
        models: vec![model],
        coefficients,
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        generator: "ipbr-rank 0.1.0".to_string(),
        methodology: "v1".to_string(),
        source_summary: BTreeMap::new(),
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
        "CRE has only 65% coverage and should be marked shrunk: {missing}"
    );
}

#[tokio::test]
async fn golden_scoreboard_matches_fixture_pipeline() {
    let tmp = tempdir().expect("tempdir should be created");
    let out_dir = tmp.path().join("out");
    let scoreboard = fixture_scoreboard("2026-01-01T00:00:00Z")
        .await
        .expect("fixture scoreboard should build");

    write_scoreboard(&scoreboard, &out_dir).expect("scoreboard should render");
    let rendered = std::fs::read_to_string(out_dir.join("scoreboard.toml"))
        .expect("rendered scoreboard should exist");
    toml::from_str::<toml::Value>(&rendered).expect("golden output should parse");

    let golden_path = repo_root().join("tests/golden/scoreboard.toml");
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        if let Some(parent) = golden_path.parent() {
            std::fs::create_dir_all(parent).expect("golden parent should exist");
        }
        std::fs::write(&golden_path, &rendered).expect("golden file should be updated");
    }

    let expected =
        std::fs::read_to_string(&golden_path).expect("golden scoreboard.toml must be present");
    assert_eq!(rendered, expected, "golden scoreboard drifted");
}

fn sample_scoreboard() -> Scoreboard {
    let coefficients = Coefficients::load_embedded().expect("embedded coefficients should parse");
    let mut model_b = ModelRecord::new(
        "openai/gpt-5.5".to_string(),
        "GPT-5.5".to_string(),
        Vendor::Openai,
    );
    model_b.thinking_effort = Some(ThinkingEffort::High);
    model_b.aliases.insert("gpt-5-5".to_string());
    model_b.aliases.insert("gpt-5.5".to_string());
    model_b.sources.insert("openrouter".to_string());
    model_b
        .raw_metrics
        .insert("AI_correctness".to_string(), 91.0);

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
        .insert("AI_correctness".to_string(), 88.0);

    let mut models = vec![model_b, model_a];
    compute_scores_with(&mut models, &coefficients);

    Scoreboard {
        models,
        coefficients,
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        generator: "ipbr-rank 0.1.0".to_string(),
        methodology: "v1".to_string(),
        source_summary: BTreeMap::new(),
    }
}

async fn fixture_scoreboard(now: &str) -> Result<Scoreboard, SourceError> {
    let fixture_dir = repo_root().join("data/fixtures");
    let mut records = required_aliases::load_embedded().expect("embedded aliases should load");
    let coefficients = Coefficients::load_embedded().expect("embedded coefficients should parse");
    let mut source_summary = BTreeMap::new();
    let http = OfflineOnlyHttp;
    let synthesis_pairs = load_embedded_pairs().expect("embedded synthesis aliases should load");
    let synthesis_cfg = coefficients.synthesis.clone().unwrap_or_default();
    let mut rows_by_source: BTreeMap<String, Vec<RawRow>> = BTreeMap::new();
    let mut fetched_rows: BTreeMap<String, usize> = BTreeMap::new();
    let mut fetched_statuses: BTreeMap<String, String> = BTreeMap::new();

    for source in registry() {
        let rows = source
            .fetch(
                &http,
                ipbr_sources::FetchOptions {
                    cache_dir: Some(&fixture_dir),
                    offline: true,
                },
                &ipbr_sources::SecretStore::default(),
            )
            .await?;
        fetched_rows.insert(source.id().to_string(), rows.len());
        fetched_statuses.insert(
            source.id().to_string(),
            format!("{:?}", source.status()).to_lowercase(),
        );
        rows_by_source.insert(source.id().to_string(), rows);
    }

    let _ = synthesize_rows(
        &mut rows_by_source,
        &synthesis_pairs,
        &records,
        &synthesis_cfg,
    );

    for (source_id, rows) in rows_by_source {
        let row_count = fetched_rows.get(&source_id).copied().unwrap_or(rows.len());
        let status = fetched_statuses
            .get(&source_id)
            .cloned()
            .unwrap_or_else(|| "verified".to_string());
        let stats = ingest_rows(&mut records, rows);
        source_summary.insert(
            source_id,
            SourceSummary {
                status,
                rows: row_count,
                matched: stats.matched,
                unmatched: stats.unmatched.len(),
            },
        );
    }

    ipbr_core::ingest::mark_synthesis_dominant(&mut records, synthesis_cfg.per_model_cap);
    compute_scores_with(&mut records, &coefficients);

    Ok(Scoreboard {
        models: records,
        coefficients,
        generated_at: now.to_string(),
        generator: "ipbr-rank 0.1.0".to_string(),
        methodology: "v1".to_string(),
        source_summary,
    })
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should exist")
}
