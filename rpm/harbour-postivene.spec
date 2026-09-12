# Sailfish/OBS packaging for Postivene.
#
# Modeled on real-world prior art for Rust+Qt5/QML Sailfish apps -- notably
# Whisperfish's rpm/harbour-whisperfish.spec, which docs/PROJECT.md names as
# the architectural and build-tooling template -- but simplified: Postivene
# has no C/C++ vendored dependencies of its own
# (no sqlcipher, no protobuf, etc.), just the Rust workspace under rust/,
# the qml/ tree, and a *separately obtained* deltachat-rpc-server binary
# (scripts/fetch-rpc-server.sh, see vendor/deltachat-rpc-server/).
#
# Two build modes:
#   sfdk build                  -- crates are fetched from crates.io by the
#                                  build engine, which has network access.
#   sfdk build -- --with vendor -- crates come from a pre-generated
#                                  vendor.tar.xz; required on OBS/Chum,
#                                  which build without network. Generate
#                                  the two extra sources with
#                                  scripts/vendor-crates.sh.
#
# Harbour's listing rules apply here: the package name, the install paths
# and every Requires are constrained by them, and ci/harbour-check.sh
# fails a build that breaks one. docs/HARBOUR.md is the map, including the
# two rules this package still breaks.

%bcond_with vendor

# Upstream publishes no 32-bit x86 musl build of deltachat-rpc-server, so
# i486 (SDK emulator) packages cannot bundle one: the app there needs a
# server supplied via POSTIVENE_RPC_SERVER instead. Every device arch
# (aarch64, armv7hl) bundles the server as usual.
%ifarch %ix86
%define bundle_rpc_server 0
%else
%define bundle_rpc_server 1
%endif

# Harbour requires the `harbour-` prefix and lowercase throughout; the
# name users see is the .desktop file's Name= and the Store's Title field,
# not this.
Name:       harbour-postivene
Summary:    Native SailfishOS client for Delta Chat
Version:    0.1.0
Release:    1
# Postivene's own code is GPL-3.0-or-later; the bundled
# deltachat-rpc-server is upstream's unmodified MPL-2.0 binary, and the tag
# describes the contents of the binary package.
%if 0%{?bundle_rpc_server}
License:    GPL-3.0-or-later AND MPL-2.0
%else
License:    GPL-3.0-or-later
%endif
Group:      Qt/Qt
URL:        https://github.com/muhnschein/postivene
Source0:    %{name}-%{version}.tar.gz
%if %{with vendor}
Source1:    vendor.tar.xz
Source2:    vendor.toml
%endif

# Unversioned, and only what Harbour's allowed_requires.conf lists: the
# validator hands each whitespace-separated token to its allow-list, so a
# versioned dependency arrives as three of them and the operator and the
# version are both rejected. Harbour derives its own compatibility range
# from these, so the OS floor this app needs -- 4.5.0, for the
# `[X-Sailjail]` section to be honoured -- goes in the submission form's
# "From OS version" field instead.
#
# No libsailfishapp-launcher: that package provides `sailfish-qml`, which
# only a QML-only app is launched through, and requiring it without using
# it is an error of its own.
Requires:   sailfishsilica-qt5
# One package per QML module the UI imports beyond Silica: QtMultimedia,
# QtSensors, QtGraphicalEffects, Nemo.Thumbnailer, Nemo.Notifications.
# All on Harbour's allowed list, and all on every device image -- but a
# package that relies on a module and does not say so depends on it by
# luck.
Requires:   qt5-qtdeclarative-import-multimedia
Requires:   qt5-qtdeclarative-import-sensors
Requires:   qt5-qtgraphicaleffects
Requires:   nemo-qml-plugin-notifications-qt5
Requires:   nemo-qml-plugin-thumbnailer-qt5
# Nemo.DBus, for the name a tapped notification calls back to.
Requires:   nemo-qml-plugin-dbus-qt5
# Nemo.Configuration, for the settings that belong to no profile: they
# live in dconf, so a change on the settings page reaches every open page.
Requires:   nemo-qml-plugin-configuration-qt5
# Sailfish.WebView, for running a webxdc app and for the store it comes
# from (qml/pages/Webxdc*Page.qml). The only two pages that name the type,
# so a device without the browser engine loses those and nothing else --
# but the package says it needs it rather than hoping.
#
# The other two are not imported here: Silica's own WebView.qml imports
# Sailfish.WebView.Popups and .Pickers, and a WebView that cannot resolve
# them is a page that will not load at all.
Requires:   sailfish-components-webview-qt5
Requires:   sailfish-components-webview-qt5-popups
Requires:   sailfish-components-webview-qt5-pickers

