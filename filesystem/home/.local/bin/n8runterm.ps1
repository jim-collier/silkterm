##	Purpose:
##		- Windows port of the bash 'n8runterm' launcher. Keeps a small pool of
##		  date-stamped SilkTerm dogfood builds in the local target dir and launches
##		  one, passing through any arguments.
##		- Three build sources, each tagged in the copy's name so they coexist:
##			gnul  the b23 cross-build over SMB   (gnu toolchain, built on Linux)
##			gnuw  local Windows gnu release      (gnu toolchain, built on Windows)
##			msvc  local Windows msvc release      (msvc toolchain, built on Windows)
##		  Copies are named 'slktrmdf_<YYYYMMDD-HHMMSS>_<tag>.exe' where the stamp is
##		  the build's own mtime, so a given build is copied once and a running copy
##		  never blocks the copy.
##		- Each run, in order: delete idle builds over 7 days old; refresh each source
##		  whose build is newer than what we already hold; then pick one to run.
##		- Which to run: the newest build by stamp. If that newest came from b23 (gnul)
##		  run it. Otherwise it's a local Windows build - if the newest gnuw and msvc
##		  are within 15 min of each other, flip a coin between them, else run the
##		  newest outright.
##		- Prepends a random background image and a build-tagged title so a dogfood
##		  window is visually distinct. Both precede the passed args, so a caller can
##		  still override them.
##		- With '--admin', runs the WHOLE launcher elevated (self-elevates via a UAC
##		  prompt), so copying a fresh build into the target dir - and the launched
##		  terminal - both run with admin rights. A shortcut click then behaves like
##		  running from an elevated shell, instead of silently launching a stale build
##		  because the medium-integrity click couldn't write the target dir.
##		- Reports a failure or a skipped build copy in a dialog when launched from a
##		  shortcut (or with '--gui'), since a click's console just flashes shut.
##		  '--admin'/'--gui' are consumed here; all other args forward to the terminal.
##		- If no dogfood build is held and no source is reachable, falls back in
##		  order to: silkterm.exe on PATH, Windows Terminal, PyCmd, then cmd.exe.
##		- Edit fMain() to launch a different terminal instead.
##	History: At bottom of script.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Configuration

## Source 'gnul': the b23 SilkTerm Windows (x86_64-pc-windows-gnu) release build,
## reached over SMB. Canonical path with the '0_links' junctions resolved, so it
## works without the mapped-folder aliases. The original alias was:
##   C:\0-0\users\collierjr\0_links\b23•collierjr•0_links\projects\dev\zf10…github∙jimcollier\silkterm\github\target\x86_64-pc-windows-gnu\release
$B23ReleaseDir = "\\b23\home-collierjr\0-0\0_links\projects\dev\zf10…github∙jimcollier\silkterm\github\target\x86_64-pc-windows-gnu\release"

## Sources 'gnuw'/'msvc': the local Windows-native release build dirs (same clone,
## two target triples).
$LocalTargetRoot = "C:\opt\0-0\users\collierjr\data\prs\dev\github\jim-collier\silkterm\github\target"
$GnuwReleaseDir  = Join-Path $LocalTargetRoot "x86_64-pc-windows-gnu\release"
$MsvcReleaseDir  = Join-Path $LocalTargetRoot "x86_64-pc-windows-msvc\release"

$ExeName = "silkterm.exe"

## Launch elevated (as administrator). Off by default; the '--admin' arg (consumed
## at the entry point below, never forwarded) flips it on. RunAs pops a UAC consent
## unless the calling session is already elevated.
$RunAsAdmin = $false

## Fallback terminals, tried in order when no dogfood build is held and no source
## is reachable. First is our own terminal (kept dressed with bg+title); the rest
## are generic, launched plainly. cmd.exe (always in System32) is the last resort.
$FallbackTerminals = @(
	@{ Name = "silkterm (PATH)";   Exe = "silkterm.exe"; Silk = $true  }
	@{ Name = "Windows Terminal";  Exe = "wt.exe";       Silk = $false }
	@{ Name = "PyCmd";             Exe = "PyCmd.exe";    Silk = $false }
	@{ Name = "cmd";               Exe = "cmd.exe";      Silk = $false }
)

