# Linux distributions

Recipes for getting Ayatsuri2D into the official channels once the source
is public. Every one builds from source, offline after the sources are
fetched, as each channel requires. Requirements were checked against each
channel's own documentation on 2026-11-10; re-check before submitting.

    packaging/linux/                the shared files: desktop entry, AppStream metainfo, MIME type
    packaging/vendor.sh             source tarball + vendored-crates tarball (Debian, Fedora)
    packaging/arch/PKGBUILD         Arch Linux
    packaging/debian/               Debian / Ubuntu source package (PPA now, Debian later)
    packaging/fedora/               Fedora / COPR spec and the licence summary script
    packaging/flatpak/              Flatpak manifest and cargo-sources.json

The AppStream file every package installs,
`packaging/linux/io.github.Ayatsuri2D.Ayatsuri2D.metainfo.xml`, is generated
by `cargo xtask metainfo` from the `.in` template and the released versions
in CHANGELOG.md; `cargo xtask ci` fails when it is stale. Edit the template,
never the generated file.

## Blockers found

1. **The source is private.** Every channel fetches the release tarball from
   GitHub; nothing below can be submitted until the repository is public.
2. **Flathub forbids AI-generated manifests.** Its requirements (Generative AI
   policy) say Flathub manifests must not contain AI-generated or
   AI-assisted content, AI tools must not open or write the submission PR,
   its description or replies, and any AI-generated code in the app itself
   must be disclosed with the parts and rough extent. The manifest in
   `packaging/flatpak/` was written with an AI tool, so it is for local and
   self-hosted Flatpak builds only. A maintainer has to write the Flathub
   manifest by hand, open the PR by hand, and disclose the project's AI use
   (CONTRIBUTING.md allows AI tools). Reviewers may reject the app over it.
3. **Flathub app id and repository name.** For `io.github.Ayatsuri2D.Ayatsuri2D`
   Flathub derives `https://github.com/Ayatsuri2D/Ayatsuri2D`, but the
   repository is `ayatsuri_2d`. Either rename the repository to `Ayatsuri2D`
   (GitHub redirects the old name) or ask the reviewers for the documented
   manual exception. Changing the app id later means resubmitting.
4. **Rust 1.95 or newer** is needed (egui 0.36). Arch, Fedora, Flathub
   (rust-stable 1.98 on the 26.08 SDK) and Debian sid (1.97) have it. On
   Ubuntu only the versioned `rustc-1.97` / `cargo-1.97` packages do, in
   24.04 (noble-updates) and 26.04; Ubuntu 22.04 tops out at 1.91 and gets
   no PPA build.
5. **Non-free screenshots.** `docs/media` shows third-party artwork used with
   permission, under no free licence. No package installs it, but a
   tarball for the Debian archive must be repacked without it
   (`Files-Excluded` in debian/copyright). Fedora reviewers may ask the
   same of the source tarball.
6. **Ubuntu Font licence.** egui embeds Ubuntu-Light (`epaint_default_fonts`,
   Ubuntu-font-1.0). Fine for Arch, Flathub and PPAs; check it against
   Debian's DFSG review and Fedora's allowed-licence list before those
   reviews.

## Arch Linux ([extra])

**Ready:** `packaging/arch/PKGBUILD` and `.SRCINFO`. It follows the Arch Rust
package guidelines: `cargo fetch --locked` for the host in `prepare()`,
`cargo build --frozen --release` of both apps, `cargo test --frozen
--workspace` in `check()`, and installs both binaries, the desktop entry,
metainfo, MIME type, icons, docs and the MIT licence files. makepkg's own LTO
is off (`!lto`): it turns ring's C objects into GCC bitcode that rust-lld
cannot link.

**Who:** only an Arch Package Maintainer can put a package in [extra]; there
is no request queue. The usual way is the AUR first.

1. When the repo is public and a release tag exists: set `pkgver`, run
   `updpkgsums` (replaces `SKIP`) and `makepkg --printsrcinfo > .SRCINFO`.
