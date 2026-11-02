mod commands;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use sui_sentinel::ingest::graphql::Graphql;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read-only MCP tools over explicitly authorized committed-state scopes.
    Mcp(commands::mcp::McpArgs),
    /// Authenticated local administration through the database operator role.
    Admin(commands::admin::AdminArgs),
    /// Apply schema changes using the dedicated migration role.
    Migrate {
        #[arg(long, env = "DIAGNOSIS_DATABASE_URL", hide_env_values = true)]
        database_url: String,
    },
    /// Development-only gRPC checkpoints with durable state and GraphQL reconciliation.
    Live(commands::live::LiveArgs),
    /// Read the public gRPC provider and configured deployment without starting a monitor.
    ProbeGrpc {
        #[arg(long, default_value = "deploy/config/suilend.toml")]
        config: PathBuf,
    },
    /// Schedule captures, evaluate freshness, replay outcomes and deliver webhooks.
    Monitor(commands::monitor::MonitorArgs),
    /// Ingest contiguous normalized checkpoint files into durable state.
    FollowCheckpoints(commands::follow::FollowArgs),
    #[command(hide = true)]
    ValidateCapture {
        #[arg(long)]
        input: PathBuf,
    },
    /// Serve a saved report and Prometheus metrics without collecting chain data.
    Serve {
        #[arg(long)]
        report: PathBuf,
        #[arg(long, default_value = "127.0.0.1:9108")]
        listen: std::net::SocketAddr,
    },
    /// Serve the generation named by `<base>/current`, reloading it on a
    /// poll interval instead of requiring a restart for each new capture.
    /// Pairs with `capture-once --out <base>`, which writes that pointer.
    ServeDir {
        #[arg(long)]
        base: PathBuf,
        #[arg(long, default_value = "127.0.0.1:9108")]
        listen: std::net::SocketAddr,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
        poll_interval_seconds: u64,
        #[arg(long, default_value = "deploy/config/monitoring.toml")]
        policy: PathBuf,
    },
    /// Validate local deployment settings without contacting mainnet.
    CheckConfig {
        /// Path to the TOML configuration file.
        #[arg(
            long,
            default_value = "deploy/config/suilend.toml",
            value_name = "FILE"
        )]
        config: PathBuf,
    },
    /// Discover market type instances (includes third-party/experimental markets).
    Markets {
        #[arg(long, default_value = "deploy/config/suilend.toml")]
        config: PathBuf,
        #[arg(long)]
        checkpoint: Option<u64>,
    },
    /// Capture raw positions and owner caps at one checkpoint, then report diagnostics.
    Snapshot {
        #[arg(long, default_value = "deploy/config/suilend.toml")]
        config: PathBuf,
        /// New output directory. Existing files are never overwritten.
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        checkpoint: Option<u64>,
        /// Repeat for each market. Defaults to the configured main market.
        #[arg(long, conflicts_with = "all_markets")]
        market: Vec<String>,
        /// Include all type instances, including unendorsed experimental markets.
        #[arg(long)]
        all_markets: bool,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=50))]
        page_size: u32,
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u32).range(1..=32))]
        concurrency: u32,
        /// Sample at most this many pages per collection; may produce an incomplete capture.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        max_pages: Option<u64>,
    },
    /// Supervised single capture attempt: a single-host lock, a unique
    /// output directory, and a timeout budget around Snapshot's capture +
    /// report. Intended to be invoked by an external scheduler (cron,
    /// systemd timer); this command does not loop or retry on its own.
    CaptureOnce {
        #[arg(long, default_value = "deploy/config/suilend.toml")]
        config: PathBuf,
        /// Base directory; each attempt gets a unique subdirectory, and a
        /// lifecycle record is written under `<out>/runs/`.
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        checkpoint: Option<u64>,
        #[arg(long, conflicts_with = "all_markets")]
        market: Vec<String>,
        #[arg(long)]
        all_markets: bool,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=50))]
        page_size: u32,
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u32).range(1..=32))]
        concurrency: u32,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        max_pages: Option<u64>,
        /// Timeout budget for this single attempt, in seconds.
        #[arg(long, default_value_t = 900, value_parser = clap::value_parser!(u64).range(1..))]
        deadline_seconds: u64,
        /// Identifies this capture job for incident dedup (e.g. the
        /// configured market name). Defaults to --out's own name.
        #[arg(long)]
        scope: Option<String>,
        /// Open/resolve capture_failure incidents in this Postgres database
        /// (temp.md task O04). Optional: with no URL, capture-once behaves
        /// exactly as before and never touches a database. Falls back to
        /// the DIAGNOSIS_DATABASE_URL environment variable.
        #[arg(long, env = "DIAGNOSIS_DATABASE_URL")]
        diagnosis_database_url: Option<String>,
    },
    /// Rebuild deterministic diagnostics offline from an existing raw snapshot.
    Report {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Scenario policy; defaults to the versioned built-in deploy/config/stress.toml.
        #[arg(long)]
        stress_config: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Mcp(args) => commands::mcp::run(args).await?,
        Command::Admin(args) => commands::admin::admin(args).await?,
        Command::Migrate { database_url } => {
            sui_sentinel::diagnosis::Store::connect(&database_url)
                .await?
                .migrate()
                .await?
        }
        Command::Live(args) => commands::live::live(args).await?,
        Command::ProbeGrpc { config } => commands::probe(config).await?,
        Command::FollowCheckpoints(args) => commands::follow::follow(args).await?,
        Command::Monitor(args) => commands::monitor::monitor(args).await?,
        Command::ValidateCapture { input } => commands::validate_capture(&input)?,
        Command::Serve { report, listen } => commands::serve(report, listen).await?,
        Command::ServeDir {
            base,
            listen,
            poll_interval_seconds,
            policy,
        } => commands::serve_dir(base, listen, poll_interval_seconds, policy).await?,
        Command::CheckConfig { config } => commands::check_config(config)?,
        Command::Markets { config, checkpoint } => commands::markets(config, checkpoint).await?,
        Command::Snapshot {
            config,
            out,
            checkpoint,
            market,
            all_markets,
            page_size,
            concurrency,
            max_pages,
        } => {
            commands::snapshot(
                commands::CaptureScope {
                    config,
                    checkpoint,
                    markets: market,
                    all_markets,
                    page_size,
                    concurrency,
                    max_pages,
                },
                out,
            )
            .await?
        }
        Command::CaptureOnce {
            config,
            out,
            checkpoint,
            market,
            all_markets,
            page_size,
            concurrency,
            max_pages,
            deadline_seconds,
            scope,
            diagnosis_database_url,
        } => {
            commands::capture_once(commands::CaptureOnceArgs {
                scope: commands::CaptureScope {
                    config,
                    checkpoint,
                    markets: market,
                    all_markets,
                    page_size,
                    concurrency,
                    max_pages,
                },
                out,
                deadline_seconds,
                incident_scope: scope,
                diagnosis_database_url,
            })
            .await?
        }
        Command::Report {
            input,
            out,
            stress_config,
        } => commands::report(input, out, stress_config)?,
    }
    Ok(())
}

fn client(concurrency: usize) -> Result<Graphql> {
    let endpoint = std::env::var("SUI_GRAPHQL_URL")
        .unwrap_or_else(|_| "https://graphql.mainnet.sui.io/graphql".to_owned());
    let api_key = std::env::var("SUI_API_KEY").ok();
    Graphql::new(&endpoint, api_key.as_deref(), concurrency)
}
