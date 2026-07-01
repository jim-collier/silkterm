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
##	   1. format (cargo fmt)
##	   2. debug build
##	   3. regression tests + lints (clippy gating, cargo-deny advisory)
##	   4. profiler (flamegraph SVG; non-gating artifact - see failure policy)
##	   5. release build (native + cross targets)
##	   6. dogfood (install native release locally)
##	   7. backup + publish to git (runs from repo root)
##	- Syntax:
##	  cicd/cicd.bash [options]
##	  Options:
##	   -y, --yes           run unattended (no confirm prompt)
##	   -m, --message MSG   publish hands-off with this commit message (no editor)
##	   --no-fmt            skip the formatter (cargo fmt) stage
##	   --no-cross          skip cross-target release builds
##	   --no-profile        skip the profiler stage
##	   --no-dogfood        skip installing the native release locally
##	   --no-publish        skip the git backup + publish stage
##	   --quick             skip the slow stages (cross-builds + profiling)
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
cd "${root}"
stamp="$(date +%Y%m%d-%H%M%S)"

## Parse options.
assume_yes=0; cli_message=""
while (($#)); do case "$1" in
	-y|--yes)         assume_yes=1; shift ;;
	--no-fmt)         FMT_CMD=(); shift ;;
	--no-cross)       BUILD_CROSS=0; shift ;;
	--no-profile)     PROFILE_ENABLE=0; shift ;;
	--no-dogfood)     DOGFOOD_DESTS=(); shift ;;
	--no-publish)     GIT_PUBLISH=(); shift ;;
	--quick)          BUILD_CROSS=0; PROFILE_ENABLE=0; shift ;;   ## skip the slow stages
	--message=*|-m=*) cli_message="${1#*=}"; shift ;;
	-m|--message)     cli_message="${2-}"; shift; (($#)) && shift ;;
	-h|--help)        sed -n '/^##	- Purpose:/,/^##	History:/p' "${BASH_SOURCE[0]}" | sed '$d; s/^##	\{0,1\}//'; exit 0 ;;
	*) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
esac; done

## Publish commit message: -m wins, then config, then a default when unattended.
## Empty -> publish interactively (git commit opens an editor); when interactive
## we offer to capture a message at the preflight prompt below.
publish_msg=""
if   [[ -n "$cli_message" ]];              then publish_msg="$cli_message"
elif [[ -n "${PUBLISH_AUTO_MESSAGE:-}" ]]; then publish_msg="$PUBLISH_AUTO_MESSAGE"
elif ((assume_yes));                       then publish_msg="${APP_NAME} CI/CD ${stamp}"
fi

## Output helpers.
b=$'\e[1m'; dim=$'\e[2m'; grn=$'\e[32m'; ylw=$'\e[33m'; red=$'\e[31m'; rst=$'\e[0m'
hr(){   echo; printf '%s\n' "${dim}••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••${rst}"; }
step(){ hr; printf '%s[ %s ] %s%s\n' "${b}" "$(date +%H:%M:%S)" "$*" "${rst}"; }
note(){ printf '  %s\n' "$*"; }
ok(){   printf '%s  OK: %s%s\n' "${grn}" "$*" "${rst}"; }
warn(){ printf '%s  WARN: %s%s\n' "${ylw}" "$*" "${rst}" >&2; }
die(){  printf '\n%sCICD FAILED: %s%s\n' "${red}" "$*" "${rst}" >&2; exit 1; }
trap 'rc=$?; printf "\n%sCICD ABORTED (exit %s) at line %s: %s%s\n" "${red}" "$rc" "$LINENO" "$BASH_COMMAND" "${rst}" >&2; exit $rc' ERR

## Preflight: show the plan with resolved paths, then confirm.
abs_script="${root}/${PROFILE_WORKLOAD_SCRIPT}"
profile_dir="$(cd "${root}" && mkdir -p "${PROFILE_OUT_DIR}" 2>/dev/null; cd "${PROFILE_OUT_DIR}" 2>/dev/null && pwd || echo "${root}/${PROFILE_OUT_DIR}")"
dogfood_dest=""; for d in "${DOGFOOD_DESTS[@]:-}"; do [[ -d "$d" && -w "$d" ]] && { dogfood_dest="$d"; break; }; done

