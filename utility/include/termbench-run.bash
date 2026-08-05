#!/usr/bin/env bash

#  shellcheck disable=2086  ## 'Double quote to prevent globbing and word splitting.' (OK for integers.)
#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.'
#  shellcheck disable=2181  ## 'Check exit code directly, not indirectly with $?.'
#  shellcheck disable=2329  ## 'This function is never invoked.' cleanup() runs from the trap.
#  shellcheck disable=2012  ## 'Use find instead of ls.' The wayland socket names are known-safe.
#  shellcheck disable=1091  ## 'Not following.' bench-common.bash is beside this script.

##	- Purpose:
##		Repeatable rig for the README terminal shootout. Brings up a private headless
##		Wayland compositor on the real GPU, launches one terminal as its only client,
##		fits every terminal to the same grid, and runs termbench.py inside it.
##
##		The rig matters more than it looks. Measured 20260730: software GL halves
##		SilkTerm (45 vs 88 MB/s on ascii) and VirtualGL still costs ~14%, while
##		CPU-rendered terminals do not move at all. A table built from mixed rigs can
##		therefore rank the wrong terminal first. Every published row comes from one rig.
##	- Syntax:
##		termbench-run.bash --term KEY [options]
##		   --term KEY      terminal to measure (--list for the known keys)
##		   --reps N        runs per scene (default 6; --reps is the only safe way to
##		                   shorten a run - see the note below)
##		   --grid CxR      grid every terminal is fitted to (default 160x42)
##		   --label TEXT    row name for the README table (default: autodetected)
##		   --scene NAME    one width class only (ascii|latin|cjk|emoji|mixed)
##		   --no-save       measure without recording or touching README.md
##		   --keep          leave the compositor up afterwards
##		   --list          list the known terminal keys and exit
##
##		Shortening a run: use --reps, never --scale. Fewer repetitions of the same
##		payloads leaves the measured rate directly comparable and only widens the
##		confidence interval. Shrinking the payload does not: Hyper reads 32 MB/s at
##		--scale 0.05 but ~3 MB/s at full size, a 10x difference. Watch the CV% column;
##		a run that got stepped on by other desktop activity shows up there.


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Setup
##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

set -Eeuo pipefail

declare -r _here="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
declare -r _repo="$(cd "${_here}/../.." && pwd)"
declare -r _work="$(mktemp -d -t termbench-XXXXXX)"

source "${_here}/bench-common.bash"                                ## fEcho, fKillPids

declare -i _swayPid=0 _termPid=0
declare -i _keepRig=0

cleanup(){
	local -i rc=$?
	if ((_termPid)); then fKillPids ${_termPid}; fi
	if ((_swayPid)) && ((!_keepRig)); then kill ${_swayPid} 2>/dev/null || true; fi
	if ((!_keepRig)); then rm -rf "${_work}" 2>/dev/null || true; fi
	exit ${rc}
}
trap cleanup EXIT INT TERM


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Terminal recipes
##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

##	Every entry launches its terminal with SCENE as the shell/command, unstyled and
##	against a throwaway config so nothing personal reaches a published run. Keys marked
##	awkward need a hook the terminal does not offer directly - see showdown-README.md.
list_terms(){
	fEcho_Clean "  silkterm    this tree's release build, as shipped"
	fEcho_Clean "  silkplain   same binary, every optional effect off"
	fEcho_Clean "  alacritty   the VT core SilkTerm builds on, as its own terminal"
	fEcho_Clean "  kitty       needs terms/bin/kitty       (see showdown-README.md)"
	fEcho_Clean "  wezterm     needs the AppImage extracted (see showdown-README.md)"
	fEcho_Clean "  xfce4 gnome terminator                  (distro packages)"
	fEcho_Clean "  xterm       X11 only - runs via Xwayland, see showdown-README.md"
	fEcho_Clean "  hyper tabby awkward, see showdown-README.md"
}

##	Resolve a terminal binary: PATH first, then the kept artifact dir, so a re-run does
##	not need to re-download anything.
find_bin(){
	local name="$1" ; local candidate=""
	if candidate="$(command -v "${name}" 2>/dev/null)"; then echo "${candidate}"; return 0; fi
	for candidate in "${_repo}/cicd/artifacts/sizebench/terms/bin/${name}" \
	                 "${_repo}/cicd/artifacts/sizebench/terms/usr/bin/${name}"; do
		[[ -x "${candidate}" ]] && { echo "${candidate}"; return 0; }
	done
	return 1
}

##	Alacritty reads the user's own config unless pointed elsewhere, and defaults TERM to
##	an entry that need not be installed. Neither matters to the measurement, so both are
##	pinned to something inert.
write_alacritty_config(){
	cat > "${_work}/alacritty.toml" <<-'EOF'
		[env]
		TERM = "xterm-256color"
	EOF
}

