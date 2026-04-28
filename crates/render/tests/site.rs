use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ipbr_core::{Coefficients, ModelRecord, SourceSummary, ThinkingEffort, Vendor};
use ipbr_render::{Scoreboard, site::render_site};
use tempfile::tempdir;

#[test]
fn site_renders_without_panicking() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    assert!(site_dir.join("index.html").is_file());
}

#[test]
fn site_emits_pages_api_toml_and_assets() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    assert!(site_dir.join("index.html").is_file());
    assert!(site_dir.join("about.html").is_file());
    assert!(site_dir.join("scoreboard.toml").is_file());
    assert!(site_dir.join("assets/style.css").is_file());
    assert!(site_dir.join("assets/app.js").is_file());

    assert!(!site_dir.join("methodology.html").exists());
    assert!(!site_dir.join("sources.html").exists());
    assert!(!site_dir.join("model").exists());
    assert!(!site_dir.join("assets/sort.js").exists());
}

fn sample_scoreboard() -> Scoreboard {
    let coefficients = Coefficients::load_embedded().expect("embedded coefficients should parse");
    let mut source_summary = BTreeMap::new();
    source_summary.insert(
        "openrouter".to_string(),
        SourceSummary {
            status: "verified".to_string(),
            rows: 2,
            matched: 2,
            unmatched: 0,
        },
    );
    source_summary.insert(
        "lmarena".to_string(),
        SourceSummary {
            status: "verified".to_string(),
            rows: 2,
            matched: 1,
            unmatched: 1,
        },
    );

    Scoreboard {
        models: vec![
            model(ModelFixture {
                canonical_id: "anthropic/claude-opus-4.7",
                display_name: "Claude Opus 4.7",
                vendor: Vendor::Anthropic,
                thinking_effort: None,
                groups: &[
                    ("CRE", 81.25),
                    ("BUILD", 76.5),
                    ("LM_ARENA_REVIEW_PROXY", 84.0),
                ],
                metrics: &[("LMArenaText", 82.0), ("SWEBenchVerified", 77.0)],
                sources: &["openrouter", "lmarena"],
                missing: &["AI_recovery"],
            }),
            model(ModelFixture {
                canonical_id: "openai/gpt-5.5+thinking-high",
                display_name: "GPT-5.5 High",
                vendor: Vendor::Openai,
                thinking_effort: Some(ThinkingEffort::High),
                groups: &[
                    ("CRE", 79.0),
                    ("BUILD", 88.25),
                    ("LM_ARENA_REVIEW_PROXY", 73.5),
                ],
                metrics: &[("LMArenaCode", 85.0), ("TerminalBench", 74.0)],
                sources: &["openrouter"],
                missing: &["AI_refusal", "AI_recovery"],
            }),
            model(ModelFixture {
                canonical_id: "google/gemini-3.1-pro",
                display_name: "Gemini 3.1 Pro",
                vendor: Vendor::Google,
                thinking_effort: None,
                groups: &[
                    ("CRE", 85.0),
                    ("BUILD", 84.0),
                    ("LM_ARENA_REVIEW_PROXY", 70.0),
                ],
                metrics: &[
                    ("LMArenaText", 86.0),
                    ("ArtificialAnalysisIntelligence", 82.0),
                ],
                sources: &["openrouter", "lmarena"],
                missing: &[],
            }),
        ],
        coefficients,
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        generator: "ipbr-rank 0.1.0".to_string(),
        methodology: "v1".to_string(),
        source_summary,
    }
}

struct ModelFixture<'a> {
    canonical_id: &'a str,
    display_name: &'a str,
    vendor: Vendor,
    thinking_effort: Option<ThinkingEffort>,
    groups: &'a [(&'a str, f64)],
    metrics: &'a [(&'a str, f64)],
    sources: &'a [&'a str],
    missing: &'a [&'a str],
}

