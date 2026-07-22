#!/usr/bin/env bash

#  shellcheck disable=1091  ## 'source is valid here, but shellcheck doesn't know the path to it.'
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

##	- Purpose: Local CI/CD pipeline. Generic engine, per-project settings live in config.bash.
##	- Stages (fail-fast, any error aborts before the next stage):
##	   0. remote sync (fetch; fast-forward if safely behind; abort if diverged)
##	   1. format (cargo fmt)
##	   2. debug build (this is what the tests + profiler run against)
##	   3. regression tests + lints (clippy gating, cargo-deny advisory, scroll harness)
##	   4. profiler (flamegraph SVG; non-gating artifact - see failure policy)
##	   5. release build (native + cross targets; optimized, for packaging + dogfood)
##	   6. packages (.deb/.rpm per Linux arch; NSIS installer .exe per Windows arch)
##	   7. dogfood (install native release locally)
##	   8. backup + publish to git (runs from repo root)
##	- Syntax:
##	  cicd/cicd.bash [options]
##	  Options:
##	   -y, --yes           run unattended (no confirm prompt)
##	   -q, --quiet         quiet + unattended (implies -y); the publish step runs quiet too
##	   -m, --message MSG   publish hands-off with this commit message (no editor)
##	       --msg MSG       alias for --message
##	   --no-fmt            skip the formatter (cargo fmt) stage
##	   --no-cross          skip cross-target release builds
##	   --no-arm            skip the ARM64 release builds + packages (x86_64 only)
##	   --no-package        skip the packages stage (.deb/.rpm/installer)
##	   --no-profile        skip the profiler stage
##	   --no-dogfood        skip installing the native release locally
##	   --no-publish        skip the git backup + publish stage
##	   --no-sync           skip the remote sync check (stage 0)
##	   --shots             refresh README screenshots (off by default)
##	   --demo              re-record the demo video (off by default)
##	   --quick             skip the slow stages (cross-builds + packages + profiling)
##	   --gate              merge gate only: fmt --check + clippy + tests, then exit
##	                       (fast local stand-in for hosted CI; the pre-push hook runs it)
## - Reuse: copy the cicd/ directory into another project and edit config.bash.

##	History: At bottom of script.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


set -Eeuo pipefail

## Find the repo root and load project config.
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${here}/.." && pwd)"   # the git repo root (cicd/..)
export PATH="${HOME}/.cargo/bin:${HOME}/.local/bin:${PATH}"       ## rustup toolchain (cross targets, edition 2024) + zig must beat system rust.
source "${here}/config.bash"
source "${here}/utility/include/gfs-rotate.bash"                  ## gfs_rotate() for the profiler artifacts
declare -p FMT_CMD &>/dev/null || FMT_CMD=()                      ## tolerate a config without the fmt stage

## Cap compile/test parallelism to at most half the cores so a pipeline run
## doesn't peg every CPU and leaves the machine usable. cargo's jobserver bounds
## total rustc + codegen parallelism to CARGO_BUILD_JOBS (covers build, test,
## clippy, and the zigbuild cross-builds); RUST_TEST_THREADS bounds the test run.
## A project can override CICD_MAX_JOBS in config.bash.
cores="$(nproc 2>/dev/null || echo 2)"
: "${CICD_MAX_JOBS:=$(( cores / 2 ))}"
(( CICD_MAX_JOBS >= 1 )) || CICD_MAX_JOBS=1
export CARGO_BUILD_JOBS="${CICD_MAX_JOBS}"
export RUST_TEST_THREADS="${CICD_MAX_JOBS}"

cd "${root}"
stamp="$(date +%Y%m%d-%H%M%S)"

## Parse options.
assume_yes=0; quiet=0; quick=0; gate=0; no_arm=0; sync=1; cli_message=""
while (($#)); do case "$1" in
	-y|--yes)                 assume_yes=1; shift ;;
	-q|--quiet)               quiet=1; assume_yes=1; shift ;;   ## quiet + unattended; publish runs quiet too
	--gate)                   gate=1; shift ;;                  ## merge gate only, then exit
	--no-fmt)                 FMT_CMD=(); shift ;;
	--no-cross)               BUILD_CROSS=0; shift ;;
	--no-arm)                 no_arm=1; shift ;;                ## drop ARM64 builds + packages
	--no-package)             PACKAGE_ENABLE=0; shift ;;
	--no-profile)             PROFILE_ENABLE=0; shift ;;
	--no-dogfood)             DOGFOOD_FIXED_DESTS=(); DOGFOOD_ROTATING_DESTS=(); shift ;;
	--no-publish)             GIT_PUBLISH=(); shift ;;
	--no-sync)                sync=0; shift ;;
	--shots)                  SHOTS_ENABLE=1; shift ;;
	--demo)                   DEMO_ENABLE=1; shift ;;
	--quick)                  quick=1; BUILD_CROSS=0; PROFILE_ENABLE=0; PACKAGE_ENABLE=0; shift ;;   ## skip the slow stages
	--message=*|--msg=*|-m=*) cli_message="${1#*=}"; shift ;;
	-m|--message|--msg)       cli_message="${2-}"; shift; (($#)) && shift ;;
	-h|--help)                sed -n '/^##	- Purpose:/,/^##	History:/p' "${BASH_SOURCE[0]}" | sed '$d; s/^##	\{0,1\}//'; exit 0 ;;
	*) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
