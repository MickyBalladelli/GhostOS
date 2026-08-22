#!/usr/bin/env bash
set -Eeuo pipefail

root_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root_dir"

python3 "$root_dir/scripts/bootstrap.py"

run_id=${GHOSTOS_TEST_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)-$$}
evidence_dir=${GHOSTOS_EVIDENCE_DIR:-$root_dir/build/test-evidence/$run_id}
mkdir -p "$evidence_dir"

run_optional() {
    local tier=$1
    shift
    local output_dir="$evidence_dir/$tier"
    mkdir -p "$output_dir"
    local command="$*"
    if [[ -z "${GHOSTOS_FULL_VALIDATION:-}" && "$tier" != docs && "$tier" != reproducibility ]]; then
        printf '{"schema":1,"state":"skipped","tier":"%s","reason":"full validation is opt-in","prerequisite":"GHOSTOS_FULL_VALIDATION=1"}\n' "$tier" > "$output_dir/result.json"
        echo "== $tier: skipped; set GHOSTOS_FULL_VALIDATION=1"
        return 0
    fi
    case "$tier" in
        qemu)
            if ! command -v "${GHOSTOS_QEMU_BIN:-qemu-system-x86_64}" >/dev/null 2>&1 \
                || ! command -v "${GHOSTOS_QEMU_IMG_BIN:-qemu-img}" >/dev/null 2>&1 \
                || [[ ! -f "${GHOSTOS_QEMU_IMAGE:-$root_dir/build/bios/ghostos-bios.img}" ]] \
                || [[ ! -f "${GHOSTOS_QEMU_UEFI_IMAGE:-${GHOSTOS_QEMU_IMAGE:-$root_dir/build/bios/ghostos-bios.img}}" ]] \
                || [[ ! -f "${GHOSTOS_QEMU_UEFI_FIRMWARE:-}" ]]; then
                printf '{"schema":1,"state":"skipped","tier":"%s","reason":"missing QEMU, qemu-img, BIOS image, UEFI image, or UEFI firmware","prerequisite":"qemu-system-x86_64, qemu-img, GHOSTOS_QEMU_IMAGE, GHOSTOS_QEMU_UEFI_IMAGE, and GHOSTOS_QEMU_UEFI_FIRMWARE"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing QEMU, qemu-img, image, or firmware"
                return 0
            fi
            ;;
        fuzz)
            if ! command -v cargo-fuzz >/dev/null 2>&1; then
                printf '{"schema":1,"state":"skipped","tier":"%s","reason":"missing prerequisite","prerequisite":"cargo-fuzz"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing cargo-fuzz"
                return 0
            fi
            ;;
        coverage)
            if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
                printf '{"schema":1,"state":"skipped","tier":"%s","reason":"missing prerequisite","prerequisite":"cargo-llvm-cov"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing cargo-llvm-cov"
                return 0
            fi
            ;;
        mutation)
            if ! command -v cargo-mutants >/dev/null 2>&1; then
                printf '{"schema":1,"state":"skipped","tier":"%s","reason":"missing prerequisite","prerequisite":"cargo-mutants"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing cargo-mutants"
                return 0
            fi
            ;;
        cluster)
            if [[ "$(uname -s)" != Linux ]]; then
                printf '{"schema":1,"state":"skipped","tier":"%s","reason":"cluster QEMU requires Linux","prerequisite":"Linux with QEMU CXL and ivshmem devices"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; cluster QEMU requires Linux"
                return 0
            fi
            if ! command -v "${GHOSTOS_QEMU_BIN:-qemu-system-x86_64}" >/dev/null 2>&1 || [[ ! -f "${GHOSTOS_DISK_IMAGE:-$root_dir/build/bios/ghostos-bios.img}" ]]; then
                printf '{"schema":1,"state":"skipped","tier":"%s","reason":"missing QEMU or cluster image","prerequisite":"qemu-system-x86_64 and GHOSTOS_DISK_IMAGE"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing QEMU or cluster image"
                return 0
            fi
            ;;
        hardware-accelerated)
            if ! command -v "${GHOSTOS_QEMU_BIN:-qemu-system-x86_64}" >/dev/null 2>&1 || [[ ! -f "${GHOSTOS_QEMU_IMAGE:-$root_dir/build/bios/ghostos-bios.img}" ]]; then
                printf '{"schema":1,"state":"skipped","tier":"%s","reason":"missing QEMU or boot image","prerequisite":"qemu-system-x86_64 and GHOSTOS_QEMU_IMAGE"}\n' "$tier" > "$output_dir/result.json"
                echo "== $tier: skipped; missing QEMU or boot image"
                return 0
            fi
            case "$(uname -s)" in
                Linux) [[ -e /dev/kvm ]] || { printf '{"schema":1,"state":"skipped","tier":"%s","reason":"KVM is unavailable","prerequisite":"/dev/kvm"}\n' "$tier" > "$output_dir/result.json"; echo "== $tier: skipped; KVM is unavailable"; return 0; } ;;
                Darwin) : ;;
                *) printf '{"schema":1,"state":"skipped","tier":"%s","reason":"no supported hardware accelerator","prerequisite":"KVM or HVF"}\n' "$tier" > "$output_dir/result.json"; echo "== $tier: skipped; no supported hardware accelerator"; return 0 ;;
            esac
            ;;
    esac
    if ! command -v "$1" >/dev/null 2>&1; then
        printf '{"schema":1,"state":"skipped","tier":"%s","reason":"missing prerequisite","prerequisite":"%s"}\n' "$tier" "$1" > "$output_dir/result.json"
        echo "== $tier: skipped; missing $1"
        return 0
    fi
    echo "== $tier: $command"
    set +e
    "$@" > >(tee "$output_dir/stdout.log") 2> >(tee "$output_dir/stderr.log" >&2)
    local status=$?
    set -e
    if [[ $status -eq 0 ]]; then
        printf '{"schema":1,"state":"passed","tier":"%s","reason":"command completed successfully"}\n' "$tier" > "$output_dir/result.json"
    else
        printf '{"schema":1,"state":"failed","tier":"%s","exit_code":%d,"reason":"command exited with status %d"}\n' "$tier" "$status" "$status" > "$output_dir/result.json"
        return "$status"
    fi
}

