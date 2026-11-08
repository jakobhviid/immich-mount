#!/usr/bin/env bash
# Immich Mount Dev: the Debug build, signed with the developer's own team, installed as /Applications/Immich Mount
# Dev.app (Finder's location runs only from Applications) and started. It has an identity of its own (the bundle
# identifier dev.hviid.ImmichMount.Dev, its App Group, Keychain group, settings and Finder location) and a blue icon,
# so it lives beside the installed release: sign it in once, to the lab server or your own, and every later build
# keeps that sign-in. It never updates itself. Its extension is switched on once, in System Settings (General, Login
# Items & Extensions), as the release's is.
#
#   apple/dev-app.sh        build, install and start (about two minutes; the bridge's build is most of it)
#
# Signing needs the login keychain, which an SSH session cannot reach: from SSH, run apple/dev-app.command in the
# developer's session (`open -a Terminal apple/dev-app.command`, the mac-dev skill), and read apple/build/dev-app.log.
set -euo pipefail
cd "$(dirname "$0")"
export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
built="build/DevData/Build/Products/Debug/Immich Mount Dev.app"
installed="/Applications/Immich Mount Dev.app"

./build-rust.sh debug
xcodegen -q
xcodebuild -project ImmichMount.xcodeproj -scheme ImmichMount -configuration Debug -destination platform=macOS \
  -derivedDataPath build/DevData -allowProvisioningUpdates build -quiet

# Only the Dev app and its extension stop, matched by their path (the extension shares the release's name); the
# release keeps running.
pkill -f "Immich Mount Dev.app/Contents/" || true
ditto "$built" "$installed.new"
rm -rf "$installed"
mv "$installed.new" "$installed"
touch "$installed"  # macOS caches an app's icon by its path (the mac-dev skill)
open "$installed"
echo "installed and started $installed"
