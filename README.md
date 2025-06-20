# ETT Summary Project

This repository contains a Rust client that records audio from online meetings and a Rust server that transcribes the audio with `whisper-rs` and keeps an incident summary using an OpenAI-compatible API.

The typical flow is:

1. The client records audio from the configured device and periodically sends WAV files to the summarization server.
2. The server transcribes each file with whisper, updates the running summary and posts it to a configured webhook.
3. If no audio is received for one hour the summary is cleared.

See [`server-rs`](server-rs/) for details about the Rust server and configuration options.

## Quickstart

1. Install the [Rust toolchain](https://www.rust-lang.org/tools/install).
2. Copy `server-rs/config.toml` and adjust the API endpoint, model and webhook URL.
3. Run the server:
   ```bash
   cargo run --release --manifest-path server-rs/Cargo.toml
   ```
4. Send a WAV file to the running server:
   ```bash
   curl -F file=@audio.wav http://localhost:8000/upload
   ```
The server posts an updated summary after each upload and clears it after an hour of inactivity.