run_optional_with_vm_evidence() {
    local runner_tier=$1
    local inventory_tier=$2
    local firmware=$3
    local cpu_count=$4
    local bios_image=$5
    local uefi_image=$6
    shift 6
    local command="$*"
    local started_at
    local ended_at
    local status
    local -a image_args=()
    started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    if run_optional "$runner_tier" "$@"; then
        status=0
    else
        status=$?
    fi
    ended_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    if [[ -f "$bios_image" ]]; then
        image_args+=(--image "$bios_image")
    fi
    if [[ -f "$uefi_image" && "$uefi_image" != "$bios_image" ]]; then
        image_args+=(--image "$uefi_image")
    fi
    python3 "$root_dir/scripts/record-vm-evidence.py" \
        --evidence-dir "$evidence_dir" \
        --tier "$inventory_tier" \
        --command "$command" \
        --firmware "$firmware" \
        --cpu-count "$cpu_count" \
        "${image_args[@]}" \
        --result-file "$evidence_dir/$runner_tier/result.json" \
        --started-at "$started_at" \
        --ended-at "$ended_at"
    [[ $status -eq 0 ]] || return "$status"
}

if ! GHOSTOS_EVIDENCE_DIR="$evidence_dir" "$root_dir/scripts/test-all.sh"; then
    echo "deterministic validation failed; evidence: $evidence_dir" >&2
    exit 1
fi

python3 "$root_dir/scripts/validate-test-status.py" \
    --evidence-dir "$evidence_dir" \
    --output "$evidence_dir/test-status.json"
python3 "$root_dir/scripts/test_validate_panic_policy.py"
python3 "$root_dir/scripts/validate-panic-policy.py"
run_optional docs "$root_dir/scripts/validate-documentation.py"
qemu_image=${GHOSTOS_QEMU_IMAGE:-$root_dir/build/bios/ghostos-bios.img}
qemu_uefi_image=${GHOSTOS_QEMU_UEFI_IMAGE:-$qemu_image}
run_optional_with_vm_evidence qemu qemu "bios,uefi" 2 "$qemu_image" "$qemu_uefi_image" env GHOSTOS_RUN_QEMU_TESTS=1 GHOSTOS_QEMU_LOG_DIR="$evidence_dir/qemu" cargo test -p ghostos-vm --test qemu_matrix_59_11 --test test_environments --test qemu_login_e2e -- --ignored
hardware_accel=${GHOSTOS_QEMU_ACCEL:-kvm}
if [[ -z "${GHOSTOS_QEMU_ACCEL:-}" && "$(uname -s)" == Darwin ]]; then
    hardware_accel=hvf
fi
run_optional hardware-accelerated env GHOSTOS_RUN_QEMU_TESTS=1 GHOSTOS_QEMU_LOG_DIR="$evidence_dir/hardware-accelerated" GHOSTOS_QEMU_ACCEL="$hardware_accel" cargo test -p ghostos-vm --test qemu_matrix_59_11 -- --ignored
run_optional cluster env GHOSTOS_RUN_QEMU_TESTS=1 "$root_dir/scripts/qemu-cluster-validation.sh"
run_optional_with_vm_evidence fuzz fuzz "not-applicable" 1 "" "" "$root_dir/scripts/fuzz-smoke.sh"
run_optional coverage "$root_dir/scripts/coverage.sh"
run_optional mutation "$root_dir/scripts/mutation.sh"
run_optional_with_vm_evidence soak soak "not-applicable" 1 "" "" env GHOSTOS_SOAK_REPORT="$evidence_dir/soak/report.json" "$root_dir/scripts/soak.sh"
run_optional reproducibility "$root_dir/scripts/check-reproducible-image.sh"
run_optional dashboard "$root_dir/scripts/test-dashboard.py" "$evidence_dir"
run_optional coverage-contract python3 "$root_dir/scripts/validate-test-coverage.py" "$evidence_dir"

python3 "$root_dir/scripts/validate-evidence-separation.py" "$evidence_dir" \
    --require-tier qemu \
    --require-tier hardware-accelerated \
    --require-tier fuzz \
    --require-tier soak

python3 "$root_dir/scripts/validate-vm-evidence.py" "$evidence_dir" \
    --require-tier fast-unit \
    --require-tier vm-integration \
    --require-tier cli \
    --require-tier qemu \
    --require-tier fuzz \
    --require-tier soak

python3 "$root_dir/scripts/evidence-manifest.py" \
    --evidence-dir "$evidence_dir" \
    --write
python3 "$root_dir/scripts/evidence-manifest.py" \
    --evidence-dir "$evidence_dir" \
    --check
"$root_dir/scripts/release-gate.sh" "$evidence_dir" "${GHOSTOS_SLO_REPORT:-}" "${GHOSTOS_RELEASE_CLAIMS:-}" "${GHOSTOS_UPGRADE_COMPATIBILITY:-}"
echo "full validation passed; evidence: $evidence_dir"
