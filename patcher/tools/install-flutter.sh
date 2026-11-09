#!/usr/bin/env bash
# Installs the Flutter SDK pinned in tools/flutter-release.json into .tools/flutter, after
# checking the archive's SHA-256. Does nothing when that version is already installed.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
release="$here/tools/flutter-release.json"
field_of() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }
field() { field_of "$release" "$1"; }
version="$(field version)"
target="$here/.tools/flutter"
stamp="$target/bin/cache/flutter.version.json"
if [ -x "$target/bin/flutter" ] && [ "$(field_of "$stamp" frameworkVersion 2>/dev/null)" = "$version" ]; then
    echo "Flutter $version already installed"
    exit 0
fi
mkdir -p "$here/.tools"
archive="$here/.tools/flutter.tar.xz"
curl -fsSL --proto '=https' --tlsv1.2 -o "$archive" "$(field base_url)/$(field archive)"
echo "$(field sha256)  $archive" | sha256sum -c --quiet
rm -rf "$target"
tar -xJf "$archive" -C "$here/.tools"
rm "$archive"
git config --global --add safe.directory "$target"
"$target/bin/flutter" --version
