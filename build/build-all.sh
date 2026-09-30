#!/bin/bash
# Build release packages for every supported target into build/release/.
#   ./build/build-all.sh [linux] [mips] [apple] [windows]   (default: all)
#   TARGETS="aarch64-unknown-linux-musl x86_64-pc-windows-gnu" ./build/build-all.sh
#     builds only those triples (each cross image is ~2 GB to pull).
# Needs only cross + Docker (linux, mips, windows-gnu) and a nightly with
# rust-src (mips). Nothing else is installed on the host. Windows MSVC and
# arm64 Windows are built natively by the release workflow on a Windows runner.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
VERSION=$(grep -E '^version' Cargo.toml | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
OUT="$ROOT/build/release"
mkdir -p "$OUT"
export RUSTUP_NO_SELF_UPDATE=1 COPYFILE_DISABLE=1
if [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]]; then
    export DOCKER_DEFAULT_PLATFORM=linux/amd64
fi

# Optional whitelist of triples.
want() { [[ -z "${TARGETS:-}" ]] || [[ " $TARGETS " == *" $1 "* ]]; }

LINUX="x86_64-unknown-linux-musl i686-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf arm-unknown-linux-musleabi arm-unknown-linux-musleabihf"
MIPS32="mipsel-unknown-linux-musl mips-unknown-linux-musl"
MIPS64="mips64el-unknown-linux-muslabi64 mips64-unknown-linux-muslabi64"
APPLE="aarch64-apple-darwin x86_64-apple-darwin"
WINDOWS="x86_64-pc-windows-gnu i686-pc-windows-gnu"

pack() { "$ROOT/build/pack.sh" "$1" "$2" "$OUT" "$VERSION"; }

# Each cross image gets its own target dir: host-side build scripts compiled
# in one image (newer glibc) do not run in another (older glibc).
# build_cross TARGET [RUSTFLAGS] [TOOLCHAIN] [EXTRA CARGO ARGS...]
# The toolchain (+nightly) must precede `build` and -Z flags must follow it,
# or cross cannot find the subcommand and silently runs the host cargo.
build_cross() {
    local t=$1 flags=${2:-} toolchain=${3:-}
    shift; shift || true; shift || true
    want "$t" || return 0
    echo "### $t"
    CARGO_TARGET_DIR="target/cross-$t" RUSTFLAGS="$flags" \
        cross ${toolchain:+"$toolchain"} build "$@" --release --target "$t"
    local bin="target/cross-$t/$t/release/srun"
    [[ "$t" == *windows* ]] && bin="$bin.exe"
    pack "$t" "$bin"
}

build_linux() {
    for t in $LINUX; do build_cross "$t"; done
}

build_mips() {
    for t in $MIPS32; do
        build_cross "$t" "-C target-feature=+crt-static -C link-self-contained=no" +nightly -Z build-std=std,panic_abort
    done
    for t in $MIPS64; do
        build_cross "$t" "-C target-feature=+crt-static,+soft-float -C link-self-contained=no" +nightly -Z build-std=std,panic_abort
    done
}

build_apple() {
    for t in $APPLE; do
        want "$t" || continue
        echo "### $t"
        rustup target add "$t" >/dev/null 2>&1 || true
        cargo build --release --target "$t"
        pack "$t" "target/$t/release/srun"
    done
}

build_windows() {
    for t in $WINDOWS; do build_cross "$t" "-C target-feature=+crt-static"; done
}

groups=("$@")
[[ ${#groups[@]} -eq 0 ]] && groups=(linux mips apple windows)
for g in "${groups[@]}"; do
    case "$g" in
        linux) build_linux ;;
        mips) build_mips ;;
        apple) build_apple ;;
        windows) build_windows ;;
        *) echo "unknown group $g" >&2; exit 2 ;;
    esac
done
echo "### packages in $OUT"
ls -l "$OUT"
