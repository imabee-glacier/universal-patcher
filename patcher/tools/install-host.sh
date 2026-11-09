#!/usr/bin/env bash
# Installs the patcher's own host configuration on appsynergy-server: its ingress guard
# (deploy/host/patcher-guard.nft, patcher-guard.service) and the /32 route drop-in for br-pub.
# Touches no file another project owns. Idempotent.
#   tools/install-host.sh
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
host="${HOST:-root@144.217.66.212}"
ssh_opts=(-o BatchMode=yes -o ControlMaster=auto -o "ControlPath=$HOME/.ssh/cm/%C" -o ControlPersist=30m)
mkdir -p "$HOME/.ssh/cm"

ssh "${ssh_opts[@]}" "$host" 'install -d -m 0700 /root/universal-patcher-host'
scp -q "${ssh_opts[@]}" "$here/deploy/host/patcher-guard.nft" "$here/deploy/host/patcher-guard.service" \
    "$here/deploy/host/30-br-pub.network.d/patcher-front.conf" "$host:/root/universal-patcher-host/"
ssh "${ssh_opts[@]}" "$host" 'set -e
cd /root/universal-patcher-host
nft -c -f patcher-guard.nft
install -m 0644 patcher-guard.nft /etc/nftables.d/patcher-guard.nft
install -m 0644 patcher-guard.service /etc/systemd/system/patcher-guard.service
install -d -m 0755 /etc/systemd/network/30-br-pub.network.d
install -m 0644 patcher-front.conf /etc/systemd/network/30-br-pub.network.d/patcher-front.conf
systemctl daemon-reload
systemctl enable --quiet patcher-guard.service
systemctl restart patcher-guard.service
systemctl is-active patcher-guard.service
ip route replace 64.112.14.67/32 dev br-pub
nft list chain inet patcher_guard front | grep -c "dport"
ip route get 64.112.14.67 | head -1'
