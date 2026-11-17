#!/bin/bash
# Build the app icon from assets/icon/ into assets/icon/build/ (committed, so building wingbar
# needs no Xcode): Assets.car (macOS 26+, from AppIcon.icon in Icon Composer format: titanium black
# in the default and dark styles, plus the tinted style) and AppIcon.icns (older macOS: the designed
# icon, from png/icon-dark-*.png). png/icon-light-* and the SVGs are for docs and the web.
# Run it after changing anything in assets/icon/. Needs Xcode (actool) and iconutil.
set -euo pipefail
cd "$(dirname "$0")/../assets/icon"
rm -rf build && mkdir -p build
xcrun actool AppIcon.icon --compile build --platform macosx --minimum-deployment-target 12.0 \
  --app-icon AppIcon --output-partial-info-plist build/partial.plist >/dev/null
rm -f build/partial.plist build/AppIcon.icns
set_dir=$(mktemp -d)/AppIcon.iconset; mkdir -p "$set_dir"
for s in 16 32 128 256 512; do
  cp "png/icon-dark-$s.png" "$set_dir/icon_${s}x${s}.png"
  cp "png/icon-dark-$((s * 2)).png" "$set_dir/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$set_dir" -o build/AppIcon.icns
rm -rf "$(dirname "$set_dir")"
ls -l build
