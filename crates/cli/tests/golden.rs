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

    // Regenerate the checked-in golden from this very run: the binary's own
    // output is the only thing it is ever compared against.
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        std::fs::create_dir_all(golden.parent().expect("golden has a parent"))
            .expect("golden parent should be creatable");
        std::fs::write(&golden, &got).expect("golden should be writable");
    }

    let expected = std::fs::read_to_string(&golden).expect("golden scoreboard should be present");
    assert_eq!(got, expected, "golden scoreboard drifted");

    assert!(out.join("missing.toml").is_file());
    assert!(out.join("coefficients.toml").is_file());
    assert!(out.join("site/index.html").is_file());
    assert!(out.join("site/about.html").is_file());
    assert!(out.join("site/scoreboard.toml").is_file());
    let assets = out.join("site/assets");
    let style_css = std::fs::read_dir(&assets)
        .expect("assets dir should exist")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("style.") && name.ends_with(".css"))
        })
        .expect("expected fingerprinted style.*.css under site/assets/");
    assert!(style_css.is_file());
    assert!(out.join("site/assets/app.js").is_file());
}
