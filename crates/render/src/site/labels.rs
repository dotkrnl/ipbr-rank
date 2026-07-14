//! Human-facing names for everything the expanded view shows.
//!
//! The scoring engine speaks in keys (`AAOmniscienceNonHallucination`,
//! `CODE_REVIEW_DIRECT`, `sweatlas_qna`). A reader does not. Every key that
//! reaches the page passes through this catalogue first; a key with no entry
//! here is engine plumbing (confidence bounds, fallback duplicates) and is
//! never rendered.

/// Subject areas the benchmark results are grouped under. These are reader
/// categories, not scoring groups — the scoring groups (`CRE`, `GEN`, …) are
/// an internal layer and never surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Area {
    Software,
    Agentic,
    Reasoning,
    Writing,
    Context,
    Review,
    /// Speed, price, and context window. Never affects a rank.
    Reference,
}

impl Area {
    pub fn title(self) -> &'static str {
        match self {
            Area::Software => "Writing and fixing software",
            Area::Agentic => "Agents, tools, and real-world tasks",
            Area::Reasoning => "Reasoning and knowledge",
            Area::Writing => "Writing and human preference",
            Area::Context => "Long context",
            Area::Review => "Code review",
            Area::Reference => "Speed, price, and context window",
        }
    }

    /// Reading order for the results section.
    pub fn all() -> [Area; 6] {
        [
            Area::Software,
            Area::Agentic,
            Area::Reasoning,
            Area::Writing,
            Area::Context,
            Area::Review,
        ]
    }
}

/// How a benchmark's measured value is written out. Benchmarks report in wildly
/// different units and a bare `1505.273` tells a reader nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Pass rate or accuracy, 0-100.
    Percent,
    /// Head-to-head rating.
    Elo,
    /// A publisher's own 0-100 index.
    Index,
    /// Defects per thousand lines — lower is better.
    Density,
    Tokens,
    DollarsPerMillionTokens,
    TokensPerSecond,
    Seconds,
}

impl Unit {
    /// Writes a raw benchmark value the way its publisher reports it.
    pub fn format(self, value: f64) -> String {
        match self {
            Unit::Percent => format!("{value:.1}%"),
            Unit::Elo => format!("{value:.0} Elo"),
            Unit::Index => format!("{value:.1}"),
            Unit::Density => format!("{value:.2} / kLOC"),
            Unit::Tokens => format_tokens(value),
            Unit::DollarsPerMillionTokens => format!("${value:.2} / M"),
            Unit::TokensPerSecond => format!("{value:.0} tok/s"),
            Unit::Seconds => format!("{value:.1} s"),
        }
    }
}

fn format_tokens(value: f64) -> String {
    if value >= 1_000_000.0 {
        let millions = value / 1_000_000.0;
        if (millions - millions.round()).abs() < 0.05 {
            format!("{millions:.0}M tokens")
        } else {
            format!("{millions:.1}M tokens")
        }
    } else if value >= 1_000.0 {
        format!("{:.0}k tokens", value / 1_000.0)
    } else {
        format!("{value:.0} tokens")
    }
}

pub struct MetricLabel {
    pub name: &'static str,
    pub area: Area,
    pub unit: Unit,
}

const fn metric(name: &'static str, area: Area, unit: Unit) -> MetricLabel {
    MetricLabel { name, area, unit }
}