## Target: where the runnable copies live. Stamped copies accumulate here. This
## is the LOCAL (non-synced) util dir on purpose - dogfood copies churn every
## build and shouldn't ride a Dropbox sync. (cicd's fixed-name install is what
## drops a build into the synced dir.)
$TargetDir = "C:\opt\0-0\common\exec\local\util\mswin\gui\by-self\win64"

## Prefix for the date-stamped copies (matches cicd's dogfood convention).
$DogfoodPrefix = "slktrmdf"

## Per-run decision log, kept in the target dir. Every note/warn/fail line lands
## here too, so a closed console can't lose the copy/skip reasons behind a launch.
$RunLog = Join-Path $TargetDir "n8runterm.log"

## Delete idle stamped copies older than this many days.
$MaxAgeDays = 7

## When the newest gnuw and msvc builds are within this many minutes, flip a coin
## on which to run instead of always taking whichever finished last.
$CoinWindowMin = 15

## Stamp format shared by the copy name and every date comparison below.
$StampFormat = "yyyyMMdd-HHmmss"


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Functions

## Entry point: what this launcher runs. Edit this to launch a different terminal.
function fMain {
	param([string[]]$PassArgs)

	if (-not (Test-Path -LiteralPath $TargetDir)) {
		New-Item -ItemType Directory -Path $TargetDir -Force | Out-Null
	}

	fTrimLog
	fLog ("=== run: PS {0}, host '{1}', script {2}, user {3} ===" -f `
		$PSVersionTable.PSVersion, $Host.Name, $PSCommandPath, $env:USERNAME)

	## 0. Strip a synced-on mark-of-the-web so a later click can't be policy-blocked.
	fSelfHealMotw

	## 1. Delete stale idle copies.
	fDeleteOldBuilds

	## 2. Refresh each source that has a newer build than we hold.
	fCopyIfNewer -SourceDir $B23ReleaseDir  -Tag "gnul"
	fCopyIfNewer -SourceDir $GnuwReleaseDir -Tag "gnuw"
	fCopyIfNewer -SourceDir $MsvcReleaseDir -Tag "msvc"

	## 3. Pick one and launch it.
	$exe = fSelectBuildToRun
	if ($exe) {
		fLaunchSilkTerm -Exe $exe -PassArgs $PassArgs
		return
	}

	## 4. Nothing held and no source reachable - fall back to any terminal we can
	##    find on PATH.
	fWarn "no SilkTerm dogfood build (no source reachable and none held); trying fallbacks"
	fLaunchFallbackTerminal -PassArgs $PassArgs
}


## Delete stamped copies whose build is older than $MaxAgeDays, skipping any that
## are running (a running .exe image is locked, so a delete that throws is also
## treated as in-use). Only ever touches files matching THIS launcher's own name
## spec ('slktrmdf_<stamp>[_<tag>].exe') - never a foreign file that merely shares
## the dir, e.g. the fixed 'SilkTerm.exe' that cicd-win.ps1 drops here.
function fDeleteOldBuilds {
	## Any tag ages out here (incl. one-off hand-dropped tags); only the known
	## tags are ever SELECTED to run (fTaggedBuilds stays strict).
	$rx      = "^$([regex]::Escape($DogfoodPrefix))_\d{8}-\d{6}(_[a-z0-9]+)?\.exe$"
	$cutoff  = (Get-Date).AddDays(-$MaxAgeDays)
	$running = @(fRunningExePaths)
	$deleted = 0

	Get-ChildItem -LiteralPath $TargetDir -File -Filter "${DogfoodPrefix}_*.exe" -ErrorAction SilentlyContinue |
		Where-Object { $_.Name -match $rx } |
		Where-Object { (fBuildTime $_) -lt $cutoff } |
		ForEach-Object {
			if (fRemoveIfIdle -FileInfo $_ -Running $running) { $deleted++ }
		}

	if ($deleted) { fNote "deleted $deleted build(s) older than $MaxAgeDays days" }
}


## Copy $SourceDir\$ExeName in as 'slktrmdf_<stamp>_<Tag>.exe' when its build is
## newer than the newest copy of that tag we already hold. No-op if the source is
## unreachable or we're already current. Each tag is checked independently.
function fCopyIfNewer {
	param(
		[Parameter(Mandatory)][string]$SourceDir,
		[Parameter(Mandatory)][string]$Tag
	)

	$src = Join-Path $SourceDir $ExeName
	if (-not (Test-Path -LiteralPath $src)) {
		fWarn "$Tag source not reachable: $src"
		return
	}

	$stamp     = (Get-Item -LiteralPath $src).LastWriteTime.ToString($StampFormat)
	$stampTime = fParseStamp $stamp
	$existing  = fNewestOfTag $Tag

	if ($existing -and $existing.Stamp -ge $stampTime) {
		fNote "$Tag already current (held $($existing.Stamp.ToString($StampFormat)), src $stamp)"
		return
	}

	$dst = Join-Path $TargetDir "${DogfoodPrefix}_${stamp}_${Tag}.exe"
	if (Test-Path -LiteralPath $dst) {
		fNote "$Tag copy already present: $(Split-Path $dst -Leaf)"
		return
	}

	try {
		Copy-Item -LiteralPath $src -Destination $dst -Force -ErrorAction Stop
		fNote "copied $Tag -> $(Split-Path $dst -Leaf)"
	} catch {
		fWarn -Gui "couldn't copy $Tag build ($($_.Exception.Message))"
	}
}


## Pick the copy to run. Newest by stamp wins; if that newest is a local Windows
## build (gnuw/msvc) and the newest of each is within $CoinWindowMin of the other,
## flip a coin between them. Falls back to the newest legacy (untagged) copy if no
## tagged builds exist. Returns a full path, or $null if the dir is empty.
function fSelectBuildToRun {
	$builds = @(fTaggedBuilds)

	if (-not $builds) {
		$legacy = Get-ChildItem -LiteralPath $TargetDir -File -Filter "${DogfoodPrefix}_*.exe" -ErrorAction SilentlyContinue |
			Sort-Object Name -Descending | Select-Object -First 1
		if (-not $legacy) { return $null }
		fNote "running (untagged): $($legacy.Name)"
		return $legacy.FullName
	}

	$latest = $builds | Sort-Object Stamp -Descending | Select-Object -First 1

	if ($latest.Tag -eq "gnul") {
		fNote "running newest (b23/gnul): $($latest.Name)"
		return $latest.File.FullName
	}

	## Newest is a local Windows build - maybe coin-flip gnuw vs msvc.
	$gnuw = $builds | Where-Object { $_.Tag -eq "gnuw" } | Sort-Object Stamp -Descending | Select-Object -First 1
	$msvc = $builds | Where-Object { $_.Tag -eq "msvc" } | Sort-Object Stamp -Descending | Select-Object -First 1

	if ($gnuw -and $msvc) {
		$gapMin = [math]::Abs(($gnuw.Stamp - $msvc.Stamp).TotalMinutes)
		if ($gapMin -le $CoinWindowMin) {
			$pick = if ((Get-Random -Minimum 0 -Maximum 2) -eq 0) { $gnuw } else { $msvc }
			fNote ("coin flip (gnuw/msvc within {0:N1} min) -> {1}: {2}" -f $gapMin, $pick.Tag, $pick.Name)
			return $pick.File.FullName
		}
	}

	fNote "running newest local ($($latest.Tag)): $($latest.Name)"
	return $latest.File.FullName
}


## All tagged copies as objects { File, Name, Tag, Stamp(DateTime) }.
function fTaggedBuilds {
	$rx = "^$([regex]::Escape($DogfoodPrefix))_(?<stamp>\d{8}-\d{6})_(?<tag>gnul|gnuw|msvc)\.exe$"
	Get-ChildItem -LiteralPath $TargetDir -File -Filter "${DogfoodPrefix}_*.exe" -ErrorAction SilentlyContinue |
		ForEach-Object {
			if ($_.Name -match $rx) {
				[pscustomobject]@{
					File  = $_
					Name  = $_.Name
					Tag   = $Matches.tag
					Stamp = fParseStamp $Matches.stamp
				}
			}
		}
}


## Newest tagged copy of one tag (object from fTaggedBuilds), or $null.
function fNewestOfTag {
	param([Parameter(Mandatory)][string]$Tag)
	fTaggedBuilds | Where-Object { $_.Tag -eq $Tag } |
		Sort-Object Stamp -Descending | Select-Object -First 1
}


## A copy's build time: the stamp embedded in its name if present, else its mtime
## (covers legacy untagged 'slktrmdf_<stamp>.exe' copies too).
function fBuildTime {
	param([Parameter(Mandatory)]$FileInfo)
	if ($FileInfo.Name -match "_(?<stamp>\d{8}-\d{6})(?:_[a-z0-9]+)?\.exe$") {
		return fParseStamp $Matches.stamp
	}
	return $FileInfo.LastWriteTime
}


## Parse a 'yyyyMMdd-HHmmss' stamp to a DateTime.
function fParseStamp {
	param([Parameter(Mandatory)][string]$Stamp)
	return [datetime]::ParseExact($Stamp, $StampFormat, [System.Globalization.CultureInfo]::InvariantCulture)
}


## Delete one copy unless it's running or locked. Returns $true if deleted.
function fRemoveIfIdle {
	param(
		[Parameter(Mandatory)]$FileInfo,
		[string[]]$Running
	)
	if ($Running -contains $FileInfo.FullName) {
		fNote "kept (running): $($FileInfo.Name)"
		return $false
	}
	try {
		Remove-Item -LiteralPath $FileInfo.FullName -Force -ErrorAction Stop
		return $true
	} catch {
		fNote "kept (locked): $($FileInfo.Name)"
		return $false
	}
}


## Full image paths of all currently running processes (best-effort; the analog
## of the bash launcher's /proc/*/exe scan). Paths we can't read are skipped.
function fRunningExePaths {
	Get-Process -ErrorAction SilentlyContinue |
		ForEach-Object { try { $_.Path } catch { $null } } |
		Where-Object { $_ }
}


## Launch SilkTerm detached (GUI subsystem, so no console attaches), prepending a
## random background image (if any) and a title tagged with the build's tag+stamp.
## Passed args come last so they win.
function fLaunchSilkTerm {
	param(
		[Parameter(Mandatory)][string]$Exe,
		[string[]]$PassArgs
	)

	## Title: a dogfood tag for a stamped copy, else a plain title (e.g. a silkterm
	## found on PATH is a real terminal, not a dogfood build).
	$leaf   = [System.IO.Path]::GetFileNameWithoutExtension($Exe)
	$prefRx = "^$([regex]::Escape($DogfoodPrefix))_"
	if ($leaf -match "${prefRx}(?<stamp>\d{8}-\d{6})_(?<tag>[a-z0-9]+)$") {
		$title = "SilkTerm [dogfood $($Matches.tag) $($Matches.stamp)]"
	} elseif ($leaf -match $prefRx) {
		$label = $leaf -replace $prefRx, ""
		$title = "SilkTerm [dogfood $label]"
	} else {
		$title = "SilkTerm"
	}

	$preArgs = @()
	$bg = fPickRandomBackground
	if ($bg) { $preArgs += "--background-image=$bg" }
	$preArgs += "--title=$title"

	$all = @($preArgs)
	if ($PassArgs) { $all += $PassArgs }

	## Start-Process joins -ArgumentList with spaces WITHOUT quoting, so an arg
	## whose value has a space (the title, or a bg path under a spaced folder)
	## would be split into separate argv entries by the target and rejected.
	## Quote any such arg ourselves.
	$quoted = @($all | ForEach-Object { fQuoteArg $_ })

	return fStartTerminal -Exe $Exe -ArgList $quoted
}


## Fall back to whatever terminal is on PATH, in $FallbackTerminals order. Our own
## silkterm keeps the bg+title dress (via fLaunchSilkTerm); generic terminals are
## launched plainly - silkterm's --background-image/--title flags don't apply and
## its pass-through args likely don't either, so they get none. cmd.exe lives in
## System32 (always on PATH), so this effectively always finds something.
function fLaunchFallbackTerminal {
	param([string[]]$PassArgs)

	foreach ($cand in $FallbackTerminals) {
		$path = fFindOnPath $cand.Exe
		if (-not $path) { continue }

		if ($cand.Silk) {
			fNote "falling back to $($cand.Name): $path"
			return fLaunchSilkTerm -Exe $path -PassArgs $PassArgs
		}

		fNote "falling back to $($cand.Name): $path"
		return fStartTerminal -Exe $path -ArgList @()
	}

	fFail ("no terminal available (no SilkTerm build/source, and none of " +
		(($FallbackTerminals | ForEach-Object { $_.Exe }) -join ", ") + " on PATH)")
}


## Resolve an executable's full path from PATH, or $null. -CommandType Application
## keeps it to real .exe's (never a shell function/alias of the same name).
function fFindOnPath {
	param([Parameter(Mandatory)][string]$Exe)
	$cmd = Get-Command $Exe -CommandType Application -ErrorAction SilentlyContinue |
		Select-Object -First 1
	if ($cmd) { return $cmd.Source }
	return $null
}


## Launch a terminal in its own process, elevated when $RunAsAdmin. Returns the
## Process so a caller (e.g. a test harness) can stop this exact instance by PID -
## matching on name/pattern risks hitting another copy launched elsewhere.
function fStartTerminal {
	param(
		[Parameter(Mandatory)][string]$Exe,
		[string[]]$ArgList
	)

	$sp = @{ FilePath = $Exe; PassThru = $true }
	if ($ArgList -and $ArgList.Count) { $sp.ArgumentList = $ArgList }
	if ($RunAsAdmin) { $sp.Verb = "RunAs" }

	try {
		$proc = Start-Process @sp
	} catch {
		## RunAs throws if UAC is declined; surface it plainly.
		fFail "launch failed for $Exe ($($_.Exception.Message))"
	}

	$how = if ($RunAsAdmin) { " (as admin)" } else { "" }
	fNote "launched$how pid $($proc.Id): $([System.IO.Path]::GetFileName($Exe))"
	return $proc
}


## Wrap an argument in double quotes if it contains whitespace, so Start-Process
## passes it as a single argv entry (see fLaunchSilkTerm).
function fQuoteArg {
	param([string]$Arg)
	if ($Arg -match '\s') { return '"' + $Arg + '"' }
	return $Arg
}


## Resolve SilkTerm's backgrounds dir the same way the app does:
## XDG_CONFIG_HOME, else HOME\.config, else APPDATA - then \silkterm\backgrounds.
function fResolveBackgroundsDir {
	$base = $null
	if ($env:XDG_CONFIG_HOME) { $base = $env:XDG_CONFIG_HOME }
	elseif ($env:HOME)        { $base = Join-Path $env:HOME ".config" }
	elseif ($env:APPDATA)     { $base = $env:APPDATA }
	if (-not $base) { return $null }
	return (Join-Path $base "silkterm\backgrounds")
}


## Pick a random image from the backgrounds dir, or $null if there are none.
function fPickRandomBackground {
	$dir = fResolveBackgroundsDir
	if (-not $dir -or -not (Test-Path -LiteralPath $dir)) { return $null }
	$imgs = Get-ChildItem -LiteralPath $dir -File |
		Where-Object { $_.Extension -in ".png", ".jpg", ".jpeg" }
	if (-not $imgs) { return $null }
	return ($imgs | Get-Random).FullName
}


## Informational note to the host (and the run log).
function fNote { param([string]$Msg); fLog $Msg; Write-Host "n8runterm: $Msg" }

## Non-fatal note to stderr (and the run log). Pass -Gui to also surface it in the
## end-of-run dialog (the shortcut case, where the console flashes shut) - reserved
## for real problems (a failed copy), not benign skips (an offline source).
function fWarn {
	param([string]$Msg, [switch]$Gui)
	fLog "WARN: $Msg"
	Write-Warning "n8runterm: $Msg"
	if ($Gui) { $script:RunWarnings += $Msg }
}

## Fatal error to stderr (and the run log), then stop. Pops a dialog first when GUI
## feedback is on, so a shortcut click shows WHY instead of a blank flash.
function fFail {
	param([string]$Msg)
	fLog "FAIL: $Msg"
	if ($script:GuiFeedback) { fGuiShow -Msg $Msg -Icon Error -Title "SilkTerm dogfood - failed" }
	Write-Error "n8runterm: $Msg"
	exit 1
}


## Append a timestamped line to the run log. Best-effort: logging must never be
## the thing that stops a launch.
function fLog {
	param([string]$Msg)
	try {
		Add-Content -LiteralPath $RunLog -Encoding utf8 -Value `
			("{0}  {1}" -f (Get-Date -Format "yyyy-MM-dd HH:mm:ss"), $Msg)
	} catch { }
}


