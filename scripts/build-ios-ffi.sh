#!/usr/bin/env bash
# Build vericonomy-ffi static libraries for iOS device + simulator and refresh Swift bindings.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SDK="$ROOT/vericonomy-sdk"
OUT="$ROOT/ios/VericonomyWallet/Rust"
BINDINGS="$ROOT/ios/VericonomyWallet/Generated/VericonomyFfi"

TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)

for target in "${TARGETS[@]}"; do
  rustup target add "$target" >/dev/null 2>&1 || true
done

cd "$SDK"
for target in "${TARGETS[@]}"; do
  echo "Building vericonomy-ffi for $target..."
  cargo build -p vericonomy-ffi --release --target "$target"
  mkdir -p "$OUT/$target"
  cp "target/$target/release/libvericonomy_ffi.a" "$OUT/$target/"
done

HOST_LIB="$SDK/target/release/libvericonomy_ffi.dylib"
if [[ ! -f "$HOST_LIB" ]]; then
  cargo build -p vericonomy-ffi --release
fi

mkdir -p "$BINDINGS"
echo "Generating Swift bindings..."
cargo run -p vericonomy-ffi --bin uniffi-bindgen -- generate \
  --library "$HOST_LIB" \
  --language swift \
  --out-dir "$BINDINGS"

echo "Done. Static libs: $OUT/{aarch64-apple-ios,aarch64-apple-ios-sim}/libvericonomy_ffi.a"
