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
    assert!(find_style_css(&site_dir).is_file());
    assert!(site_dir.join("assets/app.js").is_file());

    // Stylesheet ships under a content-hashed name; the un-fingerprinted
    // path must not be emitted, otherwise stale CDN/browser copies survive.
    assert!(!site_dir.join("assets/style.css").exists());

    assert!(!site_dir.join("methodology.html").exists());
    assert!(!site_dir.join("sources.html").exists());
    assert!(!site_dir.join("model").exists());
    assert!(!site_dir.join("assets/sort.js").exists());
}

fn find_style_css(site_dir: &Path) -> PathBuf {
    let assets = site_dir.join("assets");
    let mut matches: Vec<PathBuf> = std::fs::read_dir(&assets)
        .expect("assets dir should exist")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("style.") && name.ends_with(".css"))
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one fingerprinted style.*.css under assets/, found {matches:?}"
    );
    matches.remove(0)
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
        prev_scores: None,
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
    record
}

#[test]
fn style_css_contains_d2_theme_tokens() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let css = read(find_style_css(&site_dir));
    assert!(
        css.contains("#0f1419"),
        "expected D2 background token #0f1419 in CSS"
    );
    assert!(
        css.contains("ui-monospace"),
        "expected monospace font stack"
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
    // The GitHub repo link is the one allowlisted external URL (see
    // EXTERNAL_LINK_ALLOWLIST in crates/render/src/site/mod.rs); strip it
    // before scanning so the no-external-deps invariant still holds.
    let scrubbed = html.replace("https://github.com/dotkrnl/ipbr-rank", "");
    for needle in ["http://", "https://", "//cdn", "data:"] {
        assert!(
            !scrubbed.contains(needle),
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

    // Leaderboard table with one numeric column per role.
    assert!(index.contains("class=\"leaderboard\""));

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
        if link.starts_with('#') || link.starts_with("https://github.com/dotkrnl/ipbr-rank") {
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
    // Link to full methodology page
    assert!(index.contains("href=\"about.html\""));
    // Trust transition note
    assert!(index.contains("60-80%") || index.contains("60-80 %"));
}

#[test]
fn hero_renders_title_tagline_radar_and_per_role_leaders() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let index = read(site_dir.join("index.html"));

    // Tagline beats with `Models drift` carrying the lead emphasis class.
    assert!(
        index.contains(r#"class="beat beat-lead">Models drift."#),
        "tagline lead beat (Models drift) should carry .beat-lead"
    );
    assert!(index.contains("Agents battle."), "tagline beat 2 missing");
    assert!(index.contains("Math decides."), "tagline beat 3 missing");

    // Brand wordmark only appears in the persistent header — not duplicated
    // in the hero.
    assert!(
        !index.contains("class=\"hero-title\""),
        "hero must not duplicate the brand h1"
    );

    // Live status block.
    assert!(index.contains("live-dot"), "missing live indicator");

    // Hero radar SVG with single-letter axis labels (one per role class).
    assert!(
        index.contains("class=\"radar radar-hero\""),
        "missing hero radar SVG"
    );
    for (role, letter) in [
        ("idea", "I"),
        ("plan", "P"),
        ("build", "B"),
        ("review", "R"),
    ] {
        let needle = format!(r#"class="radar-label {role}""#);
        assert!(
            index.contains(&needle),
            "hero radar missing {role} axis label class"
        );
        let text = format!(">{letter}</text>");
        assert!(
            index.contains(&text),
            "hero radar missing single-letter axis label {letter}"
        );
    }

    // Four per-role leader columns.
    for role in ["idea", "plan", "build", "review"] {
        let cls = format!("class=\"role {role}\"");
        assert!(index.contains(&cls), "leaders sidebar missing {role}");
    }

    // Display names from the fixture appear in the leaders sidebar
    // (top-1 always renders, regardless of which role).
    assert!(index.contains("Gemini 3.1 Pro"));
    assert!(index.contains("Claude Opus 4.7"));
    assert!(index.contains("GPT-5.5 High"));
}

#[test]
fn expansion_includes_mini_radar() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let index = read(site_dir.join("index.html"));
    let mini_count = index.matches("class=\"radar radar-mini\"").count();
    assert_eq!(
        mini_count, 3,
        "expected one mini radar per model row, got {mini_count}"
    );
}

#[test]
fn deltas_render_only_when_prev_scores_are_supplied() {
    use std::collections::BTreeMap;

    let mut scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");

    // Without prev_scores, no delta chips.
    render_site(&scoreboard, &tmp.path().join("site_a")).expect("site should render");
    let index_no_prev = read(tmp.path().join("site_a/index.html"));
    assert!(
        !index_no_prev.contains("class=\"delta"),
        "no deltas should render without --prev"
    );

    // With prev_scores, at least one delta chip appears.
    let mut prev = BTreeMap::new();
    prev.insert(
        "anthropic/claude-opus-4.7".to_string(),
        ipbr_core::RoleScores {
            i_raw: 78.0, // current 80.0 → delta +2.0
            p_raw: 81.0,
            b_raw: 82.0,
            r: 83.0,
        },
    );
    scoreboard.prev_scores = Some(prev);

    render_site(&scoreboard, &tmp.path().join("site_b")).expect("site should render");
    let index_with_prev = read(tmp.path().join("site_b/index.html"));
    assert!(
        index_with_prev.contains("delta-up"),
        "expected at least one ▲ delta chip when --prev provided"
    );
}

#[test]
fn index_has_body_shell() {
    let scoreboard = sample_scoreboard();
    let tmp = tempdir().expect("tempdir should be created");
    let site_dir = tmp.path().join("site");

    render_site(&scoreboard, &site_dir).expect("site should render");

    let index = read(site_dir.join("index.html"));
    assert!(
        index.contains(">$</span>ipbr<") || index.contains("ipbr</a>"),
        "header should show the `ipbr` brand"
    );
    assert!(
        index.contains("live llm coding scoreboard"),
        "<title> should use the new tagline-aligned label"
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
