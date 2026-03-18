#!/usr/bin/env bash
set -euo pipefail

cd ..
cargo build --release --no-default-features --features provider-rustpython

cp target/release/rt riot-extension/rt
