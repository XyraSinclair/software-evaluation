use clap::{Parser, Subcommand};
use software_evaluation::service::{
    app::{self, AppState, ServiceConfig},
    dto::RepositoryProvenance,
    github::new_github_client,
    worker,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[derive(Parser)]
#[command(
    name = "sevald",
    about = "Bounded public GitHub repository analysis service"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value = "127.0.0.1:7077")]
        listen: SocketAddr,
        #[arg(long, default_value = ".seval-cache")]
        cache_dir: PathBuf,
    },
    #[command(hide = true)]
    Worker {
        #[arg(long)]
        repository_root: PathBuf,
        #[arg(long)]
        full_name: String,
        #[arg(long)]
        repository_id: u64,
        #[arg(long)]
        commit: String,
    },
}
#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("sevald: {message}");
        std::process::exit(1)
    }
}
async fn run() -> Result<(), String> {
    match Cli::parse().command.unwrap_or(Command::Serve {
        listen: "127.0.0.1:7077".parse().unwrap(),
        cache_dir: PathBuf::from(".seval-cache"),
    }) {
        Command::Serve { listen, cache_dir } => {
            let source = new_github_client(std::env::var("GITHUB_TOKEN").ok())
                .map_err(|_| "could not initialize GitHub client")?;
            let state = AppState::new(
                ServiceConfig {
                    cache_dir,
                    ..Default::default()
                },
                Arc::new(source),
            )
            .map_err(|_| "could not initialize service")?;
            let socket = tokio::net::TcpListener::bind(listen)
                .await
                .map_err(|_| "could not bind service address")?;
            axum::serve(socket, app::router(state))
                .await
                .map_err(|_| "service stopped unexpectedly".into())
        }
        Command::Worker {
            repository_root,
            full_name,
            repository_id,
            commit,
        } => {
            if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid worker provenance".into());
            }
            let result = worker::analyze(
                &repository_root,
                RepositoryProvenance {
                    full_name,
                    repository_id,
                    commit,
                    cached: false,
                },
            );
            serde_json::to_writer(std::io::stdout().lock(), &result)
                .map_err(|_| "could not write worker result".into())
        }
    }
}
