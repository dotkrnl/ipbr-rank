use std::path::Path;

use ipbr_core::{Coefficients, ModelRecord, SCHEMA_VERSION, aggregate::shrink_coverage_cutoff};

use crate::Scoreboard;

const SCOREBOARD_FILE: &str = "scoreboard.toml";
const MISSING_FILE: &str = "missing.toml";
const COEFFICIENTS_FILE: &str = "coefficients.toml";

pub fn write_scoreboard(scoreboard: &Scoreboard, out: &Path) -> Result<(), RenderError> {
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join(SCOREBOARD_FILE), render_scoreboard(scoreboard))?;
    Ok(())
}

pub fn write_missing(scoreboard: &Scoreboard, out: &Path) -> Result<(), RenderError> {
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join(MISSING_FILE), render_missing(scoreboard))?;
    Ok(())
}

pub fn write_coefficients(
    coefficients: &ipbr_core::Coefficients,
    out: &Path,
) -> Result<(), RenderError> {
    std::fs::create_dir_all(out)?;
    let payload = toml::to_string_pretty(coefficients)
        .map_err(|err| RenderError::Serialization(err.to_string()))?;
    std::fs::write(out.join(COEFFICIENTS_FILE), payload)?;
    Ok(())
}

pub(crate) fn render_scoreboard(scoreboard: &Scoreboard) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "schema_version = {}\n",
        toml_string(SCHEMA_VERSION)
    ));
    out.push_str(&format!(
        "generated_at = {}\n",
        toml_string(&scoreboard.generated_at)
    ));
    out.push_str(&format!(
        "generator = {}\n",
        toml_string(&scoreboard.generator)
    ));
    out.push_str(&format!(
        "methodology = {}\n\n",
        toml_string(&scoreboard.methodology)
    ));

    if scoreboard.source_summary.is_empty() {
        out.push_str("[sources]\n\n");
    } else {
        for (source_id, summary) in &scoreboard.source_summary {
            out.push_str(&format!("[sources.{}]\n", toml_string(source_id)));
            out.push_str(&format!("status = {}\n", toml_string(&summary.status)));
            out.push_str(&format!("n_rows_ingested = {}\n", summary.rows));
            out.push_str(&format!("n_rows_matched = {}\n", summary.matched));
            out.push_str(&format!("n_rows_unmatched = {}\n\n", summary.unmatched));
        }
    }

    let mut models: Vec<&ModelRecord> = scoreboard.models.iter().collect();
    models.sort_by(|left, right| {
        left.canonical_id
            .cmp(&right.canonical_id)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    for model in models {
        let missing = classify_missing(model, &scoreboard.coefficients);
        out.push_str("[[models]]\n");
        out.push_str(&format!(
            "canonical_id = {}\n",
            toml_string(&model.canonical_id)
        ));
        out.push_str(&format!(
            "display_name = {}\n",
            toml_string(&model.display_name)
        ));
        out.push_str(&format!(
            "vendor = {}\n",
            toml_string(model.vendor.as_str())
        ));
        out.push_str(&format!(
            "thinking_effort = {}\n",
            toml_string(serialize_thinking_effort(model.thinking_effort.as_ref()))
        ));
        out.push_str(&format!(
            "aliases = {}\n",
            toml_array(model.aliases.iter().cloned())
        ));
        out.push_str(&format!(
            "sources = {}\n\n",
            toml_array(model.sources.iter().cloned())
        ));

        out.push_str("[models.scores]\n");
        out.push_str(&format!("i_raw = {}\n", format_float(model.scores.i_raw)));
        out.push_str(&format!("p_raw = {}\n", format_float(model.scores.p_raw)));
        out.push_str(&format!("b_raw = {}\n", format_float(model.scores.b_raw)));
        out.push_str(&format!("r = {}\n", format_float(model.scores.r)));
        // i_adj/p_adj/b_adj retained as raw aliases for API back-compat.
        out.push_str(&format!("i_adj = {}\n", format_float(model.scores.i_raw)));
        out.push_str(&format!("p_adj = {}\n", format_float(model.scores.p_raw)));
        out.push_str(&format!("b_adj = {}\n\n", format_float(model.scores.b_raw)));

        out.push_str("[models.groups]\n");
        for (group, score) in &model.groups {
            out.push_str(&format!("{group} = {}\n", format_float(*score)));
        }
        out.push('\n');

        out.push_str("[models.metrics]\n");
        for (metric, score) in &model.metrics {
            out.push_str(&format!("{metric} = {}\n", format_float(*score)));
        }
        out.push('\n');

        out.push_str("[models.missing]\n");
        out.push_str(&format!("metrics = {}\n", toml_array(missing.metrics)));
        out.push_str(&format!(
            "groups_shrunk = {}\n\n",
            toml_array(missing.groups_shrunk)
        ));
    }

    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn render_missing(scoreboard: &Scoreboard) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "generated_at = {}\n\n",
        toml_string(&scoreboard.generated_at)
    ));

    let mut models: Vec<&ModelRecord> = scoreboard.models.iter().collect();
    models.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));

    for model in models {
        let missing = classify_missing(model, &scoreboard.coefficients);
        out.push_str(&format!("[models.{}]\n", toml_string(&model.canonical_id)));
        out.push_str(&format!(
            "display_name = {}\n",
            toml_string(&model.display_name)
        ));
        out.push_str(&format!("metrics = {}\n", toml_array(missing.metrics)));
        out.push_str(&format!(
            "groups_shrunk = {}\n\n",
            toml_array(missing.groups_shrunk)
        ));
    }

    out
}

