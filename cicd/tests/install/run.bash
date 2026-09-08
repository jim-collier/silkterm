#!/usr/bin/env bash

##	- Purpose:
##		Hygiene the installers and the headless rig have to keep: no secret on a
##		command line, no plain-http redirect, no adopting a directory somebody
##		else made in a shared temp folder.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier (CryptogID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
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

if ((failures)); then echo "${failures} failed"; exit 1; fi
echo "all passed"

##	History:
##		- 20260908 JC: Created.