/// The benchmark a reader sees, for every metric key that is fit to show.
///
/// Deliberately absent: `*HybridFallback`, `*CILow` / `*CIHigh`,
/// `FactoryCodeReviewF1Stdev`, and `GPQA_HLE_Reasoning`. Those are internal
/// bookkeeping — error bars, precedence duplicates, a retired blend — and a
/// reader who saw them would reasonably think they were separate results.
pub fn metric_label(key: &str) -> Option<MetricLabel> {
    Some(match key {
        // Writing and fixing software
        "SWEBenchPro" => metric("SWE-bench Pro", Area::Software, Unit::Percent),
        "SWERebench" => metric("SWE-rebench (fresh issues)", Area::Software, Unit::Percent),
        "SWEBenchMultilingual" => metric("SWE-bench Multilingual", Area::Software, Unit::Percent),
        "SWEBenchVerified" => metric("SWE-bench Verified", Area::Software, Unit::Percent),
        "SWEAtlasQnA" => metric(
            "SWE Atlas — codebase questions",
            Area::Software,
            Unit::Percent,
        ),
        "SWEAtlasTestWriting" => metric("SWE Atlas — writing tests", Area::Software, Unit::Percent),
        "SWEAtlasRefactoring" => metric("SWE Atlas — refactoring", Area::Software, Unit::Percent),
        "DeepSWE" => metric("DeepSWE — solving issues", Area::Software, Unit::Percent),
        "DeepSWEPassAt4" => metric(
            "DeepSWE — best of four attempts",
            Area::Software,
            Unit::Percent,
        ),
        "SciCode" => metric("SciCode — scientific code", Area::Software, Unit::Percent),
        "LiveCodeBench" => metric(
            "LiveCodeBench — contest problems",
            Area::Software,
            Unit::Percent,
        ),
        "AALiveCodeBench" => metric(
            "LiveCodeBench (independent run)",
            Area::Software,
            Unit::Percent,
        ),
        "CopilotArenaOrLMArenaCode" => {
            metric("Arena — coding preference", Area::Software, Unit::Elo)
        }
        "GSO" => metric("GSO — optimizing code", Area::Software, Unit::Percent),
        "AGCBench" => metric("AGC — agentic coding", Area::Software, Unit::Percent),
        "ArtificialAnalysisCoding" => metric(
            "Artificial Analysis coding index",
            Area::Software,
            Unit::Index,
        ),
        "SonarFunctionalSkill" => metric("Sonar — code that works", Area::Software, Unit::Percent),
        "SonarIssueDensity" => metric("Sonar — issues found", Area::Software, Unit::Density),
        "SonarBugDensity" => metric("Sonar — bugs found", Area::Software, Unit::Density),
        "SonarVulnerabilityDensity" => metric(
            "Sonar — vulnerabilities found",
            Area::Software,
            Unit::Density,
        ),
        "KimiCodeBenchV2" => metric("KimiCodeBench v2", Area::Software, Unit::Percent),
        "ProgramBench" => metric("ProgramBench", Area::Software, Unit::Percent),
        "MLSBenchLite" => metric("MLS-Bench Lite — ML work", Area::Software, Unit::Percent),

        // Agents, tools, and real-world tasks
        "MCPAtlas" => metric("MCP Atlas — chaining tools", Area::Agentic, Unit::Percent),
        "MCPMarkVerified" => metric("MCPMark — using tools", Area::Agentic, Unit::Percent),
        "TerminalBench21" => metric("Terminal-Bench 2.1", Area::Agentic, Unit::Percent),
        "AATerminalBench21" => metric(
            "Terminal-Bench 2.1 (independent run)",
            Area::Agentic,
            Unit::Percent,
        ),
        "TerminalBench" => metric("Terminal-Bench 1", Area::Agentic, Unit::Percent),
        "TerminalBenchHard" => metric("Terminal-Bench Hard", Area::Agentic, Unit::Percent),
        "Tau3Banking" => metric("τ³-bench — banking support", Area::Agentic, Unit::Percent),
        "Tau2Bench" => metric("τ²-bench — tool dialogue", Area::Agentic, Unit::Percent),
        "TauBanking" => metric("τ-bench — banking", Area::Agentic, Unit::Percent),
        "GDPvalAA2" => metric("GDPval — professional work", Area::Agentic, Unit::Elo),
        "GDPval" => metric(
            "GDPval — professional work (vendor-reported)",
            Area::Agentic,
            Unit::Elo,
        ),
        "EnterpriseOpsGymAA" => metric("Enterprise ops workflows", Area::Agentic, Unit::Percent),
        "AutomationBenchAA" => metric("Automating office tasks", Area::Agentic, Unit::Percent),
        "HiLBench" => metric("Knowing when to ask a human", Area::Agentic, Unit::Percent),
        "Toolathlon" => metric("Toolathlon — using tools", Area::Agentic, Unit::Percent),
        "OSWorldVerified" => metric("OSWorld — using a computer", Area::Agentic, Unit::Percent),
        "BrowseComp" => metric(
            "BrowseComp — researching the web",
            Area::Agentic,
            Unit::Percent,
        ),
        "KimiClaw247Bench" => metric(
            "Claw 24/7 — long-running agent",
            Area::Agentic,
            Unit::Percent,
        ),
        "BFCL" => metric("Berkeley function calling", Area::Agentic, Unit::Percent),
        "BFCLLive" => metric(
            "Function calling — live requests",
            Area::Agentic,
            Unit::Percent,
        ),
        "BFCLMultiTurn" => metric(
            "Function calling — over several turns",
            Area::Agentic,
            Unit::Percent,
        ),
        "BFCLWebSearch" => metric(
            "Function calling — web search",
            Area::Agentic,
            Unit::Percent,
        ),
        "BFCLMemory" => metric("Function calling — memory", Area::Agentic, Unit::Percent),
        "BFCLNonLiveAST" => metric(
            "Function calling — well-formed calls",
            Area::Agentic,
            Unit::Percent,
        ),
        "BFCLRelevanceDetection" => metric(
            "Function calling — picking the right tool",
            Area::Agentic,
            Unit::Percent,
        ),
        "BFCLIrrelevanceDetection" => metric(
            "Function calling — declining wrong tools",
            Area::Agentic,
            Unit::Percent,
        ),

        // Reasoning and knowledge
        "ARC_AGI_2" => metric(
            "ARC-AGI-2 — abstract puzzles",
            Area::Reasoning,
            Unit::Percent,
        ),
        "ARC_AGI_3" => metric(
            "ARC-AGI-3 — interactive puzzles",
            Area::Reasoning,
            Unit::Percent,
        ),
        "GPQA" => metric("GPQA — graduate science", Area::Reasoning, Unit::Percent),
        "HLE" => metric("Humanity's Last Exam", Area::Reasoning, Unit::Percent),
        "HLETools" => metric(
            "Humanity's Last Exam (tools allowed)",
            Area::Reasoning,
            Unit::Percent,
        ),
        "CritPt" => metric(
            "CritPt — physics research problems",
            Area::Reasoning,
            Unit::Percent,
        ),
        "AAOmniscienceAccuracy" => metric(
            "Omniscience — getting facts right",
            Area::Reasoning,
            Unit::Percent,
        ),
        "AAOmniscienceNonHallucination" => metric(
            "Omniscience — not making facts up",
            Area::Reasoning,
            Unit::Percent,
        ),
        "AAOmniscienceIndex" => metric("Omniscience index", Area::Reasoning, Unit::Index),
        "AIME25" => metric(
            "AIME 2025 — competition maths",
            Area::Reasoning,
            Unit::Percent,
        ),
        "MMLUPro" => metric("MMLU-Pro — broad knowledge", Area::Reasoning, Unit::Percent),
        "IFBench" => metric(
            "Following instructions exactly",
            Area::Reasoning,
            Unit::Percent,
        ),
        "ArtificialAnalysisIntelligence" => metric(
            "Artificial Analysis intelligence index",
            Area::Reasoning,
            Unit::Index,
        ),
        "ArtificialAnalysisReasoning" => metric(
            "Artificial Analysis reasoning index",
            Area::Reasoning,
            Unit::Index,
        ),
        "ArtificialAnalysisMath" => metric(
            "Artificial Analysis maths index",
            Area::Reasoning,
            Unit::Index,
        ),

        // Writing and human preference
        "EQBenchCreativeWriting" => metric("EQ-Bench — creative writing", Area::Writing, Unit::Elo),
        "LMArenaCreative" => metric("Arena — creative writing", Area::Writing, Unit::Elo),
        "LMArenaText" => metric("Arena — overall preference", Area::Writing, Unit::Elo),
        "LMArenaSearch" => metric("Arena — search answers", Area::Writing, Unit::Elo),
        "LMArenaDocument" => metric("Arena — document work", Area::Writing, Unit::Elo),

        // Long context
        "LongContextRecall" => metric("Recall across a long input", Area::Context, Unit::Percent),
        "ContextArenaMRCR128k" => {
            metric("Buried detail at 128k tokens", Area::Context, Unit::Percent)
        }
        "ContextArenaMRCR1M" => metric("Buried detail at 1M tokens", Area::Context, Unit::Percent),

        // Code review
        "FactoryCodeReviewF1" => {
            metric("Reviewing real pull requests", Area::Review, Unit::Percent)
        }
        "FactoryCodeReviewPrecision" => metric(
            "Review — flags that were real bugs",
            Area::Review,
            Unit::Percent,
        ),
        "FactoryCodeReviewRecall" => {
            metric("Review — real bugs it caught", Area::Review, Unit::Percent)
        }
        "EQBenchJudgemark" => metric("Judging other models' answers", Area::Review, Unit::Index),

        // Reference only
        "OutputSpeed" => metric("Output speed", Area::Reference, Unit::TokensPerSecond),
        "TTFT" => metric("Wait for the first token", Area::Reference, Unit::Seconds),
        "BlendedCost" => metric("Price", Area::Reference, Unit::DollarsPerMillionTokens),
        "ContextWindow" => metric("Context window", Area::Reference, Unit::Tokens),

        _ => return None,
    })
}

