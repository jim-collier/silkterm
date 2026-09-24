#!/usr/bin/env bash

##	- Purpose:
##		Hygiene the installers and the headless rig have to keep: no secret on a
##		command line, no plain-http redirect, no adopting a directory somebody
##		else made in a shared temp folder, no token file left behind, and a menu
##		launcher that still works from a path holding a space.
##		The last two run install.bash for real, against a stand-in release served
##		by a curl on PATH, in a scratch home and temp folder.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier [ID: 2უNაɘ«҂թȹɤξπ๙¿ձϖ]
##	SPDX-License-Identifier: GPL-2.0-or-later

set -euo pipefail
meDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${meDir}/../../.." && pwd)"

failures=0
fCheck(){ local -r what="${1}"; shift; if "${@}"; then echo "  ok   ${what}"; else echo "  FAIL ${what}"; failures=$((failures + 1)); fi; }

fAbsent(){ ! grep -Eq -e "${1}" -- "${2}" </dev/null; }
fPresent(){ grep -Eq -e "${1}" -- "${2}" </dev/null; }

fCheck "the token is never a curl argument" \
	fAbsent '(-H|--header)[= ]"?Authorization' "${root}/install.bash"
fCheck "https is pinned across redirects (curl)" \
	fPresent '\-\-proto-redir' "${root}/install.bash"
fCheck "https is pinned (wget)" \
	fPresent '\-\-https-only' "${root}/install.bash"
fCheck "the ps1 does not adopt an existing temp directory" \
	fAbsent 'New-Item -ItemType Directory -Force -Path \$tmpDir' "${root}/install.ps1"
## The line above only knows one spelling. This runs install.ps1's own step.
if command -v pwsh >/dev/null 2>&1; then
	fCheck "the ps1's temp folder step refuses a folder already there" \
		pwsh -NoProfile -File "${meDir}/tempdir.ps1" -Installer "${root}/install.ps1"
else
	echo "  skip the ps1's temp folder step (no pwsh)"
fi

## Everything below installs for real. One scratch tree, thrown away at the end.
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

## The stand-in release: a program that records that it ran, plus the checksums
## file the installer verifies it against.
relDir="${work}/release"
mkdir -p "${relDir}"
printf '#!/bin/sh\nprintf ran > "${HOME}/ran.txt"\n' > "${relDir}/silkterm-9.9.9-linux-x86_64"
( cd "${relDir}" && sha256sum silkterm-9.9.9-linux-x86_64 > silkterm-9.9.9-sha256sums.txt )

## A curl that serves it. Takes the URL and an -o, ignores the rest, and notes
## each --config so the token check can see the file was still passed that way.
stubDir="${work}/stub"
mkdir -p "${stubDir}"
cat > "${stubDir}/curl" <<'STUB'
#!/usr/bin/env bash
out=""; url=""
while [ "$#" -gt 0 ]; do
	case "$1" in
		-o)        out="$2"; shift 2 ;;
		--config)  echo "config $2" >>"${STUB_LOG}"; shift 2 ;;
		https://*) url="$1"; shift ;;
		*)         shift ;;
	esac
done
fServe() {
	case "${url}" in
		*/releases/latest|*/releases\?*) printf '{"tag_name":"v9.9.9"}\n' ;;
		*) cat "${STUB_DIR}/${url##*/}" ;;
	esac
}
if [ -n "${out}" ]; then fServe >"${out}"; else fServe; fi
STUB
chmod +x "${stubDir}/curl"

## Run the installer in a home and temp folder of its own. $1 is the home.
fInstall() {
	local home="$1"; shift
	mkdir -p "${home}" "${home}/.tmp"
	env -i PATH="${stubDir}:/usr/bin:/bin" HOME="${home}" TMPDIR="${home}/.tmp" \
		STUB_DIR="${relDir}" STUB_LOG="${home}/.tmp/calls.log" "$@" \
		bash "${root}/install.bash" --yes >/dev/null 2>&1
}