##	SilkTerm with every optional effect off. Only the overrides are written; the loader
##	backfills the rest, so this cannot go stale as new settings are added.
write_plain_config(){
	cp "${_here}/termbench-plain.shcl" "${_work}/plain.shcl"
}


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	The rig
##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

##	A headless sway on the real card. Headless means no monitor and no interference with
##	whatever is on the actual desktop, while still handing the client a native Vulkan
##	context on the discrete GPU - which is the whole point over Xvfb software GL.
start_rig(){
	command -v sway >/dev/null 2>&1 || fDie "sway is not installed - see showdown-README.md"
	printf 'default_border none\ndefault_floating_border none\ngaps inner 0\ngaps outer 0\n' > "${_work}/sway.cfg"

	local runtimeDir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
	## Identify our socket by age, not by which names are new: a compositor that was
	## killed leaves its socket behind and the next one reuses the same name, so a
	## set-difference finds nothing. Nothing is deleted - a live session may own one.
	local -i startedAt=$(( $(date +%s) - 1 ))

	env -u DISPLAY WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
		sway -c "${_work}/sway.cfg" > "${_work}/sway.log" 2>&1 &
	_swayPid=$!

	## The ipc socket name is derived from our own pid, so this can never latch onto
	## somebody else's compositor.
	export SWAYSOCK="${runtimeDir}/sway-ipc.$(id -u).${_swayPid}.sock"
	local -i waited=0
	while ((waited < 100)); do
		[[ -S "${SWAYSOCK}" ]] && break
		kill -0 ${_swayPid} 2>/dev/null || fDie "sway exited during startup - see ${_work}/sway.log"
		sleep 0.2; waited+=1
	done
	[[ -S "${SWAYSOCK}" ]] || fDie "sway ipc socket never appeared"

	local candidate
	unset WAYLAND_DISPLAY
	waited=0
	while ((waited < 100)); do
		for candidate in "${runtimeDir}"/wayland-*; do
			[[ -S "${candidate}" ]] || continue
			local -i mtime=$(stat -c %Y "${candidate}" 2>/dev/null || echo 0)
			if ((mtime >= startedAt)); then export WAYLAND_DISPLAY="$(basename "${candidate}")"; fi
		done
		[[ -n "${WAYLAND_DISPLAY:-}" ]] && break
		sleep 0.2; waited+=1
	done
	[[ -n "${WAYLAND_DISPLAY:-}" ]] || fDie "no wayland socket appeared - see ${_work}/sway.log"

	unset DISPLAY
	export GDK_BACKEND=wayland
	fEcho "rig: sway pid ${_swayPid}, ${WAYLAND_DISPLAY}"
	## Never end a function on a bare 'cond && action': a false condition becomes the
	## function's return value and set -e kills the script. Bit this rig once already.
	local gpu="$(/usr/bin/grep -oiE 'DRM device[^,]*|renderer:.*' "${_work}/sway.log" 2>/dev/null | head -2 | tr '\n' ' ' || true)"
	if [[ -n "${gpu}" ]]; then fEcho_Clean "      ${gpu}"; fi
	return 0
}

##	The compositor tiles its only client to the whole output, so the grid is steered by
##	the output mode instead of by each terminal's own geometry flags - which is what
##	makes one fitter work for every terminal. The scene reports its own stty size while
##	it waits, so nothing here needs to know a terminal's cell metrics.
fit_grid(){
	local -i wantC=$1 wantR=$2
	local reportFile="$3"
	local -i w=2438 h=1680 pass=0 gotC=0 gotR=0

	for ((pass = 1; pass <= 7; pass++)); do
		swaymsg output HEADLESS-1 mode ${w}x${h} >/dev/null 2>&1 || true
		rm -f "${reportFile}"
		local -i waited=0
		while ((waited < 60)); do [[ -f "${reportFile}" ]] && break; sleep 0.25; waited+=1; done
		[[ -f "${reportFile}" ]] || fDie "the terminal never reported its grid - see ${_work}"
		read -r gotR gotC < "${reportFile}" || true
		((gotC)) || fDie "unreadable grid report"
		fEcho_Clean "      fit pass ${pass}: ${gotC}x${gotR} at output ${w}x${h}"
		if ((gotC == wantC && gotR == wantR)); then fEcho "grid ${gotC}x${gotR}"; return 0; fi
		if ((gotC != wantC)); then w=$(( w * wantC / gotC )); fi
		if ((gotR != wantR)); then h=$(( h * wantR / gotR )); fi
	done
	fDie "could not fit ${wantC}x${wantR} (stopped at ${gotC}x${gotR})"
}


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Arguments
##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

declare termKey="" label="" scene="" grid="160x42"
declare -i reps=6 noSave=0