## Keep the run log from growing without bound.
function fTrimLog {
	try {
		if ((Test-Path -LiteralPath $RunLog) -and (Get-Item -LiteralPath $RunLog).Length -gt 256KB) {
			$tail = Get-Content -LiteralPath $RunLog -Tail 500
			Set-Content -LiteralPath $RunLog -Value $tail -Encoding utf8
		}
	} catch { }
}


## Remove any mark-of-the-web this script picked up from the sync layer. An unsigned
## script that carries MOTW is refused under a RemoteSigned policy - which silently
## kills a shortcut click (the body never runs, so nothing copies and nothing logs).
## This only helps the NEXT run; the current one already cleared the policy to be
## here. Belt-and-suspenders with the launcher's '-ExecutionPolicy Bypass' - either
## alone is enough. Best-effort; never let it stop a launch.
function fSelfHealMotw {
	try {
		$zone = Get-Content -LiteralPath $PSCommandPath -Stream Zone.Identifier -ErrorAction SilentlyContinue
		if ($zone) {
			Unblock-File -LiteralPath $PSCommandPath -ErrorAction Stop
			fNote "cleared mark-of-the-web on this script (would block a click under RemoteSigned)"
		}
	} catch {
		fWarn "couldn't clear mark-of-the-web on this script ($($_.Exception.Message))"
	}
}


