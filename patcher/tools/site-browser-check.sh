#!/usr/bin/env bash
# Runs tools/browser/site-check.mjs against a running front in a pinned Chromium image.
#   tools/site-browser-check.sh https://downloads.example.org/ [test-ca.pem]
# With a test CA the browser accepts the local certificate; without one it must be publicly valid.
# Screenshots go to runtime/site-check/.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
origin="${1:?usage: site-browser-check.sh <origin> [test-ca]}"
image="mcr.microsoft.com/playwright:v1.65.0-noble@sha256:6e3fb709361521eb27161606e6da6d2674e5e89fa16516cb37d6fdd91372273d"
shots="$here/runtime/site-check"
mkdir -p "$shots"
test_ca=0
[ -n "${2:-}" ] && test_ca=1
docker run --rm --network host --init --user "$(id -u):$(id -g)" \
    -e HOME=/tmp -e TEST_CA="$test_ca" -e PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 -e npm_config_update_notifier=false \
    -v "$here/tools/browser/site-check.mjs:/work/site-check.mjs:ro" -v "$shots:/shots" \
    "$image" bash -ec 'cd /tmp && npm install --silent --no-audit --no-fund playwright@1.65.0 >/dev/null && cp /work/site-check.mjs . && node site-check.mjs "$0" /shots' "$origin"
