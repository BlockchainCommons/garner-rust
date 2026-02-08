use std::io::IsTerminal;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};

use anyhow::{anyhow, Context, Result};
use arti_client::{
    config::onion_service::OnionServiceConfigBuilder, TorClient,
    TorClientConfig,
};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use mime_guess::MimeGuess;
use safelog::DisplayRedacted as _;
use std::time::Duration;
use tor_cell::relaycell::msg::{Connected, End};
use tor_hsservice::{handle_rend_requests, status::State, StreamRequest};
use tor_proto::client::stream::IncomingStreamRequest;

use crate::ui;

pub async fn run(key: Option<&str>) -> Result<()> {
    let interactive = ui::is_interactive();
    let start = Instant::now();

    // Set up bootup spinner (interactive) or log (non-interactive)
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
        ui::log("Connecting to the Tor network...");
        None
    };

    let updater = bar.as_ref().map(ui::spawn_elapsed_updater);

    // 1) Bootstrap Arti (Tor client)
    let tor =
        TorClient::create_bootstrapped(TorClientConfig::default())
            .await
            .inspect_err(|_| {
                if let Some(ref h) = updater { h.abort(); }
                if let Some(ref bar) = bar { bar.finish_and_clear(); }
            })?;

    // 2) Configure + launch onion service
    let svc_cfg = OnionServiceConfigBuilder::default()
        .nickname("garner".to_string().try_into()?)
        .build()?;

    // Launch with a user-supplied key (deterministic address) or
    // ephemerally.  The two methods return different opaque Stream
    // types, so we box-erase them into a common type.
    type RendStream = std::pin::Pin<
        Box<dyn futures_util::Stream<Item = tor_hsservice::RendRequest> + Send>,
    >;

    let launch_result: Option<(
        std::sync::Arc<tor_hsservice::RunningOnionService>,
        RendStream,
    )> = if let Some(key_ur) = key {
        let hsid_keypair = crate::key::parse_private_key(key_ur)?;
        tor.launch_onion_service_with_hsid(svc_cfg, hsid_keypair)?
            .map(|(svc, s)| (svc, Box::pin(s) as RendStream))
    } else {
        tor.launch_onion_service(svc_cfg)?
            .map(|(svc, s)| (svc, Box::pin(s) as RendStream))
    };

    let Some((svc, rend_requests)) = launch_result else {
        if let Some(ref h) = updater { h.abort(); }
        if let Some(ref bar) = bar { bar.finish_and_clear(); }
        return Err(anyhow!(
            "Onion service is disabled in config \
             (launch_onion_service returned None)"
        ));
    };

    let onion = svc
        .onion_address()
        .ok_or_else(|| {
            if let Some(ref h) = updater { h.abort(); }
            if let Some(ref bar) = bar { bar.finish_and_clear(); }
            anyhow!("Couldn't determine onion address (missing key?)")
        })?;
    let onion_url =
        format!("http://{}/", onion.display_unredacted());

    // Update spinner for descriptor publication phase
    if let Some(ref bar) = bar {
        bar.set_style(
            ProgressStyle::default_spinner()
                .template(&format!(
                    "{{spinner:.yellow}} {{prefix}} Starting server for {}...",
                    onion_url
                ))
                .expect("valid template"),
        );
    } else {
        ui::log(&format!(
            "Starting server for {}...",
            onion_url
        ));
    }

    // Wait for the descriptor to be published to the Tor network's
    // HSDir nodes before declaring the service ready.
    let mut status_stream = svc.status_events();
    let mut last_state = None;
    while let Some(status) = status_stream.next().await {
        let state = status.state();
        match state {
            State::Running | State::DegradedReachable => break,
            State::Broken => {
                if let Some(ref h) = updater { h.abort(); }
                if let Some(ref bar) = bar { bar.finish_and_clear(); }
                let problem = status
                    .current_problem()
                    .map(|p| format!("{p:?}"))
                    .unwrap_or_else(|| "unknown".into());
                return Err(anyhow!(
                    "Onion service failed: {problem}"
                ));
            }
            _ => {
                if last_state != Some(state) {
                    if let Some(ref bar) = bar {
                        bar.set_style(
                            ProgressStyle::default_spinner()
                                .template(&format!(
                                    "{{spinner:.yellow}} {{prefix}} Starting server for {} [{state:?}]",
                                    onion_url,
                                ))
                                .expect("valid template"),
                        );
                    } else {
                        ui::log(&format!("Status: {state:?}"));
                    }
                    last_state = Some(state);
                }
            }
        }
    }

    // Bootup complete
    let elapsed = start.elapsed().as_secs();
    if let Some(ref h) = updater { h.abort(); }
    if let Some(ref bar) = bar {
        bar.finish_and_clear();
        eprintln!(
            "\u{2713} Serving {} (started in {}s)",
            onion_url, elapsed
        );
    } else {
        ui::log(&format!(
            "Serving {} (started in {}s)",
            onion_url, elapsed
        ));
    }

    // Print raw URL to stdout for piping (skip in interactive mode
    // where the ✓ line already shows it)
    if !std::io::stdout().is_terminal() {
        println!("{onion_url}");
    }

    // 3) Accept rendezvous requests => stream of StreamRequest
    let mut stream_reqs = handle_rend_requests(rend_requests);

    // 4) Whitelist: URL path -> file on disk
    let files: Arc<HashMap<&'static str, PathBuf>> = Arc::new(
        [
            ("/", PathBuf::from("public/index.html")),
            ("/index.txt", PathBuf::from("public/index.txt")),
        ]
        .into_iter()
        .collect(),
    );

    // Serving spinner (interactive only)
    let serve_bar = if interactive {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} Waiting for connections...")
                .expect("valid template"),
        );
        bar.enable_steady_tick(Duration::from_millis(200));
        Some(bar)
    } else {
        None
    };

    // Handle incoming streams forever
    while let Some(req) = stream_reqs.next().await {
        let files = Arc::clone(&files);
        let serve_bar = serve_bar.clone();
        tokio::spawn(async move {
            if let Err(e) =
                handle_stream_request(req, files, serve_bar.as_ref(), interactive).await
            {
                if let Some(ref bar) = serve_bar {
                    bar.println(format!("  stream error: {e:#}"));
                } else {
                    ui::log(&format!("stream error: {e:#}"));
                }
            }
        });
    }

    Ok(())
}

