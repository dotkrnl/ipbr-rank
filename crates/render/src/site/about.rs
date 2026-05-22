use std::fmt::Write;

use crate::Scoreboard;

use super::{html_escape, layout};

pub fn render_about(scoreboard: &Scoreboard) -> String {
    let mut body = String::from(r#"<div class="doc">"#);

    body.push_str(r#"<p class="about-tagline"><strong>Models drift. Agents battle. Math decides.</strong></p>

<h2>What this is</h2>
<p>ipbr is a public-LLM coding-role scoreboard. It pulls model performance from public benchmarks, normalizes them onto a common 0-100 scale, and produces four role scores: Idea, Plan, Build, Review.</p>
<p>All inputs come from public, verifiable sources. Weights and aggregation rules are explicit and versioned. A small number of vendor-published metrics that haven't yet appeared on public leaderboards are recorded as overrides. There is no manual reranking.</p>

<h2>Fully vibe-coded</h2>
<p>No human picked these weights. Claude, Gemini, GPT, and Kimi argued every coefficient, group composition, and penalty curve in this repo through round after round of cross-model code review until the numbers settled. The human just refereed and pressed merge — the four debating models are the credited copyright holders.</p>
<p>Yes, that means the models being scored helped score themselves. The cross-review process is the safeguard: each weight had to survive scrutiny from peers ranked alongside it.</p>

<h2>The four roles</h2>
<ul>
<li><strong>Idea</strong> — open-ended creativity, general intelligence, breadth. Driven by LM Arena Text, Artificial Analysis, reasoning blends, and ARC-AGI.</li>
<li><strong>Plan</strong> — structured reasoning, function-calling, multi-step task decomposition. Driven by Terminal-Bench, tau2-bench, IFBench, MCP-Atlas, and BFCL.</li>
<li><strong>Build</strong> — actually writing code that runs. Driven by SWE-bench (Verified + Multilingual + Pro), SWE-rebench, SWE Atlas, GSO, Sonar code quality, and GDPval.</li>
<li><strong>Review</strong> — judging code quality, correctness, and preference. Driven by LM Arena, Sonar code-quality metrics, BUILD, and PLAN.</li>
</ul>

<h2>How scores are built</h2>
<ol>
<li><strong>Normalize</strong> — each metric is percentile-mapped within the active model population (5th/95th boundaries; log-scaled for cost/speed/latency). Operational metrics use a tail-penalty curve — the top 80% maps into 70-100 with mild differentiation; only the bottom 20% drops sharply.</li>
<li><strong>Aggregate</strong> — metrics roll up into groups (CRE, GEN, PLAN, BUILD, LM_ARENA_REVIEW_PROXY, OPS_*). Scores blend from shrink-to-50 to trusting the present metrics across 60-80% group coverage.</li>
<li><strong>Combine</strong> — each role score is a weighted average of groups. AISL was removed after local reproduction found it not representative enough and too noise-prone; its former weight now goes to the remaining public benchmark groups. Operational metrics carry 0.08 — fast-enough models cluster within a 1-2 point spread, but genuinely slow models lose 4-6 points.</li>
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
<li><strong>Trust transition</strong> — the 60-80% group-coverage band where sparse groups move from shrink-to-50 toward the present-weight mean.</li>
<li><strong>Composite</strong> — a metric that is itself a weighted blend of related sub-metrics (currently SWEComposite).</li>
<li><strong>Retired source</strong> — AISL source code and fixture data remain for audit history, but AISL is no longer registered or scored.</li>
</ul>

<p><a href="index.html">← back to scoreboard</a></p>
</div>"#);

    layout("ipbr · about", scoreboard, &body)
}
