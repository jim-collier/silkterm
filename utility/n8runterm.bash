#!/usr/bin/env bash

#  shellcheck disable=2001  ## 'See if you can use ${variable//search/replace} instead.' Complains about good uses of sed.
#  shellcheck disable=2016  ## 'Expressions don't expand in single quotes, use double quotes for that.' I know, and I often want an explicit '$'.
#  shellcheck disable=2046  ## 'Quote to prevent word-splitting.' (OK for integers.)
#  shellcheck disable=2086  ## 'Double quote to prevent globbing and word splitting.' (OK for integers.)
#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.' Cumbersome and unnecessary.
#  shellcheck disable=2181  ## 'Check exit code directly, not indirectly with $?.'

##	Purpose:
##		- Launch the terminal for interactive dogfooding, passing through any script
##		  arguments. By default runs the newest CI/CD dogfood build (fSilkTermDogfood).
##		- Keeps a pool of date-stamped copies in ~/.local/bin - the same dir and the
##		  same naming cicd.bash's rotating install uses, so the two share one pool.
##		- Build sources, checked in this order each run:
##			clone     this clone's target/release (found from the script's own path)
##			b23       the same dir on b23, over the network mount
##			dogfood   the fixed-name copy in the synced dogfood dir, which is how a
##			          build made on another box arrives
##			sbin      the fixed-name copy in /usr/local/sbin
##		  A source is copied in only when its build is newer than anything held. A
##		  copy is named for the build's own mtime, not for when it was copied. That
##		  alone isn't enough to recognise one build reaching us two ways - the copies
##		  don't agree on mtime - so a source that looks newer is compared byte for
##		  byte against what's held, and a match just takes the newer stamp.
##		- The network source gets a hard wait bound at every step, so an off host or
##		  a dropped link costs seconds rather than the mount's own timeout. A copy
##		  lands on a temp name and is renamed into place, so one we abandon can't
##		  leave a half-written build behind.
##		- Each run: prune idle copies over 7 days old, refresh from every source,
##		  then run the newest build held. Every step says what it did, and says it
##		  again in the run log beside the pool.
##		- If nothing is held and no source is reachable, falls back to the first
##		  installed terminal from a known list (fFallbackTerminal).
##		- Edit fMain() to launch a different terminal instead.
##	History: At bottom of script.

##	Copyright © 2026 Bubbles (ID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Configuration

## Where the runnable copies live. cicd.bash's rotating install writes here too,
## and both prune the same way, so one pool serves both.
declare -r  poolDir="${HOME}/.local/bin"

## Prefix, stamp format and binary name, matching cicd's dogfood convention.
declare -r  dogfoodPrefix="slktrmdf"
declare -r  stampFormat="%Y%m%d-%H%M%S"
declare -r  exeName="silkterm"

## Delete idle stamped copies older than this many days.
declare -ri maxAgeDays=7

## Bounds on the network source. A mount whose host is off answers nothing, and an
## unbounded stat there is what a launch reads as a hang.
declare -ri netStatTimeout=5
declare -ri netCopyTimeout=20

## b23's clone, over the network mount. Same repo, same target dir.
declare -r  b23ReleaseDir="/mnt/zfs/zf10/0-0/users/collierjr/data/prs/dev/github.com/jim-collier/silkterm/github/target/release"

## The synced dogfood dir: where cicd.bash installs the fixed-name binary, and so
## where a build made on another box arrives over Dropbox.
declare -r  syncedDogfoodDir="${HOME}/synced/0-0/common/exec/util/linux/bin"

## Per-run decision log beside the pool, so a console that closes can't take the
## copy and skip reasons with it.
declare -r  runLog="${poolDir}/n8runterm.log"
declare -ri runLogMaxLines=400

## Fallback terminals, in preference order, for when nothing is held and no source
## answers. SilkTerm's own options aren't passed to these - they wouldn't parse.
declare -ar fallbackTerminals=(
	terminator
	xfce4-terminal
	gnome-terminal
	konsole
	alacritty
	kitty
	xterm
)


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Functions

