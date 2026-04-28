use std::path::Path;

use ipbr_core::{ModelRecord, ingest_rows, required_aliases};
use ipbr_sources::{
    AiStupidLevelSource, ArtificialAnalysisSource, FetchOptions, Http, LiveCodeBenchSource,
    LmArenaSource, OpenRouterSource, SecretStore, Source, SourceError, SweBenchSource,
    SweRebenchSource, VerificationStatus, registry::registry,
};

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

fn fixture_dir() -> &'static Path {
    Box::leak(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/fixtures")
            .into_boxed_path(),
    )
}

fn ingest_fixture_rows(rows: Vec<ipbr_core::RawRow>) -> (Vec<ModelRecord>, usize) {
    let mut records = required_aliases::load_embedded().expect("embedded aliases must load");
    let stats = ingest_rows(&mut records, rows);
    (records, stats.matched)
}

const FLAGSHIPS: &[&str] = &[
    "openai/gpt-5.5",
    "openai/gpt-5.4",
    "openai/gpt-5.3-codex",
    "anthropic/claude-sonnet-4",
    "anthropic/claude-sonnet-4.5",
    "anthropic/claude-opus-4.6",
    "anthropic/claude-opus-4.7",
    "google/gemini-3.1-pro-preview",
    "google/gemini-3-pro",
    "google/gemini-3-flash",
    "google/gemini-2.5-flash",
    "google/gemini-2.5-pro",
    "moonshotai/kimi-k2.6",
    "z-ai/glm-5.1",
];

fn assert_flagship_matches(records: &[ModelRecord], metric: &str, min_hits: usize) {
    let flagship_hits = FLAGSHIPS
        .iter()
        .filter(|&&id| {
            records
                .iter()
                .any(|r| r.canonical_id == id && r.raw_metrics.contains_key(metric))
        })
        .count();
    assert!(
        flagship_hits >= min_hits,
        "expected >={min_hits}/14 flagship {metric} matches, got {flagship_hits}/14"
    );
    assert!(
        records
            .iter()
            .any(|r| r.canonical_id == "anthropic/claude-opus-4.7"
                && r.raw_metrics.contains_key(metric)),
        "claude-opus-4.7 must receive a {metric} score"
    );
}

#[tokio::test]
async fn openrouter_fixture_contract() {
    let source = OpenRouterSource;
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: Some(fixture_dir()),
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("openrouter fixture should parse");

    assert!(rows.len() >= 200, "expected a sizable model catalog");
    let context_rows = rows
        .iter()
        .filter(|row| row.fields.contains_key("ContextWindow"))
        .count();
    assert!(context_rows >= 50, "expected many context-window metrics");
    let cost_rows = rows
        .iter()
        .filter(|row| row.fields.contains_key("InverseCost"))
        .count();
    assert!(cost_rows >= 50, "expected many cost metrics");

    let recognized_rows = rows
        .iter()
        .filter(|row| row.vendor_hint.is_some() && row.model_name.contains('/'))
        .count();
    assert!(
        recognized_rows >= 50,
        "expected at least 50 source-recognized models, got {recognized_rows}"
    );

    let (records, matched) = ingest_fixture_rows(rows);
    assert!(
        matched >= 20,
        "expected at least 20 alias matches, got {matched}"
    );
    assert!(records.iter().any(|record| {
        record.canonical_id == "anthropic/claude-opus-4.7"
            && record.raw_metrics.contains_key("ContextWindow")
    }));
}

#[tokio::test]
async fn lmarena_fixture_contract() {
    let source = LmArenaSource;
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: Some(fixture_dir()),
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("lmarena fixture should parse");

    assert!(
        rows.len() >= 50,
        "expected the text fixture page to contain many rows"
    );
    // The fixture now spans all four configs (text / webdev / search / document),
    // and a row that only appears in search/document carries
    // LMArenaSearchDocument but no LMArenaText. So assert most-rows-text
    // rather than every-row.
    let text_rows = rows
        .iter()
        .filter(|row| row.fields.contains_key("LMArenaText"))
        .count();
    assert!(
        text_rows >= 50,
        "expected ≥50 rows to carry LMArenaText, got {text_rows}"
    );
    assert!(rows.iter().any(|row| {
        row.model_name.contains("thinking") && row.fields.contains_key("LMArenaCreativeOrOpenEnded")
    }));
    assert!(
        rows.iter()
            .any(|row| row.fields.contains_key("LMArenaSearchDocument")),
        "expected at least one search/document row in the fixture",
    );

    let (records, matched) = ingest_fixture_rows(rows);
    assert!(
        matched >= 20,
        "expected at least 20 known models, got {matched}"
    );
    assert!(records.iter().any(|record| {
        record.raw_metrics.contains_key("LMArenaText")
            || record
                .raw_metrics
                .contains_key("LMArenaCreativeOrOpenEnded")
    }));
}