fn model(fixture: ModelFixture<'_>) -> ModelRecord {
    let mut record = ModelRecord::new(
        fixture.canonical_id.to_string(),
        fixture.display_name.to_string(),
        fixture.vendor,
    );
    record.thinking_effort = fixture.thinking_effort;
    record.groups = fixture
        .groups
        .iter()
        .map(|(key, score)| ((*key).to_string(), *score))
        .collect();
    record.metrics = fixture
        .metrics
        .iter()
        .map(|(key, score)| ((*key).to_string(), *score))
        .collect();
    record.sources = fixture
        .sources
        .iter()
        .map(|source| (*source).to_string())
        .collect();
    record.missing.metrics = fixture
        .missing
        .iter()
        .map(|metric| (*metric).to_string())
        .collect();
    record.aliases = BTreeSet::from([fixture.display_name.to_lowercase()]);
    record.scores.i_raw = 80.0;
    record.scores.p_raw = 81.0;
    record.scores.b_raw = 82.0;
    record.scores.r = 83.0;
    record.scores.i_adj = 78.0;
    record.scores.p_adj = 77.0;
    record.scores.b_adj = 76.0;
    record
}

#[test]
fn style_css_contains_d2_theme_tokens() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let css = read(site_dir.join("assets/style.css"));
    assert!(
        css.contains("#0f1419"),
        "expected D2 background token #0f1419 in CSS"
    );
    assert!(
        css.contains("ui-monospace"),
        "expected monospace font stack"
    );
    assert!(
        css.contains("data-mode=\"raw\""),
        "expected mode-toggle CSS selector"
    );
    assert!(
        css.contains("prefers-color-scheme: light"),
        "expected light-theme fallback"
    );
}

fn html_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_html(root, &mut files);
    files
}

fn collect_html(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(path).expect("directory should be readable") {
        let entry = entry.expect("directory entry should be readable");
        let path = entry.path();
        if path.is_dir() {
            collect_html(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "html") {
            files.push(path);
        }
    }
}

fn assert_no_external_references(path: &Path, html: &str) {
    for needle in ["http://", "https://", "//cdn", "data:"] {
        assert!(
            !html.contains(needle),
            "{} unexpectedly contains external reference marker {needle}",
            path.display()
        );
    }
}

#[test]
fn leaderboard_has_row_and_expansion_per_model() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let index = read(site_dir.join("index.html"));

    // Toolbar exists
    assert!(index.contains("class=\"toolbar\""));
    assert!(index.contains("class=\"vendor-chips\""));
    assert!(index.contains("data-filter-input"));

    // Table with raw + adjusted columns
    assert!(index.contains("class=\"leaderboard\""));
    assert!(index.contains("class=\"score-raw num\"") || index.contains("class=\"num score-raw\""));
    assert!(
        index.contains("class=\"score-adjusted num\"")
            || index.contains("class=\"num score-adjusted\"")
    );

    // One row + one hidden expansion per model — three models in fixture
    let row_count = index.matches("<tr class=\"row\"").count();
    let expand_count = index.matches("<tr class=\"expand\"").count();
    assert_eq!(row_count, 3, "expected 3 model rows, got {row_count}");
    assert_eq!(
        expand_count, 3,
        "expected 3 expansion rows, got {expand_count}"
    );

    // Anchor IDs use canonical-id form
    assert!(index.contains(r#"id="anthropic/claude-opus-4.7""#));
    assert!(index.contains(r#"id="google/gemini-3.1-pro""#));

    // Expansion content includes the new ranked group/metric tables
    assert!(index.contains("class=\"exp-table\""));
    assert!(index.contains("class=\"exp-rank"));

    // Sort buttons emit data-sort attribute on header
    assert!(index.contains("data-sort=\"i\""));
    assert!(index.contains("data-sort=\"r\""));
}

#[test]
fn app_js_implements_required_features() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let js = read(site_dir.join("assets/app.js"));
    // Mode toggle
    assert!(js.contains("data-mode-value"), "missing mode toggle wiring");
    assert!(
        js.contains("localStorage"),
        "missing localStorage persistence"
    );
    // Sort
    assert!(js.contains("data-sort"), "missing sort wiring");
    assert!(
        js.contains("data-sort-active"),
        "sort should set active state"
    );
    // Filter
    assert!(
        js.contains("data-filter-input"),
        "missing filter input wiring"
    );
    assert!(js.contains("data-vendor"), "missing vendor chip wiring");
    // Expand
    assert!(js.contains("expand-toggle"), "missing expand wiring");
    // Anchor auto-expand
    assert!(
        js.contains("location.hash") || js.contains("hash"),
        "missing anchor handling"
    );
}

fn assert_links_exist(site_dir: &Path, path: &Path, html: &str) {
    for link in extract_attr_values(html, "href")
        .into_iter()
        .chain(extract_attr_values(html, "src"))
    {
        if link.starts_with('#') {
            continue;
        }
        let link = link.split('#').next().unwrap_or_default();
        if link.is_empty() {
            continue;
        }

        let target = path
            .parent()
            .expect("html file should have parent")
            .join(link);
        let normalized = normalize_path(&target);
        assert!(
            normalized.starts_with(site_dir) && normalized.exists(),
            "{} references missing target {link}",
            path.display()
        );
    }
}

fn extract_attr_values(html: &str, attr: &str) -> Vec<String> {
    let mut values = Vec::new();
    let needle = format!("{attr}=\"");
    for part in html.split(&needle).skip(1) {
        if let Some((value, _)) = part.split_once('"') {
            values.push(value.to_string());
        }
    }
    values
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).expect("file should be readable")
}

#[test]
fn about_page_has_required_sections_and_sources_table() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let about = read(site_dir.join("about.html"));
    assert!(about.contains("What this is"));
    assert!(about.contains("The four roles"));
    assert!(about.contains("Raw vs adjusted"));
    assert!(about.contains("How scores are built"));
    assert!(about.contains("Sources"));
    // Sources table reflects fixture
    assert!(about.contains("openrouter"));
    assert!(about.contains("lmarena"));
    assert!(about.contains("verified"));
    // Glossary
    assert!(about.contains("Glossary"));
    // Back link
    assert!(about.contains("href=\"index.html\""));
}

#[test]
fn scoring_panel_has_role_definitions_and_link() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let index = read(site_dir.join("index.html"));

    assert!(index.contains("<details class=\"scoring\""));
    assert!(index.contains(">how scoring works<"));
    // All four role labels appear in the panel
    for role in ["Idea", "Plan", "Build", "Review"] {
        assert!(index.contains(role), "scoring panel missing role {role}");
    }
    // Mentions raw vs adjusted explanation
    assert!(index.contains("reviewer-reservation"));
    // Link to full methodology page
    assert!(index.contains("href=\"about.html\""));
    // Trust threshold note
    assert!(index.contains("70%") || index.contains("70 %") || index.contains("0.70"));
}

