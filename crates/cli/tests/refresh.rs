use std::path::{Path, PathBuf};

use assert_cmd::Command;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures")
}

fn command(cache: &Path, out: &Path, sources: &str) -> Command {
    let mut command = Command::cargo_bin("ipbr-rank").unwrap();
    command.args([
        "--cache",
        cache.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--only",
        sources,
        "all",
    ]);
    command
}

#[test]
fn best_effort_renders_healthy_sources_and_reports_unavailable_sources() {
    let cache = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    // Both files are fresh, so neither path requires network access.
    std::fs::copy(
        fixtures().join("aa_critpt.html"),
        cache.path().join("aa_critpt.html"),
    )
    .unwrap();
    std::fs::write(
        cache.path().join("aa_omniscience.html"),
        "upstream schema changed",
    )
    .unwrap();

    command(cache.path(), output.path(), "aa_omniscience,aa_critpt")
        .arg("--best-effort")
        .assert()
        .success();
    let scoreboard: toml::Value =
        toml::from_str(&std::fs::read_to_string(output.path().join("scoreboard.toml")).unwrap())
            .unwrap();
    assert_eq!(
        scoreboard["sources"]["aa_omniscience"]["status"].as_str(),
        Some("unavailable")
    );
    assert_eq!(
        scoreboard["sources"]["aa_omniscience"]["n_rows_ingested"].as_integer(),
        Some(0)
    );
    assert_eq!(
        scoreboard["sources"]["aa_critpt"]["status"].as_str(),
        Some("verified")
    );
    assert!(
        scoreboard["sources"]["aa_critpt"]["n_rows_matched"]
            .as_integer()
            .unwrap()
            > 0
    );
    assert!(output.path().join("site/index.html").is_file());

    // Ordinary live runs and offline fixture builds retain strict semantics.
    command(cache.path(), output.path(), "aa_omniscience,aa_critpt")
        .assert()
        .failure();
    command(cache.path(), output.path(), "aa_omniscience,aa_critpt")
        .arg("--offline")
        .assert()
        .failure();
    command(cache.path(), output.path(), "aa_omniscience,aa_critpt")
        .args(["--offline", "--best-effort"])
        .assert()
        .failure();
}

#[test]
fn complete_outage_does_not_replace_existing_output_with_overrides_and_pricing() {
    let cache = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixtures().join("openrouter_models.json"),
        cache.path().join("openrouter_models.json"),
    )
    .unwrap();
    std::fs::write(
        cache.path().join("aa_omniscience.html"),
        "upstream schema changed",
    )
    .unwrap();
    let scoreboard = output.path().join("scoreboard.toml");
    std::fs::write(&scoreboard, "previous successful scoreboard").unwrap();

    let result = command(
        cache.path(),
        output.path(),
        "openrouter,aa_omniscience,overrides",
    )
    .arg("--best-effort")
    .assert()
    .failure();
    assert!(
        String::from_utf8_lossy(&result.get_output().stderr)
            .contains("no usable benchmark sources")
    );
    assert_eq!(
        std::fs::read_to_string(scoreboard).unwrap(),
        "previous successful scoreboard"
    );
    assert!(!output.path().join("site").exists());
}

#[test]
fn best_effort_exposes_stale_credentialed_source_in_scoreboard_and_site() {
    let cache = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = cache.path().join("artificial_analysis_llms.json");
    std::fs::copy(fixtures().join("artificial_analysis_llms.json"), &path).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH)
        .unwrap();
    command(cache.path(), output.path(), "artificial_analysis")
        .env_remove("AA_API_KEY")
        .arg("--best-effort")
        .assert()
        .success();
    let scoreboard: toml::Value =
        toml::from_str(&std::fs::read_to_string(output.path().join("scoreboard.toml")).unwrap())
            .unwrap();
    assert_eq!(
        scoreboard["sources"]["artificial_analysis"]["status"].as_str(),
        Some("stale")
    );
    let about = std::fs::read_to_string(output.path().join("site/about.html")).unwrap();
    assert!(about.contains("<td>artificial_analysis</td><td>stale</td>"));
}
