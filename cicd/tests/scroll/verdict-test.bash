#!/usr/bin/env bash

##	- Purpose:
##		The scroll harness once printed OK and exited zero after running no scenes
##		at all, and stayed that way through a release. This holds its exit code to
##		what actually happened, and the trace checks to what they are there to
##		catch. Nothing here needs a display.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier [ID: 2უNაɘ«҂թȹɤξπ๙¿ձϖ]
##	SPDX-License-Identifier: GPL-2.0-or-later

set -euo pipefail
meDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${meDir}/verdict.bash"

failures=0
fExpect(){
	local -r what="${1}" wantRc="${2}" rc="${3}" out="${4}"
	if [[ "${rc}" == "${wantRc}" ]]; then
		echo "  ok   ${what}"
	else
		echo "  FAIL ${what}: wanted rc ${wantRc}, got ${rc} (${out})"
		failures=$((failures + 1))
	fi
}

fCase(){
	local -r what="${1}" wantRc="${2}"; shift 2
	local out rc=0
	out="$(fScrollVerdict "${@}")" || rc=$?
	fExpect "${what}" "${wantRc}" "${rc}" "${out}"
}

##                                        pass fail miss strict
fCase "four scenes passed"            0      4    0    0    0
fCase "a measured regression"         1      3    1    0    0
fCase "nothing ran at all"            1      0    0    4    0
fCase "nothing ran, nothing skipped"  1      0    0    0    0
fCase "a skip with passes, lenient"   0      3    0    1    0
fCase "a skip with passes, strict"    1      3    0    1    1

## analyze.py on made-up traces, one frame per line: sh app_off frac alt
fAnalyze(){
	local -r what="${1}" wantRc="${2}" mode="${3}" frames="${4}"
	local out rc=0
	out="$(
		n=0
		while read -r sh app frac alt; do
			printf 'SCROLLDBG f=%d pane=0 sh=%s app_off=%s slide_sh=0.0000 st=0 sb=1 frac=%s alt=%s ob=0\n' \
				"${n}" "${sh}" "${app}" "${frac}" "${alt}"
			n=$((n + 1))
		done <<<"${frames}" | python3 "${meDir}/analyze.py" --mode "${mode}" --expect-st 0 --expect-sb 1 --label test
	)" || rc=$?
	fExpect "${what}" "${wantRc}" "${rc}" "${out}"
}

fAnalyze "a slide that eases one way" 0 slide "1 1.0 0 1
0 0.8 0 1
0 0.6 0 1
0 0.4 0 1
0 0.2 0 1
1 1.0 0 1
0 0.5 0 1"
fAnalyze "a scene that scrolled and never slid" 1 slide "1 0 0 1
0 0 0 1
0 0 0 1
1 0 0 1
0 0 0 1
1 0 0 1"
fAnalyze "a scene that never scrolled" 2 slide "0 0 0 1
0 0 0 1
0 0 0 1
0 0 0 1
0 0 0 1"
fAnalyze "an ease landed by the alt screen" 0 still "0 0 0.5 0
0 0 0.3 0
0 0 0 1
0 0 0 1"
fAnalyze "an alt screen with no ease before it" 1 still "0 0 0 1
0 0 0 1
0 0 0 1"
fAnalyze "an ease left running on the alt screen" 1 still "0 0 0.5 0
0 0 0.4 1
0 0 0 1"
fAnalyze "no alt screen at all" 2 still "0 0 0.5 0"

## run.bash where something it needs is missing: 3, so cicd says skipped, not OK
fakeBin="$(mktemp -d)"
trap 'rm -f "${fakeBin}/dirname" "${fakeBin}/python3"; rmdir "${fakeBin}" 2>/dev/null || true' EXIT
ln -s "$(command -v dirname)" "${fakeBin}/dirname"
ln -s "$(command -v python3)" "${fakeBin}/python3"
fRun(){
	local -r what="${1}" wantRc="${2}"; shift 2
	local out rc=0
	out="$(PATH="${fakeBin}" "${BASH}" "${meDir}/run.bash" "${@}" 2>&1)" || rc=$?
	fExpect "${what}" "${wantRc}" "${rc}" "$(tail -n 1 <<<"${out}")"
}
fRun "no binary"                 3 --bin /nonexistent/silkterm
fRun "no binary, strict"         1 --bin /nonexistent/silkterm --strict
fRun "no Xvfb"                   3 --bin /bin/true
fRun "no cage"                   3 --bin /bin/true --wayland

if ((failures)); then echo "${failures} failed"; exit 1; fi
echo "all passed"

##	History:
##		- 20260908 JC: Created.
##		- 20260915 JC: Trace checks and the skips before any scene runs.
