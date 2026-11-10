#!/usr/bin/env bash
# A test participant of immich-crosslink, on the Mac (the mac-dev skill): an app that registers the scheme, announces
# draft 0.2 with the kinds given, and writes each link it receives, with its bundle id, to
# crosslink-received.log in the user's temporary folder (`getconf DARWIN_USER_TEMP_DIR`). A sandboxed app reads
# another app's announcement only in /Applications (not ~/Applications or a build folder), so Immich Mount finds it
# there only: copy it in for a check, and remove it after with --remove.
#
#   lab/mac/crosslink-app.sh <folder> <bundle id> <name> <kind>...   make <folder>/<name>.app and register it
#   lab/mac/crosslink-app.sh --remove <app>                          unregister and delete it
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
if [ "${1:-}" = "--remove" ]; then
    "$lsregister" -u "$2"
    rm -rf "$2"
    exit 0
fi
folder=$1; bundle_id=$2; name=$3; shift 3
app="$folder/$name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
kinds=""
for kind in "$@"; do kinds="$kinds<string>$kind</string>"; done
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>$bundle_id</string>
<key>CFBundleName</key><string>$name</string>
<key>CFBundleExecutable</key><string>test</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSUIElement</key><true/>
<key>CFBundleURLTypes</key><array><dict><key>CFBundleURLName</key><string>immich-crosslink</string>
<key>CFBundleURLSchemes</key><array><string>immich-crosslink</string></array></dict></array>
<key>ImmichCrosslink</key><dict><key>Version</key><string>0.2</string><key>Shows</key><array>$kinds</array></dict>
</dict></plist>
PLIST
DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer} xcrun swiftc -O -o "$app/Contents/MacOS/test" "$here/crosslink-app.swift"
codesign -s - --force "$app" >/dev/null
"$lsregister" -f "$app"
echo "made $app"
