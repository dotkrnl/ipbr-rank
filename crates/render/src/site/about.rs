use std::fmt::Write;

use crate::Scoreboard;

use super::{html_escape, layout};

pub fn render_about(scoreboard: &Scoreboard) -> String {
    let mut body = String::from(r#"<div class="doc">"#);

    body.push_str(r#"<p class="about-tagline"><strong>A capability snapshot with explicit evidence quality.</strong></p>

<h2>What this is</h2>
<p>ipbr combines public benchmark observations into four 0-100 role proxies: Idea, Plan, Build, and Review. <strong>Balanced capability</strong> is the unweighted mean of those four scores; it is a summary view, not a fifth measured construct.</p>
<p>Inputs, fixed normalization anchors, evidence discounts, family caps, and weights are versioned in the repository. Direct leaderboard observations take precedence over cited reports, which take precedence over synthesized sibling fills. No score is manually reranked.</p>

<h2>The four roles</h2>
<ul>
<li><strong>Idea</strong> — open-ended generation and novel problem solving. EQ-Bench Creative Writing v3 supplies a direct creativity signal; LM Arena Text and ARC-AGI add preference and abstraction evidence.</li>
<li><strong>Plan</strong> — structured reasoning, function calling, tool orchestration, and multi-step execution. Terminal-Bench, BFCL, MCP-Atlas, tau3, long-context, and enterprise-workflow tasks contribute.</li>
<li><strong>Build</strong> — implementing and repairing software. SWE-bench variants, SWE-rebench, SWE Atlas, GSO, terminal tasks, live coding, and MCP orchestration dominate. Sonar code-quality data remains diagnostic.</li>
<li><strong>Review proxy</strong> — judge calibration and review-adjacent capability. EQ-Bench Judgemark v4 is direct judge evidence; search/document preference and Plan/Build signals broaden it. It is not claimed to be a direct code-review benchmark.</li>
</ul>

<h2>How scores are built</h2>
<ol>
<li><strong>Select one model record</strong> — observations are combined under the canonical model. Where effort is published, best-available max/high effort is preferred; sources without effort metadata keep their reported configuration.</li>
<li><strong>Normalize on fixed anchors</strong> — ranked leaves use raw-unit p5/p95 anchors frozen from the 2026-07-12 refreshed cohort. Anchors map near 5 and 95 through an asymptotic logistic curve, so future model additions do not rescale earlier observations and extreme values do not hard-clip.</li>
<li><strong>Apply evidence reliability</strong> — direct observations count at 1.00 reliability and cited same-model reports at 0.60. Sibling fills remain visible for provenance, but are prior-only (0.00) in the primary score.</li>
<li><strong>Separate capability from confidence</strong> — the point estimate averages available same-model evidence. Missing and sibling-only leaves do not imply average capability; their nominal weight remains visible in confidence and provisional status.</li>
<li><strong>Control correlation</strong> — related metrics are combined once, then role scoring caps any benchmark/source family at 30%. A role qualifies through either 60% direct nominal weight across three families or 35% across five; provisional estimates remain visible in the same rank order.</li>
</ol>

<h2>What never affects rank</h2>
<p>Price, output throughput, time to first token, and advertised context window remain available as reference diagnostics. Their path weight into Idea, Plan, Build, Review, and balanced capability is exactly zero.</p>

<h2>Configuration policy</h2>
<p>The ranking intentionally compares models, not every effort or agent-harness permutation. A score can therefore combine the best available public observation from benchmarks that expose different configuration detail. The API labels this policy <code>best_available_max_effort</code>; it should not be read as a controlled same-harness experiment.</p>

<h2>Sources</h2>
<div class="doc-scroll"><table><thead><tr><th>source</th><th>status</th><th>rows</th><th>matched</th><th>unmatched</th></tr></thead><tbody>"#);

    for (source, summary) in &scoreboard.source_summary {
        write!(
            body,
            r#"<tr><td>{name}</td><td>{status}</td><td>{rows}</td><td>{matched}</td><td>{unmatched}</td></tr>"#,
            name = html_escape(source),
            status = html_escape(&summary.status),
            rows = summary.rows,
            matched = summary.matched,
            unmatched = summary.unmatched,
        )
        .unwrap();
    }

    body.push_str(r#"</tbody></table></div>

<h2>Glossary</h2>
<ul>
<li><strong>Direct coverage</strong> — the share of a role's nominal path supported by direct benchmark observations.</li>
<li><strong>Confidence</strong> — evidence coverage after cited reports are discounted; sibling fills contribute no confidence.</li>
<li><strong>Provisional</strong> — a numeric role score that meets neither the standard direct-coverage gate nor the broader five-family corroboration gate.</li>
<li><strong>Composite</strong> — a weighted blend used to collapse overlapping components before role aggregation, such as the SWE, Sonar, or AA reasoning families.</li>
<li><strong>Fixed anchor</strong> — a versioned raw benchmark value used to keep normalization stable across changing model cohorts.</li>
</ul>

<p><a href="index.html">← back to scoreboard</a></p>
</div>"#);

    layout("ipbr · about", scoreboard, &body)
}