## Entry point: what this launcher runs. Edit this to launch a different terminal
## (e.g. `exec /usr/bin/xterm "${@}"`); by default it runs the newest dogfood build,
## and if that isn't found, falls back to a known installed terminal.
fMain(){
	fSilkTermDogfood  "${@}"  ||  fFallbackTerminal  "${@}"
}


## Refresh the pool from every source, then run the newest build in it. Returns
## non-zero when there's nothing to run, so fMain falls back.
fSilkTermDogfood(){

	mkdir -p "${poolDir}"
	fTrimLog
	fNote "=== run: ${BASH_SOURCE[0]}, user $(id -un), host $(uname -n) ==="

	fDeleteOldBuilds

	local -r cloneDir="$(fCloneReleaseDir)"
	[[ -n "${cloneDir}" ]] && fCopyIfNewer clone "${cloneDir}" 0
	fCopyIfNewer b23     "${b23ReleaseDir}"    1
	fCopyIfNewer dogfood "${syncedDogfoodDir}" 0
	fCopyIfNewer sbin    "/usr/local/sbin"     0

	local -r newest="$(fNewestHeld)"
	[[ -n "${newest}" ]] || { fWarn "no dogfood build held, and no source reachable"; return 1; }

	fLaunch "${newest}" "${@}"
}


## Where this clone's own release build would be. Derived from the script's path
## first (right whenever this is the repo copy), then a couple of known layouts for
## the deployed copy, which lives outside any clone. Empty when there's no clone.
fCloneReleaseDir(){
	local -r self="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
	local -ar candidates=(
		"${self}/../target/release"
		"${HOME}/data/prs/dev/github.com/jim-collier/silkterm/github/target/release"
		"/0-0/users/collierjr/data/prs/dev/github.com/jim-collier/silkterm/github/target/release"
	)
	local dir
	for dir in "${candidates[@]}"; do
		if [[ -d "${dir}" ]]; then
			(cd "${dir}" && pwd)
			return 0
		fi
	done
	echo ""
}


