#!/usr/bin/env bash
set -euo pipefail

uv python install 3.14 --install-dir=dist --no-config --no-bin
mv dist/cpython*/ python
rm -rf dist
uv pip install --no-config --python ./python .. --break-system-packages

case $(uname -s) in
    Linux*)     machine=linux;;
    Darwin*)    machine=macos;;
    *)          exit
esac

here=$(pwd)
if [[ $machine == "macos" ]]; then
    grep -rlI "$here" python | xargs sed -i '' "s|$here|/Users/florentin.labelle/go/src/github.com/DataDog/rt/riot-extension|g"
else
    grep -rlI "$here" python | xargs sed -i "s|$here|/Users/florentin.labelle/go/src/github.com/DataDog/rt/riot-extension|g"
fi

# Remove Python cache files and directories
find python -type d -name "__pycache__" -exec rm -rf {} + 2>/dev/null || true
find python -type f -name "*.pyc" -delete 2>/dev/null || true
find python -type f -name "*.pyo" -delete 2>/dev/null || true

# Remove terminfo directory (contains case-insensitive duplicates not supported by VSIX)
rm -rf python/share/terminfo
