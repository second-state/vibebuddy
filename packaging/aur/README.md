# AUR package

`vibebuddy-bin/` is the source of the [`vibebuddy-bin`](https://aur.archlinux.org/packages/vibebuddy-bin) AUR package. It installs the Linux tarball from a GitHub release system-wide instead of into one user's home, as `packaging/linux/install.sh` does:

- `/usr/bin`: the daemon, hook and tray app
- `/usr/lib/systemd/user/vibebuddyd.service`
- `/usr/share/applications`, `/etc/xdg/autostart`, and the icon
- `/usr/share/vibebuddy`: voice packs and firmware. The app looks there after `~/.local/share/vibebuddy`.
- `/usr/lib/udev/rules.d/70-vibebuddy.rules`: gives the logged-in user access to the box without joining `uucp`

Per-user setup can't be packaged: after installing, each user runs `systemctl --user enable --now vibebuddyd` and `vibebuddy-hook install`, as the install message says.

## Updating it for a release

After the release workflow has published `vX.Y.Z`:

```bash
cd packaging/aur/vibebuddy-bin
sed -i 's/^pkgver=.*/pkgver=X.Y.Z/; s/^pkgrel=.*/pkgrel=1/' PKGBUILD
updpkgsums                      # replaces sha256sums with the release tarball's (pacman-contrib)
makepkg -f && namcap *.pkg.tar.zst
makepkg --printsrcinfo > .SRCINFO
```

Then copy `PKGBUILD`, `vibebuddy-bin.install` and `.SRCINFO` into a clone of `ssh://aur@aur.archlinux.org/vibebuddy-bin.git`, commit and push. Pushing needs an AUR account with an SSH key; commit the PKGBUILD change here too, so the two stay in step.