## Copy "<dir>/silkterm" into the pool as "<prefix>_<stamp>_<tag>" when its build is
## newer than anything we hold. $3 non-zero means reach it over the network, which
## bounds every step. No-op when the source is missing, unreachable, or not newer.
fCopyIfNewer(){

	local -r  label="${1}"
	local -r  srcDir="${2}"
	local -ri remote="${3}"
	local -r  src="${srcDir}/${exeName}"

	## A dir that isn't there is the normal case for most of these - stay quiet about
	## it. For the network one, absent usually means the host is off, so say so.
	if ((remote)); then
		local probe=0
		timeout -k 1 "${netStatTimeout}" test -e "${src}" 2>/dev/null || probe=$?
		## 124 is timeout's own "gave up"; anything else non-zero just means the
		## file isn't there, which is a different problem and a quieter one.
		if ((probe == 124)); then
			fWarn "${label}: host not answering within ${netStatTimeout}s (${src})"
			return 0
		elif ((probe)); then
			fNote "${label}: no build there"
			return 0
		fi
	else
		[[ -f "${src}" ]] || return 0
	fi

	local srcStamp=""
	if ((remote)); then
		srcStamp="$(timeout -k 1 "${netStatTimeout}" date -r "${src}" "+${stampFormat}" 2>/dev/null || true)"
	else
		srcStamp="$(date -r "${src}" "+${stampFormat}" 2>/dev/null || true)"
	fi
	[[ -n "${srcStamp}" ]] || { fWarn "${label}: couldn't read the build date of ${src}"; return 0; }

	## One tag for every source here: they all serve the same native build, so a tag
	## naming the source would say nothing about the binary. Naming the copy for the
	## build's own mtime is what makes the same build arriving two ways collapse to
	## one copy - and it means the currency test can span every source at once.
	local -r tag="$(fBuildTag)"
	local -r heldStamp="$(fNewestHeldStamp)"

	if [[ -n "${heldStamp}" ]] && ! [[ "${heldStamp}" < "${srcStamp}" ]]; then
		fNote "${label}: nothing newer (held ${heldStamp}, source ${srcStamp})"
		return 0
	fi

	local -r dst="${poolDir}/${dogfoodPrefix}_${srcStamp}_${tag}"
	if [[ -e "${dst}" ]]; then
		chmod +x "${dst}" 2>/dev/null || true   # a copy we can't run is a copy we don't have
		fNote "${label}: copy already present ($(basename "${dst}"))"
		return 0
	fi

	## Different copies of one build don't agree on mtime - cicd dates the pool copy
	## and the synced copy separately, and Dropbox restamps what it syncs - so a
	## build we already hold keeps looking new and keeps getting copied in again.
	## Settle it on the bytes, then take the source's stamp so the cheap test above
	## answers it next time without reading 11MB.
	local -r twin="$(fHeldMatching "${src}" "${remote}")"
	if [[ -n "${twin}" ]]; then
		if mv -f "${twin}" "${dst}"; then
			fNote "${label}: same build as $(basename "${twin}") - renamed to ${srcStamp}"
		else
			fWarn "${label}: same build as $(basename "${twin}"), but couldn't restamp it"
		fi
		return 0
	fi

	## Copy to a temp name and rename it into place. A copy we abandon mid-transfer
	## otherwise leaves a half-written binary that later reads as a good build and
	## gets launched. '.partial' matches neither the selection nor the prune name
	## spec, so a leftover is inert either way.
	local -r tmp="${dst}.partial"
	rm -f "${tmp}"

	local rc=0
	if ((remote)); then
		timeout -k 1 "${netCopyTimeout}" cp -p "${src}" "${tmp}" 2>/dev/null || rc=$?
	else
		cp -p "${src}" "${tmp}" 2>/dev/null || rc=$?
	fi
	if ((rc)); then
		rm -f "${tmp}"
		fWarn "${label}: gave up copying the build (exit ${rc})"
		return 0
	fi

	chmod +x "${tmp}"
	if mv -f "${tmp}" "${dst}"; then
		fNote "${label}: copied -> $(basename "${dst}")"
	else
		rm -f "${tmp}"
		fWarn "${label}: couldn't place the copied build"
	fi
	return 0
}


