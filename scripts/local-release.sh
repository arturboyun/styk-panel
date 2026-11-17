#!/usr/bin/env bash
# Builds a release like .github/workflows/release.yml, locally and for amd64 only, signed
# with your local key. For testing install.sh and `styk update` without publishing:
#
#   scripts/local-release.sh dist v0.1.0       -> dist/ with the same files as a release
#   STYK_RELEASE_DIR=dist bash dist/install.sh panel --domain panel.localhost
#
# IMAGE in it is the local tag styk-panel:<version>; load it on the test host with
# `docker save styk-panel:<version> | docker load`.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="$(realpath -m "${1:-dist}")"
VER="${2:-v0.0.0-dev}"
KEY="${MINISIGN_KEY:-$HOME/.config/styk-release/minisign.key}"
IMG="styk-panel:$VER"
REPO="${STYK_REPO:-arturboyun/styk-panel}"

rm -rf "$OUT" && mkdir -p "$OUT"
docker build -q -f deploy/Dockerfile --build-arg CARGO_BUILD_JOBS=8 -t "$IMG" . >/dev/null
docker build -q -f deploy/Dockerfile --build-arg CARGO_BUILD_JOBS=8 --target agent \
    --output "type=local,dest=$OUT/.agent" . >/dev/null
install -m 0755 "$OUT/.agent/styk-agent" "$OUT/styk-agent-linux-amd64"
rm -rf "$OUT/.agent"
pub="$(sed -n 2p deploy/minisign.pub)"
for f in styk install.sh; do
    sed -e "s|@STYK_REPO@|$REPO|g" -e "s|@MINISIGN_PUBKEY@|$pub|g" -e "s|@VERSION@|$VER|g" "deploy/$f" >"$OUT/$f"
    chmod +x "$OUT/$f"
done
cp deploy/compose.yml deploy/env.example deploy/styk-agent.service "$OUT/"
echo "$VER" >"$OUT/VERSION"
echo "$IMG" >"$OUT/IMAGE"
(cd "$OUT" && sha256sum -- * >SHA256SUMS)
docker run --rm -v "$OUT:/d" -v "$KEY:/k/key:ro" -u "$(id -u):$(id -g)" local/minisign \
    minisign -S -s /k/key -m /d/SHA256SUMS -t "styk-panel $VER (local)" </dev/null >/dev/null
echo "$OUT: $VER, image $IMG"
