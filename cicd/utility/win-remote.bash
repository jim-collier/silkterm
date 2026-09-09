#!/usr/bin/env bash

#  shellcheck disable=2001  ## 'See if you can use ${variable//search/replace} instead.' Complains about good uses of sed.
#  shellcheck disable=2016  ## 'Expressions don't expand in single quotes, use double quotes for that.' I know, and I often want an explicit '$'.
#  shellcheck disable=2034  ## 'variable appears unused.' Complains about valid use of variable indirection (e.g. later use of local -n var=$1)
#  shellcheck disable=2046  ## 'Quote to prevent word-splitting.' (OK for integers.)
#  shellcheck disable=2086  ## 'Double quote to prevent globbing and word splitting.' (OK for integers.)
#  shellcheck disable=2155  ## 'Declare and assign separately to avoid masking return values.' Cumbersome and unnecessary.
#  shellcheck disable=2181  ## 'Check exit code directly, not indirectly with $?.'
#  shellcheck disable=2029  ## 'Note that, unescaped, this expands on the client side.' That is the intent - the remote paths are built here.

##	Purpose:
##		Run builds, tests and other jobs on the Windows boxes over ssh, from here.
##		Several platform bugs only exist on Windows and cannot be reproduced on this
##		box at all, so the alternative is doing it by hand on the other machine.
##	Syntax:
##		win-remote.bash [--host <name>] [--as <user>] [--optional] hosts
##		win-remote.bash [--host <name>] [--optional] [--ref <ref>] sync
##		win-remote.bash [--host <name>] [--as <user>] [--optional] job <name> [args...]
##		win-remote.bash [--host <name>] [--as <user>] [--optional] run <file.ps1> [args...]
##		win-remote.bash [--host <name>] [--as <user>] [--optional] fetch <remote-rel-path> <local-dir>
##		win-remote.bash [--host <name>] [--as <user>] [--optional] pull <remote-abs-path> <local-dir>
##	Notes:
##		Hosts are read from $WINRIG_CONF (default ~/.config/silkterm/winrig.conf),
##		one per line as '<name> <addr>[,<addr>...]'. First address that answers wins,
##		which is how a laptop that moves between wired and wifi stays reachable. The
##		file lives outside the repo on purpose - machine names are not project data.
##		A host that is down is skipped with a warning. Exit is 1 only if every
##		selected host was unreachable, or if a job failed on a host that was up.
##		--optional drops the first of those, so nothing reachable is a skip rather
##		than a failure. Every cicd caller passes it: the boxes are somebody's desk
##		and a laptop, so one being off is the normal case, not a broken build. A job
##		that actually ran and failed still fails, with or without it.
##		Jobs run against a clone the remote keeps at origin/dev; it is reset, not
##		merged, so local edits there are discarded. Uncommitted work here does not
##		reach it - push first. --ref names a different branch, which is how a fix
##		gets tried on Windows before it is merged; the next plain sync puts the
##		clone back on dev.
##		--as picks the remote account. The default builds and tests, because the rust
##		toolchain is a per-user rustup install under it. The unprivileged test account
##		has no toolchain but a virgin profile, which is what to run a built binary as.
##	Exit: 0 ok, 1 job or connection failure, 2 usage / no config.
##	History: At bottom of script.

##	Copyright © 2026 Bubbles (ID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


set -Eeuo pipefail

meDir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
jobsDir="${meDir}/win-jobs"
conf="${WINRIG_CONF:-${XDG_CONFIG_HOME:-${HOME}/.config}/silkterm/winrig.conf}"
sshUser="${WINRIG_USER:-$(id -un)}"

##	The remote work tree. 'wintest' is a role account, not anyone's login.
winBase='C:\Users\wintest\0-0\data\prs\dev_clone\github.com\jim-collier\silkterm'
winRepo="${winBase}\\github"
winPriv="${winBase}\\private"
winJobs="${winPriv}\\winrig"
scpBase="C:/Users/wintest/0-0/data/prs/dev_clone/github.com/jim-collier/silkterm"
scpJobs="${scpBase}/private/winrig"