## Path of a held copy holding byte-for-byte the same build as $1, or empty. Size
## is the cheap discriminator - two builds almost never match on it - so the hash
## only runs when one does. $2 non-zero bounds the reads, since the source may be
## across the network.
fHeldMatching(){

	local -r  src="${1}"
	local -ri remote="${2}"

	local srcSize=""
	if ((remote)); then
		srcSize="$(timeout -k 1 "${netStatTimeout}" stat -c %s "${src}" 2>/dev/null || true)"
	else
		srcSize="$(stat -c %s "${src}" 2>/dev/null || true)"
	fi
	[[ -n "${srcSize}" ]] || { echo ""; return 0; }

	## Only bother hashing the source if something we hold is the same size.
	local cand
	local -a sameSize=()
	for cand in "${poolDir}/${dogfoodPrefix}_"*; do
		[[ -f "${cand}" ]] || continue
		[[ -n "$(fStampOf "$(basename "${cand}")")" ]] || continue
		[[ "$(stat -c %s "${cand}" 2>/dev/null || echo -1)" == "${srcSize}" ]] && sameSize+=("${cand}")
	done
	((${#sameSize[@]})) || { echo ""; return 0; }

	## Reading the source costs about what copying it would, and it's bounded the
	## same way, so a link that dies here is no worse than one that dies mid-copy.
	local srcHash=""
	if ((remote)); then
		srcHash="$(timeout -k 1 "${netCopyTimeout}" sha256sum "${src}" 2>/dev/null | cut -d' ' -f1 || true)"
	else
		srcHash="$(sha256sum "${src}" 2>/dev/null | cut -d' ' -f1 || true)"
	fi
	[[ -n "${srcHash}" ]] || { echo ""; return 0; }

	for cand in "${sameSize[@]}"; do
		if [[ "$(sha256sum "${cand}" 2>/dev/null | cut -d' ' -f1 || true)" == "${srcHash}" ]]; then
			echo "${cand}"
			return 0
		fi
	done
	echo ""
}


## Delete stamped copies older than $maxAgeDays, skipping any that are running.
## Only ever touches files matching this launcher's own name spec, never a
## neighbour that merely shares the dir (the fixed-name 'silkterm', say).
fDeleteOldBuilds(){

	local -r cutoff="$(date -d "-${maxAgeDays} days" "+${stampFormat}")"
	local -a running=()
	mapfile -t running < <(fRunningExePaths)

	## Always keep the newest, however old it is. Age alone emptied the pool after a
	## quiet week, and with no source answering that left nothing to launch.
	local -r keep="$(fNewestHeld)"

	local cand stamp path deleted=0 inUse
	for cand in "${poolDir}/${dogfoodPrefix}_"*; do
		[[ -f "${cand}" ]] || continue                     # no-match glob, and .partial below
		[[ "${cand}" == "${keep}" ]] && continue
		stamp="$(fStampOf "$(basename "${cand}")")"
		[[ -n "${stamp}" ]] || continue
		[[ "${stamp}" < "${cutoff}" ]] || continue

		inUse=0
		for path in "${running[@]}"; do
			[[ "${path}" == "${cand}" ]] && { inUse=1; break; }
		done
		((inUse)) && continue

		rm -f "${cand}" && deleted=$((deleted + 1))
	done
	((deleted)) && fNote "deleted ${deleted} build(s) older than ${maxAgeDays} days"

	## Leftovers from a copy that was interrupted (see fCopyIfNewer).
	rm -f "${poolDir}/${dogfoodPrefix}_"*.partial 2>/dev/null || true
	return 0
}


## Full paths of every running executable we can see. A copy that's running must
## not be deleted out from under its window.
fRunningExePaths(){
	local link
	for link in /proc/[0-9]*/exe; do
		readlink -f "${link}" 2>/dev/null || true
	done
}


## The stamp embedded in a pool copy's name, or empty if it isn't one of ours.
## Matches the tagged name and the pre-2026-08 untagged one alike.
fStampOf(){
	local -r name="${1}"
	if [[ "${name}" =~ ^"${dogfoodPrefix}"_([0-9]{8}-[0-9]{6})(_[a-z0-9]+)?$ ]]; then
		echo "${BASH_REMATCH[1]}"
	else
		echo ""
	fi
}


## The newest copy held, by the stamp in its name. Empty when the pool is empty.
fNewestHeld(){
	local cand stamp best="" bestStamp=""
	for cand in "${poolDir}/${dogfoodPrefix}_"*; do
		[[ -f "${cand}" && -x "${cand}" ]] || continue
		stamp="$(fStampOf "$(basename "${cand}")")"
		[[ -n "${stamp}" ]] || continue
		if [[ -z "${bestStamp}" || "${bestStamp}" < "${stamp}" ]]; then
			bestStamp="${stamp}"
			best="${cand}"
		fi
	done
	echo "${best}"
}


## Just the stamp of the newest copy held, for the currency test.
fNewestHeldStamp(){
	local -r newest="$(fNewestHeld)"
	if [[ -n "${newest}" ]]; then
		fStampOf "$(basename "${newest}")"
	else
		echo ""
	fi
}


## What this box's own build IS, in the shared tag convention:
## '<toolchain: gnu|msvc><built on: l|m|b|w><target: l|m|b|w><arch: i|a>'. Every
## source here serves a native Linux build, so only the arch varies.
fBuildTag(){
	case "$(uname -m)" in
		x86_64)         echo "gnulli" ;;
		aarch64|arm64)  echo "gnulla" ;;
		*)              echo "gnullx" ;;
	esac
}