#[test]
fn hero_renders_top_three_per_role_with_dual_scores() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let index = read(site_dir.join("index.html"));

    // Four role columns, in order Idea, Plan, Build, Review.
    assert!(
        index.contains("class=\"role idea\""),
        "hero missing idea column"
    );
    assert!(
        index.contains("class=\"role plan\""),
        "hero missing plan column"
    );
    assert!(
        index.contains("class=\"role build\""),
        "hero missing build column"
    );
    assert!(
        index.contains("class=\"role review\""),
        "hero missing review column"
    );

    // The fixture has three models — hero shows three rows per role.
    let idea_rows = index.matches("class=\"row").count();
    assert!(
        idea_rows >= 12,
        "hero should have at least 12 rows (3 per role x 4), got {idea_rows}"
    );

    // Both raw and adjusted score spans are emitted for each model in the hero.
    assert!(index.contains("score-raw"));
    assert!(index.contains("score-adjusted"));

    // Display names appear in the hero.
    assert!(index.contains("Gemini 3.1 Pro"));
    assert!(index.contains("Claude Opus 4.7"));
    assert!(index.contains("GPT-5.5 High"));
}

#[test]
fn index_has_body_shell_and_mode_toggle() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let index = read(site_dir.join("index.html"));
    assert!(
        index.contains("data-mode=\"raw\""),
        "body should default to data-mode=\"raw\""
    );
    assert!(
        index.contains("class=\"mode-toggle\""),
        "mode toggle UI should be present"
    );
    assert!(
        index.contains("data-mode-value=\"raw\""),
        "raw mode button missing"
    );
    assert!(
        index.contains("data-mode-value=\"adjusted\""),
        "adjusted mode button missing"
    );
    assert!(
        index.contains("ipbr-rank"),
        "header should mention the project name"
    );
    assert!(
        index.contains("live llm coding-role score"),
        "header/title should use the live coding-role score label"
    );
    assert!(
        !index.contains("models ·"),
        "header meta should not include a model-count segment"
    );
    assert!(
        index.contains("about.html"),
        "header should link to about page"
    );
    assert!(
        index.contains("scoreboard.toml"),
        "header/footer should link to the TOML API"
    );
}

#[test]
fn site_uses_relative_links_and_valid_inter_page_targets() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    for html_path in html_files(&site_dir) {
        let html = read(&html_path);
        assert_no_external_references(&html_path, &html);
        assert_links_exist(&site_dir, &html_path, &html);
    }
}
