#!/usr/bin/env bash
# Build release binaries for each triple listed in scripts/release-targets.txt
# (or a path you pass as the first argument).
#
# Output: Cargo writes to target/<triple>/release/<crate-name>
# Optional: set COPY_TO_DIST=1 to also copy binaries into dist/<version>/ for uploads.
#
# Host note: every triple must be buildable from your current machine (linker +
# sysroot). macOS↔macOS cross is fine with Xcode; Linux/Windows triples from
# macOS need extra tooling or run this script on CI / the target OS instead.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGETS_FILE="${1:-scripts/release-targets.txt}"
if [[ ! -f "$TARGETS_FILE" ]]; then
  echo "error: targets file not found: $TARGETS_FILE" >&2
  exit 1
fi

CRATE_NAME="$(grep -m1 '^name = ' Cargo.toml | sed -E 's/^name = "([^"]+)".*/\1/')"
VERSION="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/^version = "([^"]+)".*/\1/')"

while IFS= read -r line || [[ -n "$line" ]]; do
  [[ "$line" =~ ^[[:space:]]*# ]] && continue
  [[ -z "${line//[[:space:]]/}" ]] && continue
  triple="$(echo "$line" | tr -d '[:space:]')"

  echo "==> rustup target add $triple (ok if already installed)"
  rustup target add "$triple" 2>/dev/null || true

  echo "==> cargo build --release --target $triple"
  cargo build --release --target "$triple"

  bin="target/$triple/release/$CRATE_NAME"
  echo "    binary: $bin"
  ls -lh "$bin"

  if [[ "${COPY_TO_DIST:-0}" == "1" ]]; then
    out_dir="dist/$VERSION/$triple"
    mkdir -p "$out_dir"
    cp -f "$bin" "$out_dir/"
    echo "    copied to: $out_dir/$CRATE_NAME"
  fi
done < "$TARGETS_FILE"
