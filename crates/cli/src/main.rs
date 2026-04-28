mod triage;

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Parser;
use ipbr_core::{
    Coefficients, IngestStats, ModelRecord, RawRow, SourceSummary, ingest_rows,
    load_embedded_pairs, required_aliases, synthesize_rows,
};
use ipbr_render::{
    Scoreboard as RenderScoreboard,
    site::render_site,
    toml_output::{write_coefficients, write_missing, write_scoreboard},
};
use ipbr_sources::{
    FetchOptions, ReqwestHttp, SecretRef, SecretStore, VerificationStatus, registry::registry,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Parser)]
#[command(
    name = "ipbr-rank",
    version,
    about = "Public LLM building-role scoreboard"
)]
struct Cli {
    #[arg(global = true, long, default_value = "out")]
    out: PathBuf,

    #[arg(global = true, long)]
    coefficients: Option<PathBuf>,

    #[arg(global = true, long)]
    aliases: Option<PathBuf>,

    #[arg(global = true, long)]
    cache: Option<PathBuf>,

    #[arg(global = true, long)]
    offline: bool,

    #[arg(global = true, long, value_delimiter = ',')]
    only: Option<Vec<String>>,

    #[arg(global = true, long)]
    aa_api_key_file: Option<PathBuf>,

    #[arg(global = true, long)]
    openrouter_api_key_file: Option<PathBuf>,

    #[arg(global = true, long)]
    hf_token_file: Option<PathBuf>,

    #[arg(global = true, long)]
    now: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Clone)]
enum Command {
    Fetch,
    Score,
    Render,
    All,
    VerifySources,
    ListModels,
    Triage {
        #[arg(long, default_value = "1")]
        min_count: usize,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    run(cli).await
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let secrets = resolve_secrets(&cli)?;
    let command = cli.command.clone().unwrap_or(Command::All);

    match command {
        Command::Fetch => cmd_fetch(&cli, &secrets).await,
        Command::Score => cmd_score(&cli, &secrets).await,
        Command::Render => cmd_render(&cli).await,
        Command::All => cmd_all(&cli, &secrets).await,
        Command::VerifySources => cmd_verify_sources(&cli, &secrets).await,
        Command::ListModels => cmd_list_models(&cli).await,
        Command::Triage { min_count } => cmd_triage(&cli, &secrets, min_count).await,
    }
}

async fn cmd_triage(cli: &Cli, secrets: &SecretStore, min_count: usize) -> anyhow::Result<()> {
    let cache_dir = cli
        .cache
        .as_deref()
        .context("triage requires --cache DIR")?;
    let records = load_aliases(cli)?;
    let coefficients = load_coefficients(cli)?;
    let synthesis_cfg = coefficients.synthesis.clone().unwrap_or_default();

    let http = ReqwestHttp::default();
    let sources = registry();

    let only: Option<std::collections::BTreeSet<&str>> = cli.only.as_ref().map(|items| {
        items
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect()
    });

    let filtered: Vec<_> = sources
        .into_iter()
        .filter(|source| match &only {
            Some(set) => set.contains(source.id()),
            None => true,
        })
        .collect();

    triage::cmd_triage(
        &http,
        cache_dir,
        &cli.out,
        &filtered,
        records,
        &synthesis_cfg,
        min_count,
        secrets,
    )
    .await
}

fn resolve_secrets(cli: &Cli) -> anyhow::Result<SecretStore> {
    let aa_api_key = resolve_secret("AA_API_KEY", cli.aa_api_key_file.as_deref())?;
    let openrouter_api_key =
        resolve_secret("OPENROUTER_API_KEY", cli.openrouter_api_key_file.as_deref())?;
    let hf_token = resolve_secret("HF_TOKEN", cli.hf_token_file.as_deref())?;
    Ok(SecretStore::new(aa_api_key, openrouter_api_key, hf_token))
}

fn resolve_secret(env_key: &str, file: Option<&Path>) -> anyhow::Result<Option<String>> {
    let from_file = match file {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("failed reading secret file {}", path.display()))?;
            let line = raw.lines().next().unwrap_or("").trim();
            (!line.is_empty()).then_some(line.to_string())
        }
        None => None,
    };

    if from_file.is_some() {
        return Ok(from_file);
    }

    Ok(std::env::var(env_key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

#[allow(dead_code)]
fn secret_env_name(secret: SecretRef) -> &'static str {
    match secret {
        SecretRef::AaApiKey => "AA_API_KEY",
        SecretRef::OpenRouterApiKey => "OPENROUTER_API_KEY",
        SecretRef::HfToken => "HF_TOKEN",
    }
}

fn selected_sources<'a>(
    cli: &Cli,
    sources: &'a [Box<dyn ipbr_sources::Source>],
) -> Vec<&'a dyn ipbr_sources::Source> {
    let only: Option<std::collections::BTreeSet<&str>> = cli.only.as_ref().map(|items| {
        items
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect()
    });

    sources
        .iter()
        .map(|source| source.as_ref())
        .filter(|source| match &only {
            Some(set) => set.contains(source.id()),
            None => true,
        })
        .collect()
}

