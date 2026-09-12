# postivene -- developer targets.
#
# `make check` is what CI runs, minus what needs the Sailfish SDK or a
# phone. It is not entirely offline: `msrv` fetches a toolchain the first
# time it runs, and `deny` wants the advisory database.
#
# Qt5 packages (Debian/Ubuntu):
#   apt install qtbase5-dev qtdeclarative5-dev qtmultimedia5-dev \
#               qtdeclarative5-dev-tools qml-module-qtquick2 \
#               qttools5-dev-tools
#
# qml-module-qtquick2 is the QtQuick runtime plugin, which the -dev
# packages omit; qttools5-dev-tools is lupdate and lrelease, for the
# catalogs, and the app's own test compiles one; qtmultimedia5-dev is
# QAudioRecorder, which the shim's voice recorder is built on.

.PHONY: check test lint fmt qml-lint packaging-lint lockfile-lint doc-lint \
        msrv deny integration harbour vendor-check fetch-server \
        sonar-report-test apt-install-test sonar-reports translations faces \
        clean

CARGO ?= cargo
# The shim's tests drive a real Qt event loop, which needs a platform
# plugin.
export QT_QPA_PLATFORM = offscreen

## What CI runs, in the same order. Keep in step with ci.yml.
##
## `msrv` fetches a toolchain the first time, so this is not quite
## network-free; `deny` needs the advisory database and is CI's job.
check: fmt lint test doc-lint msrv qml-lint lockfile-lint packaging-lint harbour \
       sonar-report-test apt-install-test vendor-check deny

## Unit, integration, and Qt event-loop tests.
##
## Through cargo-nextest when it is installed, which runs each test in its
## own process rather than one test binary at a time. This suite is over a
## hundred binaries that mostly sit waiting on Qt timers, so that is the
## difference between about ten minutes and about two and a half.
## rust/.config/nextest.toml says the rest.
##
## Without it the same tests still run, the slow way. `make check` has to
## work on a laptop that has not installed anything extra:
##   cargo install --locked cargo-nextest
##
## nextest does not run doctests, so `--doc` runs beside it either way.
## There are none today, which is exactly how one would get added and never
## run again; ci/packaging-lint.sh fails a tree that drops it.
test:
	@command -v cargo-nextest >/dev/null 2>&1 || \
		echo "test: cargo-nextest is not installed; running the slow way \
(cargo install --locked cargo-nextest)"
	@if command -v cargo-nextest >/dev/null 2>&1; then \
		cd rust && $(CARGO) nextest run --workspace; \
	else \
		cd rust && $(CARGO) test --workspace; \
	fi
	cd rust && $(CARGO) test --workspace --doc

