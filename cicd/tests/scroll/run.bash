#!/usr/bin/env bash

#  shellcheck disable=2001  ## 'See if you can use ${variable//search/replace} instead.' Complains about good uses of sed.
#  shellcheck disable=2086  ## 'Double quote to prevent globbing and word splitting.' (OK for integers.)
#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.'
#  shellcheck disable=2181  ## 'Check exit code directly, not indirectly with $?.'

##	- Purpose:
##		Headless scroll regression harness. Drives SilkTerm on a private Xvfb with
##		SILK_SCROLLDBG on and deterministic full-redraw scenes that model how real
##		full-screen apps repaint, then checks the per-frame trace for the behaviour
##		each app is supposed to have:
##		   less / vim   - no static top band: the smooth slide engages, monotone (no bounce)
##		   nano / muffer - static title bar held still, the region under it slides
##		Plain shell-output easing is covered by the library tests (cargo test); the
##		"jumping / re-listing / bottom-up" symptoms map to those monotonicity checks.
##		Scenes self-scroll on a timer - no key injection (unreliable here), so the
##		result is deterministic. --real also smoke-tests the actual apps (best effort).
##	- Syntax:
##		run.bash [options]
##		   --bin PATH      SilkTerm binary (default: target/debug then target/release)
##		   --display :N    headless display (default: $CICD_HEADLESS_DISPLAY or :98)
##		   --settle SECS   idle before scrolling, past GL warmup (default 13)
##		   --capture SECS  scrolling capture window per scene (default 16)
##		   --step SECS     seconds between scene repaints (default 0.15)
##		   --real          also launch real less/nano/vim.tiny (smoke, non-fatal)
##		   --keep          leave the Xvfb up and keep the trace files
##		   --strict        treat environment skips as failures
##		   -v, --verbose   show per-scene frame counts
##		   -h, --help
##	- Exit: 0 all pass/skip, 1 a real regression was measured.
##	- Notes: uses cicd/utility/gui-headless.bash (:98, never :0). Kills only the
##		binary it launched (PID + /proc/PID/exe path checked), never by name.
##	History: At bottom of script.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


set -Eeuo pipefail

meDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${meDir}/../../.." && pwd)"                     ## repo root (github/)
headless="${root}/cicd/utility/gui-headless.bash"

## Output helpers, same as cicd.bash: fEcho / fEcho_Clean.
declare -i _wasLastEchoBlank=0
fEcho_Clean(){ if [[ -n "${1:-}" ]]; then echo -e "$*"; _wasLastEchoBlank=0; elif [[ $_wasLastEchoBlank -eq 0 ]] && echo; then _wasLastEchoBlank=1; fi; }
fEcho(){ if [[ -n "$*" ]]; then fEcho_Clean "[ $* ]"; else fEcho_Clean ""; fi; }
_letterbox="••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••"
fSection(){ fEcho_Clean; fEcho_Clean "${_letterbox}"; fEcho "$*"; }
fDie(){ { fEcho_Clean; fEcho "FAILED: $*"; } >&2; exit 1; }
trap 'rc=$?; [[ $rc -ne 0 && $rc -ne 1 ]] && printf "\n[ scroll harness ABORTED (exit %s) at line %s: %s ]\n" "$rc" "$LINENO" "$BASH_COMMAND" >&2; exit $rc' ERR

## Options.
bin=""; display="${CICD_HEADLESS_DISPLAY:-${RPD_HEADLESS_DISPLAY:-:98}}"
settle=13; capture=16; step=0.15; do_real=0; keep=0; strict=0; verbose=0
while (($#)); do case "$1" in
	--bin)      bin="${2-}"; shift 2 ;;
	--display)  display="${2-}"; shift 2 ;;
	--settle)   settle="${2-}"; shift 2 ;;
	--capture)  capture="${2-}"; shift 2 ;;
	--step)     step="${2-}"; shift 2 ;;
	--real)     do_real=1; shift ;;
	--keep)     keep=1; shift ;;
	--strict)   strict=1; shift ;;
	-v|--verbose) verbose=1; shift ;;
	-h|--help)  sed -n '/^##	- Purpose:/,/^##	History:/p' "${BASH_SOURCE[0]}" | sed '$d; s/^##	\{0,1\}//'; exit 0 ;;
	*) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
