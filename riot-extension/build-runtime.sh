#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME="$HERE/runtime"

PY_VERSION="3.14.4"
PBS_TAG="20260414"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)              TRIPLE="aarch64-apple-darwin" ;;
  Darwin-x86_64)             TRIPLE="x86_64-apple-darwin" ;;
  Linux-aarch64|Linux-arm64) TRIPLE="aarch64-unknown-linux-gnu" ;;
  Linux-x86_64)              TRIPLE="x86_64-unknown-linux-gnu" ;;
  *) echo "unsupported: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

URL="https://github.com/astral-sh/python-build-standalone/releases/download/${PBS_TAG}/cpython-${PY_VERSION}+${PBS_TAG}-${TRIPLE}-install_only_stripped.tar.gz"

rm -rf "$RUNTIME"
mkdir -p "$RUNTIME"
curl -fsSL "$URL" | tar -xz -C "$RUNTIME" --strip-components=1

"$RUNTIME/bin/python3" -m pip install --no-compile "$HERE/.."

# Pip's generated rt script hardcodes an absolute shebang. Replace it with the
# same relocatable sh/python trick that runtime/bin/pip uses.
cat > "$RUNTIME/bin/rt" <<'EOF'
#!/bin/sh
'''exec' "$(dirname -- "$(realpath -- "$0")")/python3.14" "$0" "$@"
' '''
import sys
from rt import main
if __name__ == '__main__':
    sys.argv[0] = sys.argv[0].removesuffix('.exe')
    sys.exit(main())
EOF
chmod +x "$RUNTIME/bin/rt"

# Pyc files generated during pip install embed the builder's absolute paths
# in co_filename; ship without them and let python recompile on first use.
find "$RUNTIME/lib/python3.14" -type d -name __pycache__ -exec rm -rf {} +
