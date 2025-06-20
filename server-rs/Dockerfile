# Build stage
FROM rust:1.74 as builder
WORKDIR /usr/src/app
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y ca-certificates wget && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/server-rs /usr/local/bin/server-rs
COPY config.toml /app/config.toml
# Download default Whisper model
RUN mkdir -p models && \
    wget -O models/ggml-base.en.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin

ENV CONFIG_FILE=/app/config.toml
EXPOSE 8000
CMD ["/usr/local/bin/server-rs"]
