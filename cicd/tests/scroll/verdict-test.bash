#!/usr/bin/env bash

##	- Purpose:
##		The scroll harness once printed OK and exited zero after running no scenes
##		at all, and stayed that way through a release. This holds its exit code to
##		what actually happened.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier [ID: 2უNაɘ«҂թȹɤξπ๙¿ձϖ]
##	SPDX-License-Identifier: GPL-2.0-or-later

set -euo pipefail
meDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${meDir}/verdict.bash"

failures=0
fCase(){
	local -r what="${1}" wantRc="${2}"; shift 2
	local out rc=0
	out="$(fScrollVerdict "${@}")" || rc=$?
	if [[ "${rc}" == "${wantRc}" ]]; then
		echo "  ok   ${what}"
	else
		echo "  FAIL ${what}: wanted rc ${wantRc}, got ${rc} (${out})"
		failures=$((failures + 1))
	fi
}

##                                        pass fail miss strict
fCase "four scenes passed"            0      4    0    0    0
fCase "a measured regression"         1      3    1    0    0
fCase "nothing ran at all"            1      0    0    4    0
fCase "nothing ran, nothing skipped"  1      0    0    0    0
fCase "a skip with passes, lenient"   0      3    0    1    0
fCase "a skip with passes, strict"    1      3    0    1    1

if ((failures)); then echo "${failures} failed"; exit 1; fi
echo "all passed"

##	History:
##		- 20260908 JC: Created.