sshOpts=(-o BatchMode=yes -o ConnectTimeout=6 -o StrictHostKeyChecking=accept-new -o LogLevel=ERROR)

fWarn() { echo "win-remote: $*" >&2; }
fFail() { echo "win-remote: $1" >&2; exit "${2:-1}"; }

declare -a hostNames=() hostAddrs=()

fLoadConf() {
	##	The config lives outside the repo, so having none is the ordinary state on
	##	any box but the one it was written on - a cicd caller skips rather than dies.
	if [[ ! -r "$conf" ]]; then
		((optional)) && { fWarn "no host config at ${conf}, skipped"; exit 0; }
		fFail "no host config at ${conf}
  Create it with one line per box:  <name> <addr>[,<addr>...]" 2
	fi
	local name addrs
	while read -r name addrs _; do
		[[ -z "$name" || "${name:0:1}" == "#" ]] && continue
		hostNames+=("$name"); hostAddrs+=("${addrs:-$name}")
	done < "$conf"
	if ((! ${#hostNames[@]})); then
		((optional)) && { fWarn "no hosts listed in ${conf}, skipped"; exit 0; }
		fFail "no hosts listed in ${conf}" 2
	fi
}

##	First address that answers. Empty means the box is down or moved.
fLiveAddr() {
	local addr
	for addr in ${1//,/ }; do
		ssh "${sshOpts[@]}" "${sshUser}@${addr}" exit 0 2>/dev/null && { echo "$addr"; return 0; }
	done
	return 1
}

fSelected() {
	local i
	for i in "${!hostNames[@]}"; do
		[[ -n "$only" && "${hostNames[$i]}" != "$only" ]] && continue
		echo "$i"
	done
}

##	Paths the job scripts dot-source, so no job has to repeat them.
fPushEnv() {
	local addr="$1" tmp
	tmp="$(mktemp)"
	{
		echo "\$RepoDir    = '${winRepo}'"
		echo "\$PrivateDir = '${winPriv}'"
		echo "\$JobsDir    = '${winJobs}'"
	} > "$tmp"
	ssh "${sshOpts[@]}" "${sshUser}@${addr}" "if not exist \"${winJobs}\" mkdir \"${winJobs}\"" >/dev/null 2>&1 || true
	scp -q "${sshOpts[@]}" "$tmp" "${sshUser}@${addr}:${scpJobs}/_env.ps1"
	rm -f "$tmp"
}

fRunScript() {
	local addr="$1" script="$2"; shift 2
	local base; base="$(basename "$script")"
	fPushEnv "$addr"
	scp -q "${sshOpts[@]}" "$script" "${sshUser}@${addr}:${scpJobs}/${base}"
	local -a quoted=(); local a
	for a in "$@"; do quoted+=("\"${a}\""); done
	ssh "${sshOpts[@]}" "${sshUser}@${addr}" "pwsh -NoProfile -ExecutionPolicy Bypass -File \"${winJobs}\\${base}\" ${quoted[*]}"
}

##	Bring the remote clone to a branch on origin. Reset rather than pull: the clone
##	is a scratch checkout, and a half-merged tree there is worse than a discarded
##	edit.
fSync() {
	local addr="$1" tmp
	tmp="$(mktemp --suffix=.ps1)"
	{
		printf '$ref = "%s"\n' "${syncRef}"
		cat <<'PS'
$ErrorActionPreference = "Stop"
. "$PSScriptRoot\_env.ps1"
if (-not (Test-Path (Join-Path $RepoDir ".git"))) {
	New-Item -ItemType Directory -Force -Path $RepoDir | Out-Null
	git clone --branch dev https://github.com/jim-collier/silkterm.git $RepoDir 2>&1 | Out-Null
}
git -C $RepoDir fetch --prune origin 2>&1 | Out-Null
git -C $RepoDir reset --hard "origin/$ref" 2>&1 | Out-Null
git -C $RepoDir clean -fdx -e target 2>&1 | Out-Null
"at " + (git -C $RepoDir rev-parse --short HEAD) + " " + (git -C $RepoDir log -1 --format=%s)
PS
	} > "$tmp"
	fRunScript "$addr" "$tmp"
	rm -f "$tmp"
}

##	Walk the selected hosts, skipping whatever is down. A skipped host is a warning;
##	only a job that actually ran and failed, or nothing reachable at all, is an error.
fOverHosts() {
	local fn="$1"
	local -i up=0 bad=0
	local i addr
	for i in $(fSelected); do
		if ! addr="$(fLiveAddr "${hostAddrs[$i]}")"; then
			fWarn "${hostNames[$i]}: unreachable, skipped"
			continue
		fi
		up=$((up + 1))
		echo "== ${hostNames[$i]} (${addr})"
		if ! "$fn" "$addr"; then bad=$((bad + 1)); fWarn "${hostNames[$i]}: failed"; fi
	done
	if ((! up)); then
		((optional)) || fFail "no host reachable"
		fWarn "no host reachable, skipped"
		return 0
	fi
	((bad == 0))
}

runScript=""
declare -a runArgs=()
fDoRun() { fRunScript "$1" "$runScript" "${runArgs[@]}"; }

only=""; optional=0; syncRef="dev"
while (($#)); do case "$1" in
	--host)    only="${2:-}"; shift 2 ;;
	--as)      sshUser="${2:-}"; shift 2 ;;
	--ref)     syncRef="${2:-}"; shift 2 ;;
	--optional) optional=1; shift ;;
	-h|--help) grep -E '^##' "$0" | sed 's/^##\t\?//'; exit 0 ;;
	*) break ;;
