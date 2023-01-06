set -euxo pipefail

cargo build
RUST_BACKTRACE=full RUST_LOG=trace ./target/debug/nimbus --local-storage storage-main --mount-directory mount-main --config config/main.toml

# cargo build --release
# RUST_LOG=error ./target/release/nimbus --local-storage storage --mount-directory mount
