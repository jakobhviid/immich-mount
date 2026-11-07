#!/usr/bin/env bash
# Build the bridge (crates/immich-mount-apple) for the Xcode project: a static library for both Mac architectures,
# its Swift bindings, and an xcframework that wraps them (ARCHITECTURE.md, "The macOS edition"), and the app's
# version for Xcode. Runs on a Mac with both Rust targets added; xcodegen reads the results from apple/build/.
#
#   apple/build-rust.sh [debug|release]      (release when not given)
set -euo pipefail
cd "$(dirname "$0")/.."

profile=${1:-release}
# Never an empty array: macOS's bash 3.2 calls one unbound under `set -u`.
case "$profile" in
  release) cargo_profile=(--release) ;;
  debug) cargo_profile=(--profile dev) ;;
  *) echo "usage: apple/build-rust.sh [debug|release]" >&2; exit 2 ;;
esac
out=apple/build
# The oldest macOS the app runs on (project.yml's deploymentTarget): rustc and the C compiler behind ring and oslog
# both read it. Without it the C parts are built for the SDK's own version, newer than the app's.
export MACOSX_DEPLOYMENT_TARGET=14.0

# Where Homebrew's `rust` formula is installed beside Homebrew's rustup, its cargo comes first on the PATH and has only
# the host's standard library; rustup's own cargo has both targets. Put it first, for this script only.
if command -v brew >/dev/null && [ -x "$(brew --prefix rustup 2>/dev/null)/bin/cargo" ]; then
  PATH="$(brew --prefix rustup)/bin:$PATH"
fi
targets=(aarch64-apple-darwin x86_64-apple-darwin)

libraries=()
for target in "${targets[@]}"; do
  cargo build -p immich-mount-apple --lib "${cargo_profile[@]}" --target "$target"
  libraries+=("target/$target/$profile/libimmich_mount_apple.a")
done

rm -rf "$out/ImmichMountCore.xcframework" "$out/Generated" "$out/headers" "$out/universal"
mkdir -p "$out/Generated" "$out/headers" "$out/universal"
lipo -create "${libraries[@]}" -output "$out/universal/libimmich_mount_apple.a"

# The bindings come from the library's own metadata, so they always match what was built.
cargo run -q -p immich-mount-apple --features bindgen --bin uniffi-bindgen -- generate \
  --library "target/${targets[0]}/$profile/libimmich_mount_apple.dylib" --language swift --out-dir "$out/Generated"
mv "$out/Generated/immich_mount_appleFFI.h" "$out/headers/"
mv "$out/Generated/immich_mount_appleFFI.modulemap" "$out/headers/module.modulemap"

xcodebuild -create-xcframework -library "$out/universal/libimmich_mount_apple.a" -headers "$out/headers" \
  -output "$out/ImmichMountCore.xcframework" >/dev/null

# The release line's version (ci/version.sh), the same as the Linux packages', and a build number that grows with
# every commit; Config/Base.xcconfig reads them.
printf 'MARKETING_VERSION = %s\nCURRENT_PROJECT_VERSION = %s\n' "$(ci/version.sh)" "$(git rev-list --count HEAD)" \
  > "$out/Version.xcconfig"
echo "built $out/ImmichMountCore.xcframework and $out/Generated/immich_mount_apple.swift ($profile)"
