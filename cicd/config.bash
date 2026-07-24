#!/bin/bash

#  shellcheck disable=2001  ## 'See if you can use ${variable//search/replace} instead.' Complains about good uses of sed.
#  shellcheck disable=2016  ## 'Expressions don't expand in single quotes, use double quotes for that.' I know, and I often want an explicit '$'.
#  shellcheck disable=2034  ## 'variable appears unused.' Complains about valid use of variable indirection (e.g. later use of local -n var=$1)
#  shellcheck disable=2046  ## 'Quote to prevent word-splitting.' (OK for integers.)
#  shellcheck disable=2086  ## 'Double quote to prevent globbing and word splitting.' (OK for integers.)
#  shellcheck disable=2119  ## 'Use foo "$@" if function's $1 should mean script's $1.' Confusing and inapplicable.
#  shellcheck disable=2120  ## 'Foo references arguments, but none are ever passed.' Valid function argument overloading.
#  shellcheck disable=2128  ## 'Expanding an array without an index only gives the element in the index 0.' False hits on associative arrays.
#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.' Cumbersome and unnecessary. For integers it's sometimes required to even come into existence for counters.
#  shellcheck disable=2162  ## 'read without -r will mangle backslashes.'
#  shellcheck disable=2178  ## 'Variable was used as an array but is now assigned a string.' False hits on associative arrays with e.g. 'local -n assocArray=$1'.
#  shellcheck disable=2181  ## 'Check exit code directly, not indirectly with $?.'
#  shellcheck disable=2317  ## 'Can't reach.' (I.e. an 'exit' is used for debugging - and makes an unusable visual mess.)
## shellcheck disable=2002  ## 'Useless use of cat.'
## shellcheck disable=2004  ## '$/${} is unnecessary on arithmetic variables.' Inappropriate complaining?
## shellcheck disable=2053  ## 'Quote the right-hand sid of = in [[ ]] to prevent glob matching.' Disable for Yoda Notation.
## shellcheck disable=2143  ## 'Use grep -q instead of echo | grep'

##	Purpose:
##		- Project-specific CI/CD settings.
##		- To reuse this pipeline in another project,
##		  copy the whole cicd/ directory and edit THIS file (cicd.bash stays generic).
##		  All command arrays run from the repo root. The engine prepends ~/.cargo/bin to
##		  PATH so the rustup toolchain (cross targets, edition 2024) wins over system rust.
##	History: At bottom of script.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


## Check if sourced
declare -i isSourced_t6wqf=0; [[ "${BASH_SOURCE[0]}" == "${0}" ]] || isSourced_t6wqf=1
((isSourced_t6wqf)) || { echo -e "\nError in $(basename "${BASH_SOURCE[0]}"): This script is meant to be 'sourced' from within another script.\n"; exit ${ERRNUM_MSG_ALREADY_SHOWN}; }


## Identity
APP_NAME="SilkTerm"
EXE_NAME="silkterm"

## Stage 1: format the source in place before anything is compiled or tested.
## Empty it (FMT_CMD=()) when reusing the pipeline in a non-Rust project.
FMT_CMD=(cargo fmt)
## Non-mutating variant for the --gate mode (fails on drift instead of rewriting).
FMT_CHECK_CMD=(cargo fmt --check)

## Pinned versions of the cargo-installed helpers the pipeline probes for; the
## engine warns (non-gating) when an installed tool has drifted from its pin, so
## a box update can't silently change results. Format: "name|version|command...".
## The rustc/clippy toolchain itself is pinned by rust-toolchain.toml at repo root.
TOOL_PINS=(
	"cargo-deny|0.19.9|cargo deny --version"
	"cargo-zigbuild|0.23.0|cargo-zigbuild --version"
	"cargo-deb|3.7.0|cargo-deb --version"
	"cargo-generate-rpm|0.21.0|cargo-generate-rpm --version"
	"makensis|3.11|makensis -VERSION"
)

