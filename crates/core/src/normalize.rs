const EPS: f64 = 1e-12;

pub(crate) fn percentile_linear(sorted: &[f64], q: f64) -> f64 {
    debug_assert!(!sorted.is_empty());
    if sorted.len() == 1 {
        return sorted[0];
    }
    let idx = q * (sorted.len() as f64 - 1.0);
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = idx - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

pub fn robust_norm(
    value: f64,
    all_values: &[f64],
    higher_better: bool,
    log_scale: bool,
) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let mapped: Vec<f64> = if log_scale {
        all_values
            .iter()
            .filter(|v| v.is_finite() && **v > 0.0)
            .map(|v| v.ln())
            .collect()
    } else {
        all_values
            .iter()
            .filter(|v| v.is_finite())
            .copied()
            .collect()
    };
    if mapped.is_empty() {
        return None;
    }
    let v = if log_scale {
        if value <= 0.0 {
            return None;
        }
        value.ln()
    } else {
        value
    };
    let mut sorted = mapped;
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p5 = percentile_linear(&sorted, 0.05);
    let p95 = percentile_linear(&sorted, 0.95);
    if (p95 - p5).abs() < EPS {
        return Some(50.0);
    }
    let raw = (v - p5) / (p95 - p5);
    let clipped = raw.clamp(0.0, 1.0);
    let score = if higher_better {
        100.0 * clipped
    } else {
        100.0 * (1.0 - clipped)
    };
    Some(score)
}

/// Tail-penalty curve for operational metrics like speed/cost. The top
/// 80 % of the population maps to 70..100 (mild penalty), the bottom 20 %
/// maps to 0..70 (sharp penalty). Operates in the same percentile-linear
/// space as `robust_norm` (so log-scale + p20/p80 boundaries) but bends
/// the linear ramp at the 20th percentile instead of the 5th/95th.
pub fn tail_penalty_norm(
    value: f64,
    all_values: &[f64],
    higher_better: bool,
    log_scale: bool,
) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let mapped: Vec<f64> = if log_scale {
        all_values
            .iter()
            .filter(|v| v.is_finite() && **v > 0.0)
            .map(|v| v.ln())
            .collect()
    } else {
        all_values
            .iter()
            .filter(|v| v.is_finite())
            .copied()
            .collect()
    };
    if mapped.is_empty() {
        return None;
    }
    let v = if log_scale {
        if value <= 0.0 {
            return None;
        }
        value.ln()
    } else {
        value
    };
    let mut sorted = mapped;
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // For small populations p2/p98 is dominated by individual rows and the
    // bend point becomes noisy. Fall back to min/max so the tail keeps the
    // intended shape; switch to p2/p98 once we have enough rows that the
    // winsorization is meaningful.
    const SMALL_POP_THRESHOLD: usize = 20;
    let (lo, hi) = if sorted.len() < SMALL_POP_THRESHOLD {
        (sorted[0], sorted[sorted.len() - 1])
    } else {
        (
            percentile_linear(&sorted, 0.02),
            percentile_linear(&sorted, 0.98),
        )
    };
    if (hi - lo).abs() < EPS {
        return Some(50.0);
    }
    let raw = (v - lo) / (hi - lo);
    let clipped = raw.clamp(0.0, 1.0);
    // Map to the population position in 0..1 oriented so that "good" is 1.
    let position = if higher_better {
        clipped
    } else {
        1.0 - clipped
    };
    // Two-piece linear bend at p20 of the position scale: top 80% squeezes
    // into 70..100, bottom 20% spans 0..70. Net effect: most models cluster
    // in the high band, only outliers at the slow tail get penalized.
    let score = if position >= 0.20 {
        70.0 + (position - 0.20) / 0.80 * 30.0
    } else {
        position / 0.20 * 70.0
    };
    Some(score)
}