# Harbour allows no Provides: at all, and rpm generates one from any shared
# library it finds in the package. Neither of the app's private directories
# holds one today; this keeps intake clean if one is ever added there.
%define __provides_exclude_from ^(%{_datadir}/%{name}|%{_libexecdir}/%{name})/.*$

# Sailfish ships Rust 1.75.0 (sailfishos/rust); rust/Cargo.lock is kept in
# the v3 lockfile format because Cargo only learned to read v4 in 1.78.
# `rust-std-static` is the virtual provide of the native std library; the
# *cross* std for the target triple comes from
# rust-std-static-%%{rusttarget}, which the SDK target normally already has.
# If a build fails with "can't find crate for `std`", install it into the
# tooling, the way Whisperfish's build docs describe:
#   sfdk tools exec <tooling> zypper in rust-std-static-%%{rusttarget}
BuildRequires:  rust >= 1.75
BuildRequires:  rust-std-static >= 1.75
BuildRequires:  cargo >= 1.75
# qmetaobject-rs compiles C++ glue (the cpp/cpp_build crates) against Qt.
BuildRequires:  gcc-c++
BuildRequires:  git
BuildRequires:  desktop-file-utils
BuildRequires:  pkgconfig(Qt5Core)
BuildRequires:  pkgconfig(Qt5Qml)
BuildRequires:  pkgconfig(Qt5Quick)
# QAudioRecorder, for the voice recorder (postivene-shim/src/recorder.rs).
# The runtime package it links, qt5-qtmultimedia, is one rpm derives from
# the binary itself; Harbour allows it.
BuildRequires:  pkgconfig(Qt5Multimedia)
# lrelease, which compiles the translation catalogs in %%build.
BuildRequires:  qt5-qttools-linguist

%ifarch %arm
%define rusttarget armv7-unknown-linux-gnueabihf
%endif
%ifarch aarch64
%define rusttarget aarch64-unknown-linux-gnu
%endif
%ifarch %ix86
# NOTE: upstream publishes no 32-bit x86 deltachat-rpc-server build, so the
# i486 emulator target cannot be packaged; use the x86_64 emulator.
%define rusttarget i686-unknown-linux-gnu
%endif
%ifarch x86_64
%define rusttarget x86_64-unknown-linux-gnu
%endif

# How many cargo jobs to run inside scratchbox2, and only there: outside
# sb2 -- a native OBS worker -- cargo picks its own. Four, which device
# builds run green on the 5.2 SDK and which takes the build from 142 s to
# 82 s. It was one for a long time, because cargo was once seen to
# futex-wait forever on an unreaped child at four while qmetaobject's C++
# glue compiled; that is why the number is a define rather than a
# constant, so a build that ever hangs again drops back with
# --define "jobs 1" instead of a patch. .github/workflows/rpm.yml passes
# its cargo_jobs input this way, and docs/BUILDING.md has the numbers.
%{!?jobs: %global jobs 4}

# Where cargo leaves the binary. Under sb2, SB2_RUST_TARGET_TRIPLE (see
# %%build) makes it write to target/<triple>/release; a native build -- an
# OBS worker that is not cross-compiling -- gets plain target/release.
# Both are checked at install time rather than guessed here.
%define builddir rust/target/%{rusttarget}/release
%define nativedir rust/target/release
%define appdatadir %{_datadir}/%{name}
%define appexecdir %{_libexecdir}/%{name}

%description
Postivene is a Silica/QML SailfishOS client for Delta Chat. It contributes
only the UI, Sailfish platform integration, and packaging; all
IMAP/SMTP/MIME/encryption logic is delegated to a bundled
deltachat-rpc-server binary, spoken to over JSON-RPC on stdio. See
docs/PROJECT.md in the source tree for the full project scope and, just as
importantly, its non-goals.