esac; done

fSection "SilkTerm scroll regression (headless)"

## Resolve the binary: prefer debug (exists after the debug-build stage), then release.
if [[ -z "$bin" ]]; then
	for cand in "${root}/target/debug/silkterm" "${root}/target/release/silkterm"; do
		[[ -x "$cand" ]] && { bin="$cand"; break; }
	done
fi

## Environment preconditions -> skip (non-fatal) unless --strict.
skip=""
[[ -n "$bin" && -x "$bin" ]] || skip="no SilkTerm binary (build it first, or pass --bin)"
[[ -z "$skip" ]] && ! command -v python3 >/dev/null 2>&1 && skip="python3 not found"
[[ -z "$skip" ]] && ! command -v Xvfb    >/dev/null 2>&1 && skip="Xvfb not found (no headless display)"
[[ -z "$skip" ]] && [[ ! -x "$headless" ]] && skip="gui-headless.bash missing: ${headless}"
if [[ -n "$skip" ]]; then
	((strict)) && fDie "scroll harness: ${skip}"
	fEcho "WARNING: skipped: ${skip}"; exit 0
fi
fEcho_Clean "binary ....: ${bin}"
fEcho_Clean "display ...: ${display}   settle ${settle}s  capture ${capture}s  step ${step}s"

## Throwaway config + temp workspace (nothing personal leaks; cleaned on exit).
work="$(mktemp -d "${TMPDIR:-/tmp}/silk-scroll.XXXXXX")"
cfg="${work}/config.toml"
cat >"$cfg" <<-'TOML'
	## Throwaway config for the scroll harness - not a user config.
	smooth_scroll_apps = true
	scroll_tau_ms = 120
	transparent_background = false
	text_glow = false
	background_image = ""
	columns = 100
	rows = 34
	cursor_animation = "none"
TOML

run_dir="/tmp/cicd-gui-headless-${USER}"
auth="${run_dir}/Xauthority-${display#:}"

## Bring up the private display (with a WM - winit needs one on bare Xvfb to get
## the events that drive rendering). Only stop it on exit if we started it.
started_headless=0
if "$headless" status 2>/dev/null | grep -q 'no Xvfb'; then
	if "$headless" start --wm >/dev/null 2>&1; then started_headless=1
	else
		((strict)) && fDie "headless display failed to start"
		fEcho "WARNING: skipped: headless display failed to start"; exit 0
	fi
fi

cleanup(){
	if ((keep)); then
		fEcho_Clean "kept: traces in ${work}  (display left up)"
	else
		((started_headless)) && "$headless" stop >/dev/null 2>&1 || true
		rm -rf "$work" 2>/dev/null || true
	fi
}
trap cleanup EXIT

## Kill a PID only if it is still the binary we launched (PID + exe path), never by
## name - the repo path contains "silkterm" and a dogfood copy may be running.
kill_ours(){
	local pid="$1" want; want="$(realpath -e "$2" 2>/dev/null || true)"
	local exe; exe="$(realpath -e "/proc/${pid}/exe" 2>/dev/null || true)"
	if [[ -n "$want" && "$exe" == "$want" ]]; then
		kill "$pid" 2>/dev/null || true
		local i; for i in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
		kill -9 "$pid" 2>/dev/null || true
	fi
}

pass=0; fail=0; miss=0

