#!/usr/bin/env bash
# Each check is a quoted condition evaluated later, so its variables expand at check time.
# shellcheck disable=SC2016,SC2034
# Checks a running public front from outside: protocols, TLS versions, security headers,
# caching, Range resume against a real release object, and refusals.
#   tools/front-check.sh https://downloads.example.org/ [ca-bundle.pem]
# Exits non-zero on the first failed check. Needs curl with HTTP/3, openssl and jq.
set -euo pipefail

origin="${1:?usage: front-check.sh <origin ending in /> [ca-bundle]}"
case "$origin" in https://*/) ;; *) echo "origin must be https:// and end in /" >&2; exit 2 ;; esac
hostport="${origin#https://}"
hostport="${hostport%%/*}"
host="${hostport%%:*}"
port=443
[ "$hostport" != "$host" ] && port="${hostport#*:}"

curl_args=(-sS --max-time 20 --proto '=https')
openssl_args=()
if [ -n "${2:-}" ]; then
    curl_args+=(--cacert "$2")
    openssl_args+=(-CAfile "$2")
fi
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failures=0
pass() { printf 'ok    %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; failures=$((failures + 1)); }
check() { if eval "$2"; then pass "$1"; else fail "$1"; fi; }
fetch() { curl "${curl_args[@]}" "$@"; }
status() { fetch -o /dev/null -w '%{http_code}' "$@"; }
header() { grep -i "^$1:" "$2" | head -1 | cut -d: -f2- | tr -d '\r' | sed 's/^ *//'; }

# Protocols
check "HTTP/2 over TLS" '[ "$(fetch --http2 -o /dev/null -w "%{http_version}" "${origin}healthz")" = 2 ]'
check "HTTP/3 over QUIC" '[ "$(fetch --http3-only -o /dev/null -w "%{http_version}" "${origin}healthz")" = 3 ]'
fetch -D "$work/home" -o "$work/index.html" "$origin"
check "Alt-Svc advertises h3" 'header Alt-Svc "$work/home" | grep -q "h3=\":"'

tls_ok() { openssl s_client -connect "$host:$port" -servername "$host" "${openssl_args[@]}" -verify_return_error "$@" </dev/null >"$work/tls" 2>&1; }
check "TLS 1.3 accepted" 'tls_ok -tls1_3'
check "TLS 1.2 accepted (Windows 10 PowerShell)" 'tls_ok -tls1_2'
check "TLS 1.1 refused" '! tls_ok -tls1_1 -cipher DEFAULT@SECLEVEL=0'
check "TLS 1.2 without forward secrecy refused" '! tls_ok -tls1_2 -cipher AES128-GCM-SHA256'
tls_ok -tls1_3 -groups X25519MLKEM768:X25519 || true
check "post-quantum key exchange (X25519MLKEM768)" 'grep -q "X25519MLKEM768" "$work/tls"'

# Plain HTTP only redirects.
if [ "${HTTP_PORT:-80}" != none ]; then
    loc="$(curl -sS --max-time 20 -o /dev/null -w '%{http_code} %{redirect_url}' "http://$host:${HTTP_PORT:-80}/install.sh")"
    check "HTTP redirects to HTTPS" '[[ "$loc" == 30[18]\ https://* ]]'
fi

# Website headers
check "HSTS" 'header Strict-Transport-Security "$work/home" | grep -q "max-age=63072000"'
csp="$(header Content-Security-Policy "$work/home")"
check "CSP: scripts from this origin only" '[[ "$csp" == *"script-src '"'"'self'"'"'"* && "$csp" != *unsafe* ]]'
check "CSP: Trusted Types required" '[[ "$csp" == *"require-trusted-types-for '"'"'script'"'"'"* ]]'
check "CSP: no framing" '[[ "$csp" == *"frame-ancestors '"'"'none'"'"'"* ]]'
check "nosniff" '[ "$(header X-Content-Type-Options "$work/home")" = nosniff ]'
check "no Server header" '! grep -qi "^Server:" "$work/home"'
check "site is compressed" '[ "$(fetch -H "Accept-Encoding: zstd, gzip" -D - -o /dev/null "${origin}styles.css" | header Content-Encoding /dev/stdin)" != "" ]'
for data in catalog health; do
    check "data/$data.json is schema 1 JSON" 'fetch "${origin}data/$data.json" | jq -e ".schema == 1" >/dev/null'
done

# Release repository, found through the signed metadata the patcher reads.
fetch -D "$work/ts.h" -o "$work/ts.json" "${origin}app/metadata/timestamp.json"
check "timestamp.json is revalidated" '[ "$(header Cache-Control "$work/ts.h")" = no-cache ]'
snapshot="$(jq -r '.signed.meta["snapshot.json"].version' "$work/ts.json")"
targets="$(fetch "${origin}app/metadata/$snapshot.snapshot.json" | jq -r '.signed.meta["targets.json"].version')"
fetch -D "$work/tg.h" -o "$work/targets.json" "${origin}app/metadata/$targets.targets.json"
check "versioned metadata is immutable" 'header Cache-Control "$work/tg.h" | grep -q immutable'
name="$(jq -r '.signed.targets | keys[] | select(startswith("app-"))' "$work/targets.json" | head -1)"
hash="$(jq -r --arg n "$name" '.signed.targets[$n].hashes.sha256' "$work/targets.json")"
object="$(fetch "${origin}app/targets/$hash.$name" | jq -r .artifact.hash)"
url="${origin}app/objects/$object"

fetch -D "$work/obj.h" -o "$work/full" "$url"
etag="$(header ETag "$work/obj.h")"
size="$(stat -c %s "$work/full")"
check "object is served raw (no Content-Encoding)" '! grep -qi "^Content-Encoding:" "$work/obj.h"'
check "object Content-Type is octet-stream" '[ "$(header Content-Type "$work/obj.h")" = application/octet-stream ]'
check "object is immutable" 'header Cache-Control "$work/obj.h" | grep -q immutable'
check "object ETag is strong" '[[ "$etag" == \"* ]]'
if command -v b3sum >/dev/null; then
    check "object BLAKE3 matches its name" '[ "$(b3sum --no-names "$work/full")" = "$object" ]'
else
    echo "skip  object BLAKE3 (b3sum not installed; the patcher verifies it on download)"
fi
half=$((size / 2))
fetch -D "$work/r.h" -o "$work/tail" -H "Range: bytes=$half-" -H "If-Range: $etag" "$url"
check "Range with matching If-Range returns 206" 'head -1 "$work/r.h" | grep -q " 206"'
check "206 Content-Range is exact" '[ "$(header Content-Range "$work/r.h")" = "bytes $half-$((size - 1))/$size" ]'
check "resumed bytes join to the original" 'cmp -s <(head -c "$half" "$work/full"; cat "$work/tail") "$work/full"'
check "stale If-Range returns the full object" '[ "$(status -H "Range: bytes=$half-" -H "If-Range: \"stale\"" "$url")" = 200 ]'
check "range past the end returns 416" '[ "$(status -H "Range: bytes=$size-" "$url")" = 416 ]'
check "conditional GET returns 304" '[ "$(status -H "If-None-Match: $etag" "$url")" = 304 ]'

for path in install.sh install.ps1 app/bootstrap/latest; do
    check "$path is revalidated text" 'fetch -D - -o /dev/null "${origin}$path" | grep -qi "^Cache-Control: no-cache"'
done

# Refusals
check "POST refused" '[ "$(status -X POST "${origin}install.sh")" = 405 ]'
check "PUT refused" '[ "$(status -X PUT --data x "${origin}app/objects/$object")" = 405 ]'
check "publish lock hidden" '[ "$(status "${origin}app/publish.lock")" = 404 ]'
check "dot-files hidden" '[ "$(status "${origin}.git/config")" = 404 ]'
check "traversal refused" '[ "$(status --path-as-is "${origin}app/../app/publish.lock")" != 200 ]'
check "unlisted repository paths hidden" '[ "$(status "${origin}app/metadata/")" = 404 ]'
check "repository files only on their own paths" '[ "$(status "${origin}site/data/catalog.json")" = 404 ]'

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed"
    exit 1
fi
echo "all checks passed"
