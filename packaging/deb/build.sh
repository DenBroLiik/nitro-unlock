#!/bin/sh
set -eu

repo=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
target=${1:-"$repo/packages"}
mkdir -p -- "$target"
target=$(CDPATH='' cd -- "$target" && pwd)
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
trap 'exit 1' HUP INT TERM
chmod 755 "$stage"

doc="$stage/usr/share/doc/nitro-unlock"
install -Dm755 "$repo/nitro-unlock" "$stage/usr/bin/nitro-unlock"
install -Dm644 "$repo/README.md" "$doc/README.md"
install -Dm644 "$repo/COPYRIGHT" "$doc/copyright"
install -d -m755 "$stage/DEBIAN"
gzip -n -9 < "$repo/packaging/deb/changelog" > "$doc/changelog.gz"

sed "s/^Installed-Size: auto\$/Installed-Size: $(du -sk -- "$stage/usr" | cut -f1)/" \
    "$repo/packaging/deb/control" > "$stage/DEBIAN/control"
version=$(sed -n 's/^Version: //p' "$stage/DEBIAN/control")
(
    cd -- "$stage"
    md5sum usr/bin/nitro-unlock usr/share/doc/nitro-unlock/* > DEBIAN/md5sums
)
"${DPKG_DEB:-dpkg-deb}" --root-owner-group -Zxz --build "$stage" "$target/nitro-unlock_${version}_all.deb"
