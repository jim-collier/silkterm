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

##	Purpose:
##		Run / screenshot a GUI app on a private Xvfb display, never touching the
##		visible :0 session. No desktop environment is involved, so this sidesteps the
##		XFCE won't run twice per user" limit - Xvfb is a bare in-memory framebuffer,
##		not a session. Optional standalone xfwm4 (--wm) only if an app needs a WM.
##	Syntax:
##		gui-headless.bash start [--wm]      ## bring up Xvfb (+ optional xfwm4)
##		gui-headless.bash launch <cmd...>   ## run cmd in the background on it
##		gui-headless.bash shot <out.png>    ## capture the whole virtual screen
##		gui-headless.bash status
##		gui-headless.bash stop              ## kill only what we started
##	Notes:
##		Display/size is overridable via CICD_HEADLESS_DISPLAY / CICD_HEADLESS_SIZE
##		(legacy RPD_* names still honoured as fallbacks).
##	History: At bottom of script.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


set -euo pipefail

display="${CICD_HEADLESS_DISPLAY:-${RPD_HEADLESS_DISPLAY:-:99}}"
size="${CICD_HEADLESS_SIZE:-${RPD_HEADLESS_SIZE:-1920x1080x24}}"
num="${display#:}"
run_dir="/tmp/cicd-gui-headless-${USER}"
mkdir -p "$run_dir"
xvfb_pid="$run_dir/xvfb-${num}.pid"
wm_pid="$run_dir/wm-${num}.pid"
apps_pids="$run_dir/apps-${num}.pids"
auth="$run_dir/Xauthority-${num}"

alive() { [[ -f "$1" ]] && kill -0 "$(cat "$1")" 2>/dev/null; }

start() {
	if alive "$xvfb_pid"; then
		echo "Xvfb already on $display (pid $(cat "$xvfb_pid"))"
	else
		: > "$auth"
		Xvfb "$display" -screen 0 "$size" -nolisten tcp -auth "$auth" \
			>"$run_dir/xvfb-${num}.log" 2>&1 &
		echo $! > "$xvfb_pid"
		# Wait for the server to accept connections before returning.
		local ok=""
		for _ in $(seq 1 50); do
			if DISPLAY="$display" xdpyinfo >/dev/null 2>&1; then ok=1; break; fi
			sleep 0.1
		done
		[[ -n "$ok" ]] || { echo "Xvfb did not come up; see $run_dir/xvfb-${num}.log" >&2; exit 1; }
		echo "Started Xvfb on $display (pid $(cat "$xvfb_pid"), $size)"
	fi
	if [[ "${1:-}" == "--wm" ]] && ! alive "$wm_pid"; then
		DISPLAY="$display" xfwm4 --compositor=off >"$run_dir/wm-${num}.log" 2>&1 &
		echo $! > "$wm_pid"
		echo "Started xfwm4 on $display (pid $(cat "$wm_pid"))"
	fi
}

launch() {
	[[ $# -gt 0 ]] || { echo "usage: launch <cmd...>" >&2; exit 2; }
	alive "$xvfb_pid" || start
	DISPLAY="$display" "$@" >"$run_dir/app-${num}.log" 2>&1 &
	echo $! >> "$apps_pids"
	echo "Launched on $display (pid $!); log: $run_dir/app-${num}.log"
}

shot() {
	local out="${1:-}"
	[[ -n "$out" ]] || { echo "usage: shot <out.png>" >&2; exit 2; }
	alive "$xvfb_pid" || { echo "no Xvfb on $display - run 'start' first" >&2; exit 1; }
	import -display "$display" -window root "$out"
	echo "Wrote $out"
}

stop() {
	if [[ -f "$apps_pids" ]]; then
		while read -r p; do kill "$p" 2>/dev/null || true; done < "$apps_pids"
		rm -f "$apps_pids"
	fi
	for f in "$wm_pid" "$xvfb_pid"; do
		[[ -f "$f" ]] && { kill "$(cat "$f")" 2>/dev/null || true; rm -f "$f"; }
	done
	echo "Stopped headless session on $display"
}

case "${1:-}" in
	start)  shift; start "${1:-}" ;;
	launch) shift; launch "$@" ;;
	shot)   shift; shot "${1:-}" ;;
	status) alive "$xvfb_pid" && echo "Xvfb up on $display (pid $(cat "$xvfb_pid"))" || echo "no Xvfb on $display" ;;
	stop)   stop ;;
	*) echo "usage: gui-headless.bash {start [--wm]|launch <cmd...>|shot <out.png>|status|stop}" >&2; exit 2 ;;
esac


##	Script history:
##		- 20260701 JC: Created.