pub fn as_score_0_100(value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let scaled = if value.abs() <= 1.0 + EPS {
        value * 100.0
    } else {
        value
    };
    Some(scaled.clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
    }

    #[test]
    fn empty_population_returns_none() {
        assert!(robust_norm(5.0, &[], true, false).is_none());
    }

    #[test]
    fn single_value_returns_50() {
        let v = robust_norm(7.0, &[7.0], true, false).unwrap();
        approx(v, 50.0);
    }

    #[test]
    fn identical_population_returns_50() {
        let v = robust_norm(3.0, &[3.0, 3.0, 3.0], true, false).unwrap();
        approx(v, 50.0);
    }

    #[test]
    fn linear_distribution_endpoints() {
        let pop: Vec<f64> = (0..=100).map(|i| i as f64).collect();
        let lo = robust_norm(5.0, &pop, true, false).unwrap();
        let hi = robust_norm(95.0, &pop, true, false).unwrap();
        approx(lo, 0.0);
        approx(hi, 100.0);
        let mid = robust_norm(50.0, &pop, true, false).unwrap();
        approx(mid, 50.0);
    }

    #[test]
    fn lower_better_inverts() {
        let pop: Vec<f64> = (0..=100).map(|i| i as f64).collect();
        let v = robust_norm(95.0, &pop, false, false).unwrap();
        approx(v, 0.0);
        let v = robust_norm(5.0, &pop, false, false).unwrap();
        approx(v, 100.0);
    }

    #[test]
    fn clamps_outside_p5_p95() {
        let pop: Vec<f64> = (0..=100).map(|i| i as f64).collect();
        approx(robust_norm(-50.0, &pop, true, false).unwrap(), 0.0);
        approx(robust_norm(200.0, &pop, true, false).unwrap(), 100.0);
    }

    #[test]
    fn log_scale_normalizes_in_log_space() {
        let pop: Vec<f64> = vec![1.0, 10.0, 100.0, 1000.0, 10_000.0];
        let v = robust_norm(100.0, &pop, true, true).unwrap();
        assert!(v > 30.0 && v < 70.0, "expected mid-range, got {v}");
    }

    #[test]
    fn log_scale_non_positive_returns_none() {
        let pop: Vec<f64> = vec![1.0, 10.0, 100.0];
        assert!(robust_norm(0.0, &pop, true, true).is_none());
    }

    #[test]
    fn tail_penalty_keeps_top_band_compressed() {
        // Population: 0..=99 raw values. Position p20 is the 20th percentile,
        // so values below it land in the steep 0..70 ramp; values above land
        // in the gentle 70..100 band.
        let pop: Vec<f64> = (0..=99).map(|i| i as f64).collect();
        // The slowest model (close to p2) should sit near 0.
        let bottom = tail_penalty_norm(2.0, &pop, true, false).unwrap();
        assert!(bottom < 5.0, "expected near-zero, got {bottom}");
        // 20th percentile should land at the bend (≈70).
        let bend = tail_penalty_norm(20.0, &pop, true, false).unwrap();
        assert!((bend - 70.0).abs() < 5.0, "expected ~70, got {bend}");
        // Most of the upper population (e.g. p50) should sit comfortably
        // above 80 — the top band is intentionally compressed.
        let mid = tail_penalty_norm(50.0, &pop, true, false).unwrap();
        assert!(mid > 80.0 && mid < 90.0, "expected 80-90, got {mid}");
        // Top end approaches but caps at 100.
        let top = tail_penalty_norm(99.0, &pop, true, false).unwrap();
        assert!(top > 95.0, "expected near-100, got {top}");
    }

    #[test]
    fn tail_penalty_inverts_for_lower_better() {
        let pop: Vec<f64> = (0..=99).map(|i| i as f64).collect();
        // For lower-is-better metrics (cost/latency), the *highest* raw
        // value is the worst — should land at the steep tail.
        let worst = tail_penalty_norm(99.0, &pop, false, false).unwrap();
        assert!(worst < 5.0, "expected near-zero, got {worst}");
        let best = tail_penalty_norm(2.0, &pop, false, false).unwrap();
        assert!(best > 95.0, "expected near-100, got {best}");
    }

    #[test]
    fn as_score_scales_unit_interval() {
        approx(as_score_0_100(0.5).unwrap(), 50.0);
        approx(as_score_0_100(1.0).unwrap(), 100.0);
        approx(as_score_0_100(85.0).unwrap(), 85.0);
        approx(as_score_0_100(150.0).unwrap(), 100.0);
        approx(as_score_0_100(-5.0).unwrap(), 0.0);
    }

    #[test]
    fn tail_penalty_identical_population_returns_50() {
        let v = tail_penalty_norm(5.0, &[5.0, 5.0, 5.0], true, false).unwrap();
        assert!((v - 50.0).abs() < 1e-6, "expected 50, got {v}");
    }

    #[test]
    fn tail_penalty_single_value_returns_50() {
        let v = tail_penalty_norm(7.0, &[7.0], true, false).unwrap();
        assert!((v - 50.0).abs() < 1e-6, "expected 50, got {v}");
    }

    #[test]
    fn tail_penalty_lower_better_identical_population_returns_50() {
        let v = tail_penalty_norm(5.0, &[5.0, 5.0], false, false).unwrap();
        assert!((v - 50.0).abs() < 1e-6, "expected 50, got {v}");
    }
}