/// Name for a blended input — several correlated benchmarks the scorer
/// deliberately counts once. These appear as inputs to a role score; their
/// components are listed individually under the benchmark results.
pub fn composite_label(key: &str) -> Option<&'static str> {
    Some(match key {
        "SWEComposite" => "Fixing real software issues",
        "SWEAtlasComposite" => "Everyday engineering work",
        "LiveCodingComposite" => "Writing code from scratch",
        "TerminalBench21Composite" => "Working in a terminal",
        "LongContextComposite" => "Working over a long context",
        "EnterpriseWorkflowComposite" => "Enterprise workflows",
        "AAGeneralComposite" => "General reasoning and knowledge",
        "AAReasoningComposite" => "Science and exam reasoning",
        "SonarComposite" => "Code quality",
        "TauComposite" => "Tool-use dialogue",
        "BFCLComposite" => "Function calling",
        _ => return None,
    })
}

/// Name for anything that can feed a role score: a blend or a single benchmark.
pub fn input_label(key: &str) -> Option<&'static str> {
    composite_label(key).or_else(|| metric_label(key).map(|label| label.name))
}

/// Where an observation came from, as the publisher is known rather than as the
/// ingest pipeline names its feed.
pub fn source_label(id: &str) -> &str {
    match id {
        "aa_automation_bench" => "Artificial Analysis — Automation Bench",
        "aa_critpt" => "Artificial Analysis — CritPt",
        "aa_enterprise_ops_gym" => "Artificial Analysis — Enterprise Ops Gym",
        "aa_gdpval_v2" => "Artificial Analysis — GDPval",
        "aa_omniscience" => "Artificial Analysis — Omniscience",
        "artificial_analysis" => "Artificial Analysis",
        "agc_bench" => "AGC Bench",
        "arc_agi" => "ARC Prize",
        "bfcl" => "Berkeley Function Calling Leaderboard",
        "context_arena" => "Context Arena",
        "deep_swe_v1_1" => "DeepSWE",
        "eqbench_creative_writing" => "EQ-Bench Creative Writing",
        "eqbench_judgemark" => "EQ-Bench Judgemark",
        "factory_code_review" => "Factory Code Review",
        "gso" => "GSO",
        "hil_bench" => "HiL-Bench",
        "livecodebench" => "LiveCodeBench",
        "lmarena" => "LM Arena",
        "mcp_atlas" => "MCP Atlas",
        "openrouter" => "OpenRouter",
        "overrides" => "Vendor reports (cited)",
        "sonar" => "Sonar",
        "sweatlas_qna" => "SWE Atlas — codebase questions",
        "sweatlas_refactoring" => "SWE Atlas — refactoring",
        "sweatlas_test_writing" => "SWE Atlas — writing tests",
        "swebench" => "SWE-bench",
        "swebench_pro" => "SWE-bench Pro",
        "swerebench" => "SWE-rebench",
        "terminal_bench" => "Terminal-Bench",
        "terminal_bench_2_1" => "Terminal-Bench 2.1",
        other => other,
    }
}