## The token used to be written into a fresh 0700 folder per API call, and the
## cleanup found nothing to remove because the function that made it ran in a
## command substitution.
tokenHome="${work}/tokenhome"
fInstall "${tokenHome}" GITHUB_TOKEN="sekrit-token-42"
fCheck "the token file does not outlast the run" \
	test -z "$(find "${tokenHome}/.tmp" -type f -not -name calls.log 2>/dev/null)"
fCheck "and the token was still passed in a file, not on a command line" \
	fPresent '^config ' "${tokenHome}/.tmp/calls.log"

## install.bash's own escaping rule, lifted out of the file so the test cannot
## drift from it, against the shared case list.
bad=0
lifted="$(sed -n '/^function fDesktopExec()/,/^}/p' "${root}/install.bash")"
if [ -z "${lifted}" ]; then
	echo "    install.bash has no fDesktopExec"
	bad=1
else
	eval "${lifted}"
	while IFS=$'\t' read -r path want; do
		case "${path}" in '' | '#'*) continue ;; esac
		got="$(fDesktopExec "${path}")"
		if [ "${got}" != "${want}" ]; then
			echo "    ${path}: wanted ${want}, got ${got}"
			bad=$((bad + 1))
		fi
	done < "${meDir}/desktop-exec-cases.txt"
fi
fCheck "install.bash escapes Exec the way both rule sets read it" test "${bad}" -eq 0

## A menu launcher from a home holding a space. The desktop entry format splits
## Exec at spaces, so an unquoted path gives an entry the desktop cannot load.
spacedHome="${work}/home dir"
fInstall "${spacedHome}"
entry="${spacedHome}/.local/share/applications/silkterm.desktop"
if command -v desktop-file-validate >/dev/null 2>&1; then
	fCheck "the launcher written from a spaced home is a valid entry" \
		desktop-file-validate "${entry}"
else
	echo "  skip desktop-file-validate (not installed)"
fi
if command -v gio >/dev/null 2>&1; then
	env -u DISPLAY HOME="${spacedHome}" gio launch "${entry}" >/dev/null 2>&1 || true
	## gio returns before the program has written anything.
	for _ in 1 2 3 4 5 6 7 8 9 10; do [ -e "${spacedHome}/ran.txt" ] && break; sleep 0.2; done
	fCheck "and it starts the installed program" test -e "${spacedHome}/ran.txt"
else
	echo "  skip gio launch (not installed)"
fi

## install.ps1 writes the same entry, and its Exec goes through the same rule.
## Its own block is lifted out of the file, so the two cannot drift apart.
if command -v pwsh >/dev/null 2>&1; then
	rc=0
	pwsh -NoProfile -File "${meDir}/desktop-entry.ps1" || rc=$?
	fCheck "install.ps1 writes a launcher that loads from a spaced path" test "${rc}" -eq 0
else
	echo "  skip install.ps1 launcher (no pwsh)"
fi

## The headless rig's run directory: predictable name, so it must refuse anything
## it does not own. Driven for real, in a sandbox of its own.
headless="${root}/cicd/utility/gui-headless.bash"
sandboxUser="silktest-$$-$RANDOM"
runDir="/tmp/cicd-gui-headless-${sandboxUser}"
elsewhere="$(mktemp -d)"
ln -s "${elsewhere}" "${runDir}"
rc=0
USER="${sandboxUser}" "${headless}" status >/dev/null 2>&1 || rc=$?
fCheck "a run directory that is a link is refused" test "${rc}" -ne 0
rm -f "${runDir}"; rm -rf "${elsewhere}"

## And an ordinary one of our own is fine, with the mode it should have.
mkdir -p "${runDir}"
rc=0
USER="${sandboxUser}" "${headless}" status >/dev/null 2>&1 || rc=$?
fCheck "one of our own is used" test "${rc}" -eq 0
fCheck "and is not readable by anyone else" test "$(stat -c %a "${runDir}")" = "700"
rm -rf "${runDir}"

