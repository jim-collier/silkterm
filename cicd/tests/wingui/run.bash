#!/usr/bin/env bash

#  shellcheck disable=2016  ## 'Expressions don't expand in single quotes.' The PowerShell being generated needs literal '$'.

##	- Purpose:
##		Run a graphical scenario against a real Windows desktop and bring back the
##		verdict and the screenshots. Nothing here can be done from an ssh session
##		on its own: that lands in session 0, which has no desktop, so the scenario
##		is handed to an interactive scheduled task in the console session instead.
##	- Syntax:
##		run.bash [--host <name>] [--keep] [<scenario> ...]
##		With no scenario it runs 'smoke'. Shots come back under cicd/artifacts/wingui.
##	- Exit: 0 pass or skipped, 1 a scenario failed.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier (CryptogID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
##	SPDX-License-Identifier: GPL-2.0-or-later

set -euo pipefail
meDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "${meDir}/../../.." && pwd)"
winRemote="${root}/cicd/utility/win-remote.bash"
shotDir="${root}/cicd/artifacts/wingui"

host=(); keep=0
while (($#)); do case "$1" in
	--host) host=(--host "${2:-}"); shift 2 ;;
	--keep) keep=1; shift ;;
	-h|--help) grep -E '^##' "$0" | sed 's/^##\t\?//'; exit 0 ;;
	*) break ;;
esac; done
scenarios=("$@"); ((${#scenarios[@]})) || scenarios=(smoke)

[[ -x "${winRemote}" ]] || { echo "wingui: no win-remote.bash, skipped"; exit 0; }

##	The harness ships itself rather than coming from the remote clone, which is
##	pinned to origin/dev - otherwise every edit here would need a push before it
##	could be run once.
bundle="$(mktemp --suffix=.tgz)"
launcher=""
tar czf "${bundle}" -C "${meDir}" --exclude=run.bash .
trap 'rm -f "${bundle}" "${launcher}"' EXIT

fRun() {
	local scenario="$1" fresh="$2"
	launcher="$(mktemp --suffix=.ps1)"
	{
		printf '$ErrorActionPreference = "Stop"\n'
		printf '. "$PSScriptRoot\\_env.ps1"\n'
		printf '$scenario = "%s"\n' "${scenario}"
		printf '$fresh = %s\n' "${fresh}"
		printf '$dir = "C:\\ProgramData\\silkrig"\n'
		printf '$b64 = @"\n%s\n"@\n' "$(base64 -w120 "${bundle}")"
		cat <<'PS'
$work = Join-Path $dir "wingui"
$out  = Join-Path $dir "out"
Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
if ($fresh) { Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue }
New-Item -ItemType Directory -Force -Path $work, $out | Out-Null
$tgz = Join-Path $dir "wingui.tgz"
[IO.File]::WriteAllBytes($tgz, [Convert]::FromBase64String(($b64 -replace '\s', '')))
tar.exe -xzf $tgz -C $work
Remove-Item $tgz -Force

$exe = Join-Path $RepoDir "target\release\silkterm.exe"
if (-not (Test-Path $exe)) { $exe = Join-Path $RepoDir "source\target\release\silkterm.exe" }

##	An interactive-token task is the one route into the console session that needs
##	no stored password: it runs as whoever is logged on, with their desktop.
$pwsh = (Get-Command pwsh).Source
$name = 'silkrig-gui'
$me   = "$env:COMPUTERNAME\$env:USERNAME"
$arg  = "-NoProfile -STA -ExecutionPolicy Bypass -File `"$work\_run.ps1`" -Scenario $scenario -Exe `"$exe`" -OutDir `"$out`""
$act  = New-ScheduledTaskAction -Execute $pwsh -Argument $arg
$pri  = New-ScheduledTaskPrincipal -UserId $me -LogonType Interactive
##	Always clear the last answer first. A result file left by the scenario before
##	this one is indistinguishable from this one finishing instantly, and the poll
##	below would take it, print it, and unregister the task mid-run.
$res  = Join-Path $out "result.txt"
Remove-Item $res -Force -ErrorAction SilentlyContinue
try {
	Register-ScheduledTask -TaskName $name -Action $act -Principal $pri -Force | Out-Null
	Start-ScheduledTask -TaskName $name
	for ($i = 0; $i -lt 480; $i++) { if (Test-Path $res) { break }; Start-Sleep -Milliseconds 500 }
} finally {
	Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction SilentlyContinue
	Get-Process -Name silkterm -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}
if (-not (Test-Path $res)) { "VERDICT fail the session never answered"; exit 1 }
$said = (Get-Content $res | Where-Object { $_ -like "SCENARIO *" }) -replace '^SCENARIO ', ''
if ($said -ne $scenario) { "VERDICT fail the answer is from '$said', not '$scenario'"; exit 1 }
Get-Content $res | Where-Object { $_ -notlike "SCENARIO *" }
Get-ChildItem (Join-Path $out "shots") -Filter *.png -ErrorAction SilentlyContinue |
	ForEach-Object { "  shot $($_.Name) $($_.Length)" }
if ((Get-Content $res -TotalCount 1) -like "VERDICT fail*") { exit 1 }
PS
	} > "${launcher}"
	"${winRemote}" "${host[@]}" --optional run "${launcher}" 2>&1
}

failed=0; first=1
for scenario in "${scenarios[@]}"; do
	echo "== wingui: ${scenario}"
	if ! fRun "${scenario}" "$( ((first)) && echo '$true' || echo '$false' )" | sed 's/^/  /'; then failed=1; fi
	first=0
done

##	Shots are the whole point of a graphical test, so bring them home.
if ((! keep)); then
	mkdir -p "${shotDir}"
	"${winRemote}" "${host[@]}" --optional pull 'C:\ProgramData\silkrig\out\shots' "${shotDir}" >/dev/null 2>&1 || true
	find "${shotDir}" -name '*.png' -printf '  shot %P\n' 2>/dev/null | sort || true
fi

((failed == 0))

##	Script history:
##		- 20260908: Created.