## Stage 2: debug build (fast compile sanity)
DEBUG_BUILD_CMD=(cargo build)

## Stage 3: regression tests
TEST_CMD=(cargo test)

## Stage 3 (after tests): lints. Gating; house allows live in the workspace
## Cargo.toml [workspace.lints.clippy]. PROBE decides tool availability -
## a failed probe skips the step with a warning instead of aborting.
## rust-toolchain.toml pins one toolchain for every rustup-routed cargo, but a
## shell where system cargo wins PATH can still populate target/ with the other
## rustc (E0514: artifacts from a different compiler), so lint pins the rustup
## PATH and keeps its own target dir as insurance.
LINT_PROBE=(env "PATH=${HOME}/.cargo/bin:${PATH}" cargo clippy --version)
LINT_CMD=(env "PATH=${HOME}/.cargo/bin:${PATH}" CARGO_TARGET_DIR=target/lint cargo clippy --workspace --all-targets -- -D warnings)

## Stage 3 (after lints): dependency police (licenses/advisories/duplicates,
## policy in deny.toml). Non-gating for now; tighten once the report is tuned.
DENY_PROBE=(cargo deny --version)
DENY_CMD=(cargo deny check)

## Stage 3 (last): headless scroll regression harness. Slow (a private Xvfb + GL
## per scene), so it is skipped under --quick; non-fatal on an environment miss,
## but a measured scroll regression aborts. Empty () to disable.
SCROLL_HARNESS=(cicd/tests/scroll/run.bash)

## Also run the harness a second time under a headless Wayland compositor (cage), to
## prove the Wayland backend renders + scrolls the same as X11. Self-skips (non-fatal)
## where cage is not installed. 0/unset to disable.
SCROLL_HARNESS_WAYLAND=1

## Stage 5: native release build + its artifact (this is what gets dogfooded)
RELEASE_NATIVE_CMD=(cargo build --release)
RELEASE_NATIVE_BIN="target/release/${EXE_NAME}"
RELEASE_NATIVE_OSARCH="linux-x86_64"

## Stage 5: cross-release targets. One per line: "label|os-arch|artifact|command...".
## os-arch feeds the versioned artifact name (<exe>-<version>-<os-arch>[.exe]).
## Set BUILD_CROSS=0 to skip them for a quick local run.
BUILD_CROSS=1
CROSS_TARGETS=(
	"Windows x86_64 (mingw)|windows-x86_64|target/x86_64-pc-windows-gnu/release/${EXE_NAME}.exe|cargo build --release --target x86_64-pc-windows-gnu"
	"Linux ARM64 (zig)|linux-arm64|target/aarch64-unknown-linux-gnu/release/${EXE_NAME}|cargo zigbuild --release --target aarch64-unknown-linux-gnu"
	"Windows ARM64 (zig)|windows-arm64|target/aarch64-pc-windows-gnullvm/release/${EXE_NAME}.exe|cargo zigbuild --release --target aarch64-pc-windows-gnullvm"
)

## Stage 5 (after builds): collect the built binaries under versioned names plus
## a sha256 checksums file, ready to attach to a release as plain uploads.
## Naming scheme (stable; download links depend on it):
##   <exe>-<version>-<os-arch>[.exe]   e.g. silkterm-1.0.0-beta1-linux-x86_64
##   <exe>-<version>-sha256sums.txt
## Version comes from source/Cargo.toml alone. Empty to disable collection.
RELEASE_ARTIFACT_DIR="cicd/artifacts/release"   # relative to repo root; gitignored
VERSION_MANIFEST="source/Cargo.toml"            # the single version source