2. Build in a clean chroot (`pkgctl build` from devtools) and run `namcap` on
   the PKGBUILD and the package.
3. Publish it to the AUR as `ayatsuri2d` (a maintainer's AUR account).
4. Once it has users, ask on the aur-general mailing list or the
   #archlinux-aur IRC channel whether a Package Maintainer will adopt it
   into [extra].
5. Signing: when releases ship a signed source tarball, add the `.asc` to
   `source` and the release key's fingerprint to `validpgpkeys` (see the
   comment in the PKGBUILD).

## Ubuntu PPA (apt) and Debian

**Ready for a PPA:** `packaging/debian/` (control, rules, changelog, DEP-5
copyright, source/format `3.0 (quilt)`) and `build-source.sh`, which makes
`ayatsuri2d_<v>.orig.tar.gz` plus `ayatsuri2d_<v>.orig-vendor.tar.xz` (the
`cargo vendor` output as a second orig component, unpacked to `vendor/`) and
a source package per Ubuntu series. Launchpad builders have no network, so
`debian/rules` points cargo at `vendor/` and builds with `--frozen`.
Installing `cargo-vendor-filterer` before running it drops the Windows and
macOS crates (the unfiltered tarball is about 47 MB).

On go-public day (a maintainer with a Launchpad account, on Ubuntu or Debian
with `dpkg-dev`, `devscripts` and `dput`):

1. Create a Launchpad team `ayatsuri2d` and a PPA in it; upload the team's
   OpenPGP key to Launchpad.
2. `packaging/debian/build-source.sh --series noble`, then again with
   `--series resolute` (each series gets its own version, `~noble1` etc.).
3. `debsign -k <KEY> dist/release/deb-src/*_source.changes` and
   `dput ppa:ayatsuri2d/ppa <changes>` for each.
4. Optionally test first with `dpkg-buildpackage -b -us -uc` in the
   unpacked tree, or `sbuild` against noble.

**Official Debian (and so Ubuntu universe) is a different job.** The Debian
Rust team packages every crate as its own `rust-<crate>` source package
(debcargo), and applications build against those, not against a vendor
tarball. A check of `Cargo.lock` against Debian sid on 2026-11-10 (Linux
x86_64 dependency graph, 330 crates): 262 are in Debian at a compatible
version, 41 are there at an incompatible semver (alsa, base64, calloop,
getrandom, glow, hashbrown, itertools and others), and 27 are not packaged
at all, among them egui, eframe, egui-wgpu, egui-winit, epaint, wgpu 30,
wgpu-core, wgpu-hal, rodio, psd, egui-phosphor and pulseaudio. So about 68
crates need new packages or version bumps, the egui and wgpu stack the
largest part, before Ayatsuri2D itself can be uploaded. The route: join the
Rust team on Salsa, package the missing crates with debcargo, then file an
ITP and find a Debian Developer to sponsor the upload. Expect months.

## Fedora (dnf) and COPR

**Ready:** `packaging/fedora/ayatsuri2d.spec`, built offline from the release
tarball (Source0) and the vendored-crates tarball (Source1, from
`packaging/vendor.sh`), following the Fedora Rust guidelines for vendor
tarballs: `%cargo_prep -v vendor`, no `%cargo_generate_buildrequires`,
`%cargo_vendor_manifest` with `cargo-vendor.txt` as a `%license` file (it
generates the required `Provides: bundled(crate(...))`), `%cargo_license`
into `LICENSE.dependencies`, and a SPDX `License:` that ANDs our licences
with every distinct expression of the crates linked into the binaries.
`packaging/fedora/license-summary.sh` regenerates that expression from
Cargo.lock (`--list` gives the per-crate list); compare it with the
`%cargo_license_summary` output in the build log.

COPR now (any FAS account):

1. `packaging/vendor.sh --ref v<version>` and attach
   `ayatsuri2d-<version>-vendor.tar.xz` to the GitHub release, then change
   `Source1` to its release URL.
