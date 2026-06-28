#!/usr/bin/env bash
# Local CI/CD pipeline. Generic engine - per-project settings live in config.bash.
#
#   cicd/cicd.bash [options]
#
# Stages (fail-fast: any error aborts before the next stage):
#   1. debug build          5. dogfood (copy native release to a local bin dir)
#   2. regression tests     6. backup + publish to git
#   3. profiler (flamegraph SVG; non-gating artifact - see failure policy)
#   4. release builds (native + cross targets)
#
# Options:
#   -y, --yes           run unattended (no confirm prompt)
#   -m, --message MSG   publish hands-off with this commit message (no editor)
#   --no-cross          skip cross-target release builds
#   --no-profile        skip the profiler stage
#   --no-dogfood        skip installing the native release locally
#   --no-publish        skip the git backup + publish stage
#
# Reuse: copy the cicd/ directory into another project and edit config.bash.
#
# SPDX-License-Identifier: GPL-2.0-or-later

set -Eeuo pipefail

#----- locate + load -----------------------------------------------------------
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${here}/.." && pwd)"   # the git repo root (cicd/..)
# rustup toolchain (cross targets, edition 2024) + zig must beat system rust.
export PATH="${HOME}/.cargo/bin:${HOME}/.local/bin:${PATH}"
# shellcheck source=/dev/null
source "${here}/config.bash"
cd "${root}"
stamp="$(date +%Y%m%d-%H%M%S)"

#----- options -----------------------------------------------------------------
assume_yes=0; cli_message=""
while (($#)); do case "$1" in
	-y|--yes)         assume_yes=1; shift ;;
	--no-cross)       BUILD_CROSS=0; shift ;;
	--no-profile)     PROFILE_ENABLE=0; shift ;;
	--no-dogfood)     DOGFOOD_DESTS=(); shift ;;
	--no-publish)     GIT_PUBLISH=(); shift ;;
	--message=*|-m=*) cli_message="${1#*=}"; shift ;;
	-m|--message)     cli_message="${2-}"; shift; (($#)) && shift ;;
	-h|--help)        sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
	*) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
esac; done

# Publish commit message: -m wins, then config, then a default when unattended.
# Empty -> publish interactively (git commit opens an editor); when interactive
# we offer to capture a message at the preflight prompt below.
publish_msg=""
if   [[ -n "$cli_message" ]];              then publish_msg="$cli_message"
elif [[ -n "${PUBLISH_AUTO_MESSAGE:-}" ]]; then publish_msg="$PUBLISH_AUTO_MESSAGE"
elif ((assume_yes));                       then publish_msg="${APP_NAME} CI/CD ${stamp}"
fi

#----- output helpers ----------------------------------------------------------
b=$'\e[1m'; dim=$'\e[2m'; grn=$'\e[32m'; ylw=$'\e[33m'; red=$'\e[31m'; rst=$'\e[0m'
hr(){ printf '%s\n' "${dim}--------------------------------------------------------------------------${rst}"; }
step(){ hr; printf '%s[ %s ] %s%s\n' "${b}" "$(date +%H:%M:%S)" "$*" "${rst}"; }
note(){ printf '  %s\n' "$*"; }
ok(){   printf '%s  OK: %s%s\n' "${grn}" "$*" "${rst}"; }
warn(){ printf '%s  WARN: %s%s\n' "${ylw}" "$*" "${rst}" >&2; }
die(){  printf '\n%sCICD FAILED: %s%s\n' "${red}" "$*" "${rst}" >&2; exit 1; }
trap 'rc=$?; printf "\n%sCICD ABORTED (exit %s) at line %s: %s%s\n" "${red}" "$rc" "$LINENO" "$BASH_COMMAND" "${rst}" >&2; exit $rc' ERR

#----- preflight: say what will happen, with resolved paths --------------------
abs_script="${root}/${PROFILE_WORKLOAD_SCRIPT}"
profile_dir="$(cd "${root}" && mkdir -p "${PROFILE_OUT_DIR}" 2>/dev/null; cd "${PROFILE_OUT_DIR}" 2>/dev/null && pwd || echo "${root}/${PROFILE_OUT_DIR}")"
dogfood_dest=""; for d in "${DOGFOOD_DESTS[@]:-}"; do [[ -d "$d" ]] && { dogfood_dest="$d"; break; }; done

printf '\n%s%s local CI/CD%s\n' "${b}" "${APP_NAME}" "${rst}"
note "Repo root ............: ${root}"
note "Debug build .........: ${DEBUG_BUILD_CMD[*]}"
note "Tests ...............: ${TEST_CMD[*]}"
if ((PROFILE_ENABLE)); then
	note "Profiler ............: ${PROFILE_SECS}s run -> flamegraph SVG"
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

#----- 1. debug build ----------------------------------------------------------
step "1/6  Debug build"
"${DEBUG_BUILD_CMD[@]}"
ok "debug build"

#----- 2. regression tests -----------------------------------------------------
step "2/6  Regression tests"
"${TEST_CMD[@]}"
ok "tests passed"

