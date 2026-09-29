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

LINUX="x86_64-unknown-linux-musl i686-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf arm-unknown-linux-musleabi arm-unknown-linux-musleabihf"
MIPS32="mipsel-unknown-linux-musl mips-unknown-linux-musl"
MIPS64="mips64el-unknown-linux-muslabi64 mips64-unknown-linux-muslabi64"
APPLE="aarch64-apple-darwin x86_64-apple-darwin"
WINDOWS="x86_64-pc-windows-gnu i686-pc-windows-gnu"

# Optional whitelist of triples.
want() { [[ -z "${TARGETS:-}" ]] || [[ " $TARGETS " == *" $1 "* ]]; }

size_of() { stat -f %z "$1" 2>/dev/null || stat -c %s "$1"; }
sha() { if command -v shasum >/dev/null; then shasum -a 256 "$@"; else sha256sum "$@"; fi; }

pack() {
    local t=$1 bin=$2 name="srun-$VERSION-$t"
    if [[ "$t" == *windows* ]]; then
        rm -f "$OUT/$name.zip"
        (cd "$(dirname "$bin")" && zip -q "$OUT/$name.zip" "$(basename "$bin")")
        (cd "$OUT" && sha "$name.zip" > "$name.sha256")
    else
        tar -C "$(dirname "$bin")" -cJf "$OUT/$name.tar.xz" "$(basename "$bin")"
        (cd "$OUT" && sha "$name.tar.xz" > "$name.sha256")
    fi
    printf '%-40s %8d bytes\n' "$t" "$(size_of "$bin")" | tee -a "$OUT/SIZES.txt"
}

# Each cross image gets its own target dir: host-side build scripts compiled
# in one image (newer glibc) do not run in another (older glibc).
tdir() { echo "target/cross-$1"; }

build_linux() {
    for t in $LINUX; do
        want "$t" || continue
        echo "### $t"
        CARGO_TARGET_DIR="$(tdir "$t")" cross build --release --target "$t"
        pack "$t" "$(tdir "$t")/$t/release/srun"
    done
}

build_mips() {
    for t in $MIPS32; do
        want "$t" || continue
        echo "### $t"
        CARGO_TARGET_DIR="$(tdir "$t")" RUSTFLAGS="-C target-feature=+crt-static -C link-self-contained=no" \
            cross +nightly build --release --target "$t" -Z build-std=std,panic_abort
        pack "$t" "$(tdir "$t")/$t/release/srun"
    done
    for t in $MIPS64; do
        want "$t" || continue
        echo "### $t"
        CARGO_TARGET_DIR="$(tdir "$t")" RUSTFLAGS="-C target-feature=+crt-static,+soft-float -C link-self-contained=no" \
            cross +nightly build --release --target "$t" -Z build-std=std,panic_abort
        pack "$t" "$(tdir "$t")/$t/release/srun"
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
    for t in $WINDOWS; do
        want "$t" || continue
        echo "### $t"
        CARGO_TARGET_DIR="$(tdir "$t")" RUSTFLAGS="-C target-feature=+crt-static" cross build --release --target "$t"
        pack "$t" "$(tdir "$t")/$t/release/srun.exe"
    done
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
