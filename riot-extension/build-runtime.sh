#!/usr/bin/env bash
set -euo pipefail

cd ..
cargo build --release --target-dir .

cp target/release/rt riot-extension/rt