## Run one deterministic scene and judge its trace. shape|mode|expect_st.
run_scene(){
	local label="$1" shape="$2" mode="$3" est="$4"
	local trace="${work}/${label}.trace"
	SILK_SCROLLDBG=1 SILK_SCENE_SETTLE="$settle" SILK_SCENE_STEP="$step" \
		DISPLAY="$display" XAUTHORITY="$auth" LIBGL_ALWAYS_SOFTWARE=1 SHELL=/bin/dash \
		"$bin" --config "$cfg" --shell "/bin/dash ${meDir}/scenes/scene.bash ${shape}" \
		>"${work}/${label}.log" 2>"$trace" &
	local pid=$!

	## GL warmup under llvmpipe swings widely with machine load (cicd runs this while
	## the release + cross builds may still be busy), so a fixed sleep can expire before
	## a single frame renders = a false "0 trace frames" skip. Poll until the scene is
	## actually producing frames, then stop; bounded by a generous ceiling so a truly
	## dead binary still exits. The scene self-scrolls forever, so more wall time just
	## means more frames - never a hang.
	local want=60 ceiling=$((settle + capture + 60)) frames=0
	SECONDS=0
	while ((SECONDS < ceiling)); do
		kill -0 "$pid" 2>/dev/null || break
		frames=$(grep -c SCROLLDBG "$trace" 2>/dev/null || true); frames=${frames:-0}
		((frames >= want)) && break
		sleep 0.5
	done
	kill_ours "$pid" "$bin"
	wait "$pid" 2>/dev/null || true

	((verbose)) && fEcho_Clean "  ${label}: $(grep -c SCROLLDBG "$trace" 2>/dev/null || echo 0) trace frames"
	local rc=0
	python3 "${meDir}/analyze.py" --mode "$mode" --expect-st "$est" --label "$label" <"$trace" \
		| sed 's/^/  /' || rc=$?
	case "$rc" in
		0) pass=$((pass + 1)) ;;
		1) fail=$((fail + 1)) ;;
		*) miss=$((miss + 1)) ;;
	esac
}

fSection "Deterministic scenes"
run_scene less   less   slide 0
run_scene vim    vim    slide 0
run_scene nano   nano   slide 1
run_scene muffer muffer slide 2

## Best-effort real-app smoke (never fails the suite): prove the real apps render
## under SilkTerm (enter alt-screen, no hang) - regresses e.g. the cosmic-text hang
## and the alt-screen enter/exit hard-cut. No key injection, so it does not assert
## scroll correctness - that is what the deterministic scenes above are for.
real_smoke(){
	local app="$1" exe; exe="$(command -v "$app" 2>/dev/null || true)"
	[[ -n "$exe" ]] || { fEcho_Clean "  ${app}: not installed, skipped"; return 0; }
	local file="${work}/${app}.txt" launch="${work}/${app}.launch.bash" trace="${work}/${app}.real.trace"
	seq 1 400 | sed 's/^/line /' >"$file"
	printf '#!/bin/dash\nexec %s %s\n' "$exe" "$file" >"$launch"
	SILK_SCROLLDBG=1 DISPLAY="$display" XAUTHORITY="$auth" LIBGL_ALWAYS_SOFTWARE=1 SHELL=/bin/dash \
		"$bin" --config "$cfg" --shell "/bin/dash ${launch}" \
		>"${work}/${app}.real.log" 2>"$trace" &
	local pid=$!
	sleep "$((settle + 4))"
	local alive=0; kill -0 "$pid" 2>/dev/null && alive=1
	kill_ours "$pid" "$bin"
	wait "$pid" 2>/dev/null || true
	## An idle real app only builds on dirty frames, so a handful is expected; the
	## signal is that it stayed alive (no hang) and rendered the alt screen (frames>0).
	local n; n="$(grep -c SCROLLDBG "$trace" 2>/dev/null || echo 0)"
	if ((alive)) && ((n >= 1)); then
		fEcho_Clean "  ${app}: OK (alive, entered alt-screen: ${n} frame(s))"
	else
		fEcho_Clean "  ${app}: INFO (alive=${alive}, ${n} frames) - not asserted"
	fi
}

if ((do_real)); then
	fSection "Real-app smoke (best effort)"
	real_smoke less
	real_smoke nano
	real_smoke vim.tiny
fi

fSection "Summary"
fEcho_Clean "pass ${pass}   fail ${fail}   skip ${miss}"
if ((fail)); then
	fEcho "FAILED: ${fail} scroll regression(s) measured"
	exit 1
fi
if ((miss)) && ((strict)); then
	fEcho "FAILED: ${miss} scenario(s) skipped under --strict"
	exit 1
fi
fEcho "OK: no scroll regressions"
exit 0


##	History:
##		- 20260706 JC: Created.
