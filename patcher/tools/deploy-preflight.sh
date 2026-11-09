#!/usr/bin/env bash
# Refuses a deployment whose identifying values are not settled. Run before
# `kubectl apply -k deploy/k3s`:
#   tools/deploy-preflight.sh
# Checks: placeholders replaced, the address is in the routed public block, DNS points the domain
# at it, CAA (if published) allows Let's Encrypt, our host guard (deploy/host) allows exactly the
# front's ports for it and routes it, and every image is pinned to a real digest.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
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
# Certificate authorities use the nearest CAA set, walking up from the name (RFC 8659).
caa="" caa_at="$domain"
while [ -n "$caa_at" ]; do
    caa="$(dig +short CAA "$caa_at")"
    [ -n "$caa" ] && break
    case "$caa_at" in *.*) caa_at="${caa_at#*.}" ;; *) caa_at="" ;; esac
done
if [ -z "$caa" ]; then
    pass "no CAA anywhere above $domain; any CA may issue"
elif grep -q 'issue "letsencrypt.org' <<<"$caa"; then
    pass "CAA at $caa_at allows Let's Encrypt"
else
    fail "CAA at $caa_at does not allow letsencrypt.org"
fi

guard="$here/deploy/host/patcher-guard.nft"
route="$here/deploy/host/30-br-pub.network.d/patcher-front.conf"
allowed="$(grep -oE '(tcp|udp) dport [0-9]+ return' "$guard" | awk '{print $1"/"$3}' | sort | tr '\n' ' ')"
if grep -qE "ip daddr $start jump front$" "$guard" && [ "$allowed" = "tcp/443 tcp/80 udp/443 " ]; then
    pass "host guard allows only 80/tcp, 443/tcp, 443/udp to $start"
else
    fail "host guard does not cover $start with exactly 80/tcp, 443/tcp, 443/udp (found: $allowed)"
fi
if grep -qx "Destination=$start/32" "$route"; then pass "br-pub route for $start/32"; else fail "no br-pub route for $start/32"; fi

zero="sha256:0000000000000000000000000000000000000000000000000000000000000000"
if grep -q "$zero" "$k3s/kustomization.yaml" "$k3s/publish-app.yaml"; then fail "an image is not pinned to a published digest"; else pass "images pinned"; fi

if [ "$failures" -ne 0 ]; then
    echo "$failures check(s) failed; do not deploy"
    exit 1
fi
echo "preflight passed"
