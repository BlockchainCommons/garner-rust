# Garner

A Tor onion service that serves static files over HTTP, built with [Arti](https://gitlab.torproject.org/tpo/core/arti).

## Installing

```bash
cd garner
cargo install --path .

garner --help
```

## Quick Start

### Ephemeral mode (random .onion address)

Place files in a `public/` directory and start the server:

```bash
mkdir -p public
echo "Hello from Garner via Tor!" > public/index.txt
garner server
```

Garner connects to the Tor network, publishes a hidden-service descriptor, and prints the `.onion` URL once it is reachable.  It also prints the public key in UR format (`ur:signing-public-key/…`) so clients can use it with `garner get --key`.  In a separate terminal, fetch a file with the built-in client:

```bash
garner get http://<onion-address>.onion/
```

Each run generates a new ephemeral address.

### Deterministic mode (persistent .onion address)

Supply an Ed25519 key to get the same `.onion` address across restarts.

#### 1. Generate a keypair

```bash
garner generate keypair
```

This prints two lines: the private key UR (`ur:signing-private-key/…`) on line 1 and the public key UR (`ur:signing-public-key/…`) on line 2.  Save them to files:

```bash
garner generate keypair | { read -r priv; read -r pub; echo "$priv" > key.ur; echo "$pub" > pubkey.ur; }
```

Keep `key.ur` secret. You can share `pubkey.ur` with anyone who needs to connect to your server.

Alternatively, the [Gordian Envelope CLI](https://github.com/BlockchainCommons/bc-envelope-cli-rust) (`envelope`) can generate key bundles that garner also accepts:

```bash
envelope generate prvkeys --signing ed25519 > key.ur
envelope generate pubkeys < key.ur > pubkey.ur
```

#### 2. Start the server with a key

```bash
garner server --key "$(cat key.ur)"
```

The server derives a deterministic `.onion` address from the Ed25519 private key.  Restarting with the same key produces the same address.

#### 3. Fetch using the public key

A client with the corresponding public key can connect without knowing the full `.onion` URL:

```bash
garner get --key "$(cat pubkey.ur)" /index.txt
```

The `--key` flag derives the `.onion` host from the public key, so the positional arguments become just paths.  You can fetch multiple paths in a single invocation:

```bash
garner get --key "$(cat pubkey.ur)" / /index.txt
```

#### 4. Fetch using the .onion address directly

If you already know the `.onion` address, use `--address` instead of `--key`:

```bash
garner get --address <onion-address>.onion /index.txt
```

Like `--key`, `--address` lets you pass one or more paths as positional arguments.

You can still use a full URL without `--key` or `--address`:

```bash
garner get http://<onion-address>.onion/index.txt
```

## Environment Variables

Both subcommands read `GARNER_KEY` as a fallback for `--key`.  The `get` subcommand also reads `GARNER_ADDRESS` as a fallback for `--address`.

```bash
export GARNER_KEY="$(cat key.ur)"
garner server                        # uses GARNER_KEY as private key
```

```bash
export GARNER_KEY="$(cat pubkey.ur)"
garner get /index.txt                # uses GARNER_KEY as public key
```

```bash
export GARNER_ADDRESS="<onion-address>.onion"
garner get / /index.txt              # uses GARNER_ADDRESS
```

## Concurrency

Multiple `garner` processes can run at the same time — for example, a long-running `garner server` alongside one or more `garner get` requests, or several parallel fetches.  Each invocation creates its own ephemeral Tor state directory, so there is no lock contention between processes.  All invocations share a single Tor network cache directory, which is safe for concurrent access.  No private key material is ever written to disk — garner uses an in-memory keystore exclusively.

## Accepted Key Formats

Garner accepts two UR key formats:

| Format | UR type | Produced by |
|--------|---------|-------------|
| Key bundle (default) | `ur:crypto-prvkeys` / `ur:crypto-pubkeys` | `envelope generate prvkeys` / `pubkeys` |
| Signing key only | `ur:signing-private-key` / `ur:signing-public-key` | direct CBOR construction |

When a key bundle is provided, garner extracts the Ed25519 signing key and ignores the encapsulation key.

## Served Files

The server exposes a fixed set of paths from the document root directory (default `public/`, configurable with `--docroot`):

| URL path | File |
|----------|------|
| `/` | `<docroot>/index.html`, falling back to `<docroot>/index.txt` |
| `/index.html` | `<docroot>/index.html` |
| `/index.txt` | `<docroot>/index.txt` |

A request to `/` serves `index.html` if it exists, otherwise `index.txt`.  All other paths return 404.  The `Content-Type` header is set from the file extension (`text/html` for `.html`, `text/plain` for `.txt`).  The server exits immediately if the document root directory does not exist.

## CLI Reference

```
garner generate keypair
```

Generate a random Ed25519 keypair.  Prints the private key UR on line 1 and the public key UR on line 2.

```
garner server [--key <UR>] [--docroot <DIR>]
```

Start a Tor onion service serving files from the given document root (default `public/`).  Prints the `.onion` URL and the public key UR to stderr on startup.

| Option | Description |
|--------|-------------|
| `--key <UR>` | Ed25519 private key in UR format for a deterministic `.onion` address. Also reads `GARNER_KEY` env var. |
| `--docroot <DIR>` | Directory to serve files from. Defaults to `public`. |

```
garner get [--key <UR>] [--address <ADDR>] <URL>...
```

Fetch one or more documents from a `.onion` address over Tor.

| Option / Arg       | Description                                                                                   |
|--------------------|-----------------------------------------------------------------------------------------------|
| `<URL>...`         | Full `.onion` URL(s), or path(s) when `--key` or `--address` is set.                          |
| `--key <UR>`       | Ed25519 public key in UR format to derive the `.onion` host. Also reads `GARNER_KEY` env var. |
| `--address <ADDR>` | `.onion` address to connect to directly. Also reads `GARNER_ADDRESS` env var.                 |
