# Maintainer: DenBroLiik <128605164+DenBroLiik@users.noreply.github.com>
pkgname=nitro-unlock
pkgver=0.1.1
pkgrel=1
pkgdesc='Temporary Intel RAPL PSys boost for Acer Nitro Lite 16'
arch=('any')
url='https://github.com/DenBroLiik/nitro-unlock'
license=('LicenseRef-All-Rights-Reserved')
depends=('python' 'polkit')
optdepends=('polkit-kde-agent: graphical Polkit prompt in Niri')
source=('nitro-unlock' 'README.md' 'COPYRIGHT')
sha256sums=('9c81af413a20ade7de9363a3a806ff9b061072572efcebd1210c4ea49c8a59a4'
            '6179892291ece586e798c77da062691ddde5489bb87cfeebd97d9e68cb6f5aa9'
            'f98d6d69ba8f698fd2ea9ace818e0f7f85d508b8ef8cac70adf6b423bd70fbad')

package() {
    install -Dm755 "$srcdir/nitro-unlock" "$pkgdir/usr/bin/nitro-unlock"
    install -Dm644 "$srcdir/README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm644 "$srcdir/COPYRIGHT" "$pkgdir/usr/share/licenses/$pkgname/COPYRIGHT"
}