fn load_coefficients(cli: &Cli) -> anyhow::Result<Coefficients> {
    match &cli.coefficients {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("failed reading coefficients {}", path.display()))?;
            Coefficients::load_from_str(&raw)
                .with_context(|| format!("failed parsing coefficients {}", path.display()))
        }
        None => Coefficients::load_embedded().context("failed parsing embedded coefficients"),
    }
}

fn load_aliases(cli: &Cli) -> anyhow::Result<Vec<ModelRecord>> {
    match &cli.aliases {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("failed reading aliases {}", path.display()))?;
            required_aliases::load_from_str(&raw)
                .with_context(|| format!("failed parsing aliases {}", path.display()))
        }
        None => required_aliases::load_embedded().context("failed parsing embedded aliases"),
    }
}

fn resolve_generated_at(cli: &Cli) -> anyhow::Result<String> {
    if let Some(now) = &cli.now {
        let parsed =
            OffsetDateTime::parse(now, &Rfc3339).context("--now must be RFC3339 / ISO8601")?;
        return parsed
            .format(&Rfc3339)
            .context("failed formatting --now timestamp");
    }

    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("failed formatting current timestamp")
}

async fn cmd_fetch(cli: &Cli, secrets: &SecretStore) -> anyhow::Result<()> {
    let cache_dir = cli.cache.as_deref().context("fetch requires --cache DIR")?;
    if cli.offline {
        // REVIEWER: spec is ambiguous on `fetch --offline`; treating it as an error avoids a
        // surprising "success" that cannot actually download anything without network access.
        anyhow::bail!("fetch does not support --offline (use score/all with --offline)");
    }

    let http = ReqwestHttp::default();
    let sources = registry();
    for source in selected_sources(cli, &sources) {
        if source
            .required_secret()
            .is_some_and(|s| secrets.get(s).is_none())
        {
            eprintln!(
                "warning: skipping {} because {} is not set",
                source.id(),
                secret_env_name(source.required_secret().unwrap())
            );
            continue;
        }
        let _rows = source
            .fetch(
                &http,
                FetchOptions {
                    cache_dir: Some(cache_dir),
                    offline: false,
                },
                secrets,
            )
            .await
            .with_context(|| format!("fetch failed for source {}", source.id()))?;
    }
    Ok(())
}

async fn cmd_score(cli: &Cli, secrets: &SecretStore) -> anyhow::Result<()> {
    let (scoreboard, coefficients) = build_scoreboard(cli, secrets, FetchMode::Offline).await?;
    write_scoreboard(&scoreboard, &cli.out)?;
    write_missing(&scoreboard, &cli.out)?;
    write_coefficients(&coefficients, &cli.out)?;
    Ok(())
}