#printf '\n%s%s local CI/CD%s\n'  "${b}"  "${APP_NAME}"  "${rst}"
printf  '\n%s%s local CI/CD%s\n'   ""      "${APP_NAME}"  ""
echo
note "Repo root ...........: ${root}"
note "Format ..............: ${FMT_CMD[*]:-(skipped)}"
note "Debug build .........: ${DEBUG_BUILD_CMD[*]}"
note "Tests ...............: ${TEST_CMD[*]}"
if ((PROFILE_ENABLE)); then
	note "Profiler ............: ${PROFILE_SECS}s run -> flamegraph SVG (on headless ${RPD_HEADLESS_DISPLAY:-:98})"
	note "  output dir ........: ${profile_dir}"
	note "  workload ..........: python3 ${PROFILE_WORKLOAD_SCRIPT} ${PROFILE_WORKLOAD_ARGS}"
else
	note "Profiler ............: (disabled)"
fi
note "Release (native) ....: ${RELEASE_NATIVE_CMD[*]} -> ${RELEASE_NATIVE_BIN}"
if ((BUILD_CROSS)) && ((${#CROSS_TARGETS[@]})); then
	note "Release (cross) .....:"
	for t in "${CROSS_TARGETS[@]}"; do note "    - ${t%%|*}"; done
else
	note "Release (cross) .....: (skipped)"
fi
note "Dogfood native to ...: ${dogfood_dest:-<none of: ${DOGFOOD_DESTS[*]:-} - will skip>}"
if ((${#GIT_PUBLISH[@]} == 0)); then
	note "Publish (last) ......: (disabled)"
elif [[ -n "$publish_msg" ]]; then
	note "Publish (last) ......: ${GIT_PUBLISH[*]} (hands-off: \"${publish_msg}\")"
else
	note "Publish (last) ......: ${GIT_PUBLISH[*]} (will prompt for message; blank = editor)"
fi
printf '\n%sFail-fast: any error aborts before the next stage.%s\n\n' "${dim}" "${rst}"

if ((! assume_yes)); then
	# Capture the commit message up front so the run can finish unattended.
	if ((${#GIT_PUBLISH[@]})) && [[ -z "$publish_msg" ]]; then
		read -r -p "Publish commit message (blank = open editor at publish): " m
		[[ -n "$m" ]] && publish_msg="$m"
	fi
	read -r -p "Proceed? [y/N]: " reply
	[[ "${reply,,}" == y* ]] || { echo "Aborted by user."; exit 0; }
fi

## Stage 1: format.
step "1/7  Format"
if ((${#FMT_CMD[@]} == 0)); then
	note "format skipped"
else
	"${FMT_CMD[@]}"
	ok "formatted (${FMT_CMD[*]})"
fi

## Stage 2: debug build.
step "2/7  Debug build"
"${DEBUG_BUILD_CMD[@]}"
ok "debug build"

## Stage 3: regression tests.
step "3/7  Regression tests"
"${TEST_CMD[@]}"
if [[ -n "${LINT_CMD+x}" ]] && ((${#LINT_CMD[@]})); then
	if "${LINT_PROBE[@]}" >/dev/null 2>&1; then
		"${LINT_CMD[@]}"
		ok "lints clean"
	else
		warn "lints skipped: ${LINT_PROBE[*]} failed (component not installed?)"
	fi
fi
if [[ -n "${DENY_CMD+x}" ]] && ((${#DENY_CMD[@]})); then
	if "${DENY_PROBE[@]}" >/dev/null 2>&1; then
		## Advisory-only for now: report license/advisory/duplicate findings
		## without failing the pipeline (tighten to gating once tuned).
		"${DENY_CMD[@]}" || warn "cargo-deny reported findings (non-gating)"
	else
		warn "deps check skipped: ${DENY_PROBE[*]} failed (cargo install cargo-deny)"
	fi
fi
ok "tests passed"

## Stage 4: profiler (non-gating artifact; failures classified below).
run_profiler(){
	((PROFILE_ENABLE)) || { note "profiler disabled"; return 0; }

	## Mundane/environmental reasons -> skip with a warning (not the app's fault),
	## unless PROFILE_STRICT. Genuine run failures below still abort. The app runs
	## on a private Xvfb (gui-headless.bash), so no visible DISPLAY is needed - only
	## Xvfb + python3 + the workload.
	local skip=""
	command -v python3 >/dev/null 2>&1 || skip="python3 not found"
	[[ -z "$skip" ]] && [[ ! -f "$abs_script" ]] && skip="workload missing: ${abs_script}"
	[[ -z "$skip" ]] && ! command -v Xvfb >/dev/null 2>&1 && skip="Xvfb not found (headless display unavailable)"
	if [[ -n "$skip" ]]; then
		((PROFILE_STRICT)) && die "profiler: ${skip}"
		warn "profiler skipped: ${skip}"; return 0
	fi

	## From here, a failure means the app is at fault -> abort.
	note "building ${PROFILE_BIN} (cargo --profile ${PROFILE_PROFILE} --features ${PROFILE_FEATURE})"
	cargo build --profile "${PROFILE_PROFILE}" --features "${PROFILE_FEATURE}" || die "profiler build failed (app problem)"
	mkdir -p "${profile_dir}"

	## Bring up a private in-memory display so the profiler window never touches the
	## user's visible session (renders via software GL / llvmpipe on Xvfb).
	local headless="${here}/utility/gui-headless.bash"
	## Not :99 - rapid-photo-downloader-pro uses that display for its own testing.
	export CICD_HEADLESS_DISPLAY="${CICD_HEADLESS_DISPLAY:-${RPD_HEADLESS_DISPLAY:-:98}}"
	local hdisp="${CICD_HEADLESS_DISPLAY}"
	if ! "${headless}" start >/dev/null 2>&1; then
		((PROFILE_STRICT)) && die "profiler: headless display failed to start"
		warn "profiler skipped: headless display failed to start"; return 0
	fi

	## Born canonical (role "frequent"); the rotation retags the newest as "latest".
	local out="${profile_dir}/flame_${stamp}_frequent.svg"
	note "running app ${PROFILE_SECS}s under sampler on headless ${hdisp} ..."
	local prc=0
	SILK_PROFILE_OUT="${out}" SILK_PROFILE_SECS="${PROFILE_SECS}" DISPLAY="${hdisp}" \
		"${root}/${PROFILE_BIN}" --shell "python3 ${abs_script} ${PROFILE_WORKLOAD_ARGS}" || prc=$?
	"${headless}" stop >/dev/null 2>&1 || true
	((prc == 0)) || die "profiler run failed (non-zero exit - app problem)"
	[[ -s "$out" ]] || die "profiler produced no SVG (app problem): ${out}"
	gfs_rotate "${profile_dir}" flame svg
	## Rotation renamed this run's file (newest) to the "latest" role.
	local latest="${profile_dir}/flame_${stamp}_latest.svg"
	[[ -e "$latest" ]] || latest="$out"
	ok "flamegraph: ${latest}"
	note "open: ${latest}  (in a browser)"
}
step "4/7  Profiler"
run_profiler

## Stage 5: release builds.
step "5/7  Release build (native)"
"${RELEASE_NATIVE_CMD[@]}"
[[ -f "${RELEASE_NATIVE_BIN}" ]] || die "native release binary missing: ${RELEASE_NATIVE_BIN}"
ok "native release: ${RELEASE_NATIVE_BIN} ($(du -h "${RELEASE_NATIVE_BIN}" | cut -f1))"
if ((BUILD_CROSS)) && ((${#CROSS_TARGETS[@]})); then
	for t in "${CROSS_TARGETS[@]}"; do
		local_label="${t%%|*}"; rest="${t#*|}"; art="${rest%%|*}"; cmd="${rest#*|}"
		step "5/7  Release build: ${local_label}"
		eval "${cmd}"
		[[ -f "${art}" ]] || die "missing artifact for ${local_label}: ${art}"
		ok "${local_label}: ${art} ($(du -h "${art}" | cut -f1))"
	done
fi

## Stage 6: dogfood.
step "6/7  Dogfood (install native release locally)"
if ((${#DOGFOOD_DESTS[@]} == 0)); then
	note "dogfood disabled"
elif [[ -z "${dogfood_dest}" ]]; then
	warn "no dogfood destination exists (${DOGFOOD_DESTS[*]}); skipping"
else
	cp -fv "${RELEASE_NATIVE_BIN}" "${dogfood_dest}/${EXE_NAME}"
	ok "installed -> ${dogfood_dest}/${EXE_NAME}"
fi

## Stage 7: backup + publish.
step "7/7  Backup + publish"
if ((${#GIT_PUBLISH[@]} == 0)); then
	note "publish disabled"
elif [[ -n "$publish_msg" ]]; then
	## Hands-off: quiet env skips the script's continue-prompt; the GIT_EDITOR
	## helper fills the empty commit message so `git commit` won't open an editor.
	note "hands-off publish (commit message: \"${publish_msg}\")"
	GIT_BACKUP_AND_PUBLISH_QUIET=1 GIT_AUTO_MESSAGE="${publish_msg}" \
		GIT_EDITOR="${here}/utility/git-auto-msg.bash" "${GIT_PUBLISH[@]}"
	ok "published"
else
	"${GIT_PUBLISH[@]}"
	ok "published"
fi

hr; printf '%s%s CI/CD: done.%s\n' "${grn}${b}" "${APP_NAME}" "${rst}"


##	History:
##		- 2026-06-05 JC: Created.
