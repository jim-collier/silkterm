##	Runs one scenario inside the console session and writes a verdict file. Started
##	by an interactive scheduled task, because a process arriving over ssh is in
##	session 0 and has no desktop at all - which is what made every earlier attempt
##	at this look like a permissions problem.
param(
	[Parameter(Mandatory)] [string] $Scenario,
	[Parameter(Mandatory)] [string] $Exe,
	[Parameter(Mandatory)] [string] $OutDir
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$result = Join-Path $OutDir "result.txt"
$verdict = "fail"
$reason = ""

function fSkip($why) { $script:verdict = "skip"; $script:reason = $why; throw [OperationCanceledException]::new($why) }

try {
	. "$PSScriptRoot\_lib.ps1"
	[void][Silk.Win]::SetProcessDpiAwarenessContext([IntPtr](-4))   ## per-monitor v2
	$script:shotDir = Join-Path $OutDir "shots"

	fNote ("session " + [System.Diagnostics.Process]::GetCurrentProcess().SessionId + " as " + (whoami))
	fNote ("desktop usable: " + (fSessionUsable))
	fNote ("screen " + [Silk.Win]::GetSystemMetrics(0) + "x" + [Silk.Win]::GetSystemMetrics(1) +
	       "  remote-session " + [Silk.Win]::GetSystemMetrics(0x1000))

	if (-not (Test-Path $Exe)) { fSkip "no built binary at $Exe - run the build job first" }

	$script = Join-Path $PSScriptRoot "$Scenario.ps1"
	if (-not (Test-Path $script)) { throw "no such scenario: $Scenario" }
	. $script

	##	A scenario that asserted nothing must not read as a pass. The scroll harness
	##	printed OK for a while after it quietly stopped running any scene, and this
	##	is the same shape of hole.
	$asserted = @($script:checks | Where-Object { $_ -match '^\s+(ok|FAIL) ' }).Count
	if ($asserted -eq 0) { $reason = "the scenario made no checks"; $verdict = "fail" }
	else { $verdict = if ($script:failures -eq 0) { "pass" } else { "fail" } }
}
catch [OperationCanceledException] { }
catch {
	$reason = $_.Exception.Message
	$script:checks += "  FAIL scenario threw: $reason"
	$verdict = "fail"
}
finally {
	Get-Process -Name silkterm -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
	@("SCENARIO $Scenario", "VERDICT $verdict $reason") + $script:checks | Set-Content -Path $result -Encoding UTF8
}
