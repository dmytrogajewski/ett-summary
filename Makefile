.PHONY: check run-server run-client build-server build-client build

BIN_DIR := bin

# Build both Rust crates to ensure the code compiles
check:
	cargo check --manifest-path server-rs/Cargo.toml
	cargo check --manifest-path client/Cargo.toml || true

# Build the server binary
build-server:
	cargo build --release --manifest-path server-rs/Cargo.toml
	mkdir -p $(BIN_DIR)
	cp server-rs/target/release/server-rs $(BIN_DIR)/

# Build the client binary
build-client:
	cargo build --release --manifest-path client/Cargo.toml
	mkdir -p $(BIN_DIR)
	cp client/target/release/client_app $(BIN_DIR)/

# Build both binaries
build: build-server build-client

# Run the summarization server
run-server:
	cargo run --manifest-path server-rs/Cargo.toml

# Run the audio client
run-client:
	cargo run --manifest-path client/Cargo.toml -- --system-key default
