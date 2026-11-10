#!/usr/bin/env bash
# Regenerates app/windows/runner/resources/app_icon.ico from app/assets/icon.svg: the title bar,
# taskbar, Start menu and installer icon on Windows. Needs rsvg-convert and python3.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sizes=(16 20 24 32 40 48 64 128 256)
for size in "${sizes[@]}"; do
    rsvg-convert -w "$size" -h "$size" "$here/app/assets/icon.svg" -o "$work/$size.png"
done
python3 - "$here/app/windows/runner/resources/app_icon.ico" "$work" "${sizes[@]}" <<'PY'
import struct, sys
out, work, sizes = sys.argv[1], sys.argv[2], [int(s) for s in sys.argv[3:]]
images = [open(f"{work}/{s}.png", "rb").read() for s in sizes]
header = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries, data = b"", b""
for size, png in zip(sizes, images):
    dim = 0 if size >= 256 else size  # 0 means 256 in the ICO directory
    entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(png), offset + len(data))
    data += png
open(out, "wb").write(header + entries + data)
PY
echo "wrote app_icon.ico (${sizes[*]})"