## The rig's display: a number it did not start, a pid file whose pid is now
## something else, and a second run's stop. Each on a free number, with the
## sandbox's own run directory.
fFreeDisplay(){ local n; for n in $(seq "${1}" 299); do [[ -e "/tmp/.X${n}-lock" || -e "/tmp/.X11-unix/X${n}" ]] || { echo "${n}"; return 0; }; done; return 1; }
fGone(){ local _; for _ in {1..50}; do [[ -d "/proc/${1}" ]] || return 0; sleep 0.1; done; return 1; }
if command -v Xvfb >/dev/null && command -v xdpyinfo >/dev/null; then
	mkdir -p "${runDir}"; chmod 700 "${runDir}"
	fRig(){ USER="${sandboxUser}" CICD_HEADLESS_DISPLAY=":${1}" CICD_HEADLESS_SIZE=320x200x24 "${headless}" "${@:2}"; }

	## Taken: someone else's server already holds the number.
	n="$(fFreeDisplay 250)"
	Xvfb ":${n}" -screen 0 640x480x24 -nolisten tcp >/dev/null 2>&1 &
	foreign=$!
	for _ in {1..50}; do [[ -e "/tmp/.X${n}-lock" ]] && break; sleep 0.1; done
	rc=0; out="$(fRig "${n}" start 2>&1)" || rc=$?
	fCheck "a number another server holds is refused" test "${rc}" -ne 0
	fCheck "and not reported as started" bash -c '[[ "$1" != *Started* ]]' _ "${out}"
	fCheck "and the other server is left alone" test -d "/proc/${foreign}"
	kill "${foreign}" 2>/dev/null || true; fGone "${foreign}" || true

	## Stale: the pid file names a process that is not our server.
	n="$(fFreeDisplay 250)"
	sleep 60 &
	decoy=$!
	echo "${decoy}" >"${runDir}/xvfb-${n}.pid"
	fCheck "a stale pid file is not a running server" bash -c '[[ "$(USER="$1" CICD_HEADLESS_DISPLAY=":$2" "$3" status)" == no\ Xvfb* ]]' _ "${sandboxUser}" "${n}" "${headless}"
	fRig "${n}" stop >/dev/null 2>&1 || true
	fCheck "and stop leaves its process alone" test -d "/proc/${decoy}"
	kill "${decoy}" 2>/dev/null || true

	## Two runs: the one that started the server keeps it until it stops it.
	n="$(fFreeDisplay 250)"
	gate="$(mktemp -d)"
	bash -c 'USER="$1" CICD_HEADLESS_DISPLAY=":$2" "$3" start >/dev/null 2>&1; touch "$4/up"; while [[ ! -e "$4/go" ]]; do sleep 0.1; done; USER="$1" CICD_HEADLESS_DISPLAY=":$2" "$3" stop >/dev/null 2>&1' \
		_ "${sandboxUser}" "${n}" "${headless}" "${gate}" &
	firstRun=$!
	for _ in {1..100}; do [[ -e "${gate}/up" ]] && break; sleep 0.1; done
	server="$(tr -dc '0-9' <"/tmp/.X${n}-lock" 2>/dev/null || true)"
	fCheck "the first run's server is up" test -n "${server}"
	rc=0; fRig "${n}" start >/dev/null 2>&1 || rc=$?
	fCheck "a second run's start is refused rather than shared" test "${rc}" -ne 0
	rc=0; fRig "${n}" stop >/dev/null 2>&1 || rc=$?
	fCheck "a second run's stop is refused" test "${rc}" -ne 0
	fCheck "and the first run's server is still up" test -d "/proc/${server:-0}"
	touch "${gate}/go"; wait "${firstRun}" || true
	fCheck "the first run's own stop ends it" fGone "${server:-0}"
	rm -f "${gate}/up" "${gate}/go"; rmdir "${gate}"
	rm -rf "${runDir}"
else
	echo "  skip the rig's display checks (no Xvfb or xdpyinfo)"
fi

if ((failures)); then echo "${failures} failed"; exit 1; fi
echo "all passed"

##	History:
##		- 20260908 JC: Created.
##		- 20260917 JC: The rig's display: taken, stale and shared.
