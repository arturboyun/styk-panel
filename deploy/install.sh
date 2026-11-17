#!/usr/bin/env bash
# Styk Panel installer for Debian and Ubuntu (amd64, arm64).
#
#   curl -fsSL https://github.com/@STYK_REPO@/releases/latest/download/install.sh \
#     | sudo bash -s -- panel --domain panel.example.com [--email you@example.com]
#
# Options after `panel`:
#   --domain NAME          the panel's address; the bundled Caddy gets a certificate for it
#   --email ADDR           contact for Let's Encrypt (optional)
#   --no-caddy             do not start the bundled Caddy (you have a proxy on 80/443)
#   --proxy-network NAME   Docker network your proxy shares with the panel (default styk-proxy)
#   --version vX.Y.Z       a specific release instead of the newest
#
# This script only bootstraps: it downloads the release's SHA256SUMS, checks its minisign
# signature against the key below, verifies the `styk` CLI against it and hands over to
# `styk setup`, which verifies every other file the same way.
set -euo pipefail

REPO="${STYK_REPO:-@STYK_REPO@}"
PUBKEY="${STYK_PUBKEY:-@MINISIGN_PUBKEY@}"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run as root: curl ... | sudo bash -s -- panel --domain ..."
case "${1:-}" in
panel) ;;
agent) die "remote agents come in a later release; install the panel first" ;;
*) die "usage: install.sh panel --domain panel.example.com" ;;
esac

ver=latest
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
    [ "${args[i]}" = --version ] && ver="${args[i + 1]:-}"
done
[ -n "$ver" ] || die "--version needs a value"

export DEBIAN_FRONTEND=noninteractive
missing=()
for c in curl minisign openssl ss; do command -v "$c" >/dev/null || missing+=("$c"); done
if [ ${#missing[@]} -gt 0 ]; then
    echo "==> Installing ${missing[*]}"
    pkgs=(ca-certificates curl minisign openssl iproute2)
    apt-get update -qq
    apt-get install -y -qq "${pkgs[@]}" >/dev/null
fi

dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
if [ -n "${STYK_RELEASE_DIR:-}" ]; then
    cp "$STYK_RELEASE_DIR"/{SHA256SUMS,SHA256SUMS.minisig,styk} "$dir/"
else
    if [ "$ver" = latest ]; then
        base="https://github.com/$REPO/releases/latest/download"
    else
        base="https://github.com/$REPO/releases/download/$ver"
    fi
    for f in SHA256SUMS SHA256SUMS.minisig styk; do
        curl -fsSL --retry 3 -o "$dir/$f" "$base/$f" || die "download $base/$f"
    done
fi
minisign -Vqm "$dir/SHA256SUMS" -x "$dir/SHA256SUMS.minisig" -P "$PUBKEY" ||
    die "release signature is not valid: nothing was installed"
(cd "$dir" && grep '  styk$' SHA256SUMS | sha256sum --quiet -c -) || die "styk does not match the signed checksum"

install -m 0755 "$dir/styk" /usr/local/bin/styk
exec /usr/local/bin/styk setup "$@"