while (($#)); do
	case "$1" in
		--term)    termKey="${2:-}"; shift 2 ;;
		--reps)    reps="${2:-6}";   shift 2 ;;
		--grid)    grid="${2:-}";    shift 2 ;;
		--label)   label="${2:-}";   shift 2 ;;
		--scene)   scene="${2:-}";   shift 2 ;;
		--no-save) noSave=1;         shift ;;
		--keep)    _keepRig=1;       shift ;;
		--list)    list_terms; exit 0 ;;
		-h|--help) sed -n '/- Purpose:/,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^##\t\?//'; exit 0 ;;
		*)         fDie "unknown option: $1" ;;
	esac
done

[[ -n "${termKey}" ]] || { fEcho "a terminal is required"; list_terms; exit 2; }
declare -i wantC="${grid%x*}" wantR="${grid#*x}"


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Run
##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

fSection "Terminal throughput: ${termKey}"

declare -r outFile="${_work}/out.txt"
declare -r sizeFile="${_work}/size"
declare -r goFile="${_work}/go"

declare benchArgs="--reps ${reps}"
if [[ -n "${scene}" ]]; then benchArgs+=" --scene ${scene}"; fi
if ((noSave)); then benchArgs+=" --no-save --no-readme"; fi

start_rig

## The scene script waits on the go file, reporting its grid meanwhile, so the fitter
## can settle the size before a single byte is measured.
export REPO_DIR="${_repo}" BENCH_ARGS="${benchArgs}" LABEL="${label}" \
       OUT_FILE="${outFile}" SIZE_FILE="${sizeFile}" GO_FILE="${goFile}"
declare -r sceneCmd="/bin/dash ${_here}/termbench-scene.sh"

case "${termKey}" in
	silkterm)
		"${_repo}/target/release/silkterm" --shell "${sceneCmd}" > "${_work}/term.log" 2>&1 & ;;
	silkplain)
		write_plain_config
		"${_repo}/target/release/silkterm" --config "${_work}/plain.shcl" --shell "${sceneCmd}" > "${_work}/term.log" 2>&1 & ;;
	alacritty)
		write_alacritty_config
		"$(find_bin alacritty || fDie "alacritty not found - see showdown-README.md")" \
			--config-file "${_work}/alacritty.toml" -e ${sceneCmd} > "${_work}/term.log" 2>&1 & ;;
	kitty)
		"$(find_bin kitty || fDie "kitty not found - see showdown-README.md")" ${sceneCmd} > "${_work}/term.log" 2>&1 & ;;
	wezterm)
		"$(find_bin wezterm || fDie "wezterm not found - see showdown-README.md")" \
			--config enable_wayland=true start --always-new-process -- ${sceneCmd} > "${_work}/term.log" 2>&1 & ;;
	xfce4)
		xfce4-terminal --disable-server -x ${sceneCmd} > "${_work}/term.log" 2>&1 & ;;
	gnome)
		## gnome-terminal never resizes with the compositor output, so it is the one
		## terminal that has to be told its geometry directly.
		gnome-terminal --wait --geometry=${wantC}x${wantR} -- ${sceneCmd} > "${_work}/term.log" 2>&1 & ;;
	terminator)
		terminator -e "${sceneCmd}" > "${_work}/term.log" 2>&1 & ;;
	xterm)
		xterm -e ${sceneCmd} > "${_work}/term.log" 2>&1 & ;;
	*)
		fDie "unknown terminal key: ${termKey} (--list)" ;;
esac
_termPid=$!
fEcho "launched pid ${_termPid}"

fit_grid ${wantC} ${wantR} "${sizeFile}"
touch "${goFile}"

fEcho "measuring (${reps} runs per scene)"
declare -i waited=0
while ((waited < 900)); do
	[[ -f "${outFile}.done" ]] && break
	if ! kill -0 ${_termPid} 2>/dev/null; then
		sleep 3
		[[ -f "${outFile}.done" ]] || fDie "the terminal exited before finishing - see ${_work}/term.log"
		break
	fi
	sleep 2; waited+=1
done
[[ -f "${outFile}.done" ]] || fDie "timed out waiting for the run to finish"

fEcho_Clean
cat "${outFile}"

## A run whose scenes never answered the device-attributes query timed a timeout, not
## throughput, and must not reach the table.
if /usr/bin/grep -q "sync NONE" "${outFile}" 2>/dev/null; then
	fEcho "WARNING: this terminal never answered the barrier - the figures are not comparable"
fi

if ((_keepRig)); then fEcho "rig left up: SWAYSOCK=${SWAYSOCK} WAYLAND_DISPLAY=${WAYLAND_DISPLAY} (work: ${_work})"; fi
exit 0


##	History:
##		- 20260730 JC: Created, from the scratch rig used for the first shootout table.
