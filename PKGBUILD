pkgname=cheirizo
pkgver=0.1.0
pkgrel=1
pkgdesc="A single-port, multi-user RDP server for Linux servers running Wayland in headless mode"
arch=('x86_64')
url="https://github.com/scrabbles/cheirizo"
license=('MIT' 'GPL-2.0-only')
depends=('wayland' 'openssl' 'pam' 'pipewire' 'xorg-xwayland' 'wpa_supplicant' 'pixman' 'cairo' 'pango' 'libxkbcommon' 'glib2' 'libxml2' 'libdrm')
makedepends=('cargo' 'meson' 'ninja' 'python-yaml' 'wayland-protocols' 'cmake' 'pkgconf')
source=("git+file://${PWD}")
md5sums=('SKIP')
install=cheirizo.install

build() {
  cd "$srcdir/cheirizo"
  make build-release
  make compositor-build
}

package() {
  cd "$srcdir/cheirizo"
  
  install -d -m 0755 "${pkgdir}/usr/local/bin" "${pkgdir}/usr/lib/wrdp" "${pkgdir}/etc/wrdp" "${pkgdir}/usr/share/themes" "${pkgdir}/etc/systemd/system"

  install -m 0755 target/release/wrdp "${pkgdir}/usr/local/bin/"
  install -m 0755 target/release/wrdp-sesman "${pkgdir}/usr/local/bin/"
  install -m 0755 target/release/wrdpctl "${pkgdir}/usr/local/bin/"
  
  install -m 0755 out/wrdp-compositor-build/wrdp-compositor "${pkgdir}/usr/lib/wrdp/"
  
  cp -a vendor/wrdp-compositor/themes/PlatinumTheme-wrdp-compositor "${pkgdir}/usr/share/themes/"
  
  install -d -m 0755 "${pkgdir}/etc/wrdp/labwc" "${pkgdir}/etc/wrdp/waybar" "${pkgdir}/etc/wrdp/mako" "${pkgdir}/etc/wrdp/wallpaper"
  install -m 0644 vendor/wrdp-compositor/contrib/wrdp/labwc/autostart "${pkgdir}/etc/wrdp/labwc/autostart"
  install -m 0644 vendor/wrdp-compositor/contrib/wrdp/labwc/menu.xml "${pkgdir}/etc/wrdp/labwc/menu.xml"
  install -m 0644 vendor/wrdp-compositor/contrib/wrdp/labwc/rc.xml "${pkgdir}/etc/wrdp/labwc/rc.xml"
  install -m 0755 vendor/wrdp-compositor/contrib/wrdp/labwc/shutdown "${pkgdir}/etc/wrdp/labwc/shutdown"
  install -m 0644 vendor/wrdp-compositor/contrib/wrdp/waybar/config.jsonc "${pkgdir}/etc/wrdp/waybar/config.jsonc"
  install -m 0644 vendor/wrdp-compositor/contrib/wrdp/waybar/style.css "${pkgdir}/etc/wrdp/waybar/style.css"
  install -m 0644 vendor/wrdp-compositor/contrib/wrdp/mako/config "${pkgdir}/etc/wrdp/mako/config"
  install -m 0644 vendor/wrdp-compositor/contrib/wrdp/wallpaper/wallpaper.conf "${pkgdir}/etc/wrdp/wallpaper/wallpaper.conf"
  install -m 0755 vendor/wrdp-compositor/contrib/wrdp/bin/wrdp-desktop-action "${pkgdir}/usr/lib/wrdp/wrdp-desktop-action"
  install -m 0755 vendor/wrdp-compositor/contrib/wrdp/bin/wrdp-desktop-session "${pkgdir}/usr/lib/wrdp/wrdp-desktop-session"
  install -m 0755 vendor/wrdp-compositor/contrib/wrdp/bin/wrdp-wallpaper "${pkgdir}/usr/lib/wrdp/wrdp-wallpaper"

  install -m 0644 wrdp.socket "${pkgdir}/etc/systemd/system/"
  install -m 0644 wrdp.service "${pkgdir}/etc/systemd/system/"
  install -m 0644 wrdp-p2p.service "${pkgdir}/etc/systemd/system/"
  install -m 0755 autopostbuild.sh "${pkgdir}/usr/local/bin/wrdp-autopostbuild.sh"
}
