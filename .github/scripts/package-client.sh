#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 3 ]]; then
  echo "Usage: package-client.sh CLIENT_BINARY VERSION OUTPUT_DIRECTORY" >&2
  exit 2
fi

binary=$1
version=$2
output=$3
: "${KARABINER_SERVICE_ASSET_URL:?Set the exact versioned public service asset URL used to build the client}"
[[ -f "$binary" && -x "$binary" ]] || { echo "Client binary is missing or not executable: $binary" >&2; exit 1; }

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/karabiner-client-package.XXXXXX")
trap 'rm -rf "$work"' EXIT
app="$work/Karabiner.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$output"
install -m 755 "$binary" "$app/Contents/MacOS/karabiner"
cp "$root/apps/client/src/components/factory_canvas/NOTICE.md" "$app/Contents/Resources/React-Flow-NOTICE.md"
cp "$root/apps/client/assets/kubai/LICENSE" "$app/Contents/Resources/Kubai-LICENSE.txt"

python3 - "$app/Contents/Info.plist" "$version" "$KARABINER_SERVICE_ASSET_URL" <<'PY'
import plistlib, re, sys
from urllib.parse import urlsplit

path, version, asset = sys.argv[1:]
short_version = version.split('-')[0].split('+')[0]
if not re.fullmatch(r'\d+\.\d+\.\d+', short_version):
    raise SystemExit('Client version must be a Cargo semantic version')
url = urlsplit(asset)
if url.scheme != 'https' or not url.netloc or url.username or url.password or url.query or url.fragment:
    raise SystemExit('Service asset URL must be public HTTPS without credentials, query or fragment')
with open(path, 'wb') as output:
    plistlib.dump({
        'CFBundleDevelopmentRegion': 'en',
        'CFBundleDisplayName': 'Karabiner',
        'CFBundleExecutable': 'karabiner',
        'CFBundleIdentifier': 'com.darylcecile.karabiner',
        'CFBundleInfoDictionaryVersion': '6.0',
        'CFBundleName': 'Karabiner',
        'CFBundlePackageType': 'APPL',
        'CFBundleShortVersionString': short_version,
        'CFBundleVersion': short_version,
        'NSHighResolutionCapable': True,
        'KarabinerServiceAssetURL': asset,
    }, output)
PY

plutil -lint "$app/Contents/Info.plist"
ditto -c -k --sequesterRsrc --keepParent "$app" "$work/Karabiner-macos-arm64.zip"
mv "$work/Karabiner-macos-arm64.zip" "$output/Karabiner-macos-arm64.zip"
echo "$output/Karabiner-macos-arm64.zip"
