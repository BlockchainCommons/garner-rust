use std::{
    io::IsTerminal,
    path::PathBuf,
    time::{Duration, Instant},
};

use chrono::Utc;
use indicatif::ProgressBar;

/// Check if stderr is connected to an interactive terminal.
pub fn is_interactive() -> bool { std::io::stderr().is_terminal() }

/// Format a timestamp in Common Log Format: `DD/Mon/YYYY:HH:MM:SS +0000`.
pub fn clf_timestamp() -> String {
    Utc::now().format("%d/%b/%Y:%H:%M:%S +0000").to_string()
}

/// Print a timestamped log message to stderr.
pub fn log(message: &str) {
    eprintln!(
        "[{}] {}",
        Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        message
    );
}

/// Return the platform-specific application data directory for garner
/// (`~/Library/Application Support/garner` on macOS,
/// `$XDG_DATA_HOME/garner` or `~/.local/share/garner` on Linux).
pub fn data_dir() -> PathBuf {
    let base = data_base_dir().unwrap_or_else(|| PathBuf::from(".garner"));
    base.join("garner")
}

fn data_base_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".local/share"))
            })
    }
}

/// Spawn a background task that updates `bar`'s prefix with a
/// space-padded elapsed-seconds counter every second.
pub fn spawn_elapsed_updater(bar: &ProgressBar) -> tokio::task::JoinHandle<()> {
    let bar = bar.clone();
    tokio::spawn(async move {
        let start = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let secs = start.elapsed().as_secs();
            bar.set_prefix(format!("{secs:>2}s"));
        }
    })
}
