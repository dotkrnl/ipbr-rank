//! The expanded row: why a model scores what it scores.
//!
//! A dashboard band opens the panel — who the model is, the shape of its four
//! scores, and the scores themselves — then the panel answers three questions
//! in order, and nothing else:
//!   1. How well is each of its four scores evidenced?
//!   2. What went into each score, and how much did each input count?
//!   3. What was actually measured, and where did the numbers come from?
//!
//! Everything is named the way a reader would name it — see [`super::labels`].
//! Scoring-engine intermediates (the `CRE`/`GEN`/`OPS_*` groups, the `50.0`
//! placeholder a group takes when nothing in it was measured, error bars,
//! precedence duplicates) are engine concerns and do not appear.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use crate::Scoreboard;

use super::html_escape;
use super::index::{RoleSpec, composite, role_specs, score_tier};
use super::labels::{Area, input_label, metric_label, ordinal, source_label};
use super::radar::{RadarScales, RadarSlice, render_radar_mini};

/// Inputs below this share of a role are rolled into a single trailing line.
/// They exist, they are weighted, and they are honestly accounted for — but at
/// 1% of a score they are noise in a list a reader is trying to scan.
const MINOR_INPUT_SHARE: f64 = 0.03;

pub fn render_detail(
    scoreboard: &Scoreboard,
    model: &ipbr_core::ModelRecord,
    radar_scales: RadarScales,
) -> String {
    let mut html = String::from(r#"<div class="detail">"#);

    html.push_str(&render_band(scoreboard, model, radar_scales));

    html.push_str(r#"<div class="det-top">"#);
    for spec in role_specs() {
        html.push_str(&render_role_card(scoreboard, model, &spec));
    }
    html.push_str("</div>");

    html.push_str(&render_results(scoreboard, model));
    html.push_str(&render_reference(model));
    html.push_str(&render_provenance(scoreboard, model));

    html.push_str("</div>");
    html
}

/// The dashboard band: who this model is, the shape of its four scores, and
/// the scores themselves — everything a reader needs before deciding whether
/// to dig into the evidence below.
fn render_band(
    scoreboard: &Scoreboard,
    model: &ipbr_core::ModelRecord,
    radar_scales: RadarScales,
) -> String {
    let s = &model.scores;
    let evidence = scoreboard.coefficients.evidence.clone().unwrap_or_default();
    let radar = render_radar_mini(
        &RadarSlice {
            idea: s.i_raw,
            plan: s.p_raw,
            build: s.b_raw,
            review: s.r,
            class: "solo",
            label: Some(model.display_name.as_str()),
            provisional: ipbr_core::balanced_is_provisional(model, &evidence),
        },
        radar_scales,
    );

    // Overall rank: the unweighted mean of the four roles, as in the hero.
    let overall = rank_for(scoreboard, &model.canonical_id, |m| Some(composite(m)))
        .map(|(place, total)| format!("{} of {total} overall", ordinal(place)))
        .unwrap_or_else(|| "unranked overall".to_string());

    let mut scores = String::new();
    for spec in role_specs() {
        let value = (spec.from_record)(model);
        let rank = rank_for(scoreboard, &model.canonical_id, |m| {
            Some((spec.from_record)(m))
        });
        let provisional = model
            .evidence
            .roles
            .get(spec.evidence_key)
            .is_some_and(|coverage| coverage.provisional);
        write!(
            scores,
            r#"<div class="bs role-{id}"><span class="bs-label">{label}</span><span class="bs-score" data-tier="{tier}" data-status="{status}">{value:.1}</span><span class="bs-rank">{rank}</span></div>"#,
            id = spec.id,
            label = spec.label,
            tier = score_tier(value),
            status = if provisional {
                "provisional"
            } else {
                "ranked"
            },
            rank = match rank {
                Some((place, total)) => format!("{} of {total}", ordinal(place)),
                None => "unranked".to_string(),
            },
        )
        .unwrap();
    }

    format!(
        r#"<header class="det-band"><div class="band-id"><div class="band-name">{name}</div><div class="band-sub"><span class="band-vendor">{vendor}</span><span class="band-rank" title="Unweighted mean of the four role scores">{overall}</span></div></div><div class="band-radar">{radar}</div><div class="band-scores">{scores}</div><p class="band-summary">{summary}</p></header>"#,
        name = html_escape(&model.display_name),
        vendor = html_escape(model.vendor.as_str()),
        summary = render_band_summary_html(scoreboard, model),
    )
}

/// One strictly factual sentence for the band: the model's best and worst role
/// by rank, and in how many roles the evidence earns a ranked badge. Neutral
/// wording only — the numbers speak, no adjectives.
fn render_band_summary(scoreboard: &Scoreboard, model: &ipbr_core::ModelRecord) -> String {
    let specs = role_specs();
    let mut places: Vec<(&str, usize, usize)> = Vec::new();
    let mut ranked_roles = 0usize;
    for spec in &specs {
        if let Some((place, total)) = rank_for(scoreboard, &model.canonical_id, |m| {
            Some((spec.from_record)(m))
        }) {
            places.push((spec.label, place, total));
        }
        if model
            .evidence
            .roles
            .get(spec.evidence_key)
            .is_some_and(|coverage| !coverage.provisional)
        {
            ranked_roles += 1;
        }
    }

    let ranked = format!("Ranked in {ranked_roles} of {} roles", specs.len());
    let Some(best) = places.iter().min_by_key(|(_, place, _)| *place) else {
        return ranked;
    };
    let worst = places
        .iter()
        .max_by_key(|(_, place, _)| *place)
        .expect("places is non-empty when best exists");
    if best.1 == worst.1 {
        return format!(
            "All four roles tied at {} of {} · {ranked}",
            ordinal(best.1),
            best.2
        );
    }
    format!(
        "Strongest: {} ({} of {}) · Weakest: {} ({} of {}) · {ranked}",
        capitalize(best.0),
        ordinal(best.1),
        best.2,
        capitalize(worst.0),
        ordinal(worst.1),
        worst.2,
    )
}

/// The summary, split at its `·` separators into unbreakable segments: on
/// narrow screens it wraps segment-per-line instead of splitting mid-fact.
fn render_band_summary_html(scoreboard: &Scoreboard, model: &ipbr_core::ModelRecord) -> String {
    render_band_summary(scoreboard, model)
        .split(" · ")
        .enumerate()
        .map(|(idx, segment)| {
            let prefix = if idx == 0 { "" } else { "· " };
            format!(r#"<span class="ss">{prefix}{segment}</span>"#)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Role labels are stored lowercase for the table headers; sentence-initial
/// positions in the band want a capital.
fn capitalize(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// One role: how well its score is evidenced, and what fed it. The score
/// itself lives in the dashboard band above and is not repeated here.
fn render_role_card(
    scoreboard: &Scoreboard,
    model: &ipbr_core::ModelRecord,
    spec: &RoleSpec,
) -> String {
    let rank = rank_for(scoreboard, &model.canonical_id, |m| {
        Some((spec.from_record)(m))
    });

    let mut html = format!(
        r#"<section class="det-role role-{id}"><header class="dr-head"><h4 class="dr-title">{label}</h4><span class="dr-rank">{rank}</span></header>"#,
        id = spec.id,
        label = spec.label,
        rank = match rank {
            Some((place, total)) => format!("{} of {total}", ordinal(place)),
            None => "unranked".to_string(),
        },
    );

    html.push_str(&render_role_evidence(model, spec));
    html.push_str(&render_role_inputs(scoreboard, model, spec));
    html.push_str("</section>");
    html
}

/// How much of the role's evidence was actually observed, in words rather than
/// in the engine's `direct` / `effective` / `family_count` vocabulary. An
/// evidence *family* collapses correlated variants of one benchmark, so from a
/// reader's side it simply is "an unrelated benchmark".
fn render_role_evidence(model: &ipbr_core::ModelRecord, spec: &RoleSpec) -> String {
    let Some(coverage) = model.evidence.roles.get(spec.evidence_key) else {
        return r#"<p class="dr-evidence"><span class="dr-note">Evidence summary unavailable.</span></p>"#
            .to_string();
    };

    let (badge_class, badge, tail) = if coverage.provisional {
        (
            "provisional",
            "provisional",
            " Too little of it has been measured to rank this score with confidence.",
        )
    } else {
        ("ranked", "ranked", "")
    };

    format!(
        r#"<p class="dr-evidence"><span class="dr-badge {badge_class}">{badge}</span> <span class="dr-meter" aria-hidden="true"><i style="width:{pct:.0}%"></i></span> <span class="dr-ev-text">Measured on <b>{pct:.0}%</b> of the evidence this score looks for, across <b>{families}</b> unrelated {benchmarks}.{tail}</span></p>"#,
        pct = coverage.direct * 100.0,
        families = coverage.family_count,
        benchmarks = if coverage.family_count == 1 {
            "benchmark"
        } else {
            "benchmarks"
        },
    )
}

/// The benchmarks and blends that make up the role score, largest share first.
/// Share is the input's real weight through the scoring graph, so the numbers a
/// reader adds up are the numbers the scorer used.
fn render_role_inputs(
    scoreboard: &Scoreboard,
    model: &ipbr_core::ModelRecord,
    spec: &RoleSpec,
) -> String {
    let inputs = role_inputs(&scoreboard.coefficients, spec.evidence_key);
    if inputs.is_empty() {
        return String::new();
    }

    let mut html = String::from(r#"<ul class="dr-inputs">"#);
    let mut minor_share = 0.0;
    let mut minor_count = 0usize;

    for (key, share) in &inputs {
        let Some(name) = input_label(key) else {
            continue;
        };
        if *share < MINOR_INPUT_SHARE {
            minor_share += share;
            minor_count += 1;
            continue;
        }

        // A name too long for the column is truncated, so the hover note always
        // carries it in full — and, for a blend, what it is a blend of.
        let parts = composite_parts(&scoreboard.coefficients, key);
        let title = if parts.is_empty() {
            format!(r#" title="{}""#, html_escape(name))
        } else {
            format!(
                r#" title="{name} — blends {parts}""#,
                name = html_escape(name),
                parts = html_escape(&parts.join(", ")),
            )
        };

        match model.metrics.get(key) {
            Some(value) => {
                let key_owned = key.clone();
                let place = rank_for(scoreboard, &model.canonical_id, |m| {
                    m.metrics.get(&key_owned).copied()
                });
                write!(
                    html,
                    r#"<li><span class="di-name"{title}>{name}</span><span class="di-share">{share:.0}%</span><span class="di-bar role-{role_id}"><i style="width:{value:.0}%"></i></span><span class="di-score" data-tier="{tier}">{value:.1}</span><span class="di-rank">{place}</span></li>"#,
                    name = html_escape(name),
                    share = share * 100.0,
                    role_id = spec.id,
                    tier = score_tier(*value),
                    place = place
                        .map(|(p, _)| ordinal(p))
                        .unwrap_or_else(|| "—".to_string()),
                )
                .unwrap();
            }
            None => {
                write!(
                    html,
                    r#"<li class="di-unmeasured"><span class="di-name"{title}>{name}</span><span class="di-share">{share:.0}%</span><span class="di-none">not measured</span></li>"#,
                    name = html_escape(name),
                    share = share * 100.0,
                )
                .unwrap();
            }
        }
    }

    if minor_count > 0 {
        write!(
            html,
            r#"<li class="di-minor"><span class="di-name">{minor_count} smaller {inputs}</span><span class="di-share">{share:.0}%</span></li>"#,
            inputs = if minor_count == 1 { "input" } else { "inputs" },
            share = minor_share * 100.0,
        )
        .unwrap();
    }

    html.push_str("</ul>");
    html
}

/// Every benchmark this model was actually measured on, grouped by subject and
/// marked with the scores it feeds. Collapsed: it is the evidence behind the
/// panel above, not the first thing to read.
fn render_results(scoreboard: &Scoreboard, model: &ipbr_core::ModelRecord) -> String {
    let feeds = role_feeders(&scoreboard.coefficients);

    let mut by_area: BTreeMap<Area, Vec<(&String, &f64, super::labels::MetricLabel)>> =
        BTreeMap::new();
    for (key, raw) in &model.raw_metrics {
        let Some(label) = metric_label(key) else {
            continue;
        };
        if label.area == Area::Reference {
            continue;
        }
        by_area
            .entry(label.area)
            .or_default()
            .push((key, raw, label));
    }
    let measured: usize = by_area.values().map(Vec::len).sum();
    if measured == 0 {
        return String::new();
    }

    let mut html = format!(
        r#"<details class="det-results"><summary>Every benchmark it was measured on ({measured})</summary><p class="det-legend">Coloured dots mark which scores a benchmark feeds: {legend} A benchmark with no dot is reference only.</p>"#,
        legend = role_specs()
            .iter()
            .map(|spec| format!(
                r#"<span class="lg"><i class="dot role-{id}"></i>{label}</span>"#,
                id = spec.id,
                label = spec.label
            ))
            .collect::<String>(),
    );

    for area in Area::results_order() {
        let Some(rows) = by_area.get(&area) else {
            continue;
        };
        let mut rows: Vec<_> = rows.iter().collect();
        rows.sort_by(|a, b| a.2.name.cmp(b.2.name));

        write!(
            html,
            r#"<section class="det-area"><h5>{title}</h5><div class="res-grid"><span class="res-head">benchmark</span><span class="res-head">feeds</span><span class="res-head res-head-num">measured</span><span class="res-head res-head-bar" aria-hidden="true"></span><span class="res-head res-head-num">score</span><span class="res-head res-head-num">rank</span>"#,
            title = area.title(),
        )
        .unwrap();

        for (key, raw, label) in rows {
            let key_owned = (*key).clone();
            let score = model.metrics.get(*key).copied();
            let place = rank_for(scoreboard, &model.canonical_id, |m| {
                m.metrics.get(&key_owned).copied()
            });
            let dots: String = role_specs()
                .iter()
                .enumerate()
                .filter(|(role, _)| {
                    feeds
                        .get(key.as_str())
                        .is_some_and(|roles| roles.contains(role))
                })
                .map(|(_, spec)| {
                    format!(
                        r#"<i class="dot role-{id}" title="feeds {label}"></i>"#,
                        id = spec.id,
                        label = spec.label
                    )
                })
                .collect();

            write!(
                html,
                r#"<span class="res-name"{note}>{name}</span><span class="res-feeds">{dots}</span><span class="res-raw">{raw}</span><span class="res-bar"><i style="width:{width:.0}%"></i></span><span class="res-score" data-tier="{tier}">{score}</span><span class="res-rank">{place}</span>"#,
                note = citation_title(model, key),
                name = html_escape(label.name),
                raw = html_escape(&label.unit.format(**raw)),
                width = score.unwrap_or(0.0),
                tier = score_tier(score.unwrap_or(0.0)),
                score = score
                    .map(|value| format!("{value:.1}"))
                    .unwrap_or_else(|| "—".to_string()),
                place = match place {
                    Some((p, total)) => format!(
                        r#"{}<span class="res-of"> of {total}</span>"#,
                        ordinal(p)
                    ),
                    None => "—".to_string(),
                },
            )
            .unwrap();
        }
        html.push_str("</div></section>");
    }

    html.push_str("</details>");
    html
}

/// Speed, price, and context window: useful, and deliberately powerless. Their
/// weight into every one of the four scores is exactly zero, so they are shown
/// apart from the evidence rather than mixed into it.
fn render_reference(model: &ipbr_core::ModelRecord) -> String {
    let mut stats = String::new();
    for key in ["OutputSpeed", "TTFT", "BlendedCost", "ContextWindow"] {
        let Some(label) = metric_label(key) else {
            continue;
        };
        let Some(raw) = model.raw_metrics.get(key) else {
            continue;
        };
        write!(
            stats,
            r#"<div class="ref-stat"><span class="ref-name">{name}</span><span class="ref-value">{value}</span></div>"#,
            name = html_escape(label.name),
            value = html_escape(&label.unit.format(*raw)),
        )
        .unwrap();
    }
    if stats.is_empty() {
        return String::new();
    }
    format!(
        r#"<section class="det-ref"><h5>{title}</h5><div class="ref-row">{stats}</div><p class="det-note">Reference only — none of this moves a score.</p></section>"#,
        title = Area::Reference.title(),
    )
}

/// Who measured it, and what is still unmeasured. The gaps are as much a part
/// of reading a score as the numbers are.
fn render_provenance(scoreboard: &Scoreboard, model: &ipbr_core::ModelRecord) -> String {
    let mut html = String::from(r#"<footer class="det-prov">"#);

    html.push_str(r#"<div class="prov-block"><h5>Measured by</h5><p class="pills">"#);
    if model.sources.is_empty() {
        html.push_str(r#"<span class="det-note">No sources recorded.</span>"#);
    } else {
        for source in &model.sources {
            let status = scoreboard
                .source_summary
                .get(source)
                .map(|summary| summary.status.as_str())
                .unwrap_or("unknown");
            write!(
                html,
                r#"<span class="pill" title="{status} feed">{name}</span>"#,
                name = html_escape(source_label(source)),
                status = html_escape(status),
            )
            .unwrap();
        }
    }
    html.push_str("</p></div>");

    html.push_str(r#"<div class="prov-block"><h5>Not measured yet</h5><p class="pills">"#);
    let unmeasured: Vec<&str> = model
        .missing
        .metrics
        .iter()
        .filter_map(|key| input_label(key))
        .collect();
    if unmeasured.is_empty() {
        html.push_str(
            r#"<span class="det-note">Every benchmark that feeds a score has a result.</span>"#,
        );
    } else {
        for name in unmeasured {
            write!(
                html,
                r#"<span class="pill pill-gap">{name}</span>"#,
                name = html_escape(name),
            )
            .unwrap();
        }
    }
    html.push_str("</p></div></footer>");

    html
}

/// The real share each input carries in a role, folded through the scoring
/// groups. An input reachable by two paths (say a preference rating that feeds
/// both the creative and the general group) carries the sum of both.
fn role_inputs(coefficients: &ipbr_core::Coefficients, role: &str) -> Vec<(String, f64)> {
    let mut shares: BTreeMap<String, f64> = BTreeMap::new();
    let Some(groups) = coefficients.final_score_weights.get(role) else {
        return Vec::new();
    };
    for (group, group_weight) in groups {
        let Some(metrics) = coefficients.group_weights.get(group) else {
            continue;
        };
        for (metric, metric_weight) in metrics {
            *shares.entry(metric.clone()).or_default() += group_weight * metric_weight;
        }
    }

    let mut inputs: Vec<(String, f64)> = shares.into_iter().collect();
    inputs.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    inputs
}

/// Reader-facing names of the benchmarks a blend is made of, for its tooltip.
fn composite_parts(coefficients: &ipbr_core::Coefficients, key: &str) -> Vec<&'static str> {
    coefficients
        .composite_metrics
        .get(key)
        .map(|parts| parts.keys().filter_map(|part| input_label(part)).collect())
        .unwrap_or_default()
}

/// Which of the four scores each benchmark feeds, by index into `role_specs()`.
/// Unlike the scorer's own expansion this descends into precedence blends too:
/// either of their inputs can be the observation that lands in the score, so
/// from a reader's side both feed the role.
fn role_feeders(coefficients: &ipbr_core::Coefficients) -> BTreeMap<&str, BTreeSet<usize>> {
    fn walk<'a>(
        metric: &'a str,
        coefficients: &'a ipbr_core::Coefficients,
        role: usize,
        feeders: &mut BTreeMap<&'a str, BTreeSet<usize>>,
        visiting: &mut BTreeSet<&'a str>,
    ) {
        feeders.entry(metric).or_default().insert(role);
        let Some(parts) = coefficients.composite_metrics.get(metric) else {
            return;
        };
        if !visiting.insert(metric) {
            return;
        }
        for part in parts.keys() {
            walk(part, coefficients, role, feeders, visiting);
        }
        visiting.remove(metric);
    }

    let mut feeders: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
    for (role, spec) in role_specs().iter().enumerate() {
        let Some(groups) = coefficients.final_score_weights.get(spec.evidence_key) else {
            continue;
        };
        for group in groups.keys() {
            let Some(metrics) = coefficients.group_weights.get(group) else {
                continue;
            };
            for metric in metrics.keys() {
                walk(
                    metric,
                    coefficients,
                    role,
                    &mut feeders,
                    &mut BTreeSet::new(),
                );
            }
        }
    }
    feeders
}

/// Provenance for a single observation, as a hover note. Upstream URLs stay in
/// `scoreboard.toml`; the static site carries no external references.
fn citation_title(model: &ipbr_core::ModelRecord, metric: &str) -> String {
    let source = model
        .metric_sources
        .get(metric)
        .map(|id| source_label(id))
        .unwrap_or("a public leaderboard");
    let note = if model.curated_overrides.contains(metric) {
        format!("Reported by the vendor and cited by hand; from {source}")
    } else if let Some(citation) = model.metric_citations.get(metric) {
        format!("From {source}: {citation}")
    } else {
        format!("From {source}")
    };
    format!(r#" title="{}""#, html_escape(&sanitize_citation(&note)))
}

/// Evidence notes retain full upstream URLs in `scoreboard.toml`, but the
/// static HTML deliberately has no external-reference markers. Sanitize those
/// marker strings in tooltip text while preserving surrounding provenance.
fn sanitize_citation(input: &str) -> String {
    let mut remaining = input;
    let mut output = String::with_capacity(input.len());
    loop {
        let http = remaining.find("http://");
        let https = remaining.find("https://");
        let Some(start) = (match (http, https) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }) else {
            output.push_str(remaining);
            break;
        };

        output.push_str(&remaining[..start]);
        output.push_str("[upstream URL in scoreboard.toml]");
        let url = &remaining[start..];
        let end = url
            .find(|c: char| c.is_whitespace() || matches!(c, ';' | ',' | ')' | ']'))
            .unwrap_or(url.len());
        remaining = &url[end..];
    }
    // `data:` is also rejected by the self-contained-site validator. It can
    // occur innocently inside prose such as `metadata:` even when no data URI
    // exists, so keep the wording while changing the delimiter.
    output.replace("data:", "data —")
}

/// Dense rank of a model on one lookup, plus how many models the lookup found.
/// Ties that display as the same tenth share a place, matching the leaderboard.
pub(super) fn rank_for(
    scoreboard: &Scoreboard,
    canonical_id: &str,
    lookup: impl Fn(&ipbr_core::ModelRecord) -> Option<f64>,
) -> Option<(usize, usize)> {
    let mut scored: Vec<(f64, &str)> = scoreboard
        .models
        .iter()
        .filter_map(|m| lookup(m).map(|v| (v, m.canonical_id.as_str())))
        .collect();
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(b.1))
    });
    let mut dense_rank = 0;
    let mut previous: Option<String> = None;
    for (score, id) in &scored {
        let display_key = format!("{score:.1}");
        if previous.as_ref() != Some(&display_key) {
            dense_rank += 1;
            previous = Some(display_key);
        }
        if *id == canonical_id {
            return Some((dense_rank, scored.len()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coefficients() -> ipbr_core::Coefficients {
        ipbr_core::Coefficients::load_embedded().unwrap()
    }

    #[test]
    fn role_input_shares_account_for_the_whole_score() {
        let coefficients = coefficients();
        for role in ["I_raw", "P_raw", "B_raw", "R"] {
            let total: f64 = role_inputs(&coefficients, role)
                .iter()
                .map(|(_, share)| share)
                .sum();
            assert!(
                (total - 1.0).abs() < 1e-9,
                "{role} input shares sum to {total}, so the panel would not add up"
            );
        }
    }

    #[test]
    fn an_input_reachable_twice_carries_the_sum_of_both_paths() {
        // Overall preference feeds Idea through both the creative group (15% of
        // 65%) and the general group (35% of 35%).
        let inputs = role_inputs(&coefficients(), "I_raw");
        let (_, share) = inputs
            .iter()
            .find(|(key, _)| key == "LMArenaText")
            .expect("preference rating feeds Idea");
        assert!((share - (0.65 * 0.15 + 0.35 * 0.35)).abs() < 1e-9);
    }

    #[test]
    fn precedence_blends_credit_both_of_their_inputs() {
        let coefficients = coefficients();
        let feeders = role_feeders(&coefficients);
        // Terminal work is a precedence blend: whichever run is available lands
        // in the score, so both runs are shown as feeding Build.
        for key in ["TerminalBench21", "AATerminalBench21"] {
            assert!(
                feeders.get(key).is_some_and(|roles| roles.contains(&2)),
                "{key} feeds Build and should be marked as such"
            );
        }
        assert!(
            !feeders.contains_key("BlendedCost"),
            "price must never be shown as feeding a score"
        );
    }

    #[test]
    fn citations_redact_urls_but_keep_provenance() {
        assert_eq!(
            sanitize_citation(
                "submission metadata: Model: https://huggingface.co/zai-org/GLM-4.6; Org: Z.ai"
            ),
            "submission metadata — Model: [upstream URL in scoreboard.toml]; Org: Z.ai"
        );
        assert_eq!(
            sanitize_citation("source=http://example.test/model, agent=foo"),
            "source=[upstream URL in scoreboard.toml], agent=foo"
        );
    }
}
