# ETT Summary Project

This repository contains a Rust client that records audio from online meetings and a Rust server that transcribes the audio with `whisper-rs` and keeps an incident summary using an OpenAI-compatible API.

The typical flow is:

1. The client records audio from the configured device and periodically sends WAV files to the summarization server.
2. The server transcribes each file with whisper, updates the running summary and posts it to a configured webhook.
3. If no audio is received for one hour the summary is cleared.

See [`server-rs`](server-rs/) for details about the Rust server and configuration options.

## Quickstart

1. Install the [Rust toolchain](https://www.rust-lang.org/tools/install).
2. Copy `server-rs/config.toml` and adjust the API endpoint, model, database, systems and webhook URL.
3. Run the server:
   ```bash
   cargo run --release --manifest-path server-rs/Cargo.toml
   ```
4. Send a WAV file to the running server (include your `system_key`):
   ```bash
    curl -F file=@audio.wav -F system_key=default http://localhost:8000/upload
   ```
The server posts an updated summary after each upload and clears it after an hour of inactivity.

## Building

Build both binaries with `make`:

```bash
make build
```

The binaries are placed in the `bin/` directory. `build-server` and `build-client` targets are also available.

## Running

Run the server or client directly through `make`:

```bash
make run-server    # start the summarization server
make run-client    # start the audio client using system-key "default"
```

## Configuration

`server-rs/config.toml` contains default settings for the server:

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

Adjust these values or point `CONFIG_FILE` to another path when running the server.

If the file is missing you can generate a template using the server binary:

```bash
cargo run --manifest-path server-rs/Cargo.toml -- gen-config openai > server-rs/config.toml
```

Supported providers are `openai` (default), `openrouter`, `azure`, `ollama` and `local`.
Remember to set `OPENAI_API_KEY` when starting the server.

## Systemd Units

Example service files are provided under [`contrib/systemd`](contrib/systemd):

```bash
sudo cp contrib/systemd/summary-server.service /etc/systemd/system/
sudo cp contrib/systemd/summary-client.service /etc/systemd/system/
```

Enable and start the services with `systemctl enable --now summary-server summary-client`.

## Docker

`server-rs/Dockerfile` builds the server along with the default Whisper model. Build and run the image:

```bash
docker build -t summary-server ./server-rs
docker run -p 8000:8000 -e OPENAI_API_KEY=your-key summary-server
```

