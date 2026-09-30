#!/bin/bash
# Package one built binary: build/pack.sh TARGET BIN OUT_DIR VERSION
# Produces OUT_DIR/srun-VERSION-TARGET.{tar.xz|zip} and a .sha256 next to it.
set -euo pipefail
target=$1 bin=$2 out=$3 version=$4
name="srun-$version-$target"
mkdir -p "$out"
out=$(cd "$out" && pwd)
export COPYFILE_DISABLE=1
sha() { if command -v shasum >/dev/null; then shasum -a 256 "$@"; else sha256sum "$@"; fi; }
cd "$(dirname "$bin")"
if [[ "$target" == *windows* ]]; then
    rm -f "$out/$name.zip"
    if command -v zip >/dev/null; then
        zip -q "$out/$name.zip" "$(basename "$bin")"
    else
        7z a "$out/$name.zip" "$(basename "$bin")" >/dev/null
    fi
    pkg="$name.zip"
else
    tar -cJf "$out/$name.tar.xz" "$(basename "$bin")"
    pkg="$name.tar.xz"
fi
(cd "$out" && sha "$pkg" > "$name.sha256")
size=$(stat -f %z "$(basename "$bin")" 2>/dev/null || stat -c %s "$(basename "$bin")")
printf '%-40s %8d bytes\n' "$target" "$size" | tee -a "$out/SIZES.txt"
