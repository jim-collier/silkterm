#!/usr/bin/env bash

#  shellcheck disable=1091  ## 'Not following.' include/bench-common.bash ships beside this.
#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.'

##	- Purpose:
##		Refresh the README "Terminal showdown" table. One entry point over the two rigs
##		that feed it, because they are easy to run inconsistently: they measure different
##		things, at different grid sizes, on different displays.
##
##		  speed  include/termbench-run.bash   160x42, headless sway on the real GPU
##		  size   include/sizebench-run.bash   100x30, private Xvfb
##
##		The grids differ deliberately and must not be unified: the speed figure wants a
##		realistic working grid, while memory scales with the surface, so the size rows are
##		taken small and identical. The same SilkTerm binary reads 38 MiB heavier at its
##		default geometry than at 100x30.
##	- Syntax:
##		update-showdown.bash --term KEY [--term KEY ...] [options]
##		update-showdown.bash --all
##		   --term KEY      terminal to measure, repeatable (--list for the keys)
##		   --all           every terminal both rigs can drive here
##		   --speed-only    throughput only, leave the size columns alone
##		   --size-only     size and memory only
##		   --reps N        repetitions per speed scene (default 6)
##		   --no-readme     measure and print, write nothing
##		   --list          list the keys and exit
##
##		Nothing is published unless it is comparable. Re-measure a terminal already in the
##		table before adding a new one: the rig reproduced SilkTerm's speed within 0.6% and
##		its memory within 1.3%, and a figure taken any other way does not belong beside
##		the existing rows.
##

set -Eeuo pipefail

declare -r _here="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
declare -r _repo="$(cd "${_here}/.." && pwd)"

source "${_here}/include/bench-common.bash"                        ## fEcho, fDie

##	key : README row name : which rigs can drive it here. A terminal the size rig has no
##	recipe for still gets its speed row; the size columns are simply left as they were.
declare -ra _terms=(
	"silkterm:SilkTerm +candy:both"
	"silkplain:SilkTerm plain:both"
	"alacritty:Alacritty:both"
	"kitty:kitty:both"
	"xfce4:XFCE4 Terminal:both"
	"terminator:Terminator:both"
	"xterm:XTerm:size"
	"gnome:GNOME Terminal:speed"
	"wezterm:WezTerm:speed"
)

fRowName(){ local e; for e in "${_terms[@]}"; do [[ "${e%%:*}" == "$1" ]] && { e="${e#*:}"; echo "${e%:*}"; return 0; }; done; return 1; }
fRigs(){    local e; for e in "${_terms[@]}"; do [[ "${e%%:*}" == "$1" ]] && { echo "${e##*:}"; return 0; }; done; return 1; }

fList(){
	local e
	fEcho_Clean "  key          README row                rigs"
	for e in "${_terms[@]}"; do
		printf '  %-12s %-25s %s\n' "${e%%:*}" "$(fRowName "${e%%:*}")" "${e##*:}"
	done
}

fMain(){
	local -a keys=()
	local -i doSpeed=1 doSize=1 writeReadme=1 reps=6

	while (($# > 0)); do
		case "$1" in
			--term)       keys+=("${2:?--term needs a key}"); shift 2 ;;
			--all)        keys=(); local e; for e in "${_terms[@]}"; do keys+=("${e%%:*}"); done; shift ;;
			--speed-only) doSize=0; shift ;;
			--size-only)  doSpeed=0; shift ;;
			--reps)       reps="${2:?--reps needs a number}"; shift 2 ;;
			--no-readme)  writeReadme=0; shift ;;
			--list)       fList; return 0 ;;
			-h|--help)    awk '/- Purpose:/{f=1} f&&!/^##/{exit} f' "${BASH_SOURCE[0]}" | sed 's/^##\t\?//'; return 0 ;;
			*)            fDie "unknown option '$1'" ;;
		esac
	done
	((${#keys[@]})) || { fList; fDie "give at least one --term, or --all"; }

	local key row rigs out line
	local -a summary=()

	for key in "${keys[@]}"; do
		row="$(fRowName "${key}")" || fDie "unknown key '${key}' (try --list)"
		rigs="$(fRigs "${key}")"

		if ((doSpeed)) && [[ "${rigs}" == both || "${rigs}" == speed ]]; then
			fSection "Speed: ${row}"
			local -a speedArgs=(--term "${key}" --reps "${reps}" --label "${row}")
			((writeReadme)) || speedArgs+=(--no-save)
			"${_here}/include/termbench-run.bash" "${speedArgs[@]}" || fEcho "WARNING: speed run failed for ${key}"
		fi

		if ((doSize)) && [[ "${rigs}" == both || "${rigs}" == size ]]; then
			fSection "Size and memory: ${row}"
			## Tee so the run stays visible while the RESULT line is captured; the rig's
			## own output is the record of what was measured.
			out="$("${_here}/include/sizebench-run.bash" --term "${key}" 2>&1 | tee /dev/stderr)" || true
			line="$(printf '%s\n' "${out}" | /usr/bin/grep -m1 '^RESULT ' || true)"
			if [[ -z "${line}" ]]; then
				fEcho "WARNING: no result from the size rig for ${key}"
			else
				local fd="" mem=""
				fd="$(sed -n 's/.*filedeps=\([0-9.]*\).*/\1/p' <<<"${line}")"
				mem="$(sed -n 's/.*mem=\([0-9.]*\).*/\1/p' <<<"${line}")"
				summary+=("${row}|${fd}|${mem}")
				if ((writeReadme)); then
					python3 "${_here}/include/showdown-readme.py" --readme "${_repo}/README.md" \
						--terminal "${row}" --file-deps "${fd}" --mem "${mem}" \
						|| fEcho "WARNING: could not write the ${row} row"
				fi
			fi
		fi
	done

	if ((${#summary[@]})); then
		fSection "Size and memory measured"
		local entry
		printf '  %-25s %10s %10s\n' "Terminal" "File+deps" "Mem"
		for entry in "${summary[@]}"; do
			printf '  %-25s %10s %10s\n' "${entry%%|*}" "$(cut -d'|' -f2 <<<"${entry}")" "${entry##*|}"
		done
	fi

	fEcho_Clean
	if ((writeReadme)); then
		fEcho "README updated - check the diff before committing"
	else
		fEcho "nothing written (--no-readme)"
	fi
	fEcho_Clean
	return 0
}

fMain "$@"

##
##	History:
##		- 20260730: Written, to drive both shootout rigs from one place.
##
