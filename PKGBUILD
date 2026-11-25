# Maintainer: DenBroLiik
pkgname=nitro-unlock
pkgver=1.2.0
pkgrel=1
pkgdesc='Temporary Intel RAPL PSys boost for Acer Nitro Lite 16'
arch=('x86_64')
url='https://github.com/DenBroLiik/nitro-unlock'
license=('LicenseRef-All-Rights-Reserved')
depends=('glibc' 'polkit')
makedepends=('rust')
optdepends=('polkit-kde-agent: graphical Polkit authentication')
build() {
    cd "$startdir"
    CARGO_TARGET_DIR="$startdir/target" cargo build --release --locked --offline
}
check() {
    cd "$startdir"
    CARGO_TARGET_DIR="$startdir/target" cargo test --locked --offline
}
package() {
    install -Dm755 "$startdir/target/release/nitro-unlock" "$pkgdir/usr/bin/nitro-unlock"
    install -Dm644 "$startdir/README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm644 "$startdir/COPYRIGHT" "$pkgdir/usr/share/licenses/$pkgname/COPYRIGHT"
}