async fn handle_stream_request(
    req: StreamRequest,
    files: Arc<HashMap<&'static str, PathBuf>>,
    serve_bar: Option<&ProgressBar>,
    interactive: bool,
) -> Result<()> {
    if !matches!(req.request(), IncomingStreamRequest::Begin(_)) {
        let _ = req.reject(End::new_misc()).await;
        return Ok(());
    }

    // Accept -> DataStream
    let mut stream = req.accept(Connected::new_empty()).await?;

    let (method, path) =
        read_http_request_line(&mut stream).await?;

    let (status, body_len) = if method != "GET" {
        write_http_response(
            &mut stream,
            405,
            "text/plain",
            b"Method Not Allowed",
        )
        .await?;
        (405u16, 18usize)
    } else if let Some(file_path) = files.get(path.as_str()) {
        let body = tokio::fs::read(file_path)
            .await
            .with_context(|| format!("reading {file_path:?}"))?;
        let len = body.len();
        let mime =
            MimeGuess::from_path(file_path).first_or_octet_stream();
        write_http_response(&mut stream, 200, mime.as_ref(), &body)
            .await?;
        (200, len)
    } else {
        write_http_response(
            &mut stream,
            404,
            "text/plain",
            b"Not Found",
        )
        .await?;
        (404, 9usize)
    };

    // Log in Common Log Format:
    //   <host> - - [<timestamp>] "<method> <path> HTTP/1.1" <status> <size>
    // Host is always "-" since Tor hides the client address.
    let log_line = format!(
        "- - - [{}] \"{method} {path} HTTP/1.1\" {status} {body_len}",
        ui::clf_timestamp()
    );
    if let Some(bar) = serve_bar {
        bar.println(format!("  {log_line}"));
    } else if !interactive {
        eprintln!("{log_line}");
    }

    Ok(())
}

async fn read_http_request_line(
    stream: &mut tor_proto::client::stream::DataStream,
) -> Result<(String, String)> {
    use futures_util::io::AsyncReadExt;

    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    let s = std::str::from_utf8(&buf[..n])
        .context("request not valid UTF-8")?;

    let first_line =
        s.lines().next().ok_or_else(|| anyhow!("empty request"))?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    Ok((method, path))
}

async fn write_http_response(
    stream: &mut tor_proto::client::stream::DataStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    use futures_util::io::AsyncWriteExt;

    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };

    let header = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Length: {}\r\n\
         Content-Type: {content_type}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );

    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    // Explicitly close the write half so the remote side sees a clean
    // stream shutdown rather than an abrupt drop.
    stream.close().await?;
    Ok(())
}
