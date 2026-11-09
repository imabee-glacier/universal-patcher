#!/usr/bin/env bash
# Publishes a release built by .github/workflows/release.yml on our own site. GitHub only builds:
# this downloads the staging archive from the private repository, checks it, uploads it to the
# host's staging/app, runs the publish-app Job (the online key stays in the cluster), and checks
# the live site. Users download only from the site.
#   tools/release-to-site.sh 0.1.1
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
version="${1:?usage: release-to-site.sh X.Y.Z}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must be X.Y.Z" >&2; exit 2; }
repo="imabee-glacier/universal-patcher"
host="${HOST:-root@144.217.66.212}"
origin="$(sed -n 's/^  PATCHER_ORIGIN: *//p' "$here/deploy/k3s/site.yaml")"
ssh_opts=(-o BatchMode=yes -o ControlMaster=auto -o "ControlPath=$HOME/.ssh/cm/%C" -o ControlPersist=30m)
mkdir -p "$HOME/.ssh/cm"
work="$here/runtime/release-$version"
archive="patcher-$version-staging.tar.gz"

echo "== download app-v$version from $repo"
rm -rf "$work"
mkdir -p "$work"
gh release download "app-v$version" --repo "$repo" --dir "$work" --pattern "$archive" --pattern "$archive.sha256"
(cd "$work" && sha256sum --check --quiet "$archive.sha256")
tar -C "$work" -xzf "$work/$archive"
staging="$work/app"

echo "== check what was built"
for engine in "$staging"/packages/bootstrap/"$version"/*/patcher-engine*; do
    grep -q --text "$origin" "$engine" || { echo "$engine does not carry $origin" >&2; exit 1; }
    for key in $(jq -r '.signed.keys | keys[]' "$here/distribution/app-root.json"); do
        grep -q --text "$key" "$engine" || { echo "$engine does not carry root key $key" >&2; exit 1; }
    done
done
for manifest in "$staging"/packages/app-*.json; do
    [ "$(jq -r .app_version "$manifest")" = "$version" ] || { echo "$manifest is not $version" >&2; exit 1; }
done
echo "offered: $(tr '\n' ' ' < "$staging/offer")"

echo "== upload to $host staging/app"
remote=/var/lib/appsynergy/volumes/patcher-data/_data/staging/app
rsync -a --delete -e "ssh ${ssh_opts[*]}" "$staging/" "$host:$remote/"
# shellcheck disable=SC2029 # $remote is a fixed path from this script, expanded here on purpose.
ssh "${ssh_opts[@]}" "$host" "chown -R 65532:65532 $remote"

echo "== publish"
ssh "${ssh_opts[@]}" "$host" 'set -e
job=$(kubectl create -f - -o name)
if ! kubectl -n universal-patcher wait --for=condition=complete "$job" --timeout=10m; then
    kubectl -n universal-patcher logs "$job"; exit 1
fi
kubectl -n universal-patcher logs "$job"
kubectl -n universal-patcher create job "health-$(date +%s)" --from=cronjob/check-health -o name |
    xargs -I{} kubectl -n universal-patcher wait --for=condition=complete {} --timeout=3m >/dev/null' \
    < "$here/deploy/k3s/publish-app.yaml"

echo "== check the live site"
[ "$(curl -fsS "${origin}app/bootstrap/latest")" = "$version" ] || { echo "site does not serve $version" >&2; exit 1; }
curl -fsS "${origin}data/catalog.json" | jq -e --arg v "$version" '.releases[-1].version == $v' >/dev/null
"$here/tools/front-check.sh" "$origin" | tail -1
echo "Universal Patcher $version is live at $origin"
