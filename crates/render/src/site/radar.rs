use std::fmt::Write;

use super::html_escape;

/// One polygon's worth of role scores (0-100) plus a CSS class hook used to
/// pick its fill colour. The renderer doesn't decide which colour a given
/// rank gets — that's expressed via the supplied class so the stylesheet
/// stays the source of truth.
#[derive(Clone, Copy, Debug)]
pub struct RadarSlice<'a> {
    pub idea: f64,
    pub plan: f64,
    pub build: f64,
    pub review: f64,
    pub class: &'a str,
    pub label: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub enum RadarVariant {
    /// 320px hero radar with axis labels and tick rings.
    Hero,
    /// ~96px expansion-row radar; tick rings only, no axis labels.
    Mini,
}

/// Inline SVG for a 4-axis radar chart (Idea/Plan/Build/Review).
///
/// All polygons share a single coordinate system: the axes go to radius 50
/// at score=100. Tick rings sit at 25/50/75/100. Hero variant places axis
/// labels at radius 60; mini variant omits them.
pub fn render_radar(slices: &[RadarSlice<'_>], variant: RadarVariant) -> String {
    let (class, view_box) = match variant {
        RadarVariant::Hero => ("radar radar-hero", "-62 -62 124 124"),
        RadarVariant::Mini => ("radar radar-mini", "-56 -56 112 112"),
    };

    let mut svg = String::new();
    write!(
        svg,
        r#"<svg class="{class}" viewBox="{view_box}" preserveAspectRatio="xMidYMid meet" role="img" aria-label="Role scores across Idea, Plan, Build, Review">"#
    )
    .unwrap();

    // Concentric tick rings.
    for radius in [12.5_f64, 25.0, 37.5, 50.0] {
        write!(svg, r#"<circle class="radar-grid" r="{radius:.1}"/>"#).unwrap();
    }

    // Four axis spokes.
    for (x, y) in [(0.0, -50.0_f64), (50.0, 0.0), (0.0, 50.0), (-50.0, 0.0)] {
        write!(
            svg,
            r#"<line class="radar-axis" x1="0" y1="0" x2="{x:.1}" y2="{y:.1}"/>"#
        )
        .unwrap();
    }

    // Per-render dynamic scale: each radar maps its own [min - 10, max] of
    // shown values to [0, RADIUS]. This keeps polygons distinguishable
    // regardless of how clustered or spread the underlying scores are.
    let scale = compute_scale(slices);

    // Polygons, one per slice. Drawn in supplied order so callers control
    // z-stacking (e.g. rank-1 last so it sits on top).
    for slice in slices {
        let points = polygon_points(slice, scale);
        let title = slice
            .label
            .map(|label| format!("<title>{}</title>", html_escape(label)))
            .unwrap_or_default();
        write!(
            svg,
            r#"<polygon class="radar-poly {cls}" points="{points}">{title}</polygon>"#,
            cls = slice.class,
        )
        .unwrap();
    }

    if matches!(variant, RadarVariant::Hero) {
        for (label, x, y, role) in [
            ("I", 0.0_f64, -56.0_f64, "idea"),
            ("P", 56.0, 1.0, "plan"),
            ("B", 0.0, 58.0, "build"),
            ("R", -56.0, 1.0, "review"),
        ] {
            write!(
                svg,
                r#"<text class="radar-label {role}" x="{x:.1}" y="{y:.1}" text-anchor="middle" dominant-baseline="middle">{label}</text>"#
            )
            .unwrap();
        }
    }

    svg.push_str("</svg>");
    svg
}

const RADAR_RADIUS: f64 = 50.0;

/// Minimum span between baseline and ceiling. Without this, a slice whose
/// scores happen to be identical (e.g. a deterministic fixture) would yield
/// a zero-width range and a degenerate polygon at the centre.
const RADAR_MIN_RANGE: f64 = 10.0;

#[derive(Clone, Copy, Debug)]
struct RadarScale {
    baseline: f64,
    ceiling: f64,
}

/// Compute the [baseline, ceiling] window for a single render: take the
/// data's min and max across every slice / axis, drop the baseline by 10
/// so the lowest-scored vertex doesn't sit exactly at the centre, and
/// guarantee a minimum span so the math doesn't divide by zero.
fn compute_scale(slices: &[RadarSlice<'_>]) -> RadarScale {
    let mut min_score = f64::INFINITY;
    let mut max_score = f64::NEG_INFINITY;
    for slice in slices {
        for v in [slice.idea, slice.plan, slice.build, slice.review] {
            min_score = min_score.min(v);
            max_score = max_score.max(v);
        }
    }
    if !min_score.is_finite() || !max_score.is_finite() {
        return RadarScale {
            baseline: 60.0,
            ceiling: 100.0,
        };
    }
    let baseline = (min_score - 10.0).clamp(0.0, 90.0);
    let ceiling = max_score.max(baseline + RADAR_MIN_RANGE).min(100.0);
    RadarScale { baseline, ceiling }
}

fn polygon_points(slice: &RadarSlice<'_>, scale: RadarScale) -> String {
    let range = (scale.ceiling - scale.baseline).max(RADAR_MIN_RANGE);
    let r = |score: f64| {
        let clamped = score.clamp(scale.baseline, scale.ceiling);
        ((clamped - scale.baseline) / range) * RADAR_RADIUS
    };
    let (idea, plan, build, review) = (
        r(slice.idea),
        r(slice.plan),
        r(slice.build),
        r(slice.review),
    );
    // Order: top (idea), right (plan), bottom (build), left (review).
    format!(
        "0,{ti:.1} {pr:.1},0 0,{bb:.1} {rl:.1},0",
        ti = -idea,
        pr = plan,
        bb = build,
        rl = -review,
    )
}