esac; done

cmd="${1:-}"; shift || true
fLoadConf
[[ -n "$only" ]] && { printf '%s\n' "${hostNames[@]}" | grep -qx "$only" || fFail "unknown host: ${only}" 2; }

case "$cmd" in
	hosts)
		for i in $(fSelected); do
			if addr="$(fLiveAddr "${hostAddrs[$i]}")"
				then printf '%-8s up    %s\n' "${hostNames[$i]}" "${addr}"
				else printf '%-8s down  %s\n' "${hostNames[$i]}" "${hostAddrs[$i]}"
			fi
		done
		;;
	sync)
		fOverHosts fSync || exit 1
		;;
	job)
		name="${1:-}"; shift || true
		[[ -n "$name" ]] || fFail "job needs a name" 2
		runScript="${jobsDir}/${name}.ps1"
		[[ -r "$runScript" ]] || fFail "no such job: ${name} (looked in ${jobsDir})" 2
		runArgs=("$@")
		fOverHosts fDoRun || exit 1
		;;
	run)
		runScript="${1:-}"; shift || true
		[[ -r "${runScript:-}" ]] || fFail "no such script: ${runScript:-<none>}" 2
		runArgs=("$@")
		fOverHosts fDoRun || exit 1
		;;
	fetch)
		rel="${1:-}"; dest="${2:-}"
		[[ -n "$rel" && -n "$dest" ]] || fFail "fetch needs <remote-rel-path> <local-dir>" 2
		mkdir -p "$dest"
		fGet() { scp -q "${sshOpts[@]}" "${sshUser}@${1}:${scpBase}/${rel}" "${dest}/"; }
		fOverHosts fGet || exit 1
		;;
	pull)
		##	Anything by absolute path, for output a job wrote outside the clone.
		##	scp wants forward slashes even when the far side is Windows.
		abs="${1:-}"; dest="${2:-}"
		[[ -n "$abs" && -n "$dest" ]] || fFail "pull needs <remote-abs-path> <local-dir>" 2
		mkdir -p "$dest"
		fPull() { scp -qr "${sshOpts[@]}" "${sshUser}@${1}:${abs//\\//}" "${dest}/"; }
		fOverHosts fPull || exit 1
		;;
	*)
		echo "usage: win-remote.bash [--host <name>] [--as <user>] [--ref <ref>] [--optional] {hosts|sync|job <name> [args]|run <file.ps1> [args]|fetch <rel> <dir>|pull <abs> <dir>}" >&2
		exit 2
		;;
esac


##	Script history:
##		- 20260908: Created.
##		- 20260908: --optional, so an unreachable box is a skip.
##		- 20260908: pull, for output written outside the clone.
##		- 20260909: --ref, to try a branch on Windows before merging it.
