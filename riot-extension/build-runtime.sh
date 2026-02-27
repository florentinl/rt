#!/usr/bin/env bash
set -euo pipefail

cd ..
cargo build --profile final

cp target/final/rt riot-extension/rt