## Clippy at the workspace lint level, over tests and binaries too.
lint:
	cd rust && $(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	cd rust && $(CARGO) fmt --all --check

## Parse every .qml file; the Qt 5.6 dialect rules are a Rust test.
qml-lint:
	./ci/qml-lint.sh

## The spec parses, the desktop entry is valid, the shell scripts are
## clean, every docs/*.md a comment points at exists. A missing tool is a
## SKIP here and a failure in CI (PACKAGING_LINT_STRICT=1).
packaging-lint:
	./ci/packaging-lint.sh

## Cargo.lock must stay v3: Sailfish's cargo 1.75 cannot read v4.
lockfile-lint:
	./ci/check-lockfile.sh

## Broken intra-doc links are errors.
doc-lint:
	cd rust && RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps

## Compile against the toolchain floor Sailfish ships. Part of `check`:
## clippy on a modern toolchain does not reliably catch newer std methods or
## syntax, so only a real 1.75 build proves the device still builds.
msrv:
	rustup toolchain install 1.75.0 --profile minimal
	cd rust && $(CARGO) +1.75.0 check --workspace --all-targets

## What Harbour would reject, read off the sources. `sfdk check -s harbour`
## on a built RPM is the authority (docs/HARBOUR.md); this is what can be
## answered without the SDK, and CI runs it on every pull request.
##
## Wants `make fetch-server` and a built binary for its last few checks;
## without them it says so rather than passing quietly.
harbour:
	./ci/harbour-check.sh
	./ci/harbour-check-selftest.sh

## third_party/qmetaobject is upstream plus one three-line patch, and this
## proves it. Needs the network: it fetches the crates.io tarball to compare
## against.
vendor-check:
	./ci/vendor-check.sh

## Licences and advisories, as CI's `deny` job runs them. Needs
## `cargo install cargo-deny`, and the advisory database (network).
##
## A missing tool is a skip; a finding is a failure. The two used to share
## one `||`, which printed SKIP over a real advisory and let `make check`
## exit 0 on it.
deny:
	@command -v cargo-deny >/dev/null 2>&1 || \
		{ echo "deny: SKIP (cargo-deny not installed)"; exit 0; }
	cd rust && $(CARGO) deny check

## Fetch the pinned upstream deltachat-rpc-server binaries (network).
fetch-server:
	./scripts/fetch-rpc-server.sh

## Compile the catalogs into translations/*.qm, where a source-tree run
## of the app finds them. The RPM does the same in %%build.
translations:
	./scripts/release-translations.sh

## Repaint the art the onboarding screens draw (qml/art/) from
## tools/faces/: the field of faces behind them, and the picture over
## each fact of the introduction. Python 3 and nothing else; the results
## are committed, so a build needs neither this nor a display. Run it
## when a painter changes, and look at what it made.
faces:
	python3 tools/faces/faces.py
	python3 tools/faces/scenes.py

## Prove scripts/sonar-report.sh still reports what it claims to, against a
## stub server. The real service is unreachable from CI's network and from a
## laptop behind one, which is the reason that script exists at all.
sonar-report-test:
	./ci/sonar-report-selftest.sh

## Prove ci/apt-install.sh keeps Ubuntu's apt sources and drops the rest.
## Getting that backwards deletes the archive every job installs from.
apt-install-test:
	./ci/apt-install-selftest.sh

# What `sonar-reports` measures, whichever runner it goes through: the
# workspace, minus third_party/ and vendor/ for the reason given in
# sonar-project.properties -- upstream's code, not ours to cover.
SONAR_COV_ARGS = --workspace \
	--ignore-filename-regex '(^|/)(third_party|vendor)/' \
	--lcov --output-path target/sonar/lcov.info

## sonar-reports: the coverage report SonarQube Cloud imports, written to
## rust/target/sonar/lcov.info. The scanner does not measure coverage; it
## only imports what someone else measured, which is why the reading was
## 0.0% for as long as nothing wrote this.
##
## Through cargo-nextest when it is installed, for the reason `test` is.
## cargo-llvm-cov's own runner is `cargo test`, one binary at a time, and
## this suite is over a hundred binaries that mostly sit waiting on Qt
## timers: instrumented, that was ten and a half minutes of the scan job,
## most of them tests finishing one after another, while ci.yml's `test`
## job runs the same tests under nextest in about two, and the whole of
## ci.yml in under five. `cargo llvm-cov nextest` is the same
## instrumented build under nextest's scheduler, and
## rust/.config/nextest.toml applies to it as it does to `test`. Without
## nextest the report is still written, the slow way.
##
## No `cargo test --doc` beside it, unlike `test`: cargo-llvm-cov leaves
## doctests out on either runner (instrumenting them needs nightly), and
## this target measures rather than gates. `test` is what runs them.
##
## Clippy findings are deliberately NOT handed over. `make lint` runs clippy
## with `-D warnings`, so a warning in this project's own code fails the gate
## and never reaches a branch Sonar analyses -- the report was empty of our
## code every time, and producing it cost a `cargo clean` and a full
## recompile inside the scan job. Sonar's own clippy pass stays off for a
## different reason; sonar-project.properties says which.
##
## Needs cargo-llvm-cov, so it is opt-in rather than part of `check`:
##   rustup component add llvm-tools-preview
##   cargo install --locked cargo-llvm-cov
##   cargo install --locked cargo-nextest    (optional; serial without it)
sonar-reports:
	@echo "== coverage for SonarQube Cloud =="
	@command -v cargo-llvm-cov >/dev/null 2>&1 || { \
		echo "cargo-llvm-cov is not installed. Install it with:" >&2; \
		echo "    rustup component add llvm-tools-preview" >&2; \
		echo "    cargo install --locked cargo-llvm-cov" >&2; \
		exit 1; \
	}
	@command -v cargo-nextest >/dev/null 2>&1 || \
		echo "sonar-reports: cargo-nextest is not installed; running the slow way \
(cargo install --locked cargo-nextest)"
	@mkdir -p rust/target/sonar
	@if command -v cargo-nextest >/dev/null 2>&1; then \
		cd rust && $(CARGO) llvm-cov nextest $(SONAR_COV_ARGS); \
	else \
		cd rust && $(CARGO) llvm-cov $(SONAR_COV_ARGS); \
	fi
	@echo "== wrote rust/target/sonar/lcov.info =="

## The tests that drive the real core, offline. Needs `make fetch-server`.
integration:
	cd rust && DELTACHAT_RPC_SERVER=vendor/deltachat-rpc-server/x86_64/deltachat-rpc-server \
		$(CARGO) test -p deltachat-jsonrpc --test real_server -- --nocapture

clean:
	cd rust && $(CARGO) clean
