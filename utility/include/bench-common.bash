#!/usr/bin/env bash

#  shellcheck shell=bash
#  shellcheck disable=2034  ## _letterbox is used by the scripts that source this.
#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.'
#  shellcheck disable=2086  ## Integer pids need no quoting.

##	- Purpose:
##		Shared helpers for the two shootout rigs (termbench-run.bash, sizebench-run.bash)
##		and the update-showdown.bash wrapper. Sourced, never run.
##
##		Output matches the house style used by cicd.bash: fEcho prints a bracketed status
##		line, fEcho_Clean prints plain and collapses repeat blanks, so the blank-line
##		rhythm does the visual grouping.
##

##	Guard against being sourced twice by a wrapper that also sources a rig.
[[ -n "${_benchCommonLoaded:-}" ]] && return 0
declare -r _benchCommonLoaded=1

declare -r _letterbox="$(printf '%.0s-' {1..78})"

declare -i _wasLastEchoBlank=0
fEcho_Clean(){ if [[ -n "${1:-}" ]]; then echo -e "$*"; _wasLastEchoBlank=0; elif [[ $_wasLastEchoBlank -eq 0 ]] && echo; then _wasLastEchoBlank=1; fi; }
fEcho(){ if [[ -n "$*" ]]; then fEcho_Clean "[ $* ]"; else fEcho_Clean ""; fi; }
fSection(){ fEcho_Clean; fEcho_Clean "${_letterbox}"; fEcho "$*"; }
fDie(){ { fEcho_Clean; fEcho "FAILED: $*"; } >&2; exit 1; }

##	Kill only pids this script started, and only by pid. A pattern kill matches the
##	harness's own command line and any copy already open and in use; that has taken out a
##	session mid-run before now.
fKillPids(){
	local -i pid=0
	for pid in "$@"; do ((pid > 0)) && kill ${pid} 2>/dev/null || true; done
	sleep 1
	for pid in "$@"; do ((pid > 0)) && kill -9 ${pid} 2>/dev/null || true; done
	return 0
}

##	A launched pid plus everything under it. Diffing the system-wide process list instead
##	would sweep in whatever else the desktop started meanwhile, and a name match would find
##	copies that were already running.
fCollectTree(){
	local -i root="$1"
	local -a out=("${root}") queue=("${root}")
	local -i pid=0 kid=0
	while ((${#queue[@]} > 0)); do
		pid="${queue[0]}"; queue=("${queue[@]:1}")
		while read -r kid; do
			[[ -z "${kid}" ]] && continue
			out+=("${kid}"); queue+=("${kid}")
		done < <(pgrep -P ${pid} 2>/dev/null || true)
	done
	printf '%s\n' "${out[@]}"
}

##
##	History:
##		- 20260730: Factored out of the two rigs when they moved under utility/include/.
##