esac; done

## --no-arm: drop the ARM64 cross targets so the run (and its packages) stay
## x86_64-only. Native x86_64 is untouched; the Windows/Linux x86_64 crosses stay.
if ((no_arm)) && declare -p CROSS_TARGETS &>/dev/null; then
	kept=()
	for t in "${CROSS_TARGETS[@]}"; do case "$t" in *arm64*|*aarch64*) ;; *) kept+=("$t") ;; esac; done
	CROSS_TARGETS=("${kept[@]}")
fi
declare -p PACKAGE_ENABLE &>/dev/null || PACKAGE_ENABLE=0   ## tolerate a config predating the packages stage

## Publish commit message: -m wins, then config, then a default when unattended.
## Empty -> publish interactively (git commit opens an editor); when interactive
## we offer to capture a message at the preflight prompt below.
publish_msg=""
if   [[ -n "$cli_message" ]];              then publish_msg="$cli_message"
elif [[ -n "${PUBLISH_AUTO_MESSAGE:-}" ]]; then publish_msg="$PUBLISH_AUTO_MESSAGE"
elif ((assume_yes));                       then publish_msg="${APP_NAME} CI/CD ${stamp}"
fi

## Output helpers: fEcho / fEcho_Clean, blank-collapsing.
## fEcho "msg" -> "[ msg ]" status line; fEcho_Clean "msg" -> plain line, and a
## bare call collapses repeated blanks. fSection draws the leading-blank + rule
## letterbox before a major stage header; fDie prints a fatal line and exits.
declare -i _wasLastEchoBlank=0
fEcho_ResetBlankCounter(){ _wasLastEchoBlank=0; }
fEcho_Clean(){ if [[ -n "${1:-}" ]]; then echo -e "$*"; _wasLastEchoBlank=0; elif [[ $_wasLastEchoBlank -eq 0 ]] && echo; then _wasLastEchoBlank=1; fi; }
fEcho(){       if [[ -n "$*"     ]]; then fEcho_Clean "[ $* ]"; else fEcho_Clean ""; fi; }
fEcho_Force(){ fEcho_ResetBlankCounter; fEcho "$*"; }
_letterbox="••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••"
fSection(){ fEcho_Clean; fEcho_Clean "${_letterbox}"; fEcho "$*"; }
fDie(){ { fEcho_Force "FAILED: $*"; } >&2; exit 1; }
## True if a running process is executing the given binary (its own exe, not a
## substring match), so an in-use dogfood copy isn't pruned. Checks /proc/*/exe.
in_use(){
	local -r bin="$(realpath -e "$1" 2>/dev/null || true)"
	[[ -n "$bin" ]] || return 1
	local exe
	for e in /proc/[0-9]*/exe; do
		exe="$(realpath -e "$e" 2>/dev/null || true)"
		[[ "$exe" == "$bin" ]] && return 0
	done
	return 1
}
## (Re)write the sha256sums file over every artifact in the release dir except the
## sums file itself. Run after stage 5 (binaries) and again after stage 6 (packages),
## so the checksums cover the packages too. Uses the script-scope art_dir/ver/sums.
write_sums(){
	[[ -n "${art_dir:-}" && -d "${art_dir:-/nonexist}" ]] || return 0
	( cd "${art_dir}"
	  files=(); for x in "${EXE_NAME}-${ver}-"*; do [[ "$x" == "$sums" || ! -f "$x" ]] && continue; files+=("$x"); done
	  ((${#files[@]})) && sha256sum "${files[@]}" > "${sums}" )
}
trap 'rc=$?; printf "\n[ CICD ABORTED (exit %s) at line %s: %s ]\n" "$rc" "$LINENO" "$BASH_COMMAND" >&2; exit $rc' ERR

## Gate mode: the local merge gate (what a bare-bones hosted CI would run).
## fmt --check + clippy -D warnings + tests, fail-fast, nothing mutated, no
## artifacts/log-tee/publish. Wired as the pre-push hook for main/dev, so
## nothing reaches an integration branch unverified even outside a full run.
if ((gate)); then
	fSection "Gate 1/3  Format check"
	if declare -p FMT_CHECK_CMD &>/dev/null && ((${#FMT_CHECK_CMD[@]})); then
		"${FMT_CHECK_CMD[@]}" || fDie "format check failed (run: ${FMT_CMD[*]:-cargo fmt})"
		fEcho "OK: formatting clean"
	else
		fEcho_Clean "format check skipped (no FMT_CHECK_CMD)"
	fi
	fSection "Gate 2/3  Lints"
	if [[ -n "${LINT_CMD+x}" ]] && ((${#LINT_CMD[@]})) && "${LINT_PROBE[@]}" >/dev/null 2>&1; then
		"${LINT_CMD[@]}"
		fEcho "OK: lints clean"
	else
		fEcho_Clean "lints skipped (clippy unavailable)"
	fi
	fSection "Gate 3/3  Tests"
	"${TEST_CMD[@]}"
	fEcho "OK: tests passed"
	fSection "${APP_NAME} gate: PASSED."
	fEcho_Clean
	exit 0
fi

## Warn (non-gating) when a pinned helper tool has drifted from TOOL_PINS, so a
## box update can't silently change pipeline results.
if declare -p TOOL_PINS &>/dev/null; then
	for pin in "${TOOL_PINS[@]}"; do
		pin_name="${pin%%|*}"; pin_rest="${pin#*|}"; pin_ver="${pin_rest%%|*}"; pin_cmd="${pin_rest#*|}"
		have="$(${pin_cmd} 2>/dev/null | head -1 | sed 's/[^0-9.]*\([0-9][0-9.]*\).*/\1/')" || have=""
		if [[ -z "$have" ]]; then
			fEcho "WARNING: ${pin_name} not found (pinned ${pin_ver})"
		elif [[ "$have" != "$pin_ver" ]]; then
			fEcho "WARNING: ${pin_name} is ${have}, pinned ${pin_ver} (cargo install ${pin_name} --version ${pin_ver} --locked, or update the pin)"
		fi
	done
fi

## Preflight: show the plan with resolved paths, then confirm.
abs_script="${root}/${PROFILE_WORKLOAD_SCRIPT}"
profile_dir="$(cd "${root}" && mkdir -p "${PROFILE_OUT_DIR}" 2>/dev/null; cd "${PROFILE_OUT_DIR}" 2>/dev/null && pwd || echo "${root}/${PROFILE_OUT_DIR}")"
fixed_dest=""; for d in "${DOGFOOD_FIXED_DESTS[@]:-}"; do [[ -d "$d" && -w "$d" ]] && { fixed_dest="$d"; break; }; done
rot_dest="";   for d in "${DOGFOOD_ROTATING_DESTS[@]:-}"; do [[ -d "$d" && -w "$d" ]] && { rot_dest="$d"; break; }; done
rot_target="${rot_dest:-${DOGFOOD_ROTATING_DESTS[0]:-}}"  # created in stage 6 if it doesn't exist yet

fEcho_Clean
fEcho_Clean "${APP_NAME} local CI/CD"
fEcho_Clean
fEcho_Clean "Repo root ...........: ${root}"
fEcho_Clean "Remote sync .........: $( ((sync)) && echo 'fetch + fast-forward check' || echo '(skipped)')"
fEcho_Clean "Format ..............: ${FMT_CMD[*]:-(skipped)}"
fEcho_Clean "Debug build .........: ${DEBUG_BUILD_CMD[*]}"
fEcho_Clean "Tests ...............: ${TEST_CMD[*]}"
if ((PROFILE_ENABLE)); then
	fEcho_Clean "Profiler ............: ${PROFILE_SECS}s run -> flamegraph SVG (on headless ${RPD_HEADLESS_DISPLAY:-:98})"
	fEcho_Clean "  output dir ........: ${profile_dir}"
	fEcho_Clean "  workload ..........: python3 ${PROFILE_WORKLOAD_SCRIPT} ${PROFILE_WORKLOAD_ARGS}"
else
	fEcho_Clean "Profiler ............: (disabled)"
fi
fEcho_Clean "Release (native) ....: ${RELEASE_NATIVE_CMD[*]} -> ${RELEASE_NATIVE_BIN}"
if ((BUILD_CROSS)) && ((${#CROSS_TARGETS[@]})); then
	fEcho_Clean "Release (cross) .....:$( ((no_arm)) && echo ' (x86_64 only, --no-arm)')"
	for t in "${CROSS_TARGETS[@]}"; do fEcho_Clean "    - ${t%%|*}"; done
else
	fEcho_Clean "Release (cross) .....: (skipped)"
fi
if ((PACKAGE_ENABLE)) && ((! quick)); then
	fEcho_Clean "Packages ............: .deb/.rpm (Linux) + NSIS installer .exe (Windows), per built arch"
	fEcho_Clean "  deferred ..........: macOS (.dmg), BSD - no cross toolchain on this box"
else
	fEcho_Clean "Packages ............: $( ((quick)) && echo '(skipped --quick)' || echo '(disabled)')"
fi
if ((${#DOGFOOD_FIXED_DESTS[@]})); then
	if [[ -n "$fixed_dest" ]]; then fEcho_Clean "Dogfood, fixed name .: overwrite ${fixed_dest}/${EXE_NAME}"
	else fEcho_Clean "Dogfood, fixed name .: <none of: ${DOGFOOD_FIXED_DESTS[*]} exists - will skip>"; fi
else
	fEcho_Clean "Dogfood, fixed name .: (disabled)"
fi
if ((${#DOGFOOD_ROTATING_DESTS[@]})) && [[ -n "${DOGFOOD_PREFIX:-}" ]]; then
	fEcho_Clean "Dogfood, rotating ...: ${rot_target}/${DOGFOOD_PREFIX}_${stamp}  (dated copy; prunes idle ones)"
else
	fEcho_Clean "Dogfood, rotating ...: (disabled)"
fi
if ((${#GIT_PUBLISH[@]} == 0)); then
	fEcho_Clean "Publish (last) ......: (disabled)"
elif [[ -n "$publish_msg" ]]; then
	fEcho_Clean "Publish (last) ......: ${GIT_PUBLISH[*]} (hands-off: \"${publish_msg}\")"
else
	fEcho_Clean "Publish (last) ......: ${GIT_PUBLISH[*]} (will prompt for message; blank = editor)"
fi
fEcho_Clean
fEcho_Clean "Fail-fast: any error aborts before the next stage."
fEcho_Clean

if ((! assume_yes)); then
	## Capture the commit message up front so the run can finish unattended. This
	## is the natural place to bail on the common (publish) path - Ctrl+C here
	## aborts; there is no separate "Proceed? [y/N]" (removed to cut friction).
	if ((${#GIT_PUBLISH[@]})) && [[ -z "$publish_msg" ]]; then
		read -r -p "Publish commit message (blank = editor; Ctrl+C aborts): " m
		fEcho_ResetBlankCounter
		[[ -n "$m" ]] && publish_msg="$m"
	fi
fi

## Tee the rest of the run (all stages) to a gitignored log so warnings from any
## stage can be reviewed after the fact. Rotate the prior (closed) logs first.
if [[ -n "${LINT_LOG_DIR:-}" ]] && mkdir -p "${root}/${LINT_LOG_DIR}" 2>/dev/null; then
	gfs_rotate "${root}/${LINT_LOG_DIR}" run log >/dev/null 2>&1 || true
	exec > >(tee "${root}/${LINT_LOG_DIR}/run_${stamp}.log") 2>&1
fi

## Stage 0: remote sync. Make sure the local branch can be safely refreshed from
## its upstream BEFORE spending the build: what stage 8 pushes should be what got
## built and tested here, not an untested post-build merge. Behind-only is safe
## (fast-forward, stash-wrapped for a dirty tree); diverged aborts now rather
## than at publish. Offline just warns - a local build shouldn't need the net.
fSection "0/8  Remote sync"
if ((! sync)); then
	fEcho_Clean "remote sync skipped"
elif ! git rev-parse --abbrev-ref '@{u}' >/dev/null 2>&1; then
	fEcho_Clean "no upstream for $(git rev-parse --abbrev-ref HEAD); nothing to sync"
elif ! git fetch --quiet 2>/dev/null; then
	fEcho "WARNING: git fetch failed (offline?); continuing with the local tree"
else
	ahead="$(git rev-list --count '@{u}..HEAD')"
	behind="$(git rev-list --count 'HEAD..@{u}')"
	if ((behind == 0)); then
		if ((ahead)); then fEcho "OK: up to date with upstream (${ahead} ahead)"
		else fEcho "OK: up to date with upstream"; fi
	elif ((ahead == 0)); then
		## Behind only: a fast-forward can't lose anything. Same stash dance as
		## the publisher so a dirty tree can't block the pull.
		dirty=0
		git diff --quiet          || dirty=1
		git diff --cached --quiet || dirty=1
		[[ -n "$(git ls-files --others --exclude-standard)" ]] && dirty=1
		didStash=0
		if ((dirty)); then
			stashesBefore="$(git stash list | wc -l)"
			fEcho_Clean "git stash push --include-untracked ..."
			git stash push --include-untracked -m "auto-stash"
			stashesAfter="$(git stash list | wc -l)"
			((stashesAfter > stashesBefore)) && didStash=1
		fi
		fEcho_Clean "git pull --ff-only ..."
		git pull --ff-only
		if ((didStash)); then
			fEcho_Clean "git stash pop ..."
			git stash pop
		fi
		fEcho "OK: fast-forwarded ${behind} commit(s) from upstream"
	else
		fDie "diverged from upstream (${ahead} ahead, ${behind} behind) - reconcile first, or rerun with --no-sync"
	fi
fi

## Stage 1: format.
fSection "1/8  Format"
if ((${#FMT_CMD[@]} == 0)); then
	fEcho_Clean "format skipped"
else
	"${FMT_CMD[@]}"
	fEcho "OK: formatted (${FMT_CMD[*]})"
fi

## Stage 2: debug build.
fSection "2/8  Debug build"
"${DEBUG_BUILD_CMD[@]}"
fEcho "OK: debug build"

## Stage 3: regression tests.
fSection "3/8  Regression tests"
"${TEST_CMD[@]}"
if [[ -n "${LINT_CMD+x}" ]] && ((${#LINT_CMD[@]})); then
	if "${LINT_PROBE[@]}" >/dev/null 2>&1; then
		"${LINT_CMD[@]}"
		fEcho "OK: lints clean"
	else
		fEcho "WARNING: lints skipped: ${LINT_PROBE[*]} failed (component not installed?)"
	fi
fi
if [[ -n "${DENY_CMD+x}" ]] && ((${#DENY_CMD[@]})); then
	if "${DENY_PROBE[@]}" >/dev/null 2>&1; then
		## Advisory-only for now: report license/advisory/duplicate findings
		## without failing the pipeline (tighten to gating once tuned).
		"${DENY_CMD[@]}" || fEcho "WARNING: cargo-deny reported findings (non-gating)"
	else
		fEcho "WARNING: deps check skipped: ${DENY_PROBE[*]} failed (cargo install cargo-deny)"
	fi
fi
## Headless scroll regression harness (slow; skipped under --quick). It skips itself
## on an environment miss (no Xvfb/binary) and exits non-zero only on a measured
## regression - which aborts here.
if ((! quick)) && [[ -n "${SCROLL_HARNESS+x}" ]] && ((${#SCROLL_HARNESS[@]})); then
	fEcho_Clean "scroll regression harness (headless, X11) ..."
	if "${root}/${SCROLL_HARNESS[0]}" "${SCROLL_HARNESS[@]:1}"; then
		fEcho "OK: scroll harness (X11)"
	else
		fDie "scroll regression harness reported a regression (X11)"
	fi
	if [[ "${SCROLL_HARNESS_WAYLAND:-0}" == 1 ]]; then
		fEcho_Clean "scroll regression harness (headless, Wayland) ..."
		if "${root}/${SCROLL_HARNESS[0]}" "${SCROLL_HARNESS[@]:1}" --wayland; then
			fEcho "OK: scroll harness (Wayland)"
		else
			fDie "scroll regression harness reported a regression (Wayland)"
		fi
	fi
elif ((quick)); then
	fEcho_Clean "scroll harness skipped (--quick)"
fi
fEcho "OK: tests passed"

## Stage 4: profiler (non-gating artifact; failures classified below).
run_profiler(){
	((PROFILE_ENABLE)) || { fEcho_Clean "profiler disabled"; return 0; }

	## Mundane/environmental reasons -> skip with a warning (not the app's fault),
	## unless PROFILE_STRICT. Genuine run failures below still abort. The app runs
	## on a private Xvfb (gui-headless.bash), so no visible DISPLAY is needed - only
	## Xvfb + python3 + the workload.
	local skip=""
	command -v python3 >/dev/null 2>&1 || skip="python3 not found"
	[[ -z "$skip" ]] && [[ ! -f "$abs_script" ]] && skip="workload missing: ${abs_script}"
	[[ -z "$skip" ]] && ! command -v Xvfb >/dev/null 2>&1 && skip="Xvfb not found (headless display unavailable)"
	if [[ -n "$skip" ]]; then
		((PROFILE_STRICT)) && fDie "profiler: ${skip}"
		fEcho "WARNING: profiler skipped: ${skip}"; return 0
	fi

	## From here, a failure means the app is at fault -> abort.
	fEcho_Clean "building ${PROFILE_BIN} (cargo --profile ${PROFILE_PROFILE} --features ${PROFILE_FEATURE})"
	cargo build --profile "${PROFILE_PROFILE}" --features "${PROFILE_FEATURE}" || fDie "profiler build failed (app problem)"
	mkdir -p "${profile_dir}"

	## Bring up a private in-memory display so the profiler window never touches the
	## user's visible session (renders via software GL / llvmpipe on Xvfb).
	local headless="${here}/utility/gui-headless.bash"
	## Not :99 - rapid-photo-downloader-pro uses that display for its own testing.
	export CICD_HEADLESS_DISPLAY="${CICD_HEADLESS_DISPLAY:-${RPD_HEADLESS_DISPLAY:-:98}}"
	local hdisp="${CICD_HEADLESS_DISPLAY}"
	if ! "${headless}" start >/dev/null 2>&1; then
		((PROFILE_STRICT)) && fDie "profiler: headless display failed to start"
		fEcho "WARNING: profiler skipped: headless display failed to start"; return 0
	fi

	## Born canonical (role "frequent"); the rotation retags the newest as "latest".
	local out="${profile_dir}/flame_${stamp}_frequent.svg"
	fEcho_Clean "running app ${PROFILE_SECS}s under sampler on headless ${hdisp} ..."
	local prc=0
	SILK_PROFILE_OUT="${out}" SILK_PROFILE_SECS="${PROFILE_SECS}" DISPLAY="${hdisp}" \
		"${root}/${PROFILE_BIN}" --shell "python3 ${abs_script} ${PROFILE_WORKLOAD_ARGS}" || prc=$?
	"${headless}" stop >/dev/null 2>&1 || true
	((prc == 0)) || fDie "profiler run failed (non-zero exit - app problem)"
	[[ -s "$out" ]] || fDie "profiler produced no SVG (app problem): ${out}"
	gfs_rotate "${profile_dir}" flame svg
	## Rotation renamed this run's file (newest) to the "latest" role.
	local latest="${profile_dir}/flame_${stamp}_latest.svg"
	[[ -e "$latest" ]] || latest="$out"
	fEcho "OK: flamegraph: ${latest}"
	fEcho_Clean "open: ${latest}  (in a browser)"

	## Hot-spot summary into the log (non-fatal, no marker - the marker is for the
	## per-session --check gate, not the pipeline).
	local report="${here}/utility/flame-report.py"
	if [[ -f "$report" ]]; then
		fEcho_Clean ""
		python3 "$report" --dir "${profile_dir}" 2>/dev/null || fEcho_Clean "hot spots: (report unavailable)"
	fi
}
fSection "4/8  Profiler"
run_profiler

## Stage 5: release builds.
fSection "5/8  Release build (native)"
"${RELEASE_NATIVE_CMD[@]}"
[[ -f "${RELEASE_NATIVE_BIN}" ]] || fDie "native release binary missing: ${RELEASE_NATIVE_BIN}"
fEcho "OK: native release: ${RELEASE_NATIVE_BIN} ($(du -h "${RELEASE_NATIVE_BIN}" | cut -f1))"
built_arts=("${RELEASE_NATIVE_OSARCH:-native}|${RELEASE_NATIVE_BIN}")
if ((BUILD_CROSS)) && ((${#CROSS_TARGETS[@]})); then
	for t in "${CROSS_TARGETS[@]}"; do
		local_label="${t%%|*}"; rest="${t#*|}"; osarch="${rest%%|*}"; rest="${rest#*|}"; art="${rest%%|*}"; cmd="${rest#*|}"
		fSection "5/8  Release build: ${local_label}"
		eval "${cmd}"
		[[ -f "${art}" ]] || fDie "missing artifact for ${local_label}: ${art}"
		fEcho "OK: ${local_label}: ${art} ($(du -h "${art}" | cut -f1))"
		built_arts+=("${osarch}|${art}")
	done
fi

## Collect the built binaries under versioned names + a sha256 checksums file,
## ready to attach to a release as plain uploads. Version = Cargo.toml alone.
if [[ -n "${RELEASE_ARTIFACT_DIR:-}" ]]; then
	ver="$(sed -n 's/^version *= *"\(.*\)".*/\1/p' "${root}/${VERSION_MANIFEST}" | head -1)"
	[[ -n "$ver" ]] || fDie "no version found in ${VERSION_MANIFEST}"
	art_dir="${root}/${RELEASE_ARTIFACT_DIR}"
	rm -rf "${art_dir}"; mkdir -p "${art_dir}"
	sums="${EXE_NAME}-${ver}-sha256sums.txt"
	for pair in "${built_arts[@]}"; do
		osarch="${pair%%|*}"; src="${pair#*|}"
		ext=""; [[ "$src" == *.exe ]] && ext=".exe"
		cp -f "${src}" "${art_dir}/${EXE_NAME}-${ver}-${osarch}${ext}"
	done
	write_sums
	fEcho "OK: ${#built_arts[@]} release artifact(s) + ${sums} -> ${RELEASE_ARTIFACT_DIR}/"
	((BUILD_CROSS)) || fEcho_Clean "note: cross targets skipped - artifact set is partial (native only)"
fi

## Stage 6: packages. Build distributables from the stage-5 binaries (never rebuilt).
## Linux -> .deb + .rpm per built arch (cargo-deb / cargo-generate-rpm, metadata in
## source/Cargo.toml); Windows -> one self-contained NSIS installer .exe per arch
## (upgrades in place). macOS (.dmg) + BSD are deferred - no cross toolchain here.
## Skipped under --quick; a missing tool warns (non-gating) rather than aborting.
build_packages(){
	((PACKAGE_ENABLE)) || { fEcho_Clean "packages disabled"; return 0; }
	[[ -n "${art_dir:-}" ]] || { fEcho "WARNING: packages skipped (no RELEASE_ARTIFACT_DIR)"; return 0; }
	local pair osarch bin triple out nsi rc made=0
	local rpmver="${ver//-/\~}"   ## RPM versions forbid '-' (it splits version-release); 1.0.0-beta1 -> 1.0.0~beta1
	for pair in "${built_arts[@]}"; do
		osarch="${pair%%|*}"; bin="${pair#*|}"
		case "$osarch" in
			linux-x86_64) triple="" ;;
			linux-arm64)  triple="aarch64-unknown-linux-gnu" ;;
			windows-*)    triple="" ;;   ## handled below
			*) continue ;;
		esac

		## Linux: .deb then .rpm. Both package the existing binary (no rebuild).
		if [[ "$osarch" == linux-* ]]; then
			if command -v cargo-deb >/dev/null 2>&1; then
				local -a da=(deb --no-build --no-strip --manifest-path source/Cargo.toml
					--output "${art_dir}/${EXE_NAME}-${ver}-${osarch}.deb")
				[[ -n "$triple" ]] && da+=(--target "$triple")
				if cargo "${da[@]}" >/dev/null; then fEcho "OK: .deb (${osarch})"; made=$((made+1))
				else fEcho "WARNING: .deb build failed (${osarch})"; fi
			else fEcho "WARNING: cargo-deb missing; .deb skipped (${osarch})"; fi

			if command -v cargo-generate-rpm >/dev/null 2>&1; then
				## -p is the crate DIR (source/), assets resolve from CWD (repo root),
				## so target/release/silkterm is found; -s overrides the RPM-illegal version.
				local -a ra=(generate-rpm -p source -s "version = \"${rpmver}\""
					--output "${art_dir}/${EXE_NAME}-${ver}-${osarch}.rpm")
				[[ -n "$triple" ]] && ra+=(--target "$triple" --arch aarch64)
				if cargo "${ra[@]}" >/dev/null; then fEcho "OK: .rpm (${osarch})"; made=$((made+1))
				else fEcho "WARNING: .rpm build failed (${osarch})"; fi
			else fEcho "WARNING: cargo-generate-rpm missing; .rpm skipped (${osarch})"; fi
		fi

		## Windows: one self-contained NSIS installer .exe per arch.
		if [[ "$osarch" == windows-* ]]; then
			if command -v makensis >/dev/null 2>&1 && [[ -f "${root}/${NSIS_TEMPLATE}" ]]; then
				out="${art_dir}/${EXE_NAME}-${ver}-${osarch}-setup.exe"
				nsi="$(mktemp --suffix=.nsi)"
				sed -e "s|@VERSION@|${ver}|g" -e "s|@ARCH@|${osarch}|g" \
					-e "s|@SRCEXE@|${root}/${bin}|g" -e "s|@OUTFILE@|${out}|g" \
					"${root}/${NSIS_TEMPLATE}" > "${nsi}"
				rc=0; makensis -V2 "${nsi}" >/dev/null || rc=$?
				rm -f "${nsi}"
				if ((rc == 0)) && [[ -f "$out" ]]; then fEcho "OK: installer (${osarch})"; made=$((made+1))
				else fEcho "WARNING: NSIS installer failed (${osarch})"; fi
			else fEcho "WARNING: makensis/template missing; installer skipped (${osarch})"; fi
		fi
	done
	write_sums
	fEcho "OK: ${made} package(s) -> ${RELEASE_ARTIFACT_DIR}/ (macOS/BSD deferred)"
}
fSection "6/8  Packages"
if ((quick)); then
	fEcho_Clean "packages skipped (--quick)"
else
	build_packages
fi

## Stage 7: dogfood. Two independent installs (fixed overwrite + rotating dated copy).
fSection "7/8  Dogfood (install native release locally)"
df_did=0

## 7a. Fixed name: overwrite EXE_NAME (the stable path you launch by hand).
if ((${#DOGFOOD_FIXED_DESTS[@]})); then
	if [[ -n "$fixed_dest" ]]; then
		cp -f "${RELEASE_NATIVE_BIN}" "${fixed_dest}/${EXE_NAME}"
		fEcho "OK: installed (fixed) -> ${fixed_dest}/${EXE_NAME}"
		df_did=1
	else
		fEcho "WARNING: no fixed dogfood dest exists (${DOGFOOD_FIXED_DESTS[*]}); skipping"
	fi
fi

## 7b. Rotating name: dated copy so builds coexist; prune older ones not running.
if ((${#DOGFOOD_ROTATING_DESTS[@]})) && [[ -n "${DOGFOOD_PREFIX:-}" ]]; then
	[[ -z "$rot_dest" && -n "$rot_target" ]] && mkdir -p "$rot_target" 2>/dev/null && rot_dest="$rot_target"
	if [[ -n "$rot_dest" && -w "$rot_dest" ]]; then
		df_name="${DOGFOOD_PREFIX}_${stamp}"
		cp -f "${RELEASE_NATIVE_BIN}" "${rot_dest}/${df_name}"
		chmod +x "${rot_dest}/${df_name}"
		fEcho "OK: installed (rotating) -> ${rot_dest}/${df_name}"
		pruned=0
		for old in "${rot_dest}/${DOGFOOD_PREFIX}_"*; do
			[[ -e "$old" ]] || continue                  # no-match glob (nullglob is off)
			[[ "$(basename "$old")" == "$df_name" ]] && continue
			if in_use "$old"; then
				fEcho_Clean "kept (running): $(basename "$old")"
			else
				rm -f "$old" && pruned=$((pruned + 1))
			fi
		done
		if ((pruned)); then fEcho_Clean "pruned ${pruned} old copy(ies) not in use"; fi
		df_did=1
	else
		fEcho "WARNING: no rotating dogfood dest writable (${DOGFOOD_ROTATING_DESTS[*]}); skipping"
	fi
fi

if ((! df_did)); then fEcho_Clean "dogfood disabled"; fi

## Refresh README screenshots (skipped under --quick; non-fatal - a miss never
## aborts). Runs before publish so changed images get committed; rendering needs
## a headless X + magick, so a failure just warns.
shots_hook="${root}/cicd/utility/screenshots.bash"
if ((! SHOTS_ENABLE)); then
	fEcho_Clean "screenshots disabled"
elif ((quick)); then
	fEcho_Clean "screenshots skipped (--quick)"
elif [[ -x "$shots_hook" ]]; then
	fEcho_Clean "refreshing README screenshots ..."
	if SILK_BIN="${root}/target/release/silkterm" "$shots_hook" "${root}"; then
		fEcho "OK: screenshots"
	else
		fEcho "WARNING: screenshot hook failed (non-fatal)"
	fi
fi

## Re-record the demo video (same gating shape as screenshots: off by default,
## skipped under --quick, never aborts). The video GFS-rotates into
## ../private/demo-video/; the README highlight gif lands in assets/demo.gif.
demo_hook="${root}/cicd/utility/demo-video/demo-video.py"
if ((! ${DEMO_ENABLE:-0})); then
	fEcho_Clean "demo video disabled"
elif ((quick)); then
	fEcho_Clean "demo video skipped (--quick)"
elif [[ -f "$demo_hook" ]]; then
	fEcho_Clean "recording demo video ..."
	if SILK_BIN="${root}/target/release/silkterm" python3 "$demo_hook"; then
		fEcho "OK: demo video"
	else
		fEcho "WARNING: demo video hook failed (non-fatal)"
	fi
fi

## Stage 8: backup + publish.
fSection "8/8  Backup + publish"
## Always run the publisher quiet: cicd already gave the initial prompt, so skip
## its redundant continue-prompt. With no message it still lets git open the editor.
pub_flags=(--quiet)
if ((${#GIT_PUBLISH[@]} == 0)); then
	fEcho_Clean "publish disabled"
elif [[ -n "$publish_msg" ]]; then
	## Hands-off: quiet env skips the script's continue-prompt; the GIT_EDITOR
	## helper fills the empty commit message so `git commit` won't open an editor.
	fEcho_Clean "hands-off publish (commit message: \"${publish_msg}\")"
	GIT_BACKUP_AND_PUBLISH_QUIET=1 GIT_AUTO_MESSAGE="${publish_msg}" \
		GIT_EDITOR="${here}/utility/git-auto-msg.bash" "${GIT_PUBLISH[@]}" "${pub_flags[@]}"
	fEcho "OK: published"
else
	"${GIT_PUBLISH[@]}" "${pub_flags[@]}"
	fEcho "OK: published"
fi

fSection "${APP_NAME} CI/CD: done."
fEcho_Clean


##	History:
##		- 2026-06-05 JC: Created.
##		- 2026-07-22 JC: Stage 0 remote sync - fetch, fast-forward if safely behind, abort if diverged.
