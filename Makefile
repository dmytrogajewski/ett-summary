.PHONY: check run-server

# Build both Rust crates to ensure the code compiles
check:
	cargo check --manifest-path server-rs/Cargo.toml
	cargo check --manifest-path client/Cargo.toml || true

# Run the summarization server
run-server:
	cargo run --manifest-path server-rs/Cargo.toml
