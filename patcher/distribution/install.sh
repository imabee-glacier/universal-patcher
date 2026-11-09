#!/usr/bin/env bash
# Universal Patcher installer for Linux and macOS.
#   curl -fsSL @ORIGIN@install.sh | bash
# Downloads the engine for this platform, checks its SHA-256 against the published bootstrap
# manifest, and runs its "install" command. The engine verifies the application release with
# its built-in trusted root and installs into your home directory. No root access is needed.
set -euo pipefail

ORIGIN="@ORIGIN@"
BOOTSTRAP="${ORIGIN}app/bootstrap"

fail() {
    echo "Error: $*" >&2
    exit 1
}

if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
    fail "do not run this installer with sudo. Universal Patcher installs into your own home directory. Re-run: curl -fsSL ${ORIGIN}install.sh | bash"
fi

# Plain HTTP is accepted only when the publisher generated this script for a loopback test origin.
case "$ORIGIN" in
    https://*) curl_tls=(--proto '=https' --tlsv1.2) wget_tls=(--https-only) ;;
    *) curl_tls=(--proto '=http') wget_tls=() ;;
esac
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "${curl_tls[@]}" --retry 3 "$1"; }
    fetch_to() { curl -fsSL "${curl_tls[@]}" --retry 3 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q --max-redirect=0 ${wget_tls[@]+"${wget_tls[@]}"} -O - "$1"; }
    fetch_to() { wget -q --max-redirect=0 ${wget_tls[@]+"${wget_tls[@]}"} -O "$2" "$1"; }
else
    fail "curl or wget is required"
fi

case "$(uname -s)" in
    Linux) os="linux" ;;
    Darwin) os="macos" ;;
    MINGW* | MSYS* | CYGWIN*) fail "on Windows, run in PowerShell: irm ${ORIGIN}install.ps1 | iex" ;;
    *) fail "unsupported operating system: $(uname -s)" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch="x64" ;;
    arm64 | aarch64) arch="arm64" ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
esac
if [ "$os" = "macos" ] && [ "$arch" = "x64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = "1" ]; then
    arch="arm64"
fi
platform="${os}-${arch}"
# Only where The Sims 4 runs: Windows and Linux on x64, macOS on Intel and Apple silicon.
case "$platform" in
    linux-x64 | macos-x64 | macos-arm64) ;;
    linux-arm64) fail "The Sims 4 does not run on Linux on ARM, so Universal Patcher is not offered there" ;;
    *) fail "The Sims 4 does not run on ${platform}, so Universal Patcher is not offered there" ;;
esac

version="$(fetch "${BOOTSTRAP}/latest" | tr -d '[:space:]')"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "the download service returned an unexpected version; try again later"

manifest="$(fetch "${BOOTSTRAP}/${version}/manifest.json" | tr -d '\n\r\t ')"
if [[ "$manifest" =~ \"${platform}\":\{[^{}]*\"sha256\":\"([a-f0-9]{64})\" ]]; then
    expected="${BASH_REMATCH[1]}"
else
    fail "Universal Patcher ${version} is not available for ${platform} yet"
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
engine="${workdir}/patcher-engine"
fetch_to "${BOOTSTRAP}/${version}/${platform}/patcher-engine" "$engine" || fail "download failed"

if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$engine" | cut -d' ' -f1)"
else
    actual="$(shasum -a 256 "$engine" | cut -d' ' -f1)"
fi
[ "$actual" = "$expected" ] || fail "checksum verification failed; nothing was installed"

chmod +x "$engine"
echo "Installing Universal Patcher ${version}..."
"$engine" install --origin "$ORIGIN"
