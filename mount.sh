set -euxo pipefail

cargo build
sudo RUST_LOG=debug ./target/debug/nimbus --local-storage storage --mount-directory mount
# sudo ./target/release/nimbus --local-storage storage --mount-directory mount
