#!/usr/bin/env bash
# Build the macOS edition as it is released (ARCHITECTURE.md, "The macOS edition", "Distribution"): the bridge for
# both Mac architectures, the app archived and signed with Developer ID, notarized and stapled, in a disk image that
# is signed, notarized and stapled in turn. The release workflow runs it on GitHub's macOS runner; on the maintainer's
# Mac it runs in a Terminal window, where the login keychain is open (the mac-dev skill).
#
#   apple/release.sh [--unsigned | --not-notarized]
#
# The version is the release line's (ci/version.sh, which build-rust.sh hands to Xcode), the same as the Linux
# packages'. --unsigned builds the same disk image without signing or notarizing it, as CI does on every branch.
# --not-notarized signs it but skips the notary service, to check the signing on the Mac that built it.
#
# Signed, it reads from the environment:
#   TEAM_ID                         the Apple Developer Program team
#   APP_PROFILE, EXTENSION_PROFILE  the names of the app's and the extension's Developer ID provisioning profiles,
#                                   installed, as CI has them. Without them, Xcode signs automatically and makes the
#                                   profiles itself, which needs a Mac whose Xcode is signed in to the team.
#   NOTARY_PROFILE                  a notarytool keychain profile (`xcrun notarytool store-credentials`), or
#   NOTARY_KEY, NOTARY_KEY_ID, NOTARY_ISSUER   an App Store Connect API key (.p8 path) and its IDs
#
# The result: apple/build/release/immich-mount-VERSION.dmg
set -euo pipefail
cd "$(dirname "$0")"

usage() { echo "usage: apple/release.sh [--unsigned | --not-notarized]" >&2; exit 2; }
[ $# -le 1 ] || usage
version=$(../ci/version.sh)
signed=yes
notarized=yes
case "${1:-}" in
  "") ;;
  --unsigned) signed=no; notarized=no ;;
  --not-notarized) notarized=no ;;
  *) usage ;;
esac

out=build/release
archive="$out/ImmichMount.xcarchive"
app_name="Immich Mount.app"
dmg="$out/immich-mount-$version.dmg"
rm -rf "$out"
mkdir -p "$out"

if [ "$signed" = yes ]; then
  : "${TEAM_ID:?TEAM_ID is the Apple Developer Program team}"
  if [ -n "${APP_PROFILE:-}" ]; then
    : "${EXTENSION_PROFILE:?EXTENSION_PROFILE goes with APP_PROFILE}"
    style=manual
    provisioning=()
  else
    style=automatic
    provisioning=(-allowProvisioningUpdates)
  fi
fi
if [ "$notarized" = yes ]; then
  if [ -n "${NOTARY_KEY:-}" ]; then
    notary=(--key "$NOTARY_KEY" --key-id "${NOTARY_KEY_ID:?}" --issuer "${NOTARY_ISSUER:?}")
  else
    notary=(--keychain-profile "${NOTARY_PROFILE:?NOTARY_PROFILE or NOTARY_KEY is needed to notarize}")
  fi
fi
if [ "$signed" = yes ]; then
  # The certificate by its fingerprint, so a Mac that also holds development certificates signs with the right one.
  identity=$(security find-identity -v -p codesigning | awk -v team="($TEAM_ID)" \
    '/"Developer ID Application: / && index($0, team) { print $2; exit }')
  [ -n "$identity" ] || { echo "no Developer ID Application certificate of team $TEAM_ID in the keychain" >&2; exit 1; }
fi

# Send a file to Apple's notary service and wait; on a refusal, print the service's log, which names each problem.
notarize() {
  local result status submission
  result=$(xcrun notarytool submit "$1" "${notary[@]}" --wait --output-format json)
  status=$(plutil -extract status raw -o - - <<<"$result")
  submission=$(plutil -extract id raw -o - - <<<"$result")
  echo "notarization of $(basename "$1"): $status ($submission)"
  if [ "$status" != Accepted ]; then
    xcrun notarytool log "$submission" "${notary[@]}" >&2 || true
    exit 1
  fi
}

./build-rust.sh release
xcodegen --quiet

# The archive is universal: the Release configuration builds both architectures, and the bridge has both.
settings=()
if [ "$signed" = no ]; then
  settings+=(CODE_SIGNING_ALLOWED=NO)
elif [ "$style" = manual ]; then
  settings+=(DEVELOPMENT_TEAM="$TEAM_ID" CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY="Developer ID Application"
    APP_PROFILE="$APP_PROFILE" EXTENSION_PROFILE="$EXTENSION_PROFILE" OTHER_CODE_SIGN_FLAGS=--timestamp)
else
  # Signed for development here; the export below signs it again with Developer ID.
  settings+=(DEVELOPMENT_TEAM="$TEAM_ID")
fi
# ${array[@]+...}: macOS's bash 3.2 calls an empty array unbound under `set -u`.
xcodebuild archive -quiet -project ImmichMount.xcodeproj -scheme ImmichMount -configuration Release \
  -destination 'generic/platform=macOS' -archivePath "$archive" ${provisioning[@]+"${provisioning[@]}"} "${settings[@]}"

if [ "$signed" = yes ]; then
  # Export signs the app and the extension again for distribution, each with its own Developer ID profile: the
  # Keychain group they share is an entitlement only a profile grants (TN3125).
  if [ "$style" = manual ]; then
    profiles="<key>provisioningProfiles</key><dict>
    <key>dev.hviid.ImmichMount</key><string>$APP_PROFILE</string>
    <key>dev.hviid.ImmichMount.FileProvider</key><string>$EXTENSION_PROFILE</string></dict>"
  else
    profiles=""
  fi
  cat >"$out/export.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>developer-id</string>
  <key>signingStyle</key><string>$style</string>
  <key>teamID</key><string>$TEAM_ID</string>
  <key>signingCertificate</key><string>Developer ID Application</string>
  $profiles
</dict>
</plist>
EOF
  xcodebuild -exportArchive -quiet -archivePath "$archive" -exportPath "$out/export" \
    -exportOptionsPlist "$out/export.plist" ${provisioning[@]+"${provisioning[@]}"}
  app="$out/export/$app_name"
  codesign --verify --deep --strict "$app"
  if [ "$notarized" = yes ]; then
    # The app is notarized and stapled first, so a copy taken out of the disk image passes Gatekeeper offline too.
    ditto -c -k --keepParent "$app" "$out/app.zip"
    notarize "$out/app.zip"
    xcrun stapler staple -q "$app"
  fi
else
  app="$archive/Products/Applications/$app_name"
fi

# The disk image: the app beside a link to Applications, to drag it onto.
stage="$out/disk"
mkdir -p "$stage"
ditto "$app" "$stage/$app_name"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname "Immich Mount" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg"

if [ "$signed" = yes ]; then
  codesign --sign "$identity" --timestamp "$dmg"
fi
if [ "$notarized" = yes ]; then
  notarize "$dmg"
  xcrun stapler staple -q "$dmg"
  # What Gatekeeper decides on a downloaded copy: both must say "Notarized Developer ID".
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
  spctl --assess --type execute --verbose=2 "$app"
fi
if [ "$notarized" = yes ]; then state="signed and notarized"; elif [ "$signed" = yes ]; then state="signed, not notarized"
else state=unsigned; fi
echo "built $dmg ($state)"