%prep
%setup -q -n %{name}-%{version}
%if %{with vendor}
# vendor.tar.xz contains rust/vendor/ (cargo vendor output); vendor.toml is
# the [source] replacement stanza cargo prints when generating it.
%setup -q -T -D -a 1 -n %{name}-%{version}
mkdir -p rust/.cargo
install -m 644 %{SOURCE2} rust/.cargo/config.toml
%endif

%build
rustc --version
cargo --version

# Cross-compiling Rust under Sailfish's scratchbox2 (sb2) requires the
# Docker build engine (the VirtualBox build engine does not support it);
# see docs/BUILDING.md. Under sb2 the accelerated rustc would otherwise
# emit host (x86) code, so tell it the real target explicitly -- same
# mechanism Whisperfish's spec uses (see the xulrunner-qt5.spec comment it
# cites).
export SB2_RUST_TARGET_TRIPLE=%{rusttarget}

# qttypes' build script shells out to `qmake -query` by default, but rust
# build scripts running under sb2 cannot reliably exec the target's qmake
# binary. Both env vars set together make qttypes skip qmake entirely and
# detect the Qt version from qtcoreversion.h (verified against the crate's
# build.rs) -- these are the target-rootfs paths as seen from inside sb2.
export QT_INCLUDE_PATH=%{_includedir}/qt5
export QT_LIBRARY_PATH=%{_libdir}

# SBOX_SESSION_DIR is set by sb2 itself inside build sessions, and is what
# tells a cross build here from a native one. It gates the job count the
# preamble defines, and the linker override below.
# Build scripts and proc-macros are compiled for the tooling's own
# architecture, and rustc links them by calling plain `cc` -- which sb2
# rewrites to the *cross* compiler, producing "unrecognized command-line
# option '-m32'" and killing the build before the first crate is done.
# scratchbox2 exposes the native compiler as `host-gcc` (SBOX_HOST_GCC_NAME
# in the target's sb2.config) precisely for this; point the host triple's
# linker at it. Outside sb2 nothing is overridden.
if [ -n "${SBOX_SESSION_DIR:-}" ]; then
    host_triple=$(rustc -vV | sed -n 's/^host: //p')
    export "CARGO_TARGET_$(echo "$host_triple" | tr 'a-z-' 'A-Z_')_LINKER"=host-gcc
fi

# A parallel link under sb2 can lose an object file it has just written
# ("symbols.o: No such file or directory") when its temporaries go through
# the shared /tmp. Whisperfish's spec keeps them in the build directory for
# the same reason.
export TMPDIR=${TMPDIR:-"$PWD/.tmp"}
mkdir -p "$TMPDIR"

# Deliberately *no* `--target`: under sb2 that would make cargo treat this
# as a cross build and look for a target std it cannot find. Instead
# SB2_RUST_TARGET_TRIPLE (above) tells the sb2-accelerated rustc what to
# emit, and cargo still writes the result to target/<triple>/release --
# the same convention upstream Sailfish Rust apps rely on (Whisperfish's
# spec likewise passes no --target).
#
# From inside rust/, not the source root. cargo reads .cargo/config.toml
# from the directory it runs in and that directory's parents, never from
# beside the manifest it is pointed at -- and the vendored-sources stanza
# %%prep installs lives at rust/.cargo/config.toml. Run from the root that
# stanza was never read, and an --offline build failed on the first crate.
# target/ is the workspace's either way, so %%install's paths are the same.
(
cd rust
cargo build \
    ${SBOX_SESSION_DIR:+-j%{jobs}} \
    --release \
    --locked \
%if %{with vendor}
    --offline \
%endif
    --manifest-path Cargo.toml \
    --package postivene-app
)

# The translation catalogs: translations/postivene-<lang>.ts, compiled
# here into the .qm files the app loads. Made at build time rather than
# checked in, as the binary is.
./scripts/release-translations.sh

%install
rm -rf %{buildroot}