#[tokio::test]
async fn artificial_analysis_fixture_contract() {
    let source = ArtificialAnalysisSource;
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: Some(fixture_dir()),
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("artificial analysis fixture should parse");

    assert!(
        rows.len() >= 10,
        "expected at least 10 fixture rows, got {}",
        rows.len()
    );
    assert!(rows.iter().all(|row| {
        row.fields.contains_key("ArtificialAnalysisIntelligence")
            && row.fields.contains_key("ArtificialAnalysisCoding")
            && row.fields.contains_key("OutputSpeed")
            && row.fields.contains_key("InverseTTFT")
            && row.fields.contains_key("InverseCost")
    }));
    // Reasoning is now blended from gpqa+hle, so it's only emitted when at
    // least one of those is present in the upstream payload. We require
    // that *most* rows carry it, not all.
    let reasoning_rows = rows
        .iter()
        .filter(|row| row.fields.contains_key("ArtificialAnalysisReasoning"))
        .count();
    assert!(
        reasoning_rows * 2 >= rows.len(),
        "expected reasoning blend on majority of rows, got {reasoning_rows}/{}",
        rows.len()
    );

    let (records, matched) = ingest_fixture_rows(rows);
    assert!(
        matched >= 10,
        "expected at least 10 known models, got {matched}"
    );
    assert!(records.iter().any(|record| {
        record.canonical_id == "anthropic/claude-opus-4.7"
            && record
                .raw_metrics
                .contains_key("ArtificialAnalysisIntelligence")
            && record.raw_metrics.contains_key("OutputSpeed")
            && record.raw_metrics.contains_key("InverseTTFT")
            && record.raw_metrics.contains_key("InverseCost")
    }));
}

#[tokio::test]
async fn aistupidlevel_fixture_contract() {
    let source = AiStupidLevelSource;
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: Some(fixture_dir()),
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("aistupidlevel fixture should parse");

    assert!(
        rows.len() >= 5,
        "expected at least 5 fixture rows, got {}",
        rows.len()
    );
    assert!(rows.iter().all(|row| {
        let metric_count = [
            "AI_correctness",
            "AI_spec",
            "AI_code",
            "AI_efficiency",
            "AI_stability",
            "AI_refusal",
            "AI_recovery",
            "AI_complexity",
            "AI_edge_cases",
            "AI_hallucination_resistance",
            "AI_plan_coherence",
            "AI_memory_retention",
            "AI_context_awareness",
            "AI_task_completion",
            "AI_tool_selection",
            "AI_parameter_accuracy",
            "AI_safety_compliance",
        ]
        .iter()
        .filter(|key| row.fields.contains_key(**key))
        .count();
        metric_count >= 3
    }));

    let (records, matched) = ingest_fixture_rows(rows);
    assert!(
        matched >= 5,
        "expected at least 5 alias matches, got {matched}"
    );
    assert!(records.iter().any(|record| {
        record.canonical_id == "anthropic/claude-opus-4.7"
            && record.raw_metrics.contains_key("AI_correctness")
    }));
}

#[tokio::test]
async fn swebench_fixture_contract() {
    let source = SweBenchSource;
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: Some(fixture_dir()),
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("swebench fixture should parse");

    assert!(
        rows.len() >= 10,
        "expected at least 10 verified leaderboard rows, got {}",
        rows.len()
    );

    assert!(
        rows.iter()
            .all(|row| row.fields.contains_key("SWEBenchVerified")
                || row.fields.contains_key("SWEBenchMultilingual"))
    );
    let verified_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.fields.contains_key("SWEBenchVerified"))
        .collect();
    assert!(
        verified_rows.len() >= 10,
        "expected at least 10 verified rows, got {}",
        verified_rows.len()
    );
    assert!(verified_rows.iter().any(|row| {
        row.fields
            .get("SWEBenchVerified")
            .and_then(number_like)
            .is_some_and(|value| value.fract().abs() > f64::EPSILON)
    }));

    let multilingual_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.fields.contains_key("SWEBenchMultilingual"))
        .collect();
    assert!(
        multilingual_rows.len() >= 5,
        "expected at least 5 multilingual rows, got {}",
        multilingual_rows.len()
    );

    let (records, _matched) = ingest_fixture_rows(rows);
    assert_flagship_matches(&records, "SWEBenchVerified", 10);
    let multilingual_hits = FLAGSHIPS
        .iter()
        .filter(|&&id| {
            records
                .iter()
                .any(|r| r.canonical_id == id && r.raw_metrics.contains_key("SWEBenchMultilingual"))
        })
        .count();
    assert!(
        multilingual_hits >= 3,
        "expected >=3 flagship SWEBenchMultilingual matches, got {multilingual_hits}/14"
    );
}

