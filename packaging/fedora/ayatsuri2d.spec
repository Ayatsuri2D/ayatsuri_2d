# Ayatsuri2D for Fedora and COPR, built offline from the release tarball and
# a tarball of the vendored crates (packaging/vendor.sh writes both).
#
# Vendoring: the Rust guidelines say packages SHOULD NOT bundle crates where
# system crates exist. The egui/eframe/wgpu stack, cpal, rodio, psd, evdev
# and others this app needs are not packaged in Fedora, so the crates are
# vendored, each declared as Provides: bundled(crate(...)) through
# %%cargo_vendor_manifest. See packaging/DISTRIBUTIONS.md.

Name:           ayatsuri2d
Version:        0.6.1
Release:        %autorelease
Summary:        Open 2D puppet studio for Live2D-style models

# our own code: the editor is GPL-3.0-only, the runtime crates MIT, the C ABI Apache-2.0
SourceLicense:  GPL-3.0-only AND MIT AND Apache-2.0
# The binaries: our licences AND those of every statically linked crate.
# Regenerate with packaging/fedora/license-summary.sh when Cargo.lock changes,
# and compare with the %%cargo_license_summary output in the build log.
# LICENSE.dependencies in the package has the full per-crate breakdown.
License:        %{shrink:
    GPL-3.0-only AND
    MIT AND
    Apache-2.0 AND
    ((Apache-2.0 OR MIT) AND BSD-3-Clause) AND
    ((MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0) AND
    ((MIT OR Apache-2.0) AND Unicode-3.0) AND
    (0BSD OR MIT OR Apache-2.0) AND
    (Apache-2.0 AND ISC) AND
    (Apache-2.0 AND MIT) AND
    (Apache-2.0 OR GPL-2.0-only) AND
    (Apache-2.0 OR ISC OR MIT) AND
    (Apache-2.0 OR MIT) AND
    (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) AND
    (BSD-2-Clause OR Apache-2.0 OR MIT) AND
    BSD-3-Clause AND
    (BSD-3-Clause OR Apache-2.0) AND
    CDLA-Permissive-2.0 AND
    ISC AND
    (MIT OR Apache-2.0) AND
    (MIT OR Apache-2.0 OR Zlib) AND
    (MIT OR Zlib OR Apache-2.0) AND
    MPL-2.0 AND
    Unicode-3.0 AND
    (Unlicense OR MIT) AND
    Zlib AND
    (Zlib OR Apache-2.0 OR MIT)
}

URL:            https://github.com/Ayatsuri2D/ayatsuri_2d
Source0:        %{url}/archive/v%{version}/ayatsuri_2d-%{version}.tar.gz
# packaging/vendor.sh --ref v%%{version}; attach it to the GitHub release so
# COPR and reviewers can fetch it, then this line can become a URL
Source1:        %{name}-%{version}-vendor.tar.xz

%global appid io.github.Ayatsuri2D.Ayatsuri2D

ExclusiveArch:  %{rust_arches}

BuildRequires:  cargo-rpm-macros >= 25
BuildRequires:  pkgconfig(alsa)
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

# the window system, xkbcommon and the GPU are opened at run time (dlopen),
# so the automatic ELF dependencies do not see them
Requires:       hicolor-icon-theme
Requires:       libxkbcommon
Requires:       libwayland-client
Requires:       (vulkan-loader or libglvnd-egl)
Recommends:     libX11
Recommends:     libXcursor
Recommends:     libXi
Recommends:     libXrandr
Recommends:     mesa-vulkan-drivers
Recommends:     xdg-desktop-portal
Suggests:       ffmpeg-free

%description
Ayatsuri2D is an open, native 2D puppet studio: import artwork from
Photoshop, Krita or Clip Studio Paint, mesh and rig it with warp and
rotation deformers, add physics and expressions, animate it, and export
Cubism-compatible moc3 models for apps like VTube Studio.

%prep
%autosetup -n ayatsuri_2d-%{version} -p1 -a1
%cargo_prep -v vendor
# the MIT crates each carry their own copyright lines; give them distinct names
for c in core format import openmoc3; do
    cp -p crates/$c/LICENSE LICENSE-MIT-$c
done
cp -p crates/openmoc3/NOTICE NOTICE-openmoc3

%build
%cargo_build
%{cargo_license_summary}
%{cargo_license} > LICENSE.dependencies
%{cargo_vendor_manifest}

%install
install -Dpm 0755 target/rpm/ayatsuri -t %{buildroot}%{_bindir}
install -Dpm 0755 target/rpm/ayatsuri-launcher -t %{buildroot}%{_bindir}
install -Dpm 0644 packaging/linux/%{appid}.desktop -t %{buildroot}%{_datadir}/applications
install -Dpm 0644 packaging/linux/%{appid}.metainfo.xml -t %{buildroot}%{_metainfodir}
install -Dpm 0644 packaging/linux/%{appid}.mime.xml %{buildroot}%{_datadir}/mime/packages/%{appid}.xml
for icon in assets/app-icon/hicolor/*/apps/ayatsuri2d.png; do
    install -Dpm 0644 "$icon" "%{buildroot}%{_datadir}/icons/${icon#assets/app-icon/}"
done

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{appid}.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml
%cargo_test

%files
%license LICENSE LICENSING.md LICENSE-MIT-* NOTICE-openmoc3
%license LICENSE.dependencies
%license cargo-vendor.txt
%doc README.md CHANGELOG.md
%{_bindir}/ayatsuri
%{_bindir}/ayatsuri-launcher
%{_datadir}/applications/%{appid}.desktop
%{_metainfodir}/%{appid}.metainfo.xml
%{_datadir}/mime/packages/%{appid}.xml
%{_datadir}/icons/hicolor/*/apps/ayatsuri2d.png

%changelog
%autochangelog
