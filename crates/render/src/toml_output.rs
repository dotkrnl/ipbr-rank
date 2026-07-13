use std::path::Path;

use ipbr_core::{
    Coefficients, EligibilityQualificationPath, EvidenceCoverage, ModelRecord, SCHEMA_VERSION,
    balanced_is_provisional,
};

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
        "methodology = {}\n",
        toml_string(&scoreboard.methodology)
    ));
    out.push_str("configuration_policy = \"best_available_max_effort\"\n\n");

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
        // The ranked record is a best-available capability envelope, not one
        // runnable effort; the field is always the same policy string.
        out.push_str(&format!(
            "thinking_effort = {}\n",
            toml_string("best_available")
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
        out.push_str(&format!(
            "i_status = {}\n",
            toml_string(role_status(model, "I_raw"))
        ));
        out.push_str(&format!(
            "p_status = {}\n",
            toml_string(role_status(model, "P_raw"))
        ));
        out.push_str(&format!(
            "b_status = {}\n",
            toml_string(role_status(model, "B_raw"))
        ));
        out.push_str(&format!(
            "r_status = {}\n",
            toml_string(role_status(model, "R"))
        ));
        out.push_str(&format!(
            "balanced_status = {}\n\n",
            toml_string(if balanced_is_provisional(model) {
                "provisional"
            } else {
                "ranked"
            })
        ));

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

        out.push_str("[models.raw_metrics]\n");
        for metric in public_raw_metrics(model, &scoreboard.coefficients) {
            out.push_str(&format!(
                "{metric} = {}\n",
                format_float(model.raw_metrics[metric])
            ));
        }
        out.push('\n');

        render_metric_evidence(&mut out, model, &scoreboard.coefficients);
        render_evidence_summary(&mut out, model);

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
    let mut metrics: Vec<String> = scored_leaf_metrics(coefficients)
        .into_iter()
        .filter(|metric| !model.metrics.contains_key(metric))
        .collect();
    metrics.sort();

    let mut groups_shrunk: Vec<String> = model.missing.groups_shrunk.iter().cloned().collect();
    groups_shrunk.sort();
    groups_shrunk.dedup();

    ClassifiedMissing {
        metrics,
        groups_shrunk,
    }
}

fn scored_leaf_metrics(coefficients: &Coefficients) -> std::collections::BTreeSet<String> {
    fn expand(
        metric: &str,
        coefficients: &Coefficients,
        leaves: &mut std::collections::BTreeSet<String>,
        visiting: &mut std::collections::BTreeSet<String>,
    ) {
        if coefficients.precedence_composites.contains(metric) {
            leaves.insert(metric.to_string());
            return;
        }
        let Some(parts) = coefficients.composite_metrics.get(metric) else {
            leaves.insert(metric.to_string());
            return;
        };
        if !visiting.insert(metric.to_string()) {
            return;
        }
        for part in parts.keys() {
            expand(part, coefficients, leaves, visiting);
        }
        visiting.remove(metric);
    }

    let mut leaves = std::collections::BTreeSet::new();
    for group_weights in coefficients.final_score_weights.values() {
        for group in group_weights.keys() {
            let Some(metric_weights) = coefficients.group_weights.get(group) else {
                continue;
            };
            for metric in metric_weights.keys() {
                expand(
                    metric,
                    coefficients,
                    &mut leaves,
                    &mut std::collections::BTreeSet::new(),
                );
            }
        }
    }
    leaves
}

fn role_status<'a>(model: &'a ModelRecord, role: &str) -> &'a str {
    match model.evidence.roles.get(role) {
        Some(coverage) if !coverage.provisional => "ranked",
        _ => "provisional",
    }
}

fn is_auxiliary_metric(metric: &str) -> bool {
    metric.ends_with("Uncertainty")
        || metric.ends_with("SEM")
        || metric.ends_with("CILow")
        || metric.ends_with("CIHigh")
}

fn public_raw_metrics<'a>(
    model: &'a ModelRecord,
    coefficients: &'a Coefficients,
) -> impl Iterator<Item = &'a String> {
    model
        .raw_metrics
        .keys()
        .filter(|metric| coefficients.metrics.contains_key(*metric) || is_auxiliary_metric(metric))
}

fn render_metric_evidence(out: &mut String, model: &ModelRecord, coefficients: &Coefficients) {
    for metric in public_raw_metrics(model, coefficients) {
        out.push_str(&format!(
            "[models.metric_evidence.{}]\n",
            toml_string(metric)
        ));
        // Every ingested observation is a direct same-product measurement,
        // including cited manual overrides.
        out.push_str(&format!("class = {}\n", toml_string("direct")));
        if let Some(source) = model.metric_sources.get(metric) {
            out.push_str(&format!("source = {}\n", toml_string(source)));
        }
        if let Some(note) = model.override_notes.get(metric) {
            out.push_str(&format!("citation = {}\n", toml_string(note)));
        }
        out.push('\n');
    }
}

fn render_evidence_summary(out: &mut String, model: &ModelRecord) {
    for (group, coverage) in &model.evidence.groups {
        out.push_str(&format!(
            "[models.evidence.groups.{}]\n",
            toml_string(group)
        ));
        render_coverage(out, coverage, false);
    }
    for (role, coverage) in &model.evidence.roles {
        out.push_str(&format!("[models.evidence.roles.{}]\n", toml_string(role)));
        render_coverage(out, coverage, true);
    }
}

fn render_coverage(out: &mut String, coverage: &EvidenceCoverage, include_status: bool) {
    out.push_str(&format!("direct = {}\n", format_float(coverage.direct)));
    out.push_str(&format!("reported = {}\n", format_float(coverage.reported)));
    out.push_str(&format!("missing = {}\n", format_float(coverage.missing)));
    out.push_str(&format!(
        "effective = {}\n",
        format_float(coverage.effective)
    ));
    out.push_str(&format!("family_count = {}\n", coverage.family_count));
    out.push_str(&format!(
        "direct_families = {}\n",
        toml_array(coverage.direct_families.iter().cloned())
    ));
    if include_status {
        out.push_str(&format!(
            "core_direct = {}\n",
            format_float(coverage.core_direct)
        ));
        out.push_str(&format!(
            "core_family_count = {}\n",
            coverage.core_family_count
        ));
        out.push_str(&format!(
            "core_direct_families = {}\n",
            toml_array(coverage.core_direct_families.iter().cloned())
        ));
        out.push_str(&format!(
            "historical_family_count = {}\n",
            coverage.historical_family_count
        ));
        out.push_str(&format!(
            "historical_direct_families = {}\n",
            toml_array(coverage.historical_direct_families.iter().cloned())
        ));
        out.push_str(&format!(
            "qualification_path = {}\n",
            toml_string(qualification_path(coverage.qualification_path))
        ));
        out.push_str(&format!("provisional = {}\n", coverage.provisional));
    }
    out.push('\n');
}

fn qualification_path(path: EligibilityQualificationPath) -> &'static str {
    match path {
        EligibilityQualificationPath::Standard => "standard",
        EligibilityQualificationPath::Breadth => "breadth",
        EligibilityQualificationPath::CoreStandard => "core_standard",
        EligibilityQualificationPath::CoreCorroborated => "core_corroborated",
        EligibilityQualificationPath::CoreBreadth => "core_breadth",
        EligibilityQualificationPath::HistoricalBreadth => "historical_breadth",
        EligibilityQualificationPath::Unqualified => "unqualified",
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