#[test]
fn registry_exposes_verified_sources() {
    let entries = registry();
    let meta: Vec<_> = entries
        .iter()
        .map(|source| {
            (
                source.id().to_string(),
                source.status(),
                source.required_secret(),
            )
        })
        .collect();

    assert!(meta.contains(&("openrouter".to_string(), VerificationStatus::Verified, None,)));
    assert!(meta.contains(&("lmarena".to_string(), VerificationStatus::Verified, None,)));
    assert!(meta.contains(&(
        "artificial_analysis".to_string(),
        VerificationStatus::Verified,
        Some(ipbr_sources::SecretRef::AaApiKey),
    )));
    assert!(meta.contains(&(
        "aistupidlevel".to_string(),
        VerificationStatus::Verified,
        None,
    )));
    assert!(meta.contains(&("swebench".to_string(), VerificationStatus::Verified, None,)));
    assert!(meta.contains(&(
        "livecodebench".to_string(),
        VerificationStatus::Verified,
        None,
    )));
    assert!(meta.contains(&(
        "terminal_bench".to_string(),
        VerificationStatus::Verified,
        None,
    )));
    assert!(meta.contains(&("swerebench".to_string(), VerificationStatus::Verified, None,)));
    assert!(meta.contains(&("overrides".to_string(), VerificationStatus::Verified, None,)));
    for dropped in [
        "bigcodebench",
        "openevals",
        "bfcl",
        "aider_polyglot",
        "metr_horizons",
    ] {
        assert!(
            !meta.iter().any(|(id, _, _)| id == dropped),
            "{dropped} should be dropped from registry"
        );
    }
}

#[tokio::test]
async fn swerebench_fixture_contract() {
    let source = SweRebenchSource;
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: Some(fixture_dir()),
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("swerebench fixture should parse");

    assert!(
        rows.len() >= 10,
        "expected at least 10 fixture rows, got {}",
        rows.len()
    );
    assert!(rows.iter().all(|row| row.fields.contains_key("SWERebench")));
    assert!(rows.iter().all(|row| {
        row.fields
            .get("SWERebench")
            .and_then(number_like)
            .is_some_and(|v| v.is_finite() && v >= 0.0)
    }));

    let (records, _matched) = ingest_fixture_rows(rows);
    let hits = FLAGSHIPS
        .iter()
        .filter(|&&id| {
            records
                .iter()
                .any(|r| r.canonical_id == id && r.raw_metrics.contains_key("SWERebench"))
        })
        .count();
    assert!(
        hits >= 3,
        "expected >=3 flagship SWE-rebench matches, got {hits}/14"
    );
}

#[tokio::test]
async fn overrides_source_emits_vendor_reported_rows() {
    let source = ipbr_sources::OverridesSource::default();
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: None,
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("embedded overrides should parse");

    assert!(
        !rows.is_empty(),
        "embedded overrides should ship at least one entry"
    );
    let (records, _matched) = ingest_fixture_rows(rows);
    let opus47_swe = records
        .iter()
        .find(|r| r.canonical_id == "anthropic/claude-opus-4.7")
        .and_then(|r| r.raw_metrics.get("SWEBenchVerified"));
    assert!(
        opus47_swe.is_some(),
        "claude-opus-4.7 should pick up SWEBenchVerified from overrides"
    );
}

#[tokio::test]
async fn terminal_bench_fixture_contract() {
    let source = ipbr_sources::TerminalBenchSource;
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: Some(fixture_dir()),
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("terminal_bench fixture should parse");

    assert!(
        rows.len() >= 10,
        "expected at least 10 fixture rows, got {}",
        rows.len()
    );
    assert!(
        rows.iter()
            .all(|row| row.fields.contains_key("TerminalBench"))
    );
    assert!(rows.iter().all(|row| {
        row.fields
            .get("TerminalBench")
            .and_then(number_like)
            .is_some_and(|v| v.is_finite())
    }));
}

#[tokio::test]
async fn livecodebench_fixture_contract() {
    let source = LiveCodeBenchSource;
    let rows = source
        .fetch(
            &OfflineOnlyHttp,
            FetchOptions {
                cache_dir: Some(fixture_dir()),
                offline: true,
            },
            &SecretStore::default(),
        )
        .await
        .expect("livecodebench fixture should parse");

    assert!(
        rows.len() >= 10,
        "expected at least 10 fixture rows, got {}",
        rows.len()
    );
    assert!(
        rows.iter()
            .all(|row| row.fields.contains_key("LiveCodeBench"))
    );
    assert!(rows.iter().all(|row| {
        row.fields
            .get("LiveCodeBench")
            .and_then(number_like)
            .is_some_and(|v| v.is_finite())
    }));

    let (records, _matched) = ingest_fixture_rows(rows);
    assert_flagship_matches(&records, "LiveCodeBench", 10);
}

fn number_like(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}
