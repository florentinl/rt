#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_DIR="$ROOT_DIR/riot-extension/runtime"
PYTHON_DIR="$RUNTIME_DIR/python"

resolve_python_bin() {
  local python_dir="$1"
  local direct="$python_dir/bin/python"
  if [ -x "$direct" ]; then
    echo "$direct"
    return 0
  fi

  for candidate in "$python_dir"/*/bin/python; do
    if [ -x "$candidate" ]; then
      echo "$candidate"
      return 0
    fi
  done

  return 1
}

mkdir -p "$RUNTIME_DIR"
uv python install 3.14 --install-dir "$PYTHON_DIR"
PYTHON_BIN="$(resolve_python_bin "$PYTHON_DIR")"
uv pip install --python "$PYTHON_BIN" . --break-system-packages

ensure_wrapper() {
  local wrapper_path="$1"
  local content="$2"

  if [ ! -f "$wrapper_path" ]; then
    mkdir -p "$(dirname "$wrapper_path")"
    printf "%s\n" "$content" > "$wrapper_path"
  fi
  chmod 755 "$wrapper_path"
}

ensure_wrapper "$RUNTIME_DIR/bin/rt" '#!/bin/sh
set -e

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
PYTHON_BIN="$SCRIPT_DIR/../python/bin/python"
LIBEXEC_DIR="$SCRIPT_DIR/../libexec"

if [ ! -x "$PYTHON_BIN" ]; then
  for candidate in "$SCRIPT_DIR/../python"/*/bin/python; do
    if [ -x "$candidate" ]; then
      PYTHON_BIN="$candidate"
      break
    fi
  done
fi

if [ ! -x "$PYTHON_BIN" ]; then
  exit 1
fi

export PATH="$LIBEXEC_DIR:$SCRIPT_DIR:$PATH"
exec "$PYTHON_BIN" -m riot "$@"'

ensure_wrapper "$RUNTIME_DIR/libexec/uv" '#!/bin/sh
set -e

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
PYTHON_BIN="$SCRIPT_DIR/../python/bin/python"

if [ ! -x "$PYTHON_BIN" ]; then
  for candidate in "$SCRIPT_DIR/../python"/*/bin/python; do
    if [ -x "$candidate" ]; then
      PYTHON_BIN="$candidate"
      break
    fi
  done
fi

if [ ! -x "$PYTHON_BIN" ]; then
  exit 1
fi

UV_BIN="$(dirname "$PYTHON_BIN")/uv"

if [ ! -x "$UV_BIN" ]; then
  exit 1
fi

if head -c 2 "$UV_BIN" | grep -q "^#!"; then
  exec "$PYTHON_BIN" "$UV_BIN" "$@"
fi

exec "$UV_BIN" "$@"'

for wrapper in "$RUNTIME_DIR/bin/rt" "$RUNTIME_DIR/libexec/uv"; do
  if [ ! -f "$wrapper" ]; then
    exit 1
  fi
done
