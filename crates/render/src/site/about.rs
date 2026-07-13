use std::fmt::Write;

use crate::Scoreboard;

use super::{html_escape, layout};

pub fn render_about(scoreboard: &Scoreboard) -> String {
    let mut body = String::from(r#"<div class="doc">"#);

    body.push_str(r#"<p class="about-tagline"><strong>Models drift. Evidence accumulates. Ranks update.</strong></p>

<h2>What this is</h2>
<p>ipbr combines public benchmark observations into four 0-100 role proxies: Idea, Plan, Build, and Review.</p>
<p>Inputs, fixed normalization anchors, evidence classes, family caps, and weights are versioned in the repository. Native public observations take precedence over duplicate manually curated observations; both count as direct same-product evidence. Vendor-automatic fallback is part of the served ranked product and retains explicit provenance; separately named multi-agent or premium endpoints remain distinct. No score is manually reranked.</p>

<h2>The four roles</h2>
<ul>
<li><strong>Idea</strong> — open-ended generation and novel problem solving. EQ-Bench Creative Writing v3 supplies a direct creativity signal; LM Arena Text and ARC-AGI add preference and abstraction evidence.</li>
<li><strong>Plan</strong> — structured reasoning, tool orchestration, and multi-step execution. MCP-Atlas, tau3, long-context, enterprise-workflow, Terminal-Bench 2.1, GDPval, and a small human-escalation signal contribute.</li>
<li><strong>Build</strong> — implementing and repairing software. SWE-bench variants, SWE-rebench, live coding, MCP orchestration, SWE Atlas, terminal work, DeepSWE, and long context contribute. GSO, BFCL, and Sonar remain diagnostic.</li>
<li><strong>Review proxy</strong> — review-adjacent capability derived from search/document preference plus Plan and Build. Judgemark remains a diagnostic because judge discrimination is not direct code review, and no currently broad code-review benchmark is treated as direct evidence.</li>
</ul>

<h2>How scores are built</h2>
<ol>
<li><strong>Select one product record</strong> — observations are combined under the canonical ranked product, including vendor-automatic routing. Where effort is published, best-available max/high effort is preferred; sources without effort metadata keep their reported configuration.</li>
<li><strong>Normalize on fixed anchors</strong> — scored leaves use raw-unit p5/p95 anchors frozen from the 2026-07-12 refreshed cohort. Anchors map near 5 and 95 through an asymptotic logistic curve, so future model additions do not rescale earlier observations and extreme values do not hard-clip.</li>
<li><strong>Apply evidence reliability</strong> — every actual same-product observation counts at 1.00 reliability, including cited vendor or system-card observations curated as overrides. Every scored value is a direct same-product measurement.</li>
<li><strong>Separate capability from confidence</strong> — the point estimate averages available same-product evidence. Missing leaves do not imply average capability; their nominal weight remains visible in confidence and provisional status.</li>
<li><strong>Control correlation</strong> — related metrics are combined once, then role scoring caps any benchmark/source family at 30%.</li>
<li><strong>Qualify independently</strong> — broad core benchmarks establish current coverage; narrow supplemental benchmarks may affect scores without making their missing rows a penalty. Total-portfolio qualification still requires at least 35% direct core weight across three core families, so a favorable specialist subset cannot qualify by itself. Direct retired evidence may support an established model's coverage history but never its current score. The radar outline renders as ranked once three roles are ranked and the fourth carries at least 20% current direct coverage.</li>
</ol>

<h2>What never affects rank</h2>
<p>Price, output throughput, time to first token, and advertised context window remain available as reference diagnostics. Their path weight into Idea, Plan, Build, and Review is exactly zero.</p>

<h2>Configuration policy</h2>
<p>The ranking intentionally compares served products, not every effort or agent-harness permutation. Automatic vendor routing is included; separately named multi-agent or premium endpoints remain distinct. A score can therefore combine the best available public observation from benchmarks that expose different configuration detail. The API labels this policy <code>best_available_max_effort</code>; it should not be read as a controlled same-harness experiment.</p>

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
<li><strong>Confidence</strong> — direct same-product evidence coverage; missing leaves contribute no confidence.</li>
<li><strong>Provisional</strong> — a numeric role score that meets none of the full-current, core-current, or established-history evidence gates. It remains in the same rank order, italicized and starred, with a dotted radar outline.</li>
<li><strong>Core / supplemental / historical support</strong> — eligibility classes for broad current evidence, narrow scored evidence, and unscored retired direct evidence respectively.</li>
<li><strong>Composite</strong> — a weighted blend used to collapse overlapping components before role aggregation, such as the SWE, Sonar, or AA reasoning families.</li>
<li><strong>Fixed anchor</strong> — a versioned raw benchmark value used to keep normalization stable across changing model cohorts.</li>
</ul>

<p><a href="index.html">← back to scoreboard</a></p>
</div>"#);

    layout("ipbr · about", &body)
}
