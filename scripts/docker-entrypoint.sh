#!/bin/sh
set -eu

# ── Defaults ──────────────────────────────────────────────────────────────────
: "${SYNOS_DISK_IMAGE:="/synos/build/bios/synos-bios.img"}"
: "${SYNOS_QEMU_BIN:=qemu-system-x86_64}"
: "${SYNOS_CLUSTER_NODES:=1}"
: "${SYNOS_GUEST_MEMORY:=512M}"
: "${SYNOS_VNC_BASE:=5900}"
: "${SYNOS_QEMU_ACCEL:=tcg}"
: "${SYNOS_QEMU_EXTRA:=""}"

usage() {
    cat <<EOF
SynOS Docker – run a bootable SynOS image inside QEMU with VNC

Usage: docker run [docker-opts] synos [mode]

Modes:
  single      Single QEMU guest with serial on stdout + VNC (default)
  cluster     Multi-node QEMU cluster (use SYNOS_CLUSTER_NODES to set count)

Environment:
  SYNOS_DISK_IMAGE     Path to SynOS raw image (default: build/bios/synos-bios.img)
  SYNOS_CLUSTER_NODES  Number of cluster guests (default: 1, max: 8)
  SYNOS_GUEST_MEMORY   RAM per guest (default: 512M)
  SYNOS_VNC_BASE       Starting VNC port (default: 5900)
  SYNOS_QEMU_ACCEL     QEMU accelerator: kvm (if /dev/kvm available) or tcg
  SYNOS_QEMU_EXTRA     Extra flags appended to each QEMU invocation
EOF
}

mode="${1:-single}"

# ── Single-node mode ─────────────────────────────────────────────────────────
if [ "$mode" = "single" ]; then
    echo "=== SynOS single node ==="
    echo "  Serial output → stdout"
    echo "  VNC display   → :0 (port ${SYNOS_VNC_BASE})"
    echo "  Image         → ${SYNOS_DISK_IMAGE}"
    echo ""

    exec "$SYNOS_QEMU_BIN" \
        -machine "q35,accel=${SYNOS_QEMU_ACCEL}" \
        -cpu max \
        -smp 2 \
        -m "${SYNOS_GUEST_MEMORY}" \
        -drive "file=${SYNOS_DISK_IMAGE},format=raw,if=ide" \
        -vnc ":0" \
        -serial stdio \
        ${SYNOS_QEMU_EXTRA}
fi

# ── Cluster mode ─────────────────────────────────────────────────────────────
if [ "$mode" = "cluster" ]; then
    if ! command -v pgrep >/dev/null 2>&1; then
        echo "pgrep is required in cluster mode" >&2
        exit 1
    fi

    node_count="${SYNOS_CLUSTER_NODES}"
    case "$node_count" in
        ''|*[!0-9]*)
            echo "SYNOS_CLUSTER_NODES must be a number" >&2
            exit 1
            ;;
    esac

    if [ "$node_count" -lt 2 ] || [ "$node_count" -gt 8 ]; then
        echo "SYNOS_CLUSTER_NODES must be between 2 and 8" >&2
        exit 1
    fi

    run_dir="/tmp/synos-qemu"
    mkdir -p "$run_dir"

    ivshmem_path="$run_dir/ivshmem.raw"
    truncate -s 256M "$ivshmem_path"

    pids=""

    cleanup() {
        trap - EXIT INT TERM
        for pid in $pids; do
            kill "$pid" 2>/dev/null || true
        done
        for pid in $pids; do
            wait "$pid" 2>/dev/null || true
        done
    }
    trap cleanup EXIT INT TERM

    node=1
    while [ "$node" -le "$node_count" ]; do
        cxl_path="$run_dir/cxl-node-${node}.raw"
        serial_log="$run_dir/node-${node}.serial.log"
        vnc_port=$((SYNOS_VNC_BASE + node - 1))
        mac_suffix=$(printf '%02x' "$node")

        if [ ! -f "$cxl_path" ]; then
            truncate -s 256M "$cxl_path"
        fi

        echo "node $node → serial=${serial_log} vnc=:${vnc_port}"

        "$SYNOS_QEMU_BIN" \
            -name "synos-node-${node}" \
            -machine "q35,cxl=on,accel=${SYNOS_QEMU_ACCEL}" \
            -cpu max \
            -smp 2 \
            -m "${SYNOS_GUEST_MEMORY},maxmem=4G,slots=4" \
            -drive "file=${SYNOS_DISK_IMAGE},format=raw,if=ide" \
            -display none \
            -vnc ":${vnc_port}" \
            -serial "file:${serial_log}" \
            -netdev "socket,id=cluster,mcast=230.0.0.1:1234" \
            -device "e1000,netdev=cluster,mac=52:54:00:53:59:${mac_suffix}" \
            -object "memory-backend-file,id=cxlmem${node},share=on,mem-path=${cxl_path},size=256M" \
            -device "pxb-cxl,bus_nr=12,bus=pcie.0,id=cxl${node}" \
            -device "cxl-rp,port=0,bus=cxl${node},id=cxlrp${node},chassis=0,slot=2" \
            -device "cxl-type3,bus=cxlrp${node},volatile-memdev=cxlmem${node},id=cxltype3${node},sn=0x${mac_suffix}" \
            -object "memory-backend-file,id=ivshmem${node},share=on,mem-path=${ivshmem_path},size=256M" \
            -device "ivshmem-plain,memdev=ivshmem${node}" \
            -M "cxl-fmw.0.targets.0=cxl${node},cxl-fmw.0.size=1G" \
            ${SYNOS_QEMU_EXTRA} &

        pid=$!
        pids="$pids $pid"
        node=$((node + 1))
    done

    echo ""
    echo "=== SynOS cluster: ${node_count} nodes ==="
    echo "Connect to individual VNC displays at ports ${SYNOS_VNC_BASE}-$((SYNOS_VNC_BASE + node_count - 1))"
    echo "Ctrl-C stops all nodes."
    echo ""

    for pid in $pids; do
        wait "$pid" || true
    done

    exit 0
fi

# ── Unknown mode ─────────────────────────────────────────────────────────────
echo "Unknown mode: ${mode}" >&2
usage >&2
exit 1