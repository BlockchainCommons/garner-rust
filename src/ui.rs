use std::io::IsTerminal;
use std::time::{Duration, Instant};

use chrono::Utc;
use indicatif::ProgressBar;

/// Check if stderr is connected to an interactive terminal.
pub fn is_interactive() -> bool {
    std::io::stderr().is_terminal()
}

/// Format a timestamp in Common Log Format: `DD/Mon/YYYY:HH:MM:SS +0000`.
pub fn clf_timestamp() -> String {
    Utc::now().format("%d/%b/%Y:%H:%M:%S +0000").to_string()
}

/// Print a timestamped log message to stderr.
pub fn log(message: &str) {
    eprintln!("[{}] {}", Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"), message);
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
