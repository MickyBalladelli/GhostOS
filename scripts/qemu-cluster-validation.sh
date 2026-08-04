#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
run_dir=${SYNOS_CLUSTER_VALIDATION_DIR:-$root_dir/build/qemu-cluster-validation-$$}
command_log="$run_dir/validation.commands.log"
cluster_pid=""

if [[ "${SYNOS_RUN_QEMU_TESTS:-}" != 1 ]]; then
    echo "cluster QEMU validation skipped; set SYNOS_RUN_QEMU_TESTS=1"
    exit 0
fi

mkdir -p "$run_dir"
export SYNOS_CLUSTER_RUN_DIR="$run_dir"

cleanup() {
    if [[ -n "$cluster_pid" ]]; then
        kill "$cluster_pid" 2>/dev/null || true
        wait "$cluster_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

SYNOS_CLUSTER_NODES=2 \
  "$root_dir/scripts/qemu-cluster.sh" > "$run_dir/launcher.log" 2>&1 &
cluster_pid=$!

deadline=$((SECONDS + 30))
for node in 1 2; do
    serial_log="$run_dir/node-$node.serial.log"
    while [[ ! -f "$serial_log" ]] || ! rg -q "SynOS kernel bootstrap" "$serial_log"; do
        if (( SECONDS >= deadline )); then
            echo "node $node did not boot; see $serial_log" >&2
            exit 1
        fi
        sleep 1
    done
done

send_command() {
    local node=$1
    local command=$2
    printf 'node=%s command=%s\n' "$node" "$command" | tee -a "$command_log"
    python3 "$root_dir/scripts/qemu-send-command.py" "$run_dir/node-$node.qmp" "$command"
}

send_command 1 "CREATE CLUSTER validation /QUORUM=1"
send_command 1 "SHOW CLUSTER"
send_command 1 "LIST CLUSTERS"
send_command 1 "SHOW CLUSTER/HEALTH"
send_command 1 "SHOW CLUSTER/RESOURCES"
send_command 1 "SHOW CLUSTER/CONFIG"
send_command 1 "JOIN CLUSTER validation /INVITATION=expired /ENDPOINT=node-2"
send_command 1 "LEAVE CLUSTER validation /DRAIN /CONFIRM"
send_command 1 "REMOVE CLUSTER validation /DRAIN /CONFIRM"
send_command 1 "INVITE CLUSTER peer /EXPIRATION=100 /SCOPE=compute"
send_command 1 "SHOW CLUSTER/FEDERATION"
send_command 1 "FENCE NODE node-2 /CONFIRM"
send_command 1 "RECOVER NODE node-2 /CONFIRM"
send_command 1 "REJOIN NODE node-2 /INVITATION=expired /ENDPOINT=node-2"

"$root_dir/scripts/qemu-cluster-fail-node.sh" 2
sleep 2

for node in 1 2; do
    if rg -q "KERNEL PANIC|guest panic" "$run_dir/node-$node.serial.log"; then
        echo "node $node panicked; see $run_dir/node-$node.serial.log" >&2
        exit 1
    fi
done

echo "cluster QEMU validation captured; evidence: $run_dir"
