#!/bin/bash
set -euo pipefail

APP=${1:?usage: verify-macos-artifacts.sh APP [DMG] [strict]}
DMG=${2:-}
MODE=${3:-unsigned}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
VERSION=$(sed -n 's/^petalLinkVersion=//p' "$ROOT/gradle.properties")
PLIST="$APP/Contents/Info.plist"

test -f "$PLIST"
IDENTIFIER=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$PLIST")
SHORT_VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$PLIST")
MINIMUM=$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$PLIST")

test "$IDENTIFIER" = "io.github.yuanbaobaoo.PetalLink"
test "$SHORT_VERSION" = "$VERSION"
test "$MINIMUM" = "12.0"
test -f "$APP/Contents/Resources/PetalLink.icns"
grep -Eq '^MODULES=".*(^| )java\.sql( |$).*"$' "$APP/Contents/runtime/Contents/Home/release"
ENTITLEMENTS=$(/usr/bin/codesign -d --entitlements :- "$APP" 2>&1)
grep -q 'com.apple.security.cs.disable-library-validation' <<<"$ENTITLEMENTS"
/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP"

if [[ "$MODE" = "strict" ]]; then
  /usr/sbin/spctl --assess --type execute --verbose=2 "$APP"
  test -n "$DMG"
  /usr/bin/xcrun stapler validate "$APP"
  /usr/bin/xcrun stapler validate "$DMG"
  /usr/sbin/spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"
fi

echo "PetalLink macOS artifact verified: version=$VERSION mode=$MODE"
