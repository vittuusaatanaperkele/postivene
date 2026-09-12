# Building, testing, and packaging

Engineering standards and the build procedure. The reference for the former
is [clove](https://github.com/muhnschein/clove)'s §9: pinned toolchain,
pedantic lints, CI-parity `make` targets, tests that drive the real
binaries.

## Toolchains

`rust-toolchain.toml` pins 1.94.1 so lint results are reproducible; the
device floor is **1.75.0**, enforced by CI's `msrv` job with warnings
denied. It is a rustup mechanism, and the Sailfish SDK's cargo ignores it.

`rust/Cargo.lock` stays at **v3**: cargo learned v4 in 1.78 and the SDK's
cargo 1.75 cannot read it, while a `cargo update` on a modern host rewrites
it silently. `ci/check-lockfile.sh` catches that.

## Lints

Workspace-level, so a bare `cargo clippy` fails the way CI does:
`clippy::all` and `pedantic` at deny, `unwrap_used`/`expect_used` denied
outside tests, `missing_docs` and `unsafe_code` denied.

`unsafe_code` is deny rather than forbid because the Qt harness tests need
`env::set_var` before Qt initialises, and because two things the app does
have no safe binding: installing a `QTranslator`, and recording a voice
message through `QAudioRecorder`, neither of which qmetaobject wraps.
Those are the two `cpp!` files in the tree, `postivene-app/src/translations.rs`
and `postivene-shim/src/recorder.rs`, and the C++ build step in each
crate's `build.rs` exists for them alone. Every exception is at the
narrowest scope and says why, and every block is short enough to be
checked by reading.

The recorder is also why the host build needs `qtmultimedia5-dev` (the
`Makefile` lists the packages) and the spec `pkgconfig(Qt5Multimedia)`:
the shim links `libQt5Multimedia`, which Harbour allows.

`rust/clippy.toml` bans two methods that have already caused device-only
failures: `tokio::runtime::Runtime::new` (must go through `CoreRuntime`) and
`qmetaobject::single_shot` (truncates sub-second `Duration`s).

## Testing

Postivene parses almost nothing — protocol and crypto are the core's, and
the subprocess we talk to is one we spawned. The failure mode is misreading
the core's JSON, or calling it wrongly, with nothing noticing until the app
is on a phone. The tests aim at that.

`make check` runs all of it from a clean checkout: no phone, account or
Sailfish SDK. Not quite offline — `msrv` fetches the 1.75 toolchain the
first time, and `deny` wants the advisory database.

**How the suite is run.** Through `cargo nextest`, which puts each test in
its own process rather than running one test binary at a time. That matters
here more than in most workspaces: there are over a hundred test binaries,
almost every one starts a Qt engine and then waits on real timers, so under
`cargo test` the suite spent about ten minutes mostly idle and serialised.
The same 207 tests take about two and a half minutes on four cores. Nothing
is shared between them — the webxdc host binds port 0 and lets the kernel
choose, and the QML probes copy the tree into a directory named for their
own pid — which is what makes running them at once safe.

```
cargo install --locked cargo-nextest
```

Without it `make test` still runs everything, the slow way, because
`make check` has to work on a laptop that has installed nothing extra.
`rust/.config/nextest.toml` carries the rest: a 30-second slow warning and
a two-minute kill, so a Qt test waiting on a signal that never arrives is
reported against its own name instead of hanging until the job's timeout;
and no retries, because a test that passes on the second attempt is a
defect this project wants to see.

nextest does not run doctests. `cargo test --doc` runs beside it in both
the Makefile and `ci.yml`, and `ci/packaging-lint.sh` fails a tree where
one is there without the other — there are no doctests today, so nothing
would notice the first one silently never running.

1. **Transport unit tests** against a fake stdio server.
2. **Protocol-contract tests** against a recording double that journals
   every request, pinning the call sequence of each onboarding action.
3. **Qt event-loop tests** under `QT_QPA_PLATFORM=offscreen`.
4. **QML load tests** against stub Silica components (`tests/silica-stubs/`):
   the real page files, driven by `objectName`. The stubs imitate no layout,
   so nothing here says a page *looks* right. Silica's `EnterKey` attached
   property cannot be stubbed — QML forbids capitalised property names and
   `qmetaobject` cannot register attached types — so pages using it cannot
   be loaded. Put what such a page shows in a component that can be, and lay
   that component out with bindings rather than a `Column`: a positioner
   sizes itself in a polish pass, which never runs without a window, so its
   geometry reads as zero.
5. **Static QML tests** (`tests/qml_syntax.rs`) for what no host-Qt run
   can see: Qt 5.6 rules that Qt 5.15 accepts silently, and the rules the
   tree holds itself to -- every string the other end chose is drawn as
   plain text (Silica's own headers cannot be, so `ConversationHeader`
   exists), file URLs are encoded a segment at a time, only the picker
   pages import `Sailfish.Pickers`, and every `model.<role>` a delegate
   binds is one its model has.
6. **Real-core integration** (`real_server`, `real_core`), gated on
   `DELTACHAT_RPC_SERVER`, offline. `real_core.rs` distinguishes a request
   the real core could not decode from one it could not deliver.
7. **Packaging checks** (`ci/packaging-lint.sh`): spec parses, desktop
   entry validates, shell scripts clean, every translation catalog
   current and compiling cleanly with `lrelease`, every `docs/*.md` a
   comment points at exists. Locally a missing tool is
   a SKIP; CI sets `PACKAGING_LINT_STRICT=1` so it is a failure there, as
   `HARBOUR_CHECK_STRICT=1` already does for the Harbour check.

8. **Report-script tests** (`ci/sonar-report-selftest.sh`): the one script
   here that talks to a service outside GitHub, run against a stub server
   that answers SonarQube's four endpoints. It cannot be tested any other
   way -- the analysis it reads does not exist when the tests run, and this
   project's CI cannot reach `sonarcloud.io`, which is the reason the script
   exists at all.

9. **Runner-setup tests** (`ci/apt-install-selftest.sh`): the rule deciding
   which apt sources every CI job keeps, proved on a directory of the
   test's own. Getting it backwards deletes the archive the jobs install
   from, which fails everything.

Aspiration, tracked not gated: test volume exceeds source volume.

## Static analysis

SonarQube Cloud reads the tree on every pull request
(`.github/workflows/build.yml`, configured by `sonar-project.properties`).
It is a **report, not a gate**: `ci.yml` decides what is allowed in, and
nothing Sonar says can turn a red build green or a green build red. Keeping
that boundary is why it is a separate workflow -- folding it into the gate
would make a hosted service part of the rule that a green `make check` on a
laptop is a green CI.

The scanner **imports** coverage; it does not measure it. `make
sonar-reports` writes `rust/target/sonar/lcov.info` with `cargo llvm-cov`
over the whole workspace, and the workflow runs it before the scan. Without
it the reading is a confident 0.0% rather than "no data", which is what it
read for as long as nothing wrote a report. The target needs
`cargo-llvm-cov`, so it is opt-in rather than part of `make check`. It runs
the suite under `cargo-nextest` when that is installed too, for the reason
`make test` does: cargo-llvm-cov's own runner is `cargo test`, one binary
at a time, and instrumented that was ten minutes of the scan job. Without
nextest the report is still written, the slow way.

```
rustup component add llvm-tools-preview
cargo install --locked cargo-llvm-cov cargo-nextest
make sonar-reports
```

Clippy findings are **not** handed over, and Sonar's own Clippy pass is off
(`sonar.rust.clippy.enabled=false`), for two different reasons. Sonar's pass
invokes cargo where it finds the project, and this workspace is under
`rust/`, not at the root, so it would run a different clippy from the one
that gates this project -- or none. And a report of our own would be empty:
`make lint` denies warnings, so a warning in this project's code fails the
gate and never reaches a branch Sonar analyses. One was produced and held
four diagnostics, all in `third_party/qmetaobject`, which is excluded
anyway. Producing it cost a `cargo clean` and a full recompile inside the
scan job.

`sonar.tests` separates the fixtures from the application, so coverage and
duplication are measured on what ships. That matters more here than in most
trees, because the rule above is that test volume exceeds source volume:
indexed as main sources, the fixtures were most of what every ratio was
computed over. `sonar.exclusions` drops the vendored crates, the patched
qmetaobject, the rendered icons, and `translations/` -- a Qt catalog is
named `.ts`, so the scanner reads thirty-nine of them as TypeScript.
`sonar.coverage.exclusions` keeps `qml/` out of the coverage arithmetic
alone: nothing there can produce a report, and one changed line of QML
JavaScript otherwise reads as 0% coverage on new code and fails the gate.
`sonar.issue.ignore.multicriteria` closes the findings the project has
decided against, one rule on one path each, with the reason beside it:
qmetaobject's glob import, cognitive complexity in tests and in the fake
servers, and a character-class name Sonar took for a repeated literal.

The scanner uploads a report and exits; the server processes it afterwards,
so the run that produced an analysis finishes knowing nothing about its
result. `scripts/sonar-report.sh` asks the server from the runner that just
fed it and prints the quality gate, the measures and the open issues into
the job log and the step summary. It reports and never gates: the step is
`continue-on-error`, so a Sonar outage costs a warning, not a build.

## CI

`ci.yml` is the gate and runs what `make check` runs. Three things about
the runners are worth knowing.

**Packages come through `ci/apt-install.sh`**, not a bare `apt-get`.
`apt-get update` exits non-zero when *any* configured repository fails, and
the runner image ships several this project never installs from. On
2026-09-09 Google Chrome's index served a hash that did not match its own
Release file, and every job died before installing anything or running a
test; nothing in this repository had changed. The script drops the
third-party lists first, keeping Ubuntu's wherever the image puts them --
a list survives only if something in it names an `ubuntu.com` host, which
is what stops it deleting the archive it is about to install from.

**The Rust jobs cache their `target/`** (`Swatinem/rust-cache`, scoped to
the `rust` workspace). Every job used to compile the whole dependency
graph from nothing on every push. `msrv` carries a cache key of its own
because it builds with `+1.75.0` while the action keys on the default
toolchain, and without it the two would share a slot and neither would
ever hit. `CARGO_INCREMENTAL: 0` because a runner compiles once and throws
the machine away, so incremental state is written, cached and never read.

Caching was measured and is worth less than it looks: with a warm cache
clippy compiles the workspace in about twenty seconds, but the `test` job
barely moved, because compilation was never its cost. Ten of its twelve
minutes were the suite waiting on timers, which is what nextest addresses
above.

**The test job installs `cargo-nextest`** and runs the suite under the
`ci` profile, which differs from a laptop's in two ways: `fail-fast` is
off, because CI is asked once and should report everything it knows; and
failures are printed where they happen and again at the end, because in a
two-hundred-line log the summary is what anyone reads.

## The art the first screens draw

The onboarding screens draw pictures rather than laying out avatars
(`docs/PROJECT.md`), and the pictures live in `qml/art/`: two masks for
the field of faces, one per orientation, painted by
`tools/faces/faces.py`, and one per fact of the introduction, painted by
`tools/faces/scenes.py` with the same rasteriser. All of them are
**committed**: like the compiled catalogs they are generated but tracked,
so a build needs neither the painters nor a display. `make faces`
repaints them all -- Python 3 and its standard library, nothing to
install -- and both painters are deterministic, so the art changes only
when they do. Run it when a painter changes, look at what it wrote (the
masks are red and green on black; the app tints them), and commit the
result. `tests/qml_welcome.rs` and `tests/qml_intro.rs` check that they
are there in the shape the shaders read.

## Translations

The strings are the `qsTr()` calls in `qml/`; `translations/postivene.ts`
is the untranslated source catalog and `translations/postivene-<lang>.ts`
one catalog per language Sailfish ships in. `scripts/update-translations.sh`
regenerates all of them from the source in one `lupdate` run, so a new
string turns up as `unfinished` in every language at once, and
`ci/packaging-lint.sh` fails when a committed catalog differs from what
that run produces. `tests/translation_catalogs.rs` fails when a string in
any language is left untranslated, so a new string is not done until every
catalog has it.

The app loads `postivene-<lang>.qm`, which `scripts/release-translations.sh`
compiles with `lrelease` -- in the RPM's `%build`, and locally with
`make translations`, which leaves them beside the `.ts` files where a
source-tree run finds them. `lupdate` and `lrelease` are Debian's
`qttools5-dev-tools`, and the SDK's `qt5-qttools-linguist`; the app's own
test compiles the German catalog, so the package is a test dependency too.

`<lang>` is what `QTranslator` matches against the reader's locale from
the most specific form down: `de` serves every German locale, `pt_BR`
only Brazil, and a language with no catalog gets the English one -- the
strings are English already, and that catalog holds their plural forms.
To add one, write the three-line header `update-translations.sh` documents to
`translations/postivene-<lang>.ts` and run the script; `lupdate` fills in
every string with as many plural forms as that language has.

## Dependencies

Few, and each for a reason. `cargo tree` on the app is the list; this is
why each entry is there, so that a proposal to drop one starts from what
it would cost.

| Crate | What it is for | Why it stays |
|---|---|---|
| `tokio` | the server subprocess, its pipes, the event loop | the transport is async; `process` is what reaps the child |
| `serde`, `serde_json` | the JSON on the wire | the contract with the core is JSON-RPC |
| `qmetaobject` (vendored), `qttypes`, `cpp`, `cpp_build` | Qt from Rust | the whole UI hangs off them; `default-features = false` keeps its `log` bridge out |
| `chrono` | the viewer's timezone, for the day headings | `std` has none, and the alternative is `localtime_r`, which `unsafe_code` denies |
| `qrcode` | an invite drawn as a code | one crate, no dependencies |
| `rqrr` (+ `g2p`, `lru`) | a code read off the camera | a QR decoder is not a small thing to vendor |

`tokio`'s `net` feature is what the webxdc host binds its loopback socket
with, and it brings `socket2` -- tokio's own platform layer for sockets,
and the only crate the whole feature adds. The alternative was a zip
reader and an inflate implementation, to unpack an app the core can
already read.

What is not there any more, and where the line is: `thiserror` was two
crates for a dozen lines of `Display`, so the transport's errors are
written out; the fake servers build their tokio runtime by hand, so
`macros` is a dev-dependency and the app's build carries no
`tokio-macros`; qmetaobject's `log` feature is off. `serde`'s `derive`
could go the same way for one crate less, at the cost of hand-written
`Deserialize` for the four wire types -- more code than it saves, so it
stays. Everything else is either the vendored qmetaobject's own
(`lazy_static`, `syn 1`) or a build script's (`cc`, `regex`, `semver`,
`rustversion`), and the platform-gated crates in `Cargo.lock` --
`windows-*`, `wasm-bindgen`, `js-sys` -- are resolved for other targets
and never built here.