## True when this process is running elevated (Administrators / high integrity).
function fIsElevated {
	$id = [System.Security.Principal.WindowsIdentity]::GetCurrent()
	return (New-Object System.Security.Principal.WindowsPrincipal($id)).IsInRole(
		[System.Security.Principal.WindowsBuiltInRole]::Administrator)
}


## True when we were double-clicked (a .lnk / Explorer launch) rather than started
## from a shell - Explorer is the parent of a shortcut click, a terminal (pwsh/cmd/
## wt) is the parent of a command-line run. Used to auto-enable GUI feedback so a
## flash-and-close shortcut can still report a failure. Best-effort -> $false.
function fLaunchedFromShortcut {
	try {
		$parentId = (Get-CimInstance Win32_Process -Filter "ProcessId=$PID" -ErrorAction Stop).ParentProcessId
		$parent   = (Get-Process -Id $parentId -ErrorAction Stop).ProcessName
		return ($parent -ieq "explorer")
	} catch { return $false }
}


## Show a modal message box. Never throws - feedback must not be the thing that
## breaks a launch; a no-op if WinForms can't load.
function fGuiShow {
	param(
		[Parameter(Mandatory)][string]$Msg,
		[ValidateSet("Error", "Warning", "Information")][string]$Icon = "Information",
		[string]$Title = "SilkTerm dogfood"
	)
	try {
		Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop
		[System.Windows.Forms.MessageBox]::Show(
			$Msg, $Title,
			[System.Windows.Forms.MessageBoxButtons]::OK,
			[System.Windows.Forms.MessageBoxIcon]::$Icon) | Out-Null
	} catch { }
}


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Script entry point

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

