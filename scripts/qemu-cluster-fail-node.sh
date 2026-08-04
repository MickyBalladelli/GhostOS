#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
node=${1:-}

case "$node" in
    ''|*[!0-9]*)
        echo "usage: $0 NODE_NUMBER" >&2
        exit 1
        ;;
esac

run_dir=${SYNOS_CLUSTER_RUN_DIR:-"$project_root/build/qemu-cluster"}
pid_file="$run_dir/node-$node.pid"
if [ ! -f "$pid_file" ]; then
    echo "node $node pid file not found" >&2
    exit 1
fi

pid=$(sed -n '1p' "$pid_file")
case "$pid" in
    ''|*[!0-9]*)
        echo "invalid pid file for node $node" >&2
        exit 1
        ;;
esac

command=$(ps -p "$pid" -o command= 2>/dev/null || true)
if ! printf '%s\n' "$command" | rg -q "qemu-system|synos-node-$node"; then
    echo "pid $pid is not SynOS QEMU node $node" >&2
    exit 1
fi

kill -TERM "$pid"
echo "failure injected into node $node"
