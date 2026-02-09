mod get;
mod key;
mod server;
mod ui;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "A Tor onion service that serves static files over HTTP")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the Tor onion service, serving static files
    Server {
        /// Ed25519 private key in UR format for a deterministic .onion address
        #[arg(long, env = "GARNER_KEY")]
        key: Option<String>,
        /// Directory to serve files from [default: public]
        #[arg(long, default_value = "public")]
        docroot: String,
    },
    /// Fetch a document from a .onion URL over Tor
    Get {
        /// URL(s) or path(s) to fetch (paths when --key or --address is given)
        #[arg(required = true)]
        urls: Vec<String>,
        /// Ed25519 public key in UR format to derive the .onion address
        #[arg(long, env = "GARNER_KEY")]
        key: Option<String>,
        /// The .onion address to connect to (e.g. xxxx.onion)
        #[arg(long, env = "GARNER_ADDRESS")]
        address: Option<String>,
    },
    /// Generate keys and other artifacts
    Generate {
        #[command(subcommand)]
        command: GenerateCommands,
    },
}

#[derive(Subcommand)]
enum GenerateCommands {
    /// Generate an Ed25519 keypair for use with garner server/get
    Keypair,
}

/// Build a [`TorClientConfigBuilder`] with garner's standard settings:
/// ephemeral (in-memory) keystore so switching keys never conflicts.
/// Callers provide explicit state and cache paths obtained from
/// [`tor_dirs`].
fn tor_config(
    state_dir: impl AsRef<std::path::Path>,
    cache_dir: impl AsRef<std::path::Path>,
) -> arti_client::config::TorClientConfigBuilder {
    let mut builder =
        arti_client::config::TorClientConfigBuilder::from_directories(
            state_dir,
            cache_dir,
        );
    builder
        .storage()
        .keystore()
        .primary()
        .kind(tor_config::ExplicitOrAuto::Explicit(
            tor_keymgr::config::ArtiKeystoreKind::Ephemeral,
        ));
    builder
}

/// Create an ephemeral state directory and the shared cache directory
/// under garner's data dir.  Returns `(state_dir, cache_dir)` where
/// `state_dir` is a [`tempfile::TempDir`] that is automatically deleted
/// when dropped.
///
/// Callers must keep `state_dir` alive for the lifetime of the
/// `TorClient`, and must declare it *before* the `TorClient` so that
/// Rust's reverse drop order releases the Tor locks before the
/// directory is removed.
fn tor_dirs() -> Result<(tempfile::TempDir, PathBuf)> {
    let data_dir = ui::data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let cache_dir = data_dir.join("cache");
    let state_dir = tempfile::Builder::new()
        .prefix("state-")
        .tempdir_in(&data_dir)?;
    // Arti requires state dirs to be owner-only (0o700).  tempfile
    // inherits the default umask (typically 0o755 on macOS).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            state_dir.path(),
            std::fs::Permissions::from_mode(0o700),
        )?;
    }
    Ok((state_dir, cache_dir))
}

fn generate_keypair() -> Result<()> {
    let (priv_ur, pub_ur) = key::generate_keypair()?;
    println!("{priv_ur}");
    println!("{pub_ur}");
    Ok(())
}

#[tokio::main]
async fn main() {
    bc_components::register_tags();
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Server { key, docroot } => server::run(key.as_deref(), &docroot).await,
        Commands::Get { urls, key, address } => {
            get::run(&urls, key.as_deref(), address.as_deref()).await
        }
        Commands::Generate { command } => match command {
            GenerateCommands::Keypair => generate_keypair(),
        },
    };
    if let Err(e) = result {
        if ui::is_interactive() {
            eprintln!("\x1b[1;31merror: {e:#}\x1b[0m");
        } else {
            eprintln!("error: {e:#}");
        }
        std::process::exit(1);
    }
}
