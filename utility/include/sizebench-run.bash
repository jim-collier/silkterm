#!/usr/bin/env bash

#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.'
#  shellcheck disable=2329  ## 'This function is never invoked.' fCleanup() runs from the trap.
#  shellcheck disable=1091  ## 'Not following.' bench-common.bash is beside this script.

##	- Purpose:
##		Measure one terminal's install size and resident memory for the README shootout
##		table. Sizes it to the same grid as every other row on a private display, lets it
##		settle, then hands the whole process tree to sizebench-classify.py.
##
##		Reproduce a published row before trusting a new one. This rig reproduced
##		SilkTerm's memory within 1.3% and its excluded-driver figure within 0.4% of the
##		numbers already in the table; anything measured a different way is not comparable
##		with them. Window size is the trap - the same binary reads 38 MiB heavier at its
##		default geometry than at the table's 100x30 grid.
##	- Syntax:
##		sizebench-run.bash --term KEY [options]
##		   --term KEY      terminal to measure (--list for the known keys)
##		   --grid CxR      grid every terminal is fitted to (default 100x30)
##		   --settle N      seconds to let it finish starting (default 22)
##		   --verbose       itemize what was billed to the terminal and to the driver
##		   --keep          leave the working directory behind
##

set -Eeuo pipefail
shopt -s inherit_errexit

declare -r scriptDir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
declare -r repoDir="$(cd -- "${scriptDir}/../.." && pwd -P)"
declare -r termsDir="${repoDir}/cicd/artifacts/sizebench/terms"

declare -r display=":98"
declare -r screen="1920x1080x24"
declare -i settleSecs=22

declare _work=""
declare -i _xvfbPid=0
declare -ai _termPids=()

source "${scriptDir}/bench-common.bash"                            ## fEcho, fKillPids, fCollectTree

#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##  Teardown
#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