builddir=%{builddir}
[ -x "$builddir/%{name}" ] || builddir=%{nativedir}
install -Dm 755 "$builddir/%{name}" \
    %{buildroot}%{_bindir}/%{name}

# QML UI, installed under our own app-private data dir (not /usr/bin) so
# postivene-app's qml_dir() lookup (POSTIVENE_QML_DIR env var, then this
# path, then a source-tree-relative fallback for local dev) finds it.
# Only what the engine reads. A stray editor backup or .orig under qml/
# would otherwise ship, and Harbour would then have an opinion about it.
# The .png files are qml/art/: the face masks the first screen draws and
# the pictures the introduction puts over each fact, painted ahead of
# time by tools/faces/ (docs/PROJECT.md).
(cd qml && find . -type f \( -name '*.qml' -o -name '*.js' -o -name '*.png' -o -name qmldir \) -exec \
    install -Dm 644 "{}" "%{buildroot}%{appdatadir}/qml/{}" \; )

# The compiled catalogs, beside the QML: postivene-app looks for them at
# this path, the way it looks for qml/ above. Only the .qm files; the .ts
# sources they were built from stay in the tree.
(cd translations && find . -type f -name 'postivene-*.qm' -exec \
    install -Dm 644 "{}" "%{buildroot}%{appdatadir}/translations/{}" \; )

# Bundled deltachat-rpc-server: see vendor/deltachat-rpc-server/ and
# scripts/fetch-rpc-server.sh. Installed under a private libexec dir rather
# than %%{_bindir} so it can't be picked up as a generic system
# "deltachat-rpc-server" by anything else that happens to look for one on
# PATH -- postivene-app defaults to this exact path.
#
# %%{_target_cpu}, *not* %%{_arch}: rpm canonicalises every armv7h* target to
# the arch name "arm" (rpm's installplatform: `armv7h*) CANONARCH=arm`), so
# %%{_arch} would look for vendor/deltachat-rpc-server/arm/ while
# scripts/fetch-rpc-server.sh writes armv7hl/. %%{_target_cpu} keeps the
# Sailfish arch names (armv7hl, aarch64, x86_64) that the script uses.
%if 0%{?bundle_rpc_server}
install -Dm 755 vendor/deltachat-rpc-server/%{_target_cpu}/deltachat-rpc-server \
    %{buildroot}%{appexecdir}/deltachat-rpc-server
%endif

# LICENSE is Postivene's own GPLv3 text. SOURCE.md discharges MPL-2.0
# clause 3.2(a) for bundling deltachat-rpc-server's Executable Form:
# recipients must be told how to obtain its Source Code Form, and it names
# the upstream repository, the exact tag, and the sha256 of every binary.
install -Dm 644 vendor/deltachat-rpc-server/SOURCE.md \
    %{buildroot}%{appdatadir}/vendor/deltachat-rpc-server/SOURCE.md
install -Dm 644 LICENSE \
    %{buildroot}%{appdatadir}/LICENSE

desktop-file-install \
    --dir %{buildroot}%{_datadir}/applications \
    %{name}.desktop

install -Dm 644 icons/86x86/%{name}.png \
    %{buildroot}%{_datadir}/icons/hicolor/86x86/apps/%{name}.png
install -Dm 644 icons/108x108/%{name}.png \
    %{buildroot}%{_datadir}/icons/hicolor/108x108/apps/%{name}.png
install -Dm 644 icons/128x128/%{name}.png \
    %{buildroot}%{_datadir}/icons/hicolor/128x128/apps/%{name}.png
install -Dm 644 icons/172x172/%{name}.png \
    %{buildroot}%{_datadir}/icons/hicolor/172x172/apps/%{name}.png
install -Dm 644 icons/256x256/%{name}.png \
    %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/%{name}.png

%files
%defattr(-,root,root,-)
%{_bindir}/%{name}
%{appdatadir}
%if 0%{?bundle_rpc_server}
# The directory as well as the file: listing only the file leaves
# /usr/libexec/%%{name} behind on uninstall.
%dir %{appexecdir}
%{appexecdir}/deltachat-rpc-server
%endif
%{_datadir}/applications/%{name}.desktop
%{_datadir}/icons/hicolor/*/apps/%{name}.png
