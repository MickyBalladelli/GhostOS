#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

cd "$project_root"

echo "=== Building SynOS Docker image ==="
docker build -t synos:latest .
echo ""
echo "Build complete. Image: synos:latest"
echo "Run with: docker run --rm -it -p 5900:5900 synos single"