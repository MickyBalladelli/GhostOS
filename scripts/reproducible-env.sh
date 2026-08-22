#!/bin/sh

if [ -z "${project_root:-}" ]; then
    echo "reproducible-env.sh must be sourced after project_root is set" >&2
    exit 1
fi

revision=$(git -C "$project_root" rev-parse HEAD 2>/dev/null || printf 'unknown')
if [ -z "${SOURCE_DATE_EPOCH+x}" ]; then
    SOURCE_DATE_EPOCH=$(git -C "$project_root" show -s --format=%ct "$revision" 2>/dev/null || printf '0')
fi

case "$SOURCE_DATE_EPOCH" in
    ''|*[!0-9]*)
        echo "SOURCE_DATE_EPOCH must be a non-negative integer" >&2
        exit 1
        ;;
esac

export SOURCE_DATE_EPOCH
export TZ=UTC
export LC_ALL=C
export LANG=C
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS=1
export CARGO_TERM_COLOR=never
export ZERO_AR_DATE=1

append_rustflag() {
    flag=$1
    case " ${RUSTFLAGS:-} " in
        *" $flag "*)
            return
            ;;
    esac
    if [ -n "${RUSTFLAGS:-}" ]; then
        RUSTFLAGS="$RUSTFLAGS $flag"
    else
        RUSTFLAGS=$flag
    fi
}

target_dir=${CARGO_TARGET_DIR:-$project_root/target}
case "$target_dir" in
    /*) ;;
    *) target_dir="$project_root/$target_dir" ;;
esac
append_rustflag "--remap-path-prefix=$target_dir=/ghostos-target"
append_rustflag "--remap-path-prefix=$project_root=/ghostos-source"
export RUSTFLAGS
