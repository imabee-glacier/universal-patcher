#!/usr/bin/env bash
# End-to-end check of the one-line installer on Linux, exactly as a user runs it:
# release build with a baked origin and test root -> package -> publish -> serve through the
# production front image over HTTPS -> "curl -fsSL <origin>install.sh | bash" in an empty HOME,
# then the front checks and a browser pass over the website.
# Everything is written under runtime/e2e-install; the signing key is a fresh test key and the
# TLS certificate comes from a local test CA, trusted through SSL_CERT_FILE for this run only.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
work="$here/runtime/e2e-install"
port="${PORT:-18443}"
http_port="${HTTP_PORT:-18081}"
origin="https://localhost:${port}/"

rm -rf "$work"
mkdir -p "$work/repo" "$work/caddy"

echo "== front image, local test CA"
docker build -q -f "$here/deploy/front/Dockerfile" -t patcher-front:e2e "$here" >/dev/null
docker rm -f up-e2e >/dev/null 2>&1 || true
# Runs as the invoking user so the test CA lands in a directory that user owns.
docker run -d --name up-e2e --read-only --cap-drop ALL --cap-add NET_BIND_SERVICE \
    --user "$(id -u):$(id -g)" --security-opt no-new-privileges --tmpfs "/config:uid=$(id -u)" \
    -e PATCHER_DOMAIN=localhost -e PATCHER_TLS_MODE=internal \
    -p "127.0.0.1:${port}:443/tcp" -p "127.0.0.1:${port}:443/udp" -p "127.0.0.1:${http_port}:80" \
    -v "$work/caddy:/data" -v "$work/repo:/srv/repository:ro" patcher-front:e2e >/dev/null
trap 'docker rm -f up-e2e >/dev/null 2>&1 || true' EXIT
ca="$work/caddy/caddy/pki/authorities/local/root.crt"
for _ in $(seq 50); do [ -s "$ca" ] && break; sleep 0.2; done
if [ ! -s "$ca" ]; then
    docker logs up-e2e >&2
    echo "front did not create its test CA" >&2
    exit 1
fi
cat /etc/ssl/certs/ca-certificates.crt "$ca" > "$work/trust.pem"
export SSL_CERT_FILE="$work/trust.pem"
cargo build --quiet --manifest-path "$here/Cargo.toml" --bin patcher
"$here/target/debug/patcher" fixture "$work/keys" >/dev/null
root="$work/keys/keys/root.json"
key="$work/keys/keys/fixture-key.pem"

echo "== release build with baked origin and root"
(cd "$here/app" && PATCHER_DIST_ORIGIN="$origin" PATCHER_DIST_ROOT="$root" \
    "$here/.tools/flutter/bin/flutter" build linux --release >/dev/null)
bundle="$here/app/build/linux/x64/release/bundle"

echo "== package and publish 1.0.0"
"$here/target/debug/patcher" package-app "$bundle" --version 1.0.0 --os linux --arch x64 \
    --staging "$work/staging"
printf 'First public test release.\n\nInstalls per user with one command.\n' > "$work/notes.txt"
"$here/target/debug/patcher" publish-app "$work/staging" --root "$root" --key "$key" \
    --repository "$work/repo" --origin "$origin" --offer linux-x64 --notes "$work/notes.txt"

echo "== health status through the front"
"$here/target/debug/patcher" health --origin "$origin" --output "$work/repo/site/data/health.json"

echo "== curl -fsSL ${origin}install.sh | bash"
mkdir -p "$work/home"
env -i HOME="$work/home" PATH="/usr/local/bin:/usr/bin:/bin" SSL_CERT_FILE="$SSL_CERT_FILE" \
    bash -c "curl -fsSL ${origin}install.sh | bash"

echo "== installed layout"
data="$work/home/.local/share/universal-patcher"
test -x "$data/bin/universal-patcher"
test -x "$data/versions/1.0.0/patcher"
test -L "$work/home/.local/bin/universal-patcher"
grep -q "Exec=" "$work/home/.local/share/applications/universal-patcher.desktop"
grep -q "Icon=.*icon.svg" "$work/home/.local/share/applications/universal-patcher.desktop"
test -f "$data/share/icon.svg"
echo "ok"

echo "== tampered engine download is refused"
engine="$work/repo/app/bootstrap/1.0.0/linux-x64/patcher-engine"
cp "$engine" "$work/engine.keep"
printf 'x' >> "$engine"
if env -i HOME="$work/home2" PATH="/usr/local/bin:/usr/bin:/bin" SSL_CERT_FILE="$SSL_CERT_FILE" \
    bash -c "curl -fsSL ${origin}install.sh | bash" 2>"$work/tamper.err"; then
    echo "tampered engine was accepted" >&2
    exit 1
fi
grep -q 'checksum verification failed' "$work/tamper.err"
test ! -e "$work/home2/.local/share/universal-patcher/bin"
cp "$work/engine.keep" "$engine"
echo "ok"

echo "== front checks"
# Needs curl with HTTP/3 and OpenSSL 3.5 or later; without them locally, run in a pinned image.
if curl --version | grep -q HTTP3 && command -v jq >/dev/null; then
    HTTP_PORT="$http_port" "$here/tools/front-check.sh" "$origin" "$ca"
else
    docker run --rm --network host -e HTTP_PORT="$http_port" \
        -v "$here/tools/front-check.sh:/front-check.sh:ro" -v "$ca:/ca.pem:ro" \
        archlinux:base@sha256:b1c2fe6e717bfd5673af8fc217e1e1c3ce0dcbb1d53c3414c8425e7ce79fc6d9 \
        bash -ec 'pacman -Sy --noconfirm --needed jq b3sum diffutils >/dev/null && /front-check.sh "$0" /ca.pem' "$origin"
fi

echo "== website in a browser"
"$here/tools/site-browser-check.sh" "$origin" "$ca"
