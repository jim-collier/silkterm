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

## Stage 2: debug build (fast compile sanity)
DEBUG_BUILD_CMD=(cargo build)

## Stage 3: regression tests
TEST_CMD=(cargo test)

## Stage 3 (after tests): lints. Gating; house allows live in the workspace
## Cargo.toml [workspace.lints.clippy]. PROBE decides tool availability -
## a failed probe skips the step with a warning instead of aborting.
LINT_PROBE=(cargo clippy --version)
LINT_CMD=(cargo clippy --workspace --all-targets -- -D warnings)

## Stage 3 (after lints): dependency police (licenses/advisories/duplicates,
## policy in deny.toml). Non-gating for now; tighten once the report is tuned.
DENY_PROBE=(cargo deny --version)
DENY_CMD=(cargo deny check)

## Stage 5: native release build + its artifact (this is what gets dogfooded)
RELEASE_NATIVE_CMD=(cargo build --release)
RELEASE_NATIVE_BIN="target/release/${EXE_NAME}"

## Stage 5: cross-release targets. One per line: "label|artifact|command...".
## Set BUILD_CROSS=0 to skip them for a quick local run.
BUILD_CROSS=1
CROSS_TARGETS=(
	"Windows x86_64 (mingw)|target/x86_64-pc-windows-gnu/release/${EXE_NAME}.exe|cargo build --release --target x86_64-pc-windows-gnu"
	"Linux ARM64 (zig)|target/aarch64-unknown-linux-gnu/release/${EXE_NAME}|cargo zigbuild --release --target aarch64-unknown-linux-gnu"
	"Windows ARM64 (zig)|target/aarch64-pc-windows-gnullvm/release/${EXE_NAME}.exe|cargo zigbuild --release --target aarch64-pc-windows-gnullvm"
)

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
PROFILE_OUT_DIR="../private/profiling"  # relative to repo root; created if missing (kept out of git)
PROFILE_STRICT=0                        # 1 = any profiler failure aborts the pipeline

## Old SVGs are pruned by gfs_rotate (cicd/utility/include/gfs-rotate.bash): keeps
## ~30 - first + newest-per-hour/day/week/month/year + last 10. Tune with the
## GFS_KEEP_* env vars (GFS_KEEP_FREQUENT, GFS_KEEP_DAILY, ...) if needed.

## Stage 6: dogfood the native release here (first existing dir wins)
DOGFOOD_DESTS=(
	"${HOME}/synced/0-0/common/exec/util/linux/bin"
	"/usr/local/sbin"
)

## Stage 7: backup + publish to git (runs from repo root).
GIT_PUBLISH=(cicd/utility/n8git_backup-and-publish)

## Set a non-empty commit message to publish hands-off (suppresses the script's
## prompt and supplies the message so `git commit` won't open an editor). Left
## empty, publish is interactive unless -m/--message or -y is given (see cicd.bash).
PUBLISH_AUTO_MESSAGE=""


##	History:
##		- 2026-06-05 JC: Created.
