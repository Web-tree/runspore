#!/bin/sh
# Installs spore from a runspore release, after checking the release key's signature
# on SHA256SUMS and the archive's checksum. The contract is spec/release.md.
#
#   curl -fsSL https://raw.githubusercontent.com/Web-tree/runspore/main/install.sh | sh
#
# SPORE_VERSION       release to install (default: the newest)
# SPORE_INSTALL_DIR   install folder (default: $HOME/.local/bin)
# SPORE_DOWNLOAD_URL  folder holding the release assets, for a mirror

set -eu

RELEASE_KEY='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGy3mjvtp10UZgB9Y8u3KNBOh66DHPo3bdbZEszh9b1k runspore-release'
REPO='Web-tree/runspore'
NAMESPACE='runspore-release'

die() {
	printf 'install.sh: %s\n' "$*" >&2
	exit 1
}

need() {
	command -v "$1" >/dev/null 2>&1 || die "needs $1"
}

case $RELEASE_KEY in
'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5'*) ;;
*) die "this copy pins no release key (spec/release.md section 4)" ;;
esac

need curl
need tar
need ssh-keygen
if command -v sha256sum >/dev/null 2>&1; then
	sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
	sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
	die "needs sha256sum or shasum"
fi

os=$(uname -s)
arch=$(uname -m)
if [ "$os" = Darwin ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
	arch=arm64
fi
case $os/$arch in
Darwin/arm64 | Darwin/aarch64) target=aarch64-apple-darwin ;;
Darwin/x86_64 | Darwin/amd64) target=x86_64-apple-darwin ;;
Linux/arm64 | Linux/aarch64) target=aarch64-unknown-linux-musl ;;
Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-musl ;;
*) die "no release archive for $os $arch" ;;
esac
asset="spore-$target.tar.gz"

if [ -n "${SPORE_DOWNLOAD_URL:-}" ]; then
	base=${SPORE_DOWNLOAD_URL%/}
elif [ -n "${SPORE_VERSION:-}" ]; then
	base="https://github.com/$REPO/releases/download/v${SPORE_VERSION#v}"
else
	base="https://github.com/$REPO/releases/latest/download"
fi
if [ -n "${SPORE_INSTALL_DIR:-}" ]; then
	dir=$SPORE_INSTALL_DIR
elif [ -n "${HOME:-}" ]; then
	dir="$HOME/.local/bin"
else
	die "HOME is not set; set SPORE_INSTALL_DIR"
fi

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t spore) || die "cannot make a temporary directory"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM

for file in "$asset" SHA256SUMS SHA256SUMS.sig; do
	curl --proto '=https,file' --proto-redir '=https' --tlsv1.2 -fsL "$base/$file" -o "$tmp/$file" ||
		die "cannot download $base/$file"
done

# 1. The release key signed SHA256SUMS.
printf '%s namespaces="%s" %s\n' "$NAMESPACE" "$NAMESPACE" "$RELEASE_KEY" >"$tmp/allowed_signers"
ssh-keygen -Y verify -f "$tmp/allowed_signers" -I "$NAMESPACE" -n "$NAMESPACE" \
	-s "$tmp/SHA256SUMS.sig" <"$tmp/SHA256SUMS" >/dev/null 2>&1 ||
	die "SHA256SUMS is not signed by the runspore release key"

# 2. The archive matches its line in SHA256SUMS.
want=$(awk -v n="$asset" '$2 == n { print $1; c++ } END { exit (c != 1) }' "$tmp/SHA256SUMS") ||
	die "SHA256SUMS has no single line for $asset"
[ "$(sha256 "$tmp/$asset")" = "$want" ] || die "$asset does not match SHA256SUMS"

if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
	gh attestation verify "$tmp/$asset" --repo "$REPO" >/dev/null 2>&1 ||
		die "the attestation of $asset does not verify (gh attestation verify)"
	echo "attestation checked"
else
	echo "attestation not checked (needs gh, signed in)"
fi

tar -xzf "$tmp/$asset" -C "$tmp" spore 2>/dev/null || die "cannot extract spore from $asset"
mkdir -p "$dir" || die "cannot create $dir"
part="$dir/.spore.$$"
trap 'rm -rf "$tmp"; rm -f "$part"' EXIT
if ! { cp "$tmp/spore" "$part" && chmod 0755 "$part"; }; then
	die "cannot write into $dir"
fi
version=$("$part" --version) || die "the downloaded spore does not run on this machine"
mv -f "$part" "$dir/spore" || die "cannot write into $dir"

echo "installed $version at $dir/spore"
case ":$PATH:" in
*":$dir:"*)
	found=$(command -v spore 2>/dev/null || true)
	if [ -n "$found" ] && [ "$found" != "$dir/spore" ]; then
		echo "warning: $found comes before $dir/spore on PATH"
	fi
	;;
*) echo "warning: $dir is not on PATH; add it, for example: export PATH=\"$dir:\$PATH\"" ;;
esac
