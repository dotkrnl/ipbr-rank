use std::path::Path;

use assert_cmd::Command;

#[test]
fn offline_all_matches_golden_scoreboard() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should exist");
    let fixture_dir = repo_root.join("data/fixtures");
    let golden = repo_root.join("tests/golden/scoreboard.toml");
    let tmp = tempfile::tempdir().expect("tempdir should be created");
    let out = tmp.path().join("out");

    Command::cargo_bin("ipbr-rank")
        .expect("binary should build")
        .args([
            "all",
            "--offline",
            "--cache",
            fixture_dir.to_string_lossy().as_ref(),
            "--out",
            out.to_string_lossy().as_ref(),
            "--now",
            "2026-01-01T00:00:00Z",
        ])
        .assert()
        .success();

    let got = std::fs::read_to_string(out.join("scoreboard.toml"))
        .expect("scoreboard.toml should be written");
    let expected = std::fs::read_to_string(golden).expect("golden scoreboard should be present");
    assert_eq!(got, expected);

    assert!(out.join("missing.toml").is_file());
    assert!(out.join("coefficients.toml").is_file());
    assert!(out.join("site/index.html").is_file());
    assert!(out.join("site/about.html").is_file());
    assert!(out.join("site/assets/style.css").is_file());
    assert!(out.join("site/assets/app.js").is_file());
}

#[test]
fn render_preserves_synthesized_markers_from_persisted_scoreboard() {
    let tmp = tempfile::tempdir().expect("tempdir should be created");
    let out = tmp.path().join("out");
    std::fs::create_dir_all(&out).expect("out dir should be created");
    std::fs::write(
        out.join("scoreboard.toml"),
        persisted_synthesized_scoreboard(),
    )
    .expect("scoreboard fixture should be written");

    Command::cargo_bin("ipbr-rank")
        .expect("binary should build")
        .args(["render", "--out", out.to_string_lossy().as_ref()])
        .assert()
        .success();

    let index = std::fs::read_to_string(out.join("site/index.html"))
        .expect("index page should be rendered");
    // Synthesized metric appears with a trailing `*` marker in the inline expansion.
    assert!(index.contains("SWEBenchVerified*"));
    // Source pill for a synthesis-only source carries the same marker.
    assert!(index.contains("swebench*"));
}

fn persisted_synthesized_scoreboard() -> &'static str {
    r#"schema_version = "1.1.0"
generated_at = "2026-01-01T00:00:00Z"
generator = "ipbr-rank 0.1.0"
methodology = "v1"

[sources."openrouter"]
status = "verified"
n_rows_ingested = 1
n_rows_matched = 1
n_rows_unmatched = 0

[sources."swebench"]
status = "verified"
n_rows_ingested = 1
n_rows_matched = 1
n_rows_unmatched = 0

[[models]]
canonical_id = "openai/gpt-5.5"
display_name = "GPT-5.5"
vendor = "openai"
thinking_effort = "default"
aliases = ["gpt-5-5"]
sources = ["openrouter"]

[models.scores]
i_raw = 80.0
p_raw = 81.0
b_raw = 82.0
r = 83.0
i_adj = 78.0
p_adj = 77.0
b_adj = 76.0

[models.groups]
BUILD = 88.25

[models.metrics]
SWEBenchVerified = 70.25
TerminalBench = 74.0

[models.synthesized]
SWEBenchVerified = { source = "swebench", from = "openai/gpt-5.4" }

[models.missing]
metrics = []
groups_shrunk = []
synthesis_dominant = true
"#
}
