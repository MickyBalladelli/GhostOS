#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

cd "$project_root"

echo "=== Building GhostOS Docker image ==="
docker build -t ghostos:latest .
echo ""
echo "Build complete. Image: ghostos:latest"
echo "Run with: docker run --rm -it -p 5900:5900 ghostos single"