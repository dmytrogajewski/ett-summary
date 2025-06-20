# Rust Summarization Server

This Rust server receives transcript text and periodically summarizes it using an OpenAI-compatible API. Every five minutes the summary is sent to a webhook. If no text is received for one hour, the remaining buffered text is summarized and flushed.

## Requirements
- Rust toolchain
- `OPENAI_API_KEY` environment variable set to a valid OpenAI API key
- `config.toml` file describing API and webhook settings

## Running

```bash
cargo run --release --manifest-path server-rs/Cargo.toml
```

The server listens on `http://localhost:8000` by default.

Send transcript text to `/transcript`:

```bash
curl -X POST -H "Content-Type: application/json" \
     -d '{"text":"some transcript"}' http://localhost:8000/transcript
```

Summaries are automatically sent to the configured webhook every five minutes (or sooner after one hour of inactivity).

## Configuration

Create a `config.toml` file in the `server-rs` directory (or set the `CONFIG_FILE` environment variable to another path). Example:

```toml
openai_api_url = "https://api.openai.com/v1/chat/completions"
openai_model = "gpt-3.5-turbo"
webhook_url = "https://example.com/webhook"
webhook_template = '{"summary":"{summary}"}'
```

`webhook_template` is a JSON string where `{summary}` will be replaced with the generated summary before sending the request.
