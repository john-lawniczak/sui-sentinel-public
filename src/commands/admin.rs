//! Administration uses PostgreSQL authentication/roles; no mutation HTTP endpoint.
use anyhow::Result;
use clap::{Args, Subcommand};
use sui_sentinel::diagnosis::Store;
#[derive(Args)]
pub struct AdminArgs {
    #[arg(long, env = "DIAGNOSIS_DATABASE_URL", hide_env_values = true)]
    pub database_url: String,
    #[arg(long)]
    pub scope: String,
    #[command(subcommand)]
    pub action: Action,
}
#[derive(Subcommand)]
pub enum Action {
    Inspect,
    Acknowledge {
        id: i64,
        #[arg(long)]
        reason: String,
    },
    Retry {
        id: i64,
        #[arg(long)]
        reason: String,
    },
}
pub async fn admin(args: AdminArgs) -> Result<()> {
    let store = Store::connect(&args.database_url).await?;
    match args.action {
        Action::Inspect => println!(
            "{}",
            serde_json::to_string_pretty(&store.inspect_operations(&args.scope).await?)?
        ),
        Action::Acknowledge { id, reason } => {
            store
                .administer(&args.scope, "acknowledge", id, &reason)
                .await?
        }
        Action::Retry { id, reason } => store.administer(&args.scope, "retry", id, &reason).await?,
    }
    Ok(())
}
