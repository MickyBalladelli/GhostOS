#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

run_id=${SYNOS_TEST_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)-$$}
evidence_dir=${SYNOS_EVIDENCE_DIR:-$root_dir/build/test-evidence/$run_id}
mkdir -p "$evidence_dir"

run_optional() {
    local tier=$1
    shift
    local output_dir="$evidence_dir/$tier"
    mkdir -p "$output_dir"
    local command="$*"
    if [[ -z "${SYNOS_FULL_VALIDATION:-}" && "$tier" != docs && "$tier" != reproducibility ]]; then
        printf '{"state":"skipped","tier":"%s","reason":"full validation is opt-in","prerequisite":"SYNOS_FULL_VALIDATION=1"}\n' "$tier" > "$output_dir/result.json"
        echo "== $tier: skipped; set SYNOS_FULL_VALIDATION=1"
        return 0
    fi
    case "$tier" in
        qemu)
            if ! command -v "${SYNOS_QEMU_BIN:-qemu-system-x86_64}" >/dev/null 2>&1 || [[ ! -f "${SYNOS_QEMU_IMAGE:-$root_dir/build/bios/synos-bios.img}" ]]; then
                printf '{"state":"skipped","tier":"%s","reason":"missing QEMU or boot image","prerequisite":"qemu-system-x86_64 and SYNOS_QEMU_IMAGE"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing QEMU or boot image"
                return 0
            fi
            ;;
        fuzz)
            if ! command -v cargo-fuzz >/dev/null 2>&1; then
                printf '{"state":"skipped","tier":"%s","reason":"missing prerequisite","prerequisite":"cargo-fuzz"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing cargo-fuzz"
                return 0
            fi
            ;;
        coverage)
            if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
                printf '{"state":"skipped","tier":"%s","reason":"missing prerequisite","prerequisite":"cargo-llvm-cov"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing cargo-llvm-cov"
                return 0
            fi
            ;;
        mutation)
            if ! command -v cargo-mutants >/dev/null 2>&1; then
                printf '{"state":"skipped","tier":"%s","reason":"missing prerequisite","prerequisite":"cargo-mutants"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing cargo-mutants"
                return 0
            fi
            ;;
        cluster)
            if [[ "$(uname -s)" != Linux ]]; then
                printf '{"state":"skipped","tier":"%s","reason":"cluster QEMU requires Linux","prerequisite":"Linux with QEMU CXL and ivshmem devices"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; cluster QEMU requires Linux"
                return 0
            fi
            if ! command -v "${SYNOS_QEMU_BIN:-qemu-system-x86_64}" >/dev/null 2>&1 || [[ ! -f "${SYNOS_DISK_IMAGE:-$root_dir/build/bios/synos-bios.img}" ]]; then
                printf '{"state":"skipped","tier":"%s","reason":"missing QEMU or cluster image","prerequisite":"qemu-system-x86_64 and SYNOS_DISK_IMAGE"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing QEMU or cluster image"
                return 0
            fi
            ;;
    esac
    if ! command -v "$1" >/dev/null 2>&1; then
        printf '{"state":"skipped","tier":"%s","reason":"missing prerequisite","prerequisite":"%s"}\n' "$tier" "$1" > "$output_dir/result.json"
        echo "== $tier: skipped; missing $1"
        return 0
    fi
    echo "== $tier: $command"
    set +e
    "$@" > >(tee "$output_dir/stdout.log") 2> >(tee "$output_dir/stderr.log" >&2)
    local status=$?
    set -e
    if [[ $status -eq 0 ]]; then
        printf '{"state":"pass","tier":"%s"}\n' "$tier" > "$output_dir/result.json"
    else
        printf '{"state":"fail","tier":"%s","exit_code":%d}\n' "$tier" "$status" > "$output_dir/result.json"
        return "$status"
    fi
}

if ! SYNOS_EVIDENCE_DIR="$evidence_dir" "$root_dir/scripts/test-all.sh"; then
    echo "deterministic validation failed; evidence: $evidence_dir" >&2
    exit 1
fi

run_optional docs "$root_dir/scripts/validate-test-inventory.py"
run_optional qemu env SYNOS_RUN_QEMU_TESTS=1 SYNOS_QEMU_LOG_DIR="$evidence_dir/qemu" cargo test -p synos-vm --test qemu_matrix_59_11 -- --ignored
run_optional cluster env SYNOS_RUN_QEMU_TESTS=1 "$root_dir/scripts/qemu-cluster-validation.sh"
run_optional fuzz "$root_dir/scripts/fuzz-smoke.sh"
run_optional coverage "$root_dir/scripts/coverage.sh"
run_optional mutation "$root_dir/scripts/mutation.sh"
run_optional reproducibility "$root_dir/scripts/check-reproducible-image.sh"
run_optional dashboard "$root_dir/scripts/test-dashboard.py" "$evidence_dir"
run_optional coverage-contract python3 "$root_dir/scripts/validate-test-coverage.py" "$evidence_dir"

"$root_dir/scripts/release-gate.sh" "$evidence_dir"
echo "full validation passed; evidence: $evidence_dir"
