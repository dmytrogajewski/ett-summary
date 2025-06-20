# ETT Summary Project

This repository contains a Rust client that records audio from online meetings and a Rust server that summarizes the conversation using an OpenAI-compatible API.

The typical flow is:

1. The client records audio from the configured device, transcribes it (transcription is not implemented here) and sends transcript text to the summarization server.
2. The server accumulates the text and every five minutes posts a summary to a configured webhook.
3. If no text is received for one hour the accumulated text is summarized and flushed.

See [`server-rs`](server-rs/) for details about the Rust server and configuration options.

## Quickstart

1. Install the [Rust toolchain](https://www.rust-lang.org/tools/install).
2. Copy `server-rs/config.toml` and adjust the API endpoint, model and webhook URL.
3. Run the server:
   ```bash
   cargo run --release --manifest-path server-rs/Cargo.toml
   ```
4. Send transcript text to the running server:
   ```bash
   curl -X POST -H "Content-Type: application/json" \
        -d '{"text":"hello world"}' http://localhost:8000/transcript
   ```

The server will call the configured webhook with a summary every five minutes or after an hour of inactivity.