/// `1st`, `2nd`, `13th` — a rank a reader can say out loud.
pub fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals_handle_the_teens() {
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(2), "2nd");
        assert_eq!(ordinal(3), "3rd");
        assert_eq!(ordinal(4), "4th");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(12), "12th");
        assert_eq!(ordinal(13), "13th");
        assert_eq!(ordinal(21), "21st");
        assert_eq!(ordinal(112), "112th");
    }

    #[test]
    fn units_are_written_the_way_their_publisher_reports_them() {
        assert_eq!(Unit::Percent.format(95.0), "95.0%");
        assert_eq!(Unit::Elo.format(1505.273), "1505 Elo");
        assert_eq!(Unit::Tokens.format(1_000_000.0), "1M tokens");
        assert_eq!(Unit::Tokens.format(1_500_000.0), "1.5M tokens");
        assert_eq!(Unit::Tokens.format(256_000.0), "256k tokens");
        assert_eq!(Unit::DollarsPerMillionTokens.format(20.0), "$20.00 / M");
        assert_eq!(Unit::TokensPerSecond.format(64.899), "65 tok/s");
    }

    #[test]
    fn engine_plumbing_never_reaches_the_page() {
        for internal in [
            "GDPvalAA2CILow",
            "GDPvalAA2CIHigh",
            "DeepSWECILow",
            "DeepSWECIHigh",
            "FactoryCodeReviewF1Stdev",
            "GPQA_HLE_Reasoning",
            "CritPtHybridFallback",
            "EnterpriseOpsGymAAHybridFallback",
        ] {
            assert!(
                metric_label(internal).is_none(),
                "{internal} is internal bookkeeping and must not be shown"
            );
        }
    }

    #[test]
    fn every_scored_input_has_a_reader_facing_name() {
        let coefficients = ipbr_core::Coefficients::load_embedded().unwrap();
        for groups in coefficients.final_score_weights.values() {
            for group in groups.keys() {
                let metrics = &coefficients.group_weights[group];
                for key in metrics.keys() {
                    assert!(
                        input_label(key).is_some(),
                        "scored input {key} has no reader-facing name"
                    );
                }
            }
        }
    }
}
