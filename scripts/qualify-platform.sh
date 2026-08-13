#!/bin/sh
set -eu

profile=${1:-}
evidence_dir=${2:-}

if [ -z "$profile" ] || [ -z "$evidence_dir" ]; then
    echo "usage: $0 PROFILE EVIDENCE_DIRECTORY" >&2
    echo "profiles: hardware-boot, legacy-two-node, qemu-cluster, enterprise-cxl, enterprise-gpu" >&2
    exit 1
fi

if ! command -v rg >/dev/null 2>&1; then
    echo "ripgrep is required" >&2
    exit 1
fi

root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

failures=0

require_file() {
    path=$1
    if [ ! -s "$path" ]; then
        echo "MISSING $path"
        failures=$((failures + 1))
    fi
}

require_pattern() {
    pattern=$1
    path=$2
    if [ ! -s "$path" ] || ! rg -q "$pattern" "$path"; then
        echo "MISSING pattern '$pattern' in $path"
        failures=$((failures + 1))
    fi
}

require_cluster_evidence() {
    require_pattern "SynOS kernel bootstrap" "$evidence_dir/node-1.serial.log"
    require_pattern "SynOS kernel bootstrap" "$evidence_dir/node-2.serial.log"
    require_pattern "heartbeat.*node" "$evidence_dir/fabric.log"
    require_pattern "page.*(fetch|migration)" "$evidence_dir/fabric.log"
    require_pattern "failover.*complete" "$evidence_dir/failover.log"
}

case "$profile" in
    hardware-boot)
        python3 "$root_dir/scripts/validate-hardware-boot-evidence.py" "$evidence_dir"
        exit $?
        ;;
    legacy-two-node)
        require_cluster_evidence
        require_pattern "(E1000|RTL8169)" "$evidence_dir/inventory.txt"
        require_pattern "bare-metal" "$evidence_dir/inventory.txt"
        ;;
    qemu-cluster)
        require_cluster_evidence
        require_pattern "cxl-type3" "$evidence_dir/node-1.command.txt"
        require_pattern "ivshmem-plain" "$evidence_dir/node-1.command.txt"
        require_pattern "netdev socket" "$evidence_dir/node-1.command.txt"
        ;;
    enterprise-cxl)
        require_cluster_evidence
        require_pattern "CXL.*Type-3" "$evidence_dir/inventory.txt"
        require_pattern "CXL.*switch" "$evidence_dir/inventory.txt"
        require_pattern "HDM.*committed" "$evidence_dir/fabric.log"
        require_pattern "mirrored.*redirect" "$evidence_dir/failover.log"
        ;;
    enterprise-gpu)
        require_cluster_evidence
        require_pattern "(PCIe|NVLink)" "$evidence_dir/inventory.txt"
        require_pattern "VRAM.*pool" "$evidence_dir/fabric.log"
        require_pattern "peer.*memory" "$evidence_dir/fabric.log"
        ;;
    *)
        echo "unknown profile: $profile" >&2
        exit 1
        ;;
esac

if [ "$failures" -ne 0 ]; then
    echo "qualification failed: $failures evidence checks missing" >&2
    exit 1
fi

echo "qualification passed: $profile"
