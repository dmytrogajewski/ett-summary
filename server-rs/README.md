# Rust Summarization Server

This Rust server receives WAV audio files, transcribes them with `whisper-rs` and updates an incident summary using an OpenAI-compatible API. If no audio is received for one hour, the summary is cleared.

## Requirements
- Rust toolchain
- `OPENAI_API_KEY` environment variable set to a valid OpenAI API key
- `config.toml` file describing API, webhook, database and whisper model settings

## Running

```bash
cargo run --release --manifest-path server-rs/Cargo.toml
```

The server listens on `http://localhost:8000` by default.

Send a WAV file to `/upload` using multipart form data. Include the `system_key` for the target system:

```bash
curl -F file=@audio.wav -F system_key=default http://localhost:8000/upload
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
database_url = "postgres://user:password@localhost/summary"

[[systems]]
key = "default"
initial_prompt = "..."
update_prompt = "..."
```

If you don't have a configuration file, run:

```bash
cargo run --manifest-path server-rs/Cargo.toml -- gen-config openai > server-rs/config.toml
```

Change `openai` to one of `openrouter`, `azure`, `ollama` or `local` to use another provider. Remember to supply `OPENAI_API_KEY` when starting the server.

`webhook_template` is a JSON string where `{summary}` will be replaced with the generated summary before sending the request.

## Pulling the Whisper model

`whisper-rs` expects a `.bin` model file. The default configuration points to
`models/ggml-base.en.bin`. Download the model once before running the server:

```bash
mkdir -p server-rs/models
curl -L -o server-rs/models/ggml-base.en.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
```

## Docker

`server-rs/Dockerfile` builds the server binary and fetches the whisper model at
image build time. Build and run the image:

```bash
docker build -t summary-server ./server-rs
docker run -p 8000:8000 -e OPENAI_API_KEY=your-key summary-server
```

The container exposes port `8000` and uses `config.toml` from the image. Supply a
valid `OPENAI_API_KEY` at runtime.
