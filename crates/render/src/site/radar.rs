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
    /// Whether this shape represents an estimate that does not yet meet the
    /// direct-evidence requirements. Provisional shapes use a dotted outline.
    pub provisional: bool,
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
/// Each render scales its observed score range to radius 50 so differences
/// among the displayed models remain visible. Tick rings divide that scaled
/// range into quarters. Hero variant places axis labels at radius 60; mini
/// variant omits them.
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

    // Per-render range scale. Mapping the displayed [min - 10, max] range to
    // the full radius keeps nearby frontier-model scores distinguishable.
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
            r#"<polygon class="radar-poly {cls}{provisional}" points="{points}">{title}</polygon>"#,
            cls = slice.class,
            provisional = if slice.provisional {
                " provisional"
            } else {
                ""
            },
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
/// scores are identical would yield a zero-width range and a degenerate
/// polygon at the centre.
const RADAR_MIN_RANGE: f64 = 10.0;

#[derive(Clone, Copy, Debug)]
struct RadarScale {
    baseline: f64,
    ceiling: f64,
}

fn compute_scale(slices: &[RadarSlice<'_>]) -> RadarScale {
    let mut min_score = f64::INFINITY;
    let mut max_score = f64::NEG_INFINITY;
    for slice in slices {
        for value in [slice.idea, slice.plan, slice.build, slice.review] {
            min_score = min_score.min(value);
            max_score = max_score.max(value);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(provisional: bool) -> RadarSlice<'static> {
        RadarSlice {
            idea: 80.0,
            plan: 81.0,
            build: 82.0,
            review: 83.0,
            class: "solo",
            label: Some("Example"),
            provisional,
        }
    }

    #[test]
    fn range_scales_each_render() {
        let svg = render_radar(&[slice(false)], RadarVariant::Mini);
        assert!(svg.contains(r#"points="0,-38.5 42.3,0 0,46.2 -50.0,0""#));
    }

    #[test]
    fn provisional_slices_use_a_dedicated_class() {
        let svg = render_radar(&[slice(true)], RadarVariant::Mini);
        assert!(svg.contains(r#"class="radar-poly solo provisional""#));
    }
}