struct ClassifiedMissing {
    metrics: Vec<String>,
    groups_shrunk: Vec<String>,
}

fn classify_missing(model: &ModelRecord, coefficients: &Coefficients) -> ClassifiedMissing {
    let mut metrics: Vec<String> = model
        .missing
        .metrics
        .iter()
        .filter(|metric| !metric.contains('/'))
        .cloned()
        .collect();
    metrics.sort();
    metrics.dedup();

    // Start with whatever core already recorded (now includes A_* perspectives).
    let mut groups_shrunk: Vec<String> = model.missing.groups_shrunk.iter().cloned().collect();
    let aggregation = coefficients.aggregation.clone().unwrap_or_default();
    let shrink_cutoff = shrink_coverage_cutoff(&aggregation);

    // Defensive re-computation for both regular groups and AISL perspectives.
    for (group, weights) in coefficients
        .group_weights
        .iter()
        .chain(coefficients.ai_stupid_perspective_weights.iter())
    {
        let total_weight: f64 = weights.values().sum();
        if total_weight <= 0.0 {
            continue;
        }

        let missing_weight: f64 = weights
            .iter()
            .filter(|(metric, _)| !model.metrics.contains_key(*metric))
            .map(|(_, weight)| *weight)
            .sum();

        let present_coverage = 1.0 - missing_weight / total_weight;
        if present_coverage < shrink_cutoff {
            groups_shrunk.push(group.clone());
        }
    }
    groups_shrunk.sort();
    groups_shrunk.dedup();

    ClassifiedMissing {
        metrics,
        groups_shrunk,
    }
}

fn serialize_thinking_effort(effort: Option<&ipbr_core::ThinkingEffort>) -> &'static str {
    match effort {
        Some(ipbr_core::ThinkingEffort::Low) => "low",
        Some(ipbr_core::ThinkingEffort::Medium) => "medium",
        Some(ipbr_core::ThinkingEffort::High) => "high",
        None => "default",
    }
}

fn format_float(value: f64) -> String {
    format!("{value:.6}")
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

fn toml_array(values: impl IntoIterator<Item = String>) -> String {
    let arr = toml::Value::Array(values.into_iter().map(toml::Value::String).collect());
    arr.to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
}
