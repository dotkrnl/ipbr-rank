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

    // Polygons, one per slice. Drawn in supplied order so callers control
    // z-stacking (e.g. rank-1 last so it sits on top).
    for slice in slices {
        let points = polygon_points(slice);
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

/// Use the same absolute 0-100 scale for every radar. A dynamic per-model or
/// per-cohort scale exaggerates small differences and makes two polygons with
/// the same geometry represent different scores.
fn polygon_points(slice: &RadarSlice<'_>) -> String {
    let r = |score: f64| score.clamp(0.0, 100.0) / 100.0 * RADAR_RADIUS;
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