## Problems worth surfacing at the end (failed copies etc.), shown in a dialog when
## launched from a shortcut. Must exist before any fWarn -Gui / fFail can run.
$script:RunWarnings = @()

## Consume our own flags; forward everything else to the terminal.
##   --admin  run the WHOLE launcher elevated (self-elevates below) - copy, log and
##            the launched terminal all get admin rights.
##   --gui    force the end-of-run / failure dialog on (auto-on for a shortcut click).
$wantAdmin = $false
$forceGui  = $false
$passArgs  = @()
foreach ($arg in $args) {
	switch -Regex ($arg) {
		'^--admin$' { $wantAdmin = $true; continue }
		'^--gui$'   { $forceGui  = $true; continue }
		default     { $passArgs += $arg }
	}
}

$script:GuiFeedback = $forceGui -or (fLaunchedFromShortcut)

## Self-elevate: with '--admin' but not already elevated, relaunch the whole script
## elevated and hand off. Everything then runs high-integrity, so it no longer
## matters whether the target dir grants a normal user write - the real fix for
## "a shortcut click launches a stale build". The relaunch carries the original args
## plus '--gui' (its parent is the UAC broker, not Explorer, so it can't re-detect
## the shortcut). If consent is declined we DON'T abort - we fall through and run
## non-elevated so the user still gets a terminal, with a dialog saying it may be
## stale (the granted target-dir ACL usually lets even that copy succeed).
if ($wantAdmin -and -not (fIsElevated)) {
	$self = (Get-Process -Id $PID).Path      # the pwsh.exe hosting this script
	$fwd  = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $PSCommandPath) + $args + "--gui"
	$fwd  = @($fwd | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } })
	try {
		Start-Process -FilePath $self -Verb RunAs -ArgumentList $fwd -ErrorAction Stop | Out-Null
		exit 0
	} catch {
		fWarn "elevation declined; running without admin (a newer build may not copy)"
		if ($script:GuiFeedback) {
			fGuiShow -Icon Warning -Title "SilkTerm dogfood - not elevated" -Msg (
				"Administrator access was declined.`n`nRunning without it - a newer " +
				"build may not copy in, so an older one could launch.")
		}
	}
}

