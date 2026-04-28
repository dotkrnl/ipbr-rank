use std::fmt::Write;

use crate::Scoreboard;

use super::{html_escape, layout};

pub fn render_about(scoreboard: &Scoreboard) -> String {
    let mut body = String::from(r#"<div class="doc">"#);

    body.push_str(r#"<h2>What this is</h2>
<p>ipbr-rank is a public-LLM coding-role scoreboard. It pulls model performance from public benchmarks, normalizes them onto a common 0-100 scale, and produces four role scores: Idea, Plan, Build, Review.</p>
<p>All inputs come from public, verifiable sources. Weights and aggregation rules are explicit and versioned. A small number of vendor-published metrics that haven't yet appeared on public leaderboards are recorded as overrides. There is no manual reranking.</p>

<h2>The four roles</h2>
<ul>
<li><strong>Idea</strong> — open-ended creativity, general intelligence, breadth. Driven by LM Arena Text, AI Stupid Level idea-shaped axes, and reasoning blends.</li>
<li><strong>Plan</strong> — structured reasoning, function-calling, multi-step task decomposition. Driven by Terminal-Bench, tau2-bench, IFBench, MCP-Atlas, and AISL plan axes.</li>
<li><strong>Build</strong> — actually writing code that runs. Driven by SWE-bench (Verified + Multilingual + Pro), SWE-rebench, LiveCodeBench, Sonar code quality, and AISL build axes.</li>
<li><strong>Review</strong> — judging code quality, correctness, and preference. Driven by LM Arena, Sonar issue density, and AISL review axes. <em>Review has no adjusted variant — it is the source of the penalty applied to the other three.</em></li>
</ul>

<h2>Raw vs adjusted</h2>
<p>The raw score is the benchmark composite, normalized. The adjusted score subtracts a <strong>reviewer-reservation</strong> penalty: when one vendor's models dominate Review, that lead gets discounted from their other scores.</p>
<pre><code>L_v   = max(0, max(R_all) - max(R_outside_v))
I_adj = I_raw - 0.08 × L_v
P_adj = P_raw - 0.18 × L_v
B_adj = B_raw - 0.32 × L_v</code></pre>
<p>Coefficients reflect how easy each role is to game with biased preference evaluations. Build is hardest hit; Idea is barely touched; Plan sits in between.</p>

<h2>How scores are built</h2>
<ol>
<li><strong>Normalize</strong> — each metric is percentile-mapped within the active model population (5th/95th boundaries; log-scaled for cost/speed/latency). Operational metrics use a tail-penalty curve — the top 80% maps into 70-100 with mild differentiation; only the bottom 20% drops sharply.</li>
<li><strong>Aggregate</strong> — metrics roll up into groups (CRE, GEN, PLAN, CODE, JUDGE, OPS_*, A_I/A_P/A_B/A_R). If at least 70% of a group's weight is present, the score is the present-weight mean. Below that threshold, it shrinks toward 50 proportional to missing weight.</li>
<li><strong>Combine</strong> — each role score is a weighted average of groups. AISL's role-shaped perspective (A_*) is the dominant signal at 0.50 in every formula. Operational metrics carry 0.08 — fast-enough models cluster within a 1-2 point spread, but genuinely slow models lose 4-6 points.</li>
<li><strong>Synthesize last</strong> — when a known sibling pair has a metric on one model but not the other, the missing field is filled from the sibling and softened toward 50 by 15% so it reads as a softer signal.</li>
</ol>

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
<li><strong>Synthesized</strong> — a metric value filled from a known sibling model when the source did not directly cover this model.</li>
<li><strong>Trust threshold</strong> — the 70% group-coverage cutoff above which the present-weight mean is trusted directly.</li>
<li><strong>Composite</strong> — a metric that is itself a weighted blend of related sub-metrics (currently SWEComposite).</li>
<li><strong>A_* perspective</strong> — AISL's 17 stupid-axes weighted four ways (one weighting per role).</li>
</ul>

<p><a href="index.html">← back to scoreboard</a></p>
</div>"#);

    layout("ipbr-rank · about", scoreboard, &body)
}
