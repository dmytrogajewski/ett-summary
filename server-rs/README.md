# Rust Summarization Server

This Rust server receives WAV audio files, transcribes them with `whisper-rs` and updates an incident summary using an OpenAI-compatible API. If no audio is received for one hour, the summary is cleared.

## Requirements
- Rust toolchain
- `OPENAI_API_KEY` environment variable set to a valid OpenAI API key
- `config.toml` file describing API, webhook and whisper model settings

## Running

```bash
cargo run --release --manifest-path server-rs/Cargo.toml
```

The server listens on `http://localhost:8000` by default.

Send a WAV file to `/upload` using multipart form data:

```bash
curl -F file=@audio.wav http://localhost:8000/upload
```

Each uploaded audio file updates the summary which is sent to the configured webhook. If no files are uploaded for an hour, the summary is cleared.

## Configuration

Create a `config.toml` file in the `server-rs` directory (or set the `CONFIG_FILE` environment variable to another path). Example:

```toml
openai_api_url = "https://api.openai.com/v1/chat/completions"
openai_model = "gpt-3.5-turbo"
webhook_url = "https://example.com/webhook"
webhook_template = '{"summary":"{summary}"}'
whisper_model_path = "models/ggml-base.en.bin"
```

`webhook_template` is a JSON string where `{summary}` will be replaced with the generated summary before sending the request.
