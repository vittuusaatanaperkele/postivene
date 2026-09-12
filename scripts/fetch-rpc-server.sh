#!/bin/sh
# Fetch upstream deltachat-rpc-server binaries for the architectures
# Postivene packages, and place them where rpm/harbour-postivene.spec expects
#
#     vendor/deltachat-rpc-server/<sailfish-arch>/deltachat-rpc-server
#
# Upstream (chatmail/core) builds these statically against musl libc
# ("to avoid problems with glibc version incompatibility", per their CI
# workflow), so a binary depends only on the kernel -- this is why a
# generic upstream build is expected to run on Sailfish unmodified.
#
# Source: the exact same binaries upstream attaches to its GitHub release
# are also published inside its PyPI wheels (see
# .github/workflows/deltachat-rpc-server.yml upstream: both are the nix
# build's `result/bin/deltachat-rpc-server`). PyPI is used here because
# it's fetchable with plain HTTPS and the wheel is just a zip.
#
# Everything is pinned: bump VERSION *and* refresh all checksums together
# (fetch the new wheels, run `sha256sum`, update below), then update
# vendor/deltachat-rpc-server/SOURCE.md to match -- the MPL-2.0
# source-availability notice must always describe the binaries actually
# bundled (see vendor/deltachat-rpc-server/SOURCE.md).

set -eu

VERSION="2.60.0"

# sailfish-arch  upstream-arch  wheel-tag                                                                     wheel-sha256                                                       binary-sha256
TABLE="
aarch64 aarch64 py3-none-manylinux_2_17_aarch64.manylinux2014_aarch64.musllinux_1_1_aarch64 abacae1388fc9638e2828fa1cbe9af91c830fa2a7cd447e82db0a8dda5ee0cef ccae50468d5faf15bde7df1b88bd396d5266ee802388f278e64ed4993738b626
armv7hl armv7l py3-none-linux_armv7l.manylinux_2_17_armv7l.manylinux2014_armv7l.musllinux_1_1_armv7l 0499dac38270688b23a7082264a113828b3823f80a1f8b5022abf93fc94ba39c 301a3b65fe9ee309d7d9acfd56a8ba78e1f1ecdd5d87b591d7a24cf3e4b9d488
x86_64 x86_64 py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.musllinux_1_1_x86_64 5f7a847e51772c4e96472433dc4e8943970273722c4676422055bfa53c127f80 360b1ece949dcb6948e25b8a81e6399cdbbcf69b28728fa4ae0c7f14291c8764
"

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
vendor_dir="$repo_root/vendor/deltachat-rpc-server"
workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

fetch_one() {
    sfos_arch="$1"; upstream_arch="$2"; wheel_tag="$3"; wheel_sha="$4"; bin_sha="$5"

    wheel="deltachat_rpc_server-$VERSION-$wheel_tag.whl"

    echo ">> $sfos_arch (upstream $upstream_arch)"
    # pip resolves PyPI's hash-addressed file URL; --no-deps and the exact
    # version pin keep it honest, and the sha256 check below keeps it
    # honest even if the index were tampered with. --isolated and the
    # explicit index: a PIP_INDEX_URL or pip.conf on the machine running
    # this would otherwise point it at a mirror without anyone knowing --
    # the hashes would still catch a substitution, but as a puzzling
    # failure rather than a named one.
    pip download "deltachat-rpc-server==$VERSION" --no-deps \
        --isolated --index-url https://pypi.org/simple --no-cache-dir \
        --platform "musllinux_1_1_$upstream_arch" --only-binary=:all: \
        -d "$workdir" >/dev/null

    echo "$wheel_sha  $workdir/$wheel" | sha256sum -c - >/dev/null || {
        echo "fetch-rpc-server: checksum mismatch for $wheel; refusing to install it" >&2
        exit 1
    }

    unzip -oq "$workdir/$wheel" -d "$workdir/$sfos_arch"
    bin="$workdir/$sfos_arch/deltachat_rpc_server/deltachat-rpc-server"
    echo "$bin_sha  $bin" | sha256sum -c - >/dev/null || {
        echo "fetch-rpc-server: checksum mismatch for the $sfos_arch binary inside $wheel; refusing to install it" >&2
        exit 1
    }

    install -Dm 755 "$bin" "$vendor_dir/$sfos_arch/deltachat-rpc-server"
    echo "   -> vendor/deltachat-rpc-server/$sfos_arch/deltachat-rpc-server"
}

echo "$TABLE" | while read -r sfos_arch upstream_arch wheel_tag wheel_sha bin_sha; do
    [ -n "$sfos_arch" ] || continue
    fetch_one "$sfos_arch" "$upstream_arch" "$wheel_tag" "$wheel_sha" "$bin_sha"
done

echo "Done. Binaries are deltachat-rpc-server v$VERSION, statically linked (musl)."
echo "Remember: vendor/deltachat-rpc-server/SOURCE.md must describe this exact version."