## Comments

One sentence where one will do. A comment states what is true now and why.
It is not a changelog, a bug report, or a story about how the code got here
— that belongs in git history. Delete a comment rather than update it into a
history of its own subject.

## Packaging: the supported path

`.github/workflows/rpm.yml` builds a device RPM unattended on an
`ubuntu-latest` runner, from a `docker run` of the Sailfish SDK. Dispatch
it from the Actions tab (the SDK version and cargo's job count are inputs)
or push a `v*` tag. It builds **aarch64**, which is the only architecture
this project targets and the only one it has ever built.

```sh
./scripts/fetch-rpc-server.sh                        # bundled server binaries
mb2 -t SailfishOS-<ver>-<arch> -X build-init
mb2 -t SailfishOS-<ver>-<arch> -X build --no-check
```

- `-X` (`--no-fix-version`) uses the spec's `Version:` rather than deriving
  one from git tags. It is needed **by `build-init`** too: without it that
  step gives up at version-fixing and never writes `.mb2/spec`, so `build`
  fails identically and the flag looks innocent.
- `build-init` must precede `build`, which queries `.mb2/spec` within a
  second of starting.
- `--no-check`: the tests are host-oriented, and the spec has no `%check`.

Environment requirements, each of which cost an attempt:

- **Mount the tree inside the SDK user's home** (`/home/mersdk/<name>`), not
  `/build` or `/share`. rpm runs under scratchbox2, which redirects
  unrecognised absolute paths into the target rootfs; with the tree
  elsewhere `mb2` writes `.mb2/spec` outside and rpm reads inside. A file
  that exists and cannot be opened is the signature. The directory must keep
  the package's name — `mb2` derives the package from it.
- **The i686 rustlib at the SDK's own `/usr/lib/rustlib`.** `mb2` installs
  rust into the *target*, but build-script links run in sb2's host mode
  where `/usr` maps to the SDK filesystem. Copy it from the tooling.
  `ci/build-sdk-image.sh` does this once, into the image, so a build no
  longer carries it; a build against upstream's image still has to.
- **Not root.** `sdk-manage` refuses ("Cannot determine Mer SDK user") and
  the target snapshot never initialises. Chown the checkout to the
  container's `mersdk` uid — read it from the image, don't assume it — and
  hand it back so the artifact upload can read the result.

`scripts/build-rpm.sh` wraps the ordinary developer path, `sfdk build`.

## What a device build costs

Seven and a quarter minutes before this, and a little over three now. The
before column is run 89, the last one built the old way; the two after it
are runs 98 and 99, both against a published SDK image and a warm cache.

| Step | Before | One job | Four jobs |
|---|---|---|---|
| Pull the SDK image | 144 s | 105 s | 80 s |
| Build the RPM | 270 s | 142 s | 82 s |
| Validate against Harbour | 12 s | 10 s | 10 s |
| **The whole run** | **437 s** | **284 s** | **190 s** |

Three changes, in the order they pay:

**The SDK image is derived, not upstream's.** `ci/build-sdk-image.sh` takes
`coderus/sailfishos-platform-sdk` by digest and produces an image with one
architecture instead of three, this package's `BuildRequires` already
installed, and the i686 rustlib already at `/usr/lib/rustlib`. It has to
flatten the result rather than layer it, because files deleted in a new
layer still weigh what they weighed. 5.04 GB of pull becomes about 2.3 GB,
and `zypper` leaves the critical path: `build-init` and `build-requires`
together took 30 s and now take 3.

A target here is two rootfs -- the pristine one, and the `<target>.default`
snapshot that mb2 actually builds in and that `build-requires` installs
into. Both are kept. Deleting the snapshot as a redundant copy is what
made the first derived image come out with no rust in it.

`sdk-image.yml` publishes the image to the repository's registry; `rpm.yml`
derives and publishes one itself when it finds none, so a new SDK version
needs a pinned digest in `ci/build-sdk-image.sh` and nothing else. That
first run pays for it: run 97 took 850 s, of which 576 was deriving and
pushing.

**`rust/target` and the crates are carried between runs.** Keyed on the
lockfile and on the image, because they are artifacts for one target triple
built by the rust that image ships. It is worth 103 s: the same build cold
took 245 s and warm 142 s. Of the 56 crates, 52 come from the lockfile and
change only when it does. A fresh `actions/checkout` gives every file a new
mtime and does *not* defeat this -- cargo fingerprints registry crates by
content, so only the path crates rebuild. The two caches are small, 112 MB
and 12 MB.

**cargo runs four jobs inside scratchbox2**, which is worth another 60 s.
See the job count under "Spec constraints" below for what that setting is
and why it was one for so long.

## Spec constraints

Landmines encoded in `rpm/harbour-postivene.spec`, each found the hard way:

- **The cargo job count under sb2 is a define.** At `-j4`
  cargo was seen to futex-wait forever on an unreaped child while
  qmetaobject's C++ glue compiled, and `%{jobs}` exists so that is a
  setting rather than a rediscovery: `mb2 build --define "jobs N"`, which
  is what `rpm.yml`'s `cargo_jobs` input passes. It applies only inside
  sb2; a native OBS worker lets cargo pick. The same spec also keeps the
  build's temporaries in the build directory, because a parallel link
  through the shared `/tmp` under sb2 can lose an object file it has just
  written -- Whisperfish's spec does the same.

  It defaults to **4**, which device builds have run green on the 5.2 SDK
  (runs 99 and 100) and which takes the `Build the RPM` step from 142 s to
  82 s. If one ever hangs there again, `--define "jobs 1"` is the way
  back, and that is the whole reason the number is a setting.

  Why it is worth so much: CPU time equals wall time at `-j1`, because
  cargo hands rustc its codegen threads from the same jobserver, so one
  job is one thread through the entire build. On a host, against the same
  crate graph and the same rustc 1.75 the SDK ships, a cold build takes
  144 s at `-j1` and 39 s at `-j4`; one file changed in the shim takes
  72 s at `-j1`, 41 s at `-j2` and 27 s at `-j4`.
- **No `--target` for cargo.** Jolla's cargo pins build scripts to the
  tooling's host triple; `--target` on top makes cargo treat the whole build
  as a cross build. `SB2_RUST_TARGET_TRIPLE` already tells the accelerated
  rustc what to emit. Whisperfish's spec passes none either.
- **`CARGO_TARGET_<HOST>_LINKER=host-gcc` inside sb2.** rustc links build
  scripts by calling plain `cc`, which sb2 rewrites to the *cross* compiler
  (`aarch64-meego-linux-gnu-cc: unrecognized option '-m32'`). scratchbox2
  exposes the native compiler as `host-gcc`. Pointing at the tooling's gcc
  by absolute path is not enough — sb2 still rewrites the `ld` that gcc
  invokes, giving `cannot find /lib/libgcc_s.so.1`.
- **`QT_INCLUDE_PATH`/`QT_LIBRARY_PATH` exported in `%build`**: qttypes
  cannot exec the target `qmake` under sb2. `QT_LIBRARY_PATH` uses
  `%{_libdir}` — Qt is in `/usr/lib64` on aarch64, not `/usr/lib`.
- **`%{_target_cpu}`, not `%{_arch}`**, for the bundled server path.
- **`Exec=harbour-postivene`** in the desktop file: the invoker does not
  honour an `Exec=env FOO=bar` wrapper, so the bundled server path is a
  fallback inside the binary.
- **Harbour constrains the name, the paths and every `Requires:`.**
  `ci/harbour-check.sh` fails a build that breaks one; `HARBOUR.md` is
  the map, including the two rules this package still breaks.
- **No bare `%` in a spec comment.** rpm expands macros inside comments, and
  on the SDK's older rpm a comment mentioning `%build` expands to a preamble
  starting `LANG=C`, which rpm reads as a tag. Host rpm 4.18 leaves comments
  alone and had parsed the same file through an entire successful build.
  `ci/packaging-lint.sh` checks for this directly.