fCleanup() {
	fKillPids "${_termPids[@]:-0}"
	((_xvfbPid > 0)) && kill "${_xvfbPid}" 2>/dev/null || true
	if [[ -n "${_work}" && -z "${optKeep:-}" && "${_work}" == /tmp/* ]]; then
		rm -rf "${_work}" || true
	fi
	return 0
}
trap fCleanup EXIT

#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##  The terminals
#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

## Each entry: key|binary|how it is told its grid. The keep-alive shell matters - a
## terminal whose child exits takes the window with it before anything can be measured.
fTermBinary() {
	local -r key="$1"
	local path=""
	case "${key}" in
		silkterm|silkplain) path="${repoDir}/target/release/silkterm" ;;
		alacritty)          path="$(command -v alacritty || true)"
		                    [[ -z "${path}" ]] && path="${termsDir}/usr/bin/alacritty" ;;
		xterm)              path="$(command -v xterm || true)" ;;
		kitty)              path="$(command -v kitty || true)" ;;
		xfce4)              path="$(command -v xfce4-terminal || true)" ;;
		terminator)         path="$(command -v terminator || true)" ;;
		*)                  fDie "unknown terminal key '${key}' (try --list)" ;;
	esac
	[[ -x "${path}" ]] || fDie "no binary for '${key}' (looked at '${path:-nothing}')"
	printf '%s' "${path}"
}

fLaunch() {
	local -r key="$1" bin="$2" cols="$3" rows="$4"
	local -r keepAlive="/bin/dash -c 'exec sleep 1000000'"

	export DISPLAY="${display}"
	export XDG_CONFIG_HOME="${_work}/xdg"
	mkdir -p "${XDG_CONFIG_HOME}"

	case "${key}" in
		silkplain)
			mkdir -p "${XDG_CONFIG_HOME}/silkterm"
			cp "${scriptDir}/termbench-plain.toml" \
			   "${XDG_CONFIG_HOME}/silkterm/config.toml"
			"${bin}" --columns "${cols}" --rows "${rows}" --shell "${keepAlive}" \
				>"${_work}/term.log" 2>&1 &
			;;
		silkterm)
			"${bin}" --columns "${cols}" --rows "${rows}" --shell "${keepAlive}" \
				>"${_work}/term.log" 2>&1 &
			;;
		alacritty)
			"${bin}" -o "window.dimensions.columns=${cols}" \
			         -o "window.dimensions.lines=${rows}" \
			         -e /bin/dash -c 'exec sleep 1000000' \
				>"${_work}/term.log" 2>&1 &
			;;
		xterm)
			## X toolkit option, one dash - '--geometry' is not accepted.
			"${bin}" -geometry "${cols}x${rows}" -e /bin/dash -c 'exec sleep 1000000' \
				>"${_work}/term.log" 2>&1 &
			;;
		kitty)
			"${bin}" -o "initial_window_width=${cols}c" -o "initial_window_height=${rows}c" \
				/bin/dash -c 'exec sleep 1000000' >"${_work}/term.log" 2>&1 &
			;;
		xfce4|terminator)
			"${bin}" --geometry "${cols}x${rows}" -e "/bin/dash -c 'exec sleep 1000000'" \
				>"${_work}/term.log" 2>&1 &
			;;
	esac
	printf '%s' "$!"
}

#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##  Main
#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

fUsage() {
	awk '/- Purpose:/{f=1} f&&!/^##/{exit} f' "${BASH_SOURCE[0]}" | sed 's/^##[[:space:]]\{0,2\}//'
}

fMain() {
	local key="" grid="100x30"
	optKeep=""; optVerbose=""

	while (($# > 0)); do
		case "$1" in
			--term)    key="${2:-}"; shift 2 ;;
			--grid)    grid="${2:-}"; shift 2 ;;
			--settle)  settleSecs="${2:-22}"; shift 2 ;;
			--verbose) optVerbose=1; shift ;;
			--keep)    optKeep=1; shift ;;
			--list)    fEcho_Clean "silkterm silkplain alacritty xterm kitty xfce4 terminator"; return 0 ;;
			-h|--help) fUsage; return 0 ;;
			*)         fDie "unknown option '$1'" ;;
		esac
	done
	[[ -n "${key}" ]] || { fUsage; fDie "--term is required"; }

	local -i cols="${grid%x*}" rows="${grid#*x}"
	local -r bin="$(fTermBinary "${key}")"

	_work="$(mktemp -d /tmp/sizebench.XXXXXX)"

	fSection "Rig"
	fEcho "terminal ${key} -> ${bin}"
	if ! DISPLAY="${display}" xdpyinfo >/dev/null 2>&1; then
		Xvfb "${display}" -screen 0 "${screen}" -nolisten tcp >"${_work}/xvfb.log" 2>&1 &
		_xvfbPid=$!
		sleep 2
		DISPLAY="${display}" xdpyinfo >/dev/null 2>&1 || fDie "could not start Xvfb on ${display}"
		fEcho "started Xvfb on ${display} (pid ${_xvfbPid})"
	else
		fEcho "reusing the display already on ${display}"
	fi

	fSection "Launch"
	local -i root=0
	root="$(fLaunch "${key}" "${bin}" "${cols}" "${rows}")"
	fEcho "pid ${root}, settling ${settleSecs}s at ${cols}x${rows}"
	sleep "${settleSecs}"
	kill -0 "${root}" 2>/dev/null || { tail -5 "${_work}/term.log" >&2; fDie "terminal exited before it could be measured"; }

	mapfile -t _termPids < <(fCollectTree "${root}")
	fEcho "process tree: ${_termPids[*]}"

	fSection "Measurement"
	local -a extra=()
	[[ -n "${optVerbose}" ]] && extra+=(--verbose)
	python3 "${scriptDir}/sizebench-classify.py" "${_termPids[@]}" --exe "${bin}" --summary ${extra[@]+"${extra[@]}"}

	fEcho_Clean ""
	return 0
}

#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##  Script entry point
#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
	fMain "$@"
fi

##
##  History:
##  - 20260730: Written, after the previous pass's scripts were lost with their scratch dir.
##