## Stage 6: distributable packages, built from the stage-5 release binaries (never
## rebuilt) when --quick is NOT passed. Linux -> .deb + .rpm (cargo-deb /
## cargo-generate-rpm, metadata in source/Cargo.toml). Windows -> a single self-
## contained NSIS installer .exe per arch (makensis), which upgrades an existing
## install in place. macOS (.dmg) and BSD are deferred: this box has no Apple SDK
## / FreeBSD sysroot to cross-build their binaries. ARM64 packages follow the same
## --no-arm gate as the ARM release builds. Packages land in RELEASE_ARTIFACT_DIR
## and fold into the sha256sums. Set PACKAGE_ENABLE=0 (or --no-package) to skip.
PACKAGE_ENABLE=1
NSIS_TEMPLATE="cicd/packaging/windows/installer.nsi.in"

## Stage 4: profiler (non-gating artifact, not a pass/fail test). Builds an
## optimized+symbols binary (cargo --profile $PROFILE_PROFILE --features
## $PROFILE_FEATURE), runs the real app under an in-process sampler against a heavy
## workload for $PROFILE_SECS, and writes a flamegraph SVG. See cicd.bash for the
## skip-vs-abort failure policy ($PROFILE_STRICT to force abort on any failure).
PROFILE_ENABLE=1
PROFILE_SECS=8
PROFILE_FEATURE="profiling"
PROFILE_PROFILE="profiling"
PROFILE_BIN="target/profiling/${EXE_NAME}"
PROFILE_WORKLOAD_SCRIPT="cicd/utility/n8output-random-unicode.py"
PROFILE_WORKLOAD_ARGS="600 0"          # <duration_s> <delay_s>; duration >> PROFILE_SECS, no delay = max output
PROFILE_OUT_DIR="cicd/artifacts/profiling"  # relative to repo root; created if missing (gitignored)
PROFILE_STRICT=0                        # 1 = any profiler failure aborts the pipeline

## Pre-publish README screenshot refresh (cicd/utility/screenshots.bash). Currently
## off; flip to 1 (or pass --shots) to re-enable. Also skipped under --quick.
SHOTS_ENABLE=0

## Demo video re-record (cicd/utility/demo-video/demo-video.py). Off by default -
## only worth re-recording after major visual/feature changes; flip to 1 or pass
## --demo for one run. Also skipped under --quick. Video GFS-rotates into
## ../private/demo-video/; the README highlight gif lands in assets/demo.gif.
DEMO_ENABLE=0

## Full run output is tee'd here (gitignored) so warnings from any stage can be
## reviewed after the fact. Kept rotated like the flamegraphs.
LINT_LOG_DIR="cicd/artifacts/lint"      # relative to repo root; created if missing (gitignored)

## Old SVGs are pruned by gfs_rotate (cicd/utility/include/gfs-rotate.bash): keeps
## ~30 - first + newest-per-hour/day/week/month/year + last 10. Tune with the
## GFS_KEEP_* env vars (GFS_KEEP_FREQUENT, GFS_KEEP_DAILY, ...) if needed.

## Stage 6: dogfood the native release two ways. Empty either list to skip it.
## Fixed: overwrite EXE_NAME in the first existing dir here (the stable path you run).
DOGFOOD_FIXED_DESTS=(
	"${HOME}/synced/0-0/common/exec/util/linux/bin"
	"/usr/local/sbin"
)
## Rotating: also drop a dated copy "<DOGFOOD_PREFIX>_<YYYYmmDD-HHMMSS>" here (created
## if missing), so builds coexist under unique paths - an automated test killing one
## can't hit an unrelated version - pruning older copies that aren't running. Launch
## the newest via utility/n8runterm.bash. Set DOGFOOD_PREFIX empty to disable the rotating copy.
DOGFOOD_ROTATING_DESTS=(
	"${HOME}/.local/bin"
)
DOGFOOD_PREFIX="slktrmdf"

## Stage 7: backup + publish to git (runs from repo root).
GIT_PUBLISH=(cicd/utility/n8git_backup-and-publish)

## Set a non-empty commit message to publish hands-off (suppresses the script's
## prompt and supplies the message so `git commit` won't open an editor). Left
## empty, publish is interactive unless -m/--message or -y is given (see cicd.bash).
PUBLISH_AUTO_MESSAGE=""


##	History:
##		- 2026-06-05 JC: Created.
