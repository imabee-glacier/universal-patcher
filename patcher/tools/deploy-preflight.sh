#!/usr/bin/env bash
# Refuses a deployment whose identifying values are not settled. Run from the patcher/ directory
# before `kubectl apply -k deploy/k3s`:
#   tools/deploy-preflight.sh <path to an appsynergy-rs checkout>
# Checks: placeholders replaced, the address is in the routed public block, DNS points the domain
# at it, CAA (if published) allows Let's Encrypt, exposure.toml declares exactly the front's
# ports for it, and every image is pinned to a real digest.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
exposure="${1:?usage: deploy-preflight.sh <appsynergy-rs checkout>}/ops/netpolicy/exposure.toml"
k3s="$here/deploy/k3s"
failures=0
fail() { printf 'FAIL  %s\n' "$1"; failures=$((failures + 1)); }
pass() { printf 'ok    %s\n' "$1"; }

domain="$(sed -n 's/^  PATCHER_DOMAIN: *//p' "$k3s/site.yaml")"
origin="$(sed -n 's/^  PATCHER_ORIGIN: *//p' "$k3s/site.yaml")"
start="$(sed -n 's/.*"rangeStart": "\([0-9.]*\)".*/\1/p' "$k3s/site.yaml")"
end="$(sed -n 's/.*"rangeEnd": "\([0-9.]*\)".*/\1/p' "$k3s/site.yaml")"

if [ "$domain" = example.invalid ] || [ -z "$domain" ]; then fail "PATCHER_DOMAIN is still a placeholder"; else pass "domain $domain"; fi
if [ "$origin" = "https://$domain/" ]; then pass "origin matches domain"; else fail "PATCHER_ORIGIN must be https://$domain/"; fi
if [ "$start" != "$end" ]; then
    fail "attachment range must be a single address"
elif [[ ! "$start" =~ ^64\.112\.14\.([0-9]+)$ ]] || [ "${BASH_REMATCH[1]}" -lt 2 ] || [ "${BASH_REMATCH[1]}" -gt 126 ]; then
    fail "address $start is not a usable host in the routed block 64.112.14.0/25"
else
    pass "address $start"
fi

resolved="$(dig +short A "$domain" | sort -u | tr '\n' ' ')"
if [ "$resolved" = "$start " ]; then pass "$domain resolves only to $start"; else fail "$domain resolves to '${resolved% }', expected $start"; fi
if [ -n "$(dig +short AAAA "$domain")" ]; then fail "$domain has an AAAA record; the front has no IPv6 address"; else pass "no AAAA record"; fi
caa="$(dig +short CAA "$domain")"
if [ -z "$caa" ] || grep -q 'issue "letsencrypt.org"' <<<"$caa"; then pass "CAA allows Let's Encrypt (${caa:-none published})"; else fail "CAA does not allow letsencrypt.org: $caa"; fi

if python3 - "$exposure" "$start" "$domain" <<'EOF'; then
import sys, tomllib
spec = tomllib.load(open(sys.argv[1], "rb"))
stacks = [s for s in spec["stack"] if s.get("address") == sys.argv[2]]
assert len(stacks) == 1, f"{len(stacks)} stacks hold {sys.argv[2]}"
ports = sorted((a["proto"], a["port"]) for a in stacks[0]["allow"])
assert ports == [("tcp", 80), ("tcp", 443), ("udp", 443)], ports
assert sys.argv[3] in stacks[0]["names"], stacks[0]["names"]
EOF
    pass "exposure.toml declares 80/tcp, 443/tcp, 443/udp for $start"
else
    fail "exposure.toml does not declare the front correctly"
fi

zero="sha256:0000000000000000000000000000000000000000000000000000000000000000"
if grep -q "$zero" "$k3s/kustomization.yaml" "$k3s/publish-app.yaml"; then fail "an image is not pinned to a published digest"; else pass "images pinned"; fi

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed; do not deploy"
    exit 1
fi
echo "preflight passed"