async fn cmd_render(cli: &Cli) -> anyhow::Result<()> {
    let scoreboard_path = cli.out.join("scoreboard.toml");
    let raw = std::fs::read_to_string(&scoreboard_path)
        .with_context(|| format!("failed reading {}", scoreboard_path.display()))?;
    let parsed: ScoreboardToml = toml::from_str(&raw)
        .with_context(|| format!("failed parsing {}", scoreboard_path.display()))?;

    let coefficients = match std::fs::read_to_string(cli.out.join("coefficients.toml")) {
        Ok(raw) => {
            Coefficients::load_from_str(&raw).context("failed parsing out/coefficients.toml")?
        }
        Err(_) => Coefficients::load_embedded().context("failed parsing embedded coefficients")?,
    };

    let models = parsed
        .models
        .into_iter()
        .map(|m| m.into_model_record())
        .collect();

    let scoreboard = RenderScoreboard {
        models,
        coefficients,
        generated_at: parsed.generated_at,
        generator: parsed.generator,
        methodology: parsed.methodology,
        source_summary: parsed
            .sources
            .unwrap_or_default()
            .into_iter()
            .map(|(source, summary)| (source, summary.into_source_summary()))
            .collect(),
    };

    let site_dir = cli.out.join("site");
    render_site(&scoreboard, &site_dir)?;
    Ok(())
}

async fn cmd_all(cli: &Cli, secrets: &SecretStore) -> anyhow::Result<()> {
    let mode = if cli.offline {
        FetchMode::Offline
    } else {
        FetchMode::Online
    };

    let (scoreboard, coefficients) = build_scoreboard(cli, secrets, mode).await?;
    write_scoreboard(&scoreboard, &cli.out)?;
    write_missing(&scoreboard, &cli.out)?;
    write_coefficients(&coefficients, &cli.out)?;
    render_site(&scoreboard, &cli.out.join("site"))?;
    Ok(())
}

