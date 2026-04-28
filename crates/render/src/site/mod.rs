use std::path::{Component, Path, PathBuf};

use crate::Scoreboard;
use crate::toml_output::RenderError;
use crate::toml_output::render_scoreboard;

mod about;
mod index;
mod scripts;
mod theme;

use about::render_about;
use index::render_index;
use scripts::APP_JS;
use theme::STYLE_CSS;

pub fn render_site(scoreboard: &Scoreboard, out: &Path) -> Result<(), RenderError> {
    std::fs::create_dir_all(out.join("assets"))?;
    std::fs::write(out.join("assets/style.css"), STYLE_CSS)?;
    std::fs::write(out.join("assets/app.js"), APP_JS)?;
    std::fs::write(out.join("scoreboard.toml"), render_scoreboard(scoreboard))?;
    std::fs::write(out.join("index.html"), render_index(scoreboard))?;
    std::fs::write(out.join("about.html"), render_about(scoreboard))?;
    validate_site(out)?;
    Ok(())
}

pub(crate) fn layout(title: &str, scoreboard: &Scoreboard, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title><link rel="stylesheet" href="assets/style.css"><script defer src="assets/app.js"></script></head><body data-mode="raw"><div class="shell"><header><div class="brand"><span class="prompt">$</span>ipbr-rank · live llm coding-role score<div class="meta">refreshed <time datetime="{generated_at}" data-local-time>{generated_at}</time> · {source_count} sources</div></div><nav><a href="about.html">about</a><a href="scoreboard.toml">api</a></nav></header><main>{body}</main></div></body></html>"#,
        title = html_escape(title),
        generated_at = html_escape(&scoreboard.generated_at),
        source_count = scoreboard.source_summary.len(),
        body = body,
    )
}

fn validate_site(root: &Path) -> Result<(), RenderError> {
    for html_path in html_files(root)? {
        let html = std::fs::read_to_string(&html_path)?;
        for marker in ["http://", "https://", "//cdn", "data:"] {
            if html.contains(marker) {
                return Err(RenderError::Serialization(format!(
                    "{} contains external reference marker {marker}",
                    html_path.display()
                )));
            }
        }
        for link in attr_values(&html, "href")
            .into_iter()
            .chain(attr_values(&html, "src"))
        {
            if link.starts_with('#') {
                continue;
            }
            let link = link.split('#').next().unwrap_or_default();
            if link.is_empty() {
                continue;
            }
            let target = normalize_path(&html_path.parent().unwrap_or(root).join(link));
            if !target.starts_with(root) || !target.exists() {
                return Err(RenderError::Serialization(format!(
                    "{} references missing target {link}",
                    html_path.display()
                )));
            }
        }
    }
    Ok(())
}

fn html_files(root: &Path) -> Result<Vec<PathBuf>, RenderError> {
    let mut files = Vec::new();
    collect_html(root, &mut files)?;
    Ok(files)
}

fn collect_html(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), RenderError> {
    for entry in std::fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_html(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "html") {
            files.push(path);
        }
    }
    Ok(())
}

fn attr_values(html: &str, attr: &str) -> Vec<String> {
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
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub(crate) fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