#----- 3. profiler (non-gating artifact; classified failure) -------------------
rotate_profiles(){   # GFS retention: keep first + newest-per-day/month/year + last N
	local dir="$1"; shopt -s nullglob; local all=("$dir"/flame_*.svg); shopt -u nullglob
	local -a f=(); local x; for x in "${all[@]}"; do [[ "$x" == *"/flame_latest.svg" ]] || f+=("$x"); done
	local n=${#f[@]}; ((n)) || return 0
	IFS=$'\n' read -r -d '' -a f < <(printf '%s\n' "${f[@]}" | sort && printf '\0')
	declare -A keep=(); keep["${f[0]}"]=1
	local i; for ((i = n>PROFILE_KEEP_FREQUENT ? n-PROFILE_KEEP_FREQUENT : 0; i<n; i++)); do keep["${f[i]}"]=1; done
	declare -A day mon yr; for x in "${f[@]}"; do
		local ts; ts="$(basename "$x")"; ts="${ts#flame_}"; ts="${ts%.svg}"; local d="${ts%%-*}"
		day["$d"]="$x"; mon["${d:0:6}"]="$x"; yr["${d:0:4}"]="$x"
	done
	for x in "${day[@]}" "${mon[@]}" "${yr[@]}"; do keep["$x"]=1; done
	for x in "${f[@]}"; do [[ -n "${keep[$x]:-}" ]] || { rm -f "$x"; note "pruned $(basename "$x")"; }; done
}
run_profiler(){
	((PROFILE_ENABLE)) || { note "profiler disabled"; return 0; }
	# Mundane/environmental reasons -> skip with a warning (not the app's fault),
	# unless PROFILE_STRICT. Genuine run failures below still abort.
	local skip=""
	[[ -n "${DISPLAY:-}" ]]            || skip="no DISPLAY (headless session)"
	[[ -z "$skip" ]] && ! command -v python3 >/dev/null 2>&1 && skip="python3 not found"
	[[ -z "$skip" ]] && [[ ! -f "$abs_script" ]] && skip="workload missing: ${abs_script}"
	[[ -z "$skip" ]] && command -v xset >/dev/null 2>&1 && ! timeout 3 xset -q >/dev/null 2>&1 && skip="X server not reachable on ${DISPLAY}"
	if [[ -n "$skip" ]]; then
		((PROFILE_STRICT)) && die "profiler: ${skip}"
		warn "profiler skipped: ${skip}"; return 0
	fi
	# From here, a failure means the app is at fault -> abort.
	note "building ${PROFILE_BIN} (cargo --profile ${PROFILE_PROFILE} --features ${PROFILE_FEATURE})"
	cargo build --profile "${PROFILE_PROFILE}" --features "${PROFILE_FEATURE}" || die "profiler build failed (app problem)"
	mkdir -p "${profile_dir}"
	local out="${profile_dir}/flame_${stamp}.svg"
	note "running app ${PROFILE_SECS}s under sampler (a window will appear briefly) ..."
	if ! SILK_PROFILE_OUT="${out}" SILK_PROFILE_SECS="${PROFILE_SECS}" \
		"${root}/${PROFILE_BIN}" --shell "python3 ${abs_script} ${PROFILE_WORKLOAD_ARGS}"; then
		die "profiler run failed (non-zero exit - app problem)"
	fi
	[[ -s "$out" ]] || die "profiler produced no SVG (app problem): ${out}"
	cp -f "$out" "${profile_dir}/flame_latest.svg"
	rotate_profiles "${profile_dir}"
	ok "flamegraph: ${out}"
	note "latest: ${profile_dir}/flame_latest.svg  (open in a browser)"
}
step "3/6  Profiler"
run_profiler

#----- 4. release builds -------------------------------------------------------
step "4/6  Release build (native)"
"${RELEASE_NATIVE_CMD[@]}"
[[ -f "${RELEASE_NATIVE_BIN}" ]] || die "native release binary missing: ${RELEASE_NATIVE_BIN}"
ok "native release: ${RELEASE_NATIVE_BIN} ($(du -h "${RELEASE_NATIVE_BIN}" | cut -f1))"
if ((BUILD_CROSS)) && ((${#CROSS_TARGETS[@]})); then
	for t in "${CROSS_TARGETS[@]}"; do
		local_label="${t%%|*}"; rest="${t#*|}"; art="${rest%%|*}"; cmd="${rest#*|}"
		step "4/6  Release build: ${local_label}"
		# shellcheck disable=2086
		eval ${cmd}
		[[ -f "${art}" ]] || die "missing artifact for ${local_label}: ${art}"
		ok "${local_label}: ${art} ($(du -h "${art}" | cut -f1))"
	done
fi

#----- 5. dogfood --------------------------------------------------------------
step "5/6  Dogfood (install native release locally)"
if ((${#DOGFOOD_DESTS[@]} == 0)); then
	note "dogfood disabled"
elif [[ -z "${dogfood_dest}" ]]; then
	warn "no dogfood destination exists (${DOGFOOD_DESTS[*]}); skipping"
else
	cp -fv "${RELEASE_NATIVE_BIN}" "${dogfood_dest}/${EXE_NAME}"
	ok "installed -> ${dogfood_dest}/${EXE_NAME}"
fi

#----- 6. backup + publish -----------------------------------------------------
step "6/6  Backup + publish"
if ((${#GIT_PUBLISH[@]} == 0)); then
	note "publish disabled"
elif [[ -n "$publish_msg" ]]; then
	# Hands-off: quiet env skips the script's continue-prompt; the GIT_EDITOR
	# helper fills the empty commit message so `git commit` won't open an editor.
	note "hands-off publish (commit message: \"${publish_msg}\")"
	GIT_BACKUP_AND_PUBLISH_QUIET=1 GIT_AUTO_MESSAGE="${publish_msg}" \
		GIT_EDITOR="${here}/utility/git-auto-msg.bash" "${GIT_PUBLISH[@]}"
	ok "published"
else
	"${GIT_PUBLISH[@]}"
	ok "published"
fi

hr; printf '%s%s CI/CD: done.%s\n' "${grn}${b}" "${APP_NAME}" "${rst}"