2. Create a COPR project `ayatsuri2d`, add a package from the spec (SCM
   source pointing at the repo, spec path `packaging/fedora/ayatsuri2d.spec`,
   or upload an SRPM built with `rpmbuild -bs`), and build for current
   Fedora releases on x86_64 and aarch64.

Fedora proper:

1. The guidelines say packages SHOULD NOT bundle crates where system crates
   exist. Most of the stack is not in Fedora (egui, eframe, wgpu, cpal,
   rodio, psd, evdev, pulseaudio, discord-rich-presence are missing from
   rawhide), so explain the vendoring in the review, or package the missing
   crates with rust2rpm first.
2. File a review request in Red Hat Bugzilla (product Fedora, component
   Package Review) with the spec and SRPM URLs, and run `fedora-review`.
3. A first-time packager needs a sponsor from the packager group; ask on
   the devel list or the Fedora packaging Matrix room.
4. Fedora's AI-assisted contributions policy wants AI help disclosed and the
   contributor accountable; say in the review that the spec was drafted
   with an AI tool and has been checked by hand.

## Flathub

**For local and self-hosted builds only** (blocker 2). In
`packaging/flatpak/`:

- `io.github.Ayatsuri2D.Ayatsuri2D.yml`: org.freedesktop.Platform 26.08 with
  the rust-stable SDK extension (no LLVM extension is needed: nothing uses
  bindgen or clang), both apps built with `--offline --locked` from
  `cargo-sources.json`, icons renamed to the app id by `rename-icon`.
- `gen-cargo-sources.sh` regenerates `cargo-sources.json` with
  flatpak-cargo-generator at a pinned commit; run it after every
  Cargo.lock change.
- `finish-args`, each justified in the manifest: Wayland with X11 fallback,
  ipc, `dri` for the GPU, `input` for pen tablets (with the documented
  `--device-if=all:!has-input-device` fallback), PulseAudio, network (update
  check, the ffmpeg download, the WebSocket API server), `home` (atomic
  saves write `model.aom.tmp` beside the model, model3.json bundles read
  their sibling files, audio layers are relative to the .aom), and the
  Discord IPC socket. Reviewers will question `home`; dropping it needs the
  app to save in place through the portal and to ask for the model folder.
- No `flathub.json`: it is only needed to limit architectures, and both
  x86_64 and aarch64 build.

Open points for the reviewers: the app downloads and runs an ffmpeg build
for video export (consider the ffmpeg runtime extension or the system
ffmpeg through a portal instead), and its update notice says "install the
new .deb or .rpm" when run from a package; it should name Flathub, or the
check should be off inside Flatpak (`FLATPAK_ID` is set there).

Steps (a human maintainer, per blocker 2):

1. Resolve blocker 3 (repo name or exception).
2. Write the manifest by hand from the Flathub docs, build it with
   `flatpak-builder --user --install --force-clean`, and run
   `flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest
   <manifest>` and the `repo` check.
3. Fork flathub/flathub with all branches, branch from `new-pr`, add the
   manifest and `cargo-sources.json`, and open the PR against `new-pr`
   titled "Add io.github.Ayatsuri2D.Ayatsuri2D", disclosing the project's AI
   use.
4. After acceptance, accept the collaborator invite, log in to flathub.org
   with a GitHub account that is an admin of the Ayatsuri2D organization,
   and verify the app in the Developer Portal.

## Every release from then on

1. Release as usual; the CHANGELOG section becomes the metainfo release
   entry (`cargo xtask metainfo`, checked by `cargo xtask ci`).
2. If Cargo.lock changed: `packaging/flatpak/gen-cargo-sources.sh` and
   `packaging/fedora/license-summary.sh` (update `License:` in the spec).
3. Bump the version in the PKGBUILD, spec and Flathub manifest; refresh the
   checksums; upload the vendor tarball to the release.
