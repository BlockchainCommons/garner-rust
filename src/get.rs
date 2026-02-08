use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use arti_client::TorClient;
use futures_util::io::{AsyncReadExt, AsyncWriteExt};
use indicatif::{ProgressBar, ProgressStyle};

use crate::ui;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

pub async fn run(url: &str, key: Option<&str>) -> Result<()> {
    let interactive = ui::is_interactive();

    // Set up spinner (interactive only)
    let bar = if interactive {
        let bar = ProgressBar::new_spinner();
        bar.set_prefix(" 0s");
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.yellow} {prefix} Connecting to the Tor network...")
                .expect("valid template"),
        );
        bar.enable_steady_tick(Duration::from_millis(100));
        Some(bar)
    } else {
        None
    };

    let updater = bar.as_ref().map(ui::spawn_elapsed_updater);

    // When a public key is provided, derive the .onion host from it
    // and treat `url` as a path-only argument.
    let resolved_url;
    let fetch_url = if let Some(key_ur) = key {
        let onion_host = crate::key::parse_public_key_to_onion_host(key_ur)?;
        let path = if url.starts_with('/') { url } else { &format!("/{url}") };
        resolved_url = format!("{onion_host}{path}");
        &resolved_url
    } else {
        url
    };

    let result = do_fetch(fetch_url, bar.as_ref()).await;

    // Clean up spinner *before* writing to stdout so finish_and_clear
    // doesn't erase the output line.
    if let Some(ref h) = updater { h.abort(); }
    if let Some(ref bar) = bar { bar.finish_and_clear(); }

    let body = result?;

    use std::io::Write;
    std::io::stdout().write_all(&body)?;

    Ok(())
}

async fn do_fetch(url: &str, bar: Option<&ProgressBar>) -> Result<Vec<u8>> {
    // Parse the URL to extract host and path
    let url = url.strip_prefix("http://").unwrap_or(url);
    let (host, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };

    if !host.ends_with(".onion") {
        return Err(anyhow!(
            "expected a .onion address, got: {host}"
        ));
    }

    // Use a separate state directory so the client doesn't interfere
    // with a concurrently running garner server instance.
    let data_dir = dirs();
    let mut builder =
        arti_client::config::TorClientConfigBuilder::from_directories(
            data_dir.join("state"),
            data_dir.join("cache"),
        );
    builder
        .stream_timeouts()
        .connect_timeout(CONNECT_TIMEOUT);
    let config = builder.build()?;

    // Bootstrap Tor client
    let tor = TorClient::create_bootstrapped(config).await?;

    // Switch to connect phase
    if let Some(bar) = bar {
        bar.set_style(
            ProgressStyle::default_spinner()
                .template(&format!(
                    "{{spinner:.cyan}} {{prefix}} Connecting to {}...",
                    host
                ))
                .expect("valid template"),
        );
    }

    let mut stream = tor
        .connect((host, 80))
        .await
        .context("connecting to onion service")?;

    // Send a minimal HTTP/1.1 GET request
    let request = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Connection: close\r\n\
         \r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .context("writing request")?;
    stream.flush().await.context("flushing request")?;

    // Workaround for arti bug https://gitlab.torproject.org/tpo/core/arti/-/issues/1931
    //
    // The Tor spec requires stream originators to close with END reason
    // MISC (not DONE), and arti has no public API to send END DONE.
    // However, arti's reader treats END MISC as an error rather than
    // EOF, so read_to_end() fails even though all response bytes were
    // already delivered into the buffer.  We tolerate only that
    // specific error when data has already been received.
    let mut response = Vec::new();
    if let Err(e) = stream.read_to_end(&mut response).await {
        let is_end_misc = e
            .to_string()
            .contains("END cell with reason MISC");
        if !is_end_misc || response.is_empty() {
            return Err(
                anyhow!(e).context("reading response")
            );
        }
    }

    let response_str = String::from_utf8_lossy(&response);

    // Parse status line
    let status_line = response_str
        .lines()
        .next()
        .ok_or_else(|| anyhow!("empty response"))?;

    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| {
            anyhow!("malformed status line: {status_line}")
        })?
        .parse()
        .context("parsing status code")?;

    if status_code != 200 {
        return Err(anyhow!(
            "server returned HTTP {status_code}: {status_line}"
        ));
    }

    // Find the end of headers (\r\n\r\n) and return body
    let header_end = response_str
        .find("\r\n\r\n")
        .ok_or_else(|| anyhow!("no header/body separator found"))?;

    let body_start = header_end + 4;
    Ok(response[body_start..].to_vec())
}

fn dirs() -> PathBuf {
    if let Some(data) = dirs_data_dir() {
        data.join("garner-client")
    } else {
        PathBuf::from(".garner-client")
    }
}

fn dirs_data_dir() -> Option<PathBuf> {
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
