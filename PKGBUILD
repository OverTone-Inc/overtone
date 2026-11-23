# Local Arch package. Run makepkg from the root of a checkout.
pkgname=overtone
pkgver=0.4.0
pkgrel=1
pkgdesc='Лаунчер и мод Яндекс Музыки, менеджер плееров'
arch=('x86_64')
license=('MIT')
depends=('gtk4>=4.16' 'glib2' 'glibc' 'libgcc' 'libx11' 'python' 'iproute2' 'zstd')
makedepends=('rust' 'pkgconf' 'python')
optdepends=('yandex-music: запуск настольной Яндекс Музыки' 'discord: публикация Discord Rich Presence' 'mpv: воспроизведение локальных файлов и автозамена' 'mpv-mpris: управление mpv через MPRIS' 'git: получение исходников аддонов' 'nodejs: сборка аддонов' 'npm: установка зависимостей аддонов')
options=('!debug')
source=()
sha256sums=()

build() {
    cd "$startdir"
    python3 packaging/install.py --version "$pkgver"
    export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
    cargo build --locked --release
}

package() {
    cd "$startdir"
    python3 packaging/install.py --stage "$pkgdir" --bin-dir "${CARGO_TARGET_DIR:-$startdir/target}/release"
}
