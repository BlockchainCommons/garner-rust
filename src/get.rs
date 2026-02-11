use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use arti_client::TorClient;
use futures_util::io::{AsyncReadExt, AsyncWriteExt};
use indicatif::{ProgressBar, ProgressStyle};

use crate::ui;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

pub async fn run(
    urls: &[String],
    key: Option<&str>,
    address: Option<&str>,
) -> Result<()> {
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

    // Resolve the .onion host when --key or --address is provided.
    let onion_host: Option<String> = if let Some(key_ur) = key {
        Some(crate::key::parse_public_key_to_onion_host(key_ur)?)
    } else if let Some(addr) = address {
        let host = addr.strip_prefix("http://").unwrap_or(addr);
        let host = host.strip_suffix('/').unwrap_or(host);
        Some(host.to_string())
    } else {
        None
    };

    // Build full URLs from paths (when host is known) or use as-is.
    let resolved: Vec<String> = urls
        .iter()
        .map(|u| {
            if let Some(ref host) = onion_host {
                let path = if u.starts_with('/') {
                    u.clone()
                } else {
                    format!("/{u}")
                };
                format!("{host}{path}")
            } else {
                u.clone()
            }
        })
        .collect();

    // Bootstrap Tor once, then fetch each URL.
    // Ephemeral state dir avoids lock contention with concurrent
    // invocations.  Declared before `tor` so it drops (and is deleted)
    // after the TorClient releases its locks.
    let (state_dir, cache_dir) = crate::tor_dirs()?;
    let mut builder = crate::tor_config(state_dir.path(), &cache_dir);
    builder.stream_timeouts().connect_timeout(CONNECT_TIMEOUT);
    let config = builder.build()?;
    let tor = TorClient::create_bootstrapped(config).await?;

    let mut bodies: Vec<Vec<u8>> = Vec::with_capacity(resolved.len());
    for url in &resolved {
        bodies.push(fetch_url(&tor, url, bar.as_ref()).await?);
    }

    // Clean up spinner *before* writing to stdout so finish_and_clear
    // doesn't erase the output line.
    if let Some(ref h) = updater {
        h.abort();
    }
    if let Some(ref bar) = bar {
        bar.finish_and_clear();
    }

    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for (i, body) in bodies.iter().enumerate() {
        if i > 0 {
            out.write_all(b"\n")?;
        }
        out.write_all(body)?;
    }

    Ok(())
}

/// Connect to an onion service and fetch a single URL, reusing an
/// already-bootstrapped Tor client.
async fn fetch_url<R: tor_rtcompat::Runtime>(
    tor: &TorClient<R>,
    url: &str,
    bar: Option<&ProgressBar>,
) -> Result<Vec<u8>> {
    // Parse the URL to extract host and path
    let url = url.strip_prefix("http://").unwrap_or(url);
    let (host, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };

    if !host.ends_with(".onion") {
        return Err(anyhow!("expected a .onion address, got: {host}"));
    }

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
        let is_end_misc = e.to_string().contains("END cell with reason MISC");
        if !is_end_misc || response.is_empty() {
            return Err(anyhow!(e).context("reading response"));
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
        .ok_or_else(|| anyhow!("malformed status line: {status_line}"))?
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