## Run the build, replacing this process. The title is tagged with the build so a
## dogfood window is identifiable; it precedes "$@" so a caller can still override.
fLaunch(){
	local -r  path="${1}"; shift
	local -r  name="$(basename "${path}")"
	local -r  suffix="${name#"${dogfoodPrefix}"_}"
	local -r  buildStamp="${suffix%%_*}"
	local     buildTag="${suffix#*_}"
	[[ "${buildTag}" == "${suffix}" ]] && buildTag=""   # untagged (pre-2026-08) copy
	local -r  buildLabel="${buildTag:+${buildTag} }${buildStamp}"

	## Picking a wallpaper here is disabled: the terminal rotates its own now, and a
	## wallpaper named on the command line pins it for the session - which would hide
	## exactly what we want to see.

	fNote "running ${name}"
	exec "${path}" "--title=SilkTerm [dogfood ${buildLabel}]" "${@}"
}


## Fallback when no dogfood build exists: run the first installed terminal from the
## known list, passing the args through.
fFallbackTerminal(){
	local term
	for term in "${fallbackTerminals[@]}"; do
		if command -v "${term}" >/dev/null 2>&1; then
			fWarn "no dogfood build; launching '${term}' instead"
			exec "${term}" "${@}"
		fi
	done
	fWarn "no dogfood build, and no fallback terminal installed (${fallbackTerminals[*]})"
	return 1
}


## Status lines go to the console and to the run log, so a console that closes
## can't take the reasons behind a launch with it.
fNote(){
	echo "${1}" >&2
	echo "$(date '+%Y-%m-%d %H:%M:%S')  ${1}" >> "${runLog}" 2>/dev/null || true
	return 0
}

fWarn(){
	fNote "WARNING: ${1}"
}

## Keep the log from growing without bound.
fTrimLog(){
	[[ -f "${runLog}" ]] || return 0
	local -i lines=0
	lines="$(wc -l < "${runLog}")"
	((lines > runLogMaxLines)) || return 0
	tail -n "${runLogMaxLines}" "${runLog}" > "${runLog}.tmp"  &&  mv -f "${runLog}.tmp" "${runLog}"
	return 0
}


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Script entry point

## Bash environment settings
 set -u  #..................: Require variable declaration.
 set -e  #..................: Exit on errors.
 set -E  #..................: Propagate ERR trap into functions and subshells.
 set   -o pipefail  #.......: Fail a pipe if any stage fails.
 shopt -s inherit_errexit  #: Propagate 'set -e' into command substitutions. (Bash >=4.4.)

## This is a launcher, not a library.
[[ "${BASH_SOURCE[0]}" == "${0}" ]] || { echo -e "\nError in $(basename "${BASH_SOURCE[0]}"): Not meant to be 'sourced'.\n" >&2; return 1; }

## Kick everything off.
fMain  "${@}"


##	History:
##		- 2026-08-24: Tell builds apart by their bytes, not their mtime - copies of
##		              one build disagreed on it, so the same binary kept getting
##		              copied in again. A match takes the newer stamp. Never prune
##		              the newest copy, whatever its age.
##		- 2026-08-23: Same source model the Windows launcher uses: check this clone's
##		              release build, b23 over the mount with a bounded wait, and the
##		              synced dogfood dir; copy in whatever is newer than what is held;
##		              prune idle copies; log every step. Was: run the newest copy
##		              found, in place.
##		- 2026-08-02: Stop picking a wallpaper; the terminal rotates its own.
##		- 2026-08-01: Show the build tag in the title, now that copies are named
##		              "<prefix>_<stamp>_<tag>". Untagged copies still work.
##		- 2026-07-03: Created.
