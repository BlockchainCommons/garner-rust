# Garner

A Tor onion service that serves static files over HTTP, built with [Arti](https://gitlab.torproject.org/tpo/core/arti).

## Building

```bash
cargo build -p garner --release
```

The binary is written to `target/release/garner`.

## Quick Start

### Ephemeral mode (random .onion address)

Place files in a `public/` directory and start the server:

```bash
mkdir -p public
echo "Hello from Tor" > public/index.txt
garner server
```

Garner connects to the Tor network, publishes a hidden-service descriptor,
and prints the `.onion` URL once it is reachable.  Fetch a file with the
built-in client:

```bash
garner get http://<onion-address>.onion/index.txt
```

Each run generates a new ephemeral address.

### Deterministic mode (persistent .onion address)

Supply an Ed25519 key to get the same `.onion` address across restarts.

#### 1. Generate a keypair

Use the [Gordian Envelope CLI](https://github.com/BlockchainCommons/bc-envelope-cli-rust)
(`envelope`) to generate an Ed25519 key bundle:

```bash
envelope generate prvkeys --signing ed25519 > key.ur
envelope generate pubkeys < key.ur > pubkey.ur
```

`key.ur` contains a `ur:crypto-prvkeys/...` string (private + encryption keys).
`pubkey.ur` contains the matching `ur:crypto-pubkeys/...` string.

Keep `key.ur` secret. You can share `pubkey.ur` with anyone who needs to
connect to your server.

#### 2. Start the server with a key

```bash
garner server --key "$(cat key.ur)"
```

The server derives a deterministic `.onion` address from the Ed25519 private
key.  Restarting with the same key produces the same address.

#### 3. Fetch using the public key

A client with the corresponding public key can connect without knowing the
full `.onion` URL:

```bash
garner get --key "$(cat pubkey.ur)" /index.txt
```

The `--key` flag derives the `.onion` host from the public key, so the
positional `URL` argument becomes just a path.

You can still use a full URL without `--key`:

```bash
garner get http://<onion-address>.onion/index.txt
```

## Environment Variable

Both subcommands read the `GARNER_KEY` environment variable as a fallback
for the `--key` flag:

```bash
export GARNER_KEY="$(cat key.ur)"
garner server                        # uses GARNER_KEY as private key
```

```bash
export GARNER_KEY="$(cat pubkey.ur)"
garner get /index.txt                # uses GARNER_KEY as public key
```

## Accepted Key Formats

Garner accepts two UR key formats:

| Format | UR type | Produced by |
|--------|---------|-------------|
| Key bundle (default) | `ur:crypto-prvkeys` / `ur:crypto-pubkeys` | `envelope generate prvkeys` / `pubkeys` |
| Signing key only | `ur:signing-private-key` / `ur:signing-public-key` | direct CBOR construction |

When a key bundle is provided, garner extracts the Ed25519 signing key and
ignores the encapsulation key.

## Served Files

The server currently exposes a fixed set of paths from the `public/`
directory relative to the working directory:

| URL path | File |
|----------|------|
| `/` | `public/index.html` |
| `/index.txt` | `public/index.txt` |

All other paths return 404.

## CLI Reference

```
garner server [--key <UR>]
```

Start a Tor onion service serving files from `public/`.

| Option | Description |
|--------|-------------|
| `--key <UR>` | Ed25519 private key in UR format for a deterministic `.onion` address. Also reads `GARNER_KEY` env var. |

```
garner get [--key <UR>] <URL>
```

Fetch a document from a `.onion` address over Tor.

| Option / Arg | Description |
|--------------|-------------|
| `<URL>` | Full `.onion` URL, or a path when `--key` is set. |
| `--key <UR>` | Ed25519 public key in UR format to derive the `.onion` host. Also reads `GARNER_KEY` env var. |
