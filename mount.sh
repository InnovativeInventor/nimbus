set -euxo pipefail

cargo build
sudo RUST_LOG=info ./target/debug/nimbus --local-storage storage --mount-directory mount