## Elevated (self- or from an elevated shell): also launch the terminal elevated.
if ($wantAdmin) { $RunAsAdmin = $true }

## Kick everything off, passing through whatever's left.
fMain -PassArgs $passArgs

## Surface any real problems (failed copies etc.) for the shortcut case.
if ($script:GuiFeedback -and $script:RunWarnings.Count) {
	fGuiShow -Icon Warning -Title "SilkTerm dogfood" -Msg (
		"Launched, but with issues:`n`n - " + ($script:RunWarnings -join "`n - "))
}


##	History:
##		- 2026-07-19 JC: '--admin' now self-elevates the whole launcher (was only the
##		  launched terminal), so a non-elevated shortcut click copies the fresh build
##		  instead of silently launching a stale one. Report failures / skipped copies
##		  in a dialog for the shortcut case (console flashes shut); new '--gui' flag,
##		  auto-on when double-clicked.
##		- 2026-07-17 JC: Strip a synced-on mark-of-the-web at startup so a later
##		  click under RemoteSigned isn't silently blocked.
##		- 2026-07-17 JC: Log every run's per-source copy decision (and each note/
##		  warn) to n8runterm.log in the target dir, trimmed at 256KB.
##		- 2026-07-16 JC: Age-prune stamped copies with any tag, not just the known
##		  three (one-off tags could never be deleted); selection still known-tags-only.
##		- 2026-07-15 JC: Elevate only on '--admin' (consumed, not forwarded); default
##		  is the normal token.
##		- 2026-07-15 JC: Launch elevated by default; fall back to silkterm on PATH /
##		  Windows Terminal / PyCmd / cmd.exe when no build or source is available.
##		- 2026-07-15 JC: Target the local (non-synced) util dir, not the Dropbox one.
##		- 2026-07-15 JC: Prune only files matching our own name spec (leave foreign
##		  files like cicd-win.ps1's fixed SilkTerm.exe alone).
##		- 2026-07-15 JC: Reorder copy name to stamp-then-tag (slktrmdf_<stamp>_<tag>).
##		- 2026-07-15 JC: Three tagged sources (gnul/gnuw/msvc); age-based delete;
##		  newest-by-stamp run with a gnuw/msvc coin flip when close in time.
##		- 2026-07-14 JC: Return the launched Process so callers can target it by PID.
##		- 2026-07-14 JC: Quote args with spaces (title/bg path) so they aren't split.
##		- 2026-07-14 JC: Rotating stamped copies + prune idle ones (was fixed-name).
##		- 2026-07-14 JC: Created (Windows port of the bash n8runterm).
