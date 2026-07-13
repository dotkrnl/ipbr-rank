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
/// `scales` fixes each axis's [min, max] mapping to radius 50 — the same
/// reference frame for every radar on the page (hero and per-row) so shapes
/// are comparable to one another, not just internally self-consistent. Tick
/// rings divide the scaled range into quarters. Hero variant places axis
/// labels at radius 60; mini variant omits them.
pub fn render_radar(
    slices: &[RadarSlice<'_>],
    variant: RadarVariant,
    scales: RadarScales,
) -> String {
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
        let points = polygon_points(slice, scales);
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

/// Minimum span between an axis's min and max. Without this, an axis whose
/// reference cohort ties on that metric would yield a zero-width range and a
/// divide-by-zero.
const RADAR_MIN_RANGE: f64 = 10.0;

/// One axis's [min, max] mapping: min plots at the centre, max at the outer
/// ring. Scores outside the range clamp to the nearer end.
#[derive(Clone, Copy, Debug)]
pub struct RadarScale {
    min: f64,
    max: f64,
}

impl RadarScale {
    /// Build a scale from an observed (min, max) pair, e.g. the low/high of a
    /// reference cohort on one axis. Falls back to a neutral 60-100 range if
    /// the inputs aren't finite (empty cohort).
    pub fn from_range(min: f64, max: f64) -> Self {
        if !min.is_finite() || !max.is_finite() {
            return RadarScale {
                min: 60.0,
                max: 100.0,
            };
        }
        RadarScale {
            min,
            max: max.max(min + RADAR_MIN_RANGE),
        }
    }
}

/// The four axes' scales, fixed once per page render (typically from a
/// reference cohort) and shared by every radar drawn on that page.
#[derive(Clone, Copy, Debug)]
pub struct RadarScales {
    pub idea: RadarScale,
    pub plan: RadarScale,
    pub build: RadarScale,
    pub review: RadarScale,
}

fn polygon_points(slice: &RadarSlice<'_>, scales: RadarScales) -> String {
    let r = |score: f64, scale: RadarScale| {
        let range = (scale.max - scale.min).max(RADAR_MIN_RANGE);
        let clamped = score.clamp(scale.min, scale.max);
        ((clamped - scale.min) / range) * RADAR_RADIUS
    };
    let idea = r(slice.idea, scales.idea);
    let plan = r(slice.plan, scales.plan);
    let build = r(slice.build, scales.build);
    let review = r(slice.review, scales.review);
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

    fn uniform_scales(scale: RadarScale) -> RadarScales {
        RadarScales {
            idea: scale,
            plan: scale,
            build: scale,
            review: scale,
        }
    }

    #[test]
    fn from_range_maps_min_to_zero_and_max_to_full_radius() {
        let scales = uniform_scales(RadarScale::from_range(70.0, 90.0));
        let svg = render_radar(&[slice(false)], RadarVariant::Mini, scales);
        // (80-70)/20*50=25, (81-70)/20*50=27.5, (82-70)/20*50=30, (83-70)/20*50=32.5
        assert!(svg.contains(r#"points="0,-25.0 27.5,0 0,30.0 -32.5,0""#));
    }

    #[test]
    fn from_range_floors_a_degenerate_span() {
        let s = RadarScale::from_range(50.0, 52.0);
        assert_eq!(s.max - s.min, RADAR_MIN_RANGE);
    }

    #[test]
    fn from_range_falls_back_when_bounds_are_not_finite() {
        let s = RadarScale::from_range(f64::NAN, 10.0);
        assert_eq!((s.min, s.max), (60.0, 100.0));
    }

    #[test]
    fn each_axis_maps_independently_against_its_own_scale() {
        let scales = RadarScales {
            idea: RadarScale::from_range(0.0, 100.0),
            plan: RadarScale::from_range(50.0, 100.0),
            build: RadarScale::from_range(80.0, 100.0),
            review: RadarScale::from_range(90.0, 100.0),
        };
        let mid = RadarSlice {
            idea: 50.0,
            plan: 75.0,
            build: 90.0,
            review: 95.0,
            class: "solo",
            label: None,
            provisional: false,
        };
        let svg = render_radar(&[mid], RadarVariant::Mini, scales);
        // Each score sits at the midpoint of its own axis's range, despite
        // the four ranges having wildly different widths (100/50/20/10) —
        // every axis should still plot at exactly half the radius.
        assert!(svg.contains(r#"points="0,-25.0 25.0,0 0,25.0 -25.0,0""#));
    }

    #[test]
    fn scores_outside_the_reference_range_clamp_to_centre_or_ring() {
        let scales = uniform_scales(RadarScale::from_range(40.0, 60.0));
        let below = RadarSlice {
            idea: 10.0,
            plan: 10.0,
            build: 10.0,
            review: 10.0,
            class: "solo",
            label: None,
            provisional: false,
        };
        let above = RadarSlice {
            idea: 200.0,
            plan: 200.0,
            build: 200.0,
            review: 200.0,
            ..below
        };
        let svg = render_radar(&[below, above], RadarVariant::Mini, scales);
        assert!(svg.contains(r#"points="0,-0.0 0.0,0 0,0.0 -0.0,0""#));
        assert!(svg.contains(r#"points="0,-50.0 50.0,0 0,50.0 -50.0,0""#));
    }

    #[test]
    fn provisional_slices_use_a_dedicated_class() {
        let scales = uniform_scales(RadarScale::from_range(70.0, 90.0));
        let svg = render_radar(&[slice(true)], RadarVariant::Mini, scales);
        assert!(svg.contains(r#"class="radar-poly solo provisional""#));
    }
}
