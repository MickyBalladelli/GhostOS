#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
run_dir=${GHOSTOS_CLUSTER_RUN_DIR:-"$project_root/build/qemu-cluster"}
image=${GHOSTOS_DISK_IMAGE:-"$project_root/build/bios/ghostos-bios.img"}
qemu_bin=${GHOSTOS_QEMU_BIN:-qemu-system-x86_64}
node_count=${GHOSTOS_CLUSTER_NODES:-2}
guest_memory=${GHOSTOS_GUEST_MEMORY:-1G}
cxl_memory=${GHOSTOS_CXL_MEMORY:-256M}
shared_memory=${GHOSTOS_SHARED_MEMORY:-256M}
cluster_bus=${GHOSTOS_CLUSTER_BUS:-230.0.0.1:1234}

case "$node_count" in
    ''|*[!0-9]*)
        echo "GHOSTOS_CLUSTER_NODES must be a number" >&2
        exit 1
        ;;
esac

if [ "$node_count" -lt 2 ] || [ "$node_count" -gt 8 ]; then
    echo "GHOSTOS_CLUSTER_NODES must be between 2 and 8" >&2
    exit 1
fi

if ! command -v "$qemu_bin" >/dev/null 2>&1; then
    echo "QEMU executable not found: $qemu_bin" >&2
    exit 1
fi

if ! command -v rg >/dev/null 2>&1; then
    echo "ripgrep is required" >&2
    exit 1
fi

if [ "$(uname -s)" != Linux ]; then
    echo "The shared ivshmem cluster sandbox requires a Linux host" >&2
    exit 1
fi

if [ ! -f "$image" ]; then
    echo "GhostOS image not found: $image" >&2
    echo "Build it first with ./scripts/build-bios-image.sh" >&2
    exit 1
fi

device_list=$("$qemu_bin" -device help)
for required_device in cxl-type3 cxl-rp pxb-cxl ivshmem-plain e1000; do
    if ! printf '%s\n' "$device_list" | rg -q "$required_device"; then
        echo "QEMU lacks required device: $required_device" >&2
        exit 1
    fi
done

if ! "$qemu_bin" -machine help | rg -q 'q35'; then
    echo "QEMU lacks the q35 machine" >&2
    exit 1
fi

if [ -n "${GHOSTOS_QEMU_ACCEL:-}" ]; then
    accelerator=$GHOSTOS_QEMU_ACCEL
elif [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
    accelerator=kvm
else
    accelerator=tcg
fi

if [ "$accelerator" = kvm ]; then
    cpu=host
else
    cpu=max
fi

mkdir -p "$run_dir"
"$qemu_bin" --version > "$run_dir/qemu-version.txt"

ivshmem_path="$run_dir/ivshmem.raw"
if [ ! -f "$ivshmem_path" ]; then
    truncate -s "$shared_memory" "$ivshmem_path"
fi

pids=

shutdown_cluster() {
    trap - EXIT INT TERM
    for pid in $pids; do
        kill "$pid" 2>/dev/null || true
    done
    for pid in $pids; do
        wait "$pid" 2>/dev/null || true
    done
}

trap shutdown_cluster EXIT INT TERM

node=1
while [ "$node" -le "$node_count" ]; do
    cxl_path="$run_dir/cxl-node-$node.raw"
    serial_log="$run_dir/node-$node.serial.log"
    pid_file="$run_dir/node-$node.pid"
    command_log="$run_dir/node-$node.command.txt"
    qmp_socket="$run_dir/node-$node.qmp"
    mac_suffix=$(printf '%02x' "$node")

    if [ ! -f "$cxl_path" ]; then
        truncate -s "$cxl_memory" "$cxl_path"
    fi

    {
        printf '%s\n' "$qemu_bin"
        printf '%s\n' "-machine q35,cxl=on,accel=$accelerator"
        printf '%s\n' "-netdev socket,id=cluster,mcast=$cluster_bus"
        printf '%s\n' "-device e1000,netdev=cluster,mac=52:54:00:53:59:$mac_suffix"
        printf '%s\n' "-qmp unix:$qmp_socket,server=on,wait=off"
        printf '%s\n' "-device cxl-type3,volatile-memdev=cxlmem$node"
        printf '%s\n' "-device ivshmem-plain,memdev=ivshmem$node"
    } > "$command_log"

    "$qemu_bin" \
        -name "ghostos-node-$node" \
        -machine "q35,cxl=on,accel=$accelerator" \
        -cpu "$cpu" \
        -smp 2 \
        -m "$guest_memory,maxmem=4G,slots=4" \
        -drive "file=$image,format=raw,if=ide,readonly=on" \
        -display none \
        -monitor none \
        -qmp "unix:$qmp_socket,server=on,wait=off" \
        -serial "file:$serial_log" \
        -netdev "socket,id=cluster,mcast=$cluster_bus" \
        -device "e1000,netdev=cluster,mac=52:54:00:53:59:$mac_suffix" \
        -object "memory-backend-file,id=cxlmem$node,share=on,mem-path=$cxl_path,size=$cxl_memory" \
        -device "pxb-cxl,bus_nr=12,bus=pcie.0,id=cxl$node" \
        -device "cxl-rp,port=0,bus=cxl$node,id=cxlrp$node,chassis=0,slot=2" \
        -device "cxl-type3,bus=cxlrp$node,volatile-memdev=cxlmem$node,id=cxltype3$node,sn=0x$mac_suffix" \
        -object "memory-backend-file,id=ivshmem$node,share=on,mem-path=$ivshmem_path,size=$shared_memory" \
        -device "ivshmem-plain,memdev=ivshmem$node" \
        -M "cxl-fmw.0.targets.0=cxl$node,cxl-fmw.0.size=1G" &

    pid=$!
    pids="$pids $pid"
    printf '%s\n' "$pid" > "$pid_file"
    echo "node $node pid=$pid serial=$serial_log"
    node=$((node + 1))
done

echo "cluster running; Ctrl-C stops every node"

status=0
for pid in $pids; do
    if ! wait "$pid"; then
        status=1
    fi
done
exit "$status"
