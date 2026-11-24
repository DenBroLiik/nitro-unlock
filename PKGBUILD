# Maintainer: DenBroLiik <128605164+DenBroLiik@users.noreply.github.com>
pkgname=nitro-unlock
pkgver=0.1.0
pkgrel=1
pkgdesc='Temporary Intel RAPL PSys boost for Acer Nitro Lite 16'
arch=('any')
url='https://github.com/DenBroLiik/nitro-unlock'
license=('LicenseRef-All-Rights-Reserved')
depends=('python' 'polkit')
optdepends=('polkit-kde-agent: graphical Polkit prompt in Niri')
source=('nitro-unlock' 'README.md' 'COPYRIGHT')
sha256sums=('20bec1044e0967ab42ce74d0fa9ae056b53f50854fe0bf11b96e45ec5a3e824d'
            '0d0e47cc0796b0c54382ded40267746b5c24aea2b942178bfae67b9270c25df2'
            'f98d6d69ba8f698fd2ea9ace818e0f7f85d508b8ef8cac70adf6b423bd70fbad')

package() {
    install -Dm755 "$srcdir/nitro-unlock" "$pkgdir/usr/bin/nitro-unlock"
    install -Dm644 "$srcdir/README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm644 "$srcdir/COPYRIGHT" "$pkgdir/usr/share/licenses/$pkgname/COPYRIGHT"
}
