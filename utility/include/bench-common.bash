#!/usr/bin/env bash

#  shellcheck shell=bash
#  shellcheck disable=2034  ## _letterbox is used by the scripts that source this.
#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.'
#  shellcheck disable=2086  ## Integer pids need no quoting.

##	- Purpose:
##		Shared helpers for the two shootout rigs (termbench-run.bash, sizebench-run.bash).
##		Sourced, never run. The wrapper above them is Python and carries its own copy of
##		the same output style.
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

##	Where cargo put the build. CARGO_TARGET_DIR moves it, and a relative one is taken
##	from the repository, where cargo runs.
fTargetDir(){
	local -r repo="$1" dir="${CARGO_TARGET_DIR:-target}"
	if [[ "${dir}" == /* ]]; then printf '%s' "${dir}"; else printf '%s' "${repo}/${dir}"; fi
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

##	A terminal under test runs as it would on a new account: a home, the XDG folders
##	and a session bus that the rig makes and removes. The measuring account's own
##	settings then cannot reach a published figure, and a terminal that writes its
##	settings on launch writes them here. GNOME Terminal and xfce4-terminal keep theirs
##	behind the session bus, which is why the bus is part of it. XDG_RUNTIME_DIR is left
##	alone, since the compositor's socket is in it.
##	Usage: fPrivateAccount <dir>, then: env "${_privateEnv[@]}" dbus-run-session -- <terminal...>
declare -a _privateEnv=()
fPrivateAccount(){
	local -r home="$1"
	command -v dbus-run-session >/dev/null 2>&1 || fDie "dbus-run-session is not installed (package dbus-daemon)"
	mkdir -p "${home}/.config" "${home}/.local/share" "${home}/.local/state" "${home}/.cache"
	_privateEnv=(
		"HOME=${home}"
		"XDG_CONFIG_HOME=${home}/.config"
		"XDG_DATA_HOME=${home}/.local/share"
		"XDG_STATE_HOME=${home}/.local/state"
		"XDG_CACHE_HOME=${home}/.cache"
		"BENCH_REAL_HOME=${HOME}"
		"BENCH_REAL_XDG_DATA_HOME=${XDG_DATA_HOME:-}"
	)
}

##	What a SilkTerm settings file says about its performance profile, in either the
##	nested or the dotted spelling: "automatic=<value> profile=<value>".
fSilkProfile(){
	local -r file="$1"
	awk '
		function bare(v){ gsub(/["\047]/, "", v); return v }
		/^performance:/              { inside = 1; next }
		/^[^ \t#]/                   { inside = 0 }
		inside && $1 == "automatic:" { automatic = bare($2) }
		inside && $1 == "profile:"   { profile = bare($2) }
		$1 == "performance.automatic:" { automatic = bare($2) }
		$1 == "performance.profile:"   { profile = bare($2) }
		END { printf "automatic=%s profile=%s", (automatic == "" ? "?" : automatic), (profile == "" ? "?" : profile) }
	' "${file}"
}

##	The +candy row is only that if nothing turned its effects down.
fRequireCandyProfile(){
	local -r file="$1"
	local state=""
	state="$(fSilkProfile "${file}")"
	fEcho "SilkTerm profile in force: ${state}"
	if [[ "${state}" != "automatic=false profile=custom" ]]; then
		fDie "the +candy row ran with '${state}', so its effects may have been turned down"
	fi
}

##
##	History:
##		- 20260730: Factored out of the two rigs when they moved under utility/include/.
##		- 20260918: fPrivateAccount, fSilkProfile, fRequireCandyProfile.
##