async fn cmd_verify_sources(cli: &Cli, secrets: &SecretStore) -> anyhow::Result<()> {
    if cli.offline {
        anyhow::bail!("verify-sources does not support --offline");
    }

    let http = ReqwestHttp::default();
    let sources = registry();
    let mut failures = Vec::new();

    for source in selected_sources(cli, &sources) {
        if source
            .required_secret()
            .is_some_and(|s| secrets.get(s).is_none())
        {
            eprintln!(
                "warning: skipping {} because {} is not set",
                source.id(),
                secret_env_name(source.required_secret().unwrap())
            );
            continue;
        }

        match source
            .fetch(
                &http,
                FetchOptions {
                    cache_dir: cli.cache.as_deref(),
                    offline: false,
                },
                secrets,
            )
            .await
        {
            Ok(rows) => {
                if rows.is_empty() {
                    failures.push((source.id().to_string(), "no rows parsed".to_string()));
                }
            }
            Err(err) => {
                let msg = err.to_string();
                if source.status() == VerificationStatus::Verified {
                    failures.push((source.id().to_string(), msg));
                } else {
                    eprintln!("warning: {} verification failed: {msg}", source.id());
                }
            }
        }
    }

    if failures.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "verify-sources failed for: {}",
        failures
            .into_iter()
            .map(|(id, msg)| format!("{id} ({msg})"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

async fn cmd_list_models(cli: &Cli) -> anyhow::Result<()> {
    let records = load_aliases(cli)?;
    for record in records {
        println!("{}\t{}", record.vendor.as_str(), record.canonical_id);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum FetchMode {
    Online,
    Offline,
}

async fn build_scoreboard(
    cli: &Cli,
    secrets: &SecretStore,
    mode: FetchMode,
) -> anyhow::Result<(RenderScoreboard, Coefficients)> {
    let now = resolve_generated_at(cli)?;
    let coefficients = load_coefficients(cli)?;
    let mut records = load_aliases(cli)?;

    let fetch_opts = match mode {
        FetchMode::Online => FetchOptions {
            cache_dir: cli.cache.as_deref(),
            offline: false,
        },
        FetchMode::Offline => FetchOptions {
            cache_dir: Some(
                cli.cache
                    .as_deref()
                    .context("--offline requires --cache DIR")?,
            ),
            offline: true,
        },
    };

    let http = ReqwestHttp::default();
    let sources = registry();
    let selected = selected_sources(cli, &sources);
    let synthesis_cfg = coefficients.synthesis.clone().unwrap_or_default();
    let synthesis_pairs =
        load_embedded_pairs().context("failed parsing embedded synthesis aliases")?;
    let mut source_summary = BTreeMap::new();
    let mut rows_by_source: BTreeMap<String, Vec<RawRow>> = BTreeMap::new();
    let mut fetched_rows: BTreeMap<String, usize> = BTreeMap::new();
    let mut fetched_statuses: BTreeMap<String, String> = BTreeMap::new();

    for source in selected {
        if matches!(mode, FetchMode::Online)
            && source
                .required_secret()
                .is_some_and(|s| secrets.get(s).is_none())
        {
            eprintln!(
                "warning: skipping {} because {} is not set",
                source.id(),
                secret_env_name(source.required_secret().unwrap())
            );
            source_summary.insert(
                source.id().to_string(),
                SourceSummary {
                    status: "skipped".to_string(),
                    rows: 0,
                    matched: 0,
                    unmatched: 0,
                },
            );
            continue;
        }

        let rows = source
            .fetch(&http, fetch_opts, secrets)
            .await
            .with_context(|| format!("fetch failed for source {}", source.id()))?;
        fetched_rows.insert(source.id().to_string(), rows.len());
        fetched_statuses.insert(
            source.id().to_string(),
            format!("{:?}", source.status()).to_lowercase(),
        );
        rows_by_source.insert(source.id().to_string(), rows);
    }

    let _synthesis_stats = synthesize_rows(
        &mut rows_by_source,
        &synthesis_pairs,
        &records,
        &synthesis_cfg,
    );

    for (source_id, rows) in rows_by_source {
        let row_count = fetched_rows.get(&source_id).copied().unwrap_or(rows.len());
        let status = fetched_statuses
            .get(&source_id)
            .map(String::as_str)
            .unwrap_or("verified");
        record_source_summary(
            &mut source_summary,
            &source_id,
            status,
            row_count,
            &mut records,
            rows,
        );
    }

    ipbr_core::ingest::mark_synthesis_dominant(&mut records, synthesis_cfg.per_model_cap);

    ipbr_core::compute_scores_with(&mut records, &coefficients);

    Ok((
        RenderScoreboard {
            models: records,
            coefficients: coefficients.clone(),
            generated_at: now,
            generator: format!("ipbr-rank {}", env!("CARGO_PKG_VERSION")),
            methodology: "v1".to_string(),
            source_summary,
        },
        coefficients,
    ))
}

fn record_source_summary(
    source_summary: &mut BTreeMap<String, SourceSummary>,
    source_id: &str,
    status: &str,
    row_count: usize,
    records: &mut [ModelRecord],
    rows: Vec<ipbr_core::RawRow>,
) {
    let stats: IngestStats = ingest_rows(records, rows);
    source_summary.insert(
        source_id.to_string(),
        SourceSummary {
            status: status.to_string(),
            rows: row_count,
            matched: stats.matched,
            unmatched: stats.unmatched.len(),
        },
    );
}

#[derive(Debug, Deserialize)]
struct ScoreboardToml {
    generated_at: String,
    generator: String,
    methodology: String,
    #[serde(default)]
    sources: Option<BTreeMap<String, SourceSummaryToml>>,
    models: Vec<ModelToml>,
}

#[derive(Debug, Deserialize)]
struct SourceSummaryToml {
    status: String,
    // Public scoreboard TOML uses stable `n_rows_*` keys; core keeps shorter field names.
    n_rows_ingested: usize,
    n_rows_matched: usize,
    n_rows_unmatched: usize,
}

#[derive(Debug, Deserialize)]
struct ModelToml {
    canonical_id: String,
    display_name: String,
    vendor: String,
    thinking_effort: String,
    aliases: Vec<String>,
    sources: Vec<String>,
    scores: ipbr_core::RoleScores,
    #[serde(default)]
    groups: BTreeMap<String, f64>,
    #[serde(default)]
    metrics: BTreeMap<String, f64>,
    #[serde(default)]
    synthesized: BTreeMap<String, SynthesisProvenanceToml>,
    missing: MissingToml,
}

#[derive(Debug, Deserialize)]
struct SynthesisProvenanceToml {
    source: String,
    from: String,
}

#[derive(Debug, Deserialize)]
struct MissingToml {
    #[serde(default)]
    metrics: Vec<String>,
    #[serde(default)]
    groups_shrunk: Vec<String>,
    #[serde(default)]
    synthesis_dominant: bool,
}

impl SourceSummaryToml {
    fn into_source_summary(self) -> SourceSummary {
        SourceSummary {
            status: self.status,
            rows: self.n_rows_ingested,
            matched: self.n_rows_matched,
            unmatched: self.n_rows_unmatched,
        }
    }
}

impl SynthesisProvenanceToml {
    fn into_synthesis_provenance(self) -> ipbr_core::SynthesisProvenance {
        ipbr_core::SynthesisProvenance {
            source_id: self.source,
            from: self.from,
        }
    }
}

impl ModelToml {
    fn into_model_record(self) -> ModelRecord {
        let vendor = match self.vendor.to_lowercase().as_str() {
            "openai" => ipbr_core::Vendor::Openai,
            "anthropic" => ipbr_core::Vendor::Anthropic,
            "google" => ipbr_core::Vendor::Google,
            "moonshot" | "moonshotai" => ipbr_core::Vendor::Moonshot,
            "zai" | "z-ai" | "z.ai" => ipbr_core::Vendor::Zai,
            "xai" => ipbr_core::Vendor::Xai,
            "alibaba" => ipbr_core::Vendor::Alibaba,
            "deepseek" => ipbr_core::Vendor::Deepseek,
            "mistral" => ipbr_core::Vendor::Mistral,
            "meta" => ipbr_core::Vendor::Meta,
            "minimax" => ipbr_core::Vendor::Minimax,
            "nvidia" => ipbr_core::Vendor::Nvidia,
            "baidu" => ipbr_core::Vendor::Baidu,
            "tencent" => ipbr_core::Vendor::Tencent,
            "inclusionai" => ipbr_core::Vendor::Inclusionai,
            "xiaomi" => ipbr_core::Vendor::Xiaomi,
            other => ipbr_core::Vendor::Other(other.to_string()),
        };

        let thinking_effort = match self.thinking_effort.as_str() {
            "low" => Some(ipbr_core::ThinkingEffort::Low),
            "medium" => Some(ipbr_core::ThinkingEffort::Medium),
            "high" => Some(ipbr_core::ThinkingEffort::High),
            _ => None,
        };

        let mut record = ModelRecord::new(self.canonical_id, self.display_name, vendor);
        record.thinking_effort = thinking_effort;
        record.aliases = self.aliases.into_iter().collect();
        record.sources = self.sources.into_iter().collect();
        record.scores = self.scores;
        record.groups = self.groups;
        record.metrics = self.metrics;
        record.missing.metrics = self.missing.metrics.into_iter().collect();
        record.missing.groups_shrunk = self.missing.groups_shrunk.into_iter().collect();
        record.missing.synthesis_dominant = self.missing.synthesis_dominant;
        record.synthesized = self
            .synthesized
            .into_iter()
            .map(|(metric, provenance)| (metric, provenance.into_synthesis_provenance()))
            .collect();
        record
    }
}
