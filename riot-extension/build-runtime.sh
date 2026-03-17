#!/usr/bin/env bash
set -euo pipefail

cd ..
cargo build --release

cp target/release/rt riot-extension/rt
