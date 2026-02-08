mod get;
mod key;
mod server;
mod ui;

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
    },
    /// Fetch a document from a .onion URL over Tor
    Get {
        /// The .onion URL to fetch (e.g. http://<addr>.onion/path)
        url: String,
        /// Ed25519 public key in UR format to derive the .onion address
        #[arg(long, env = "GARNER_KEY")]
        key: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Server { key } => server::run(key.as_deref()).await,
        Commands::Get { url, key } => get::run(&url, key.as_deref()).await,
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
