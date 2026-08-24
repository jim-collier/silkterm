##	Purpose:
##		- Windows port of the bash 'n8runterm' launcher. Keeps a small pool of
##		  date-stamped SilkTerm dogfood builds in the local target dir and launches
##		  one, passing through any arguments.
##		- Four build sources, each tagged in the copy's name so they coexist. A tag
##		  is '<toolchain: gnu|msvc><built on: l|m|b|w><target: l|m|b|w><arch: i|a>':
##			gnulwi   the b23 cross-build over SMB  (gnu, built on Linux, x86_64)
##			gnuwwi   local Windows gnu release     (gnu, built on Windows, x86_64)
##			msvcwwi  local Windows msvc release    (msvc, built on Windows, x86_64)
##		  plus one that does not follow the convention, because it can't:
##			dfsync   the fixed-name copy in the SYNCED dogfood dir. Whichever box
##			         ran its pipeline last put it there and the file doesn't say
##			         which, so the tag names the source instead of the build. This
##			         is what keeps a launch current when b23 is off and nothing was
##			         built here - Dropbox carries it. It only copies when it is
##			         newer than EVERY copy held AND its bytes differ from every copy
##			         held, so re-taking a build we already have under its real tag
##			         can't happen.
##		  Copies are named 'slktrmdf_<YYYYMMDD-HHMMSS>_<tag>.exe' where the stamp is
##		  the build's own mtime, so a running copy never blocks the copy. Copies of
##		  one build don't reliably agree on that mtime, so what actually keeps a
##		  build to one copy is the byte comparison, not the stamp.
##		- Each run, in order: delete idle builds over 7 days old; refresh each source
##		  whose build is newer than what we already hold; then pick one to run.
##		- A source reached over the network gets a hard wait bound at every step, so
##		  an off host or a link that drops mid-copy costs seconds rather than the
##		  redirector's own timeout. A copy lands on a temp name and is renamed into
##		  place, so one we abandon can't leave a half-written build behind.
##		- Which to run: the newest build by stamp. If that newest came from b23
##		  (gnulwi) or the synced dogfood dir (dfsync), run it. Otherwise it's a local
##		  Windows build - if the newest gnuwwi and msvcwwi are within 15 min of each
##		  other, flip a coin between them, else run the newest outright.
##		- Prepends a build-tagged title so a dogfood window is visually distinct. It
##		  precedes the passed args, so a caller can still override it. (Picking a
##		  wallpaper here is disabled - the terminal rotates its own.)
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

##	Copyright © 2026 Bubbles (ID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Configuration

## Source 'gnulwi': the b23 SilkTerm Windows (x86_64-pc-windows-gnu) release build,
## reached over SMB.
$B23ReleaseDir = "\\b23\zfs\zf10\0-0\users\collierjr\data\prs\dev\github.com\jim-collier\silkterm\github\target\x86_64-pc-windows-gnu\release"

## Sources 'gnuwwi'/'msvcwwi': the local Windows-native release build dirs (same
## clone, two target triples). The clone root differs per host, so try the known
## candidates and take the first that exists; if none do, keep the first so the
## per-source copy below warn-skips it like any other unreachable source.
$LocalTargetRootCandidates = @(
	"C:\0-0\users\collierjr\data\prs\dev\github.com\jim-collier\silkterm\github\target"
	"C:\opt\0-0\users\collierjr\data\prs\dev\github\jim-collier\silkterm\github\target"
	"C:\opt\0-0\users\collierjr\data\prs\dev\github.com\jim-collier\silkterm\github\target"
)
$LocalTargetRoot = $LocalTargetRootCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $LocalTargetRoot) { $LocalTargetRoot = $LocalTargetRootCandidates[0] }
$LocalGnuReleaseDir  = Join-Path $LocalTargetRoot "x86_64-pc-windows-gnu\release"
$LocalMsvcReleaseDir = Join-Path $LocalTargetRoot "x86_64-pc-windows-msvc\release"

## Source 'dfsync': the fixed-name dogfood copy in the SYNCED util dir - the same
## dir cicd-win.ps1 installs to, and where the Linux pipeline's Windows cross-build
## arrives over Dropbox. A local path, so no bounded wait applies to it.
$SyncedDogfoodDir = "C:\opt\0-0\common\exec\synced\util\mswin\gui\by-self\win64"

## The tag each source's copies carry, spelled once so the copy, the selection and
## the window title can't drift apart. Same convention the Linux pipeline uses:
## '<toolchain: gnu|msvc><built on: l|m|b|w><target: l|m|b|w><arch: i|a>'.
$TagB23       = "gnulwi"
$TagLocalGnu  = "gnuwwi"
$TagLocalMsvc = "msvcwwi"
$TagSynced    = "dfsync"    ## names its source, not the build - see the header

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

## How long to wait on a source reached over the network before giving up on it.
## Measured here against a host that resolves but doesn't answer: a single stat of
## the UNC path sits for 21s on TCP retries alone, and the copy that follows has no
## bound at all - which a shortcut click reads as a hang. So each remote step gets
## its own wall-clock limit, all of them under the redirector's: a TCP probe of the
## host first (a dead host is the common case, and it settles in one round trip),
## then a bounded stat, then a bounded copy. The copy gets the most - it moves the
## whole binary, and a slow link is not the same thing as a dead one.
$NetProbeTimeoutMs = 2000
$NetStatTimeoutSec = 5
$NetCopyTimeoutSec = 20

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
	fCopyIfNewer -SourceDir $B23ReleaseDir       -Tag $TagB23
	fCopyIfNewer -SourceDir $LocalGnuReleaseDir  -Tag $TagLocalGnu
	fCopyIfNewer -SourceDir $LocalMsvcReleaseDir -Tag $TagLocalMsvc
	fCopyIfNewer -SourceDir $SyncedDogfoodDir    -Tag $TagSynced -BeatsEveryTag

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

	## Always keep the newest, however old it is. Age alone emptied the dir after a
	## quiet week, and with no source answering that left nothing to launch.
	$keep = (fTaggedBuilds | Sort-Object Stamp -Descending | Select-Object -First 1).File.FullName

	Get-ChildItem -LiteralPath $TargetDir -File -Filter "${DogfoodPrefix}_*.exe" -ErrorAction SilentlyContinue |
		Where-Object { $_.Name -match $rx } |
		Where-Object { $_.FullName -ne $keep } |
		Where-Object { (fBuildTime $_) -lt $cutoff } |
		ForEach-Object {
			if (fRemoveIfIdle -FileInfo $_ -Running $running) { $deleted++ }
		}

	if ($deleted) { fNote "deleted $deleted build(s) older than $MaxAgeDays days" }

	## Leftovers from a copy that was abandoned or interrupted (see fCopyIfNewer).
	## Best-effort: one still held open by a dying copy just waits for the next run.
	Get-ChildItem -LiteralPath $TargetDir -File -Filter "${DogfoodPrefix}_*.exe.partial" -ErrorAction SilentlyContinue |
		ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue }
}


## Copy $SourceDir\$ExeName in as 'slktrmdf_<stamp>_<Tag>.exe' when its build is
## newer than the newest copy of that tag we already hold. No-op if the source is
## unreachable or we're already current. Each tag is checked independently, unless
## -BeatsEveryTag: then it has to beat the newest copy of ANY tag, which is what
## keeps a source that re-serves someone else's build (dfsync) from taking a second
## copy of one we already hold under its own tag.
function fCopyIfNewer {
	param(
		[Parameter(Mandatory)][string]$SourceDir,
		[Parameter(Mandatory)][string]$Tag,
		[switch]$BeatsEveryTag
	)

	$src    = Join-Path $SourceDir $ExeName
	$remote = fIsUncPath $src

	## A host that's simply off is the usual reason a launch stalls, so settle that
	## first - the probe answers in one round trip where the redirector would sit
	## through its own retries.
	if ($remote -and -not (fUncHostReachable $src)) {
		fWarn "$Tag source host not answering: $src"
		return
	}

	$stat = fRunBounded -Remote:$remote -TimeoutSec $NetStatTimeoutSec -Arguments @($src) -Script {
		param($SrcPath)
		$item = Get-Item -LiteralPath $SrcPath -ErrorAction SilentlyContinue
		if ($item) { $item.LastWriteTime } else { $null }
	}
	if (-not $stat.Done) {
		fWarn "$Tag source stopped answering; gave up after $NetStatTimeoutSec s: $src"
		return
	}

	$mtime = $stat.Value | Select-Object -First 1
	if (-not $mtime) {
		fWarn "$Tag source not reachable: $src"
		return
	}

	$stamp     = ([datetime]$mtime).ToString($StampFormat)
	$stampTime = fParseStamp $stamp
	$existing  = if ($BeatsEveryTag) {
		fTaggedBuilds | Sort-Object Stamp -Descending | Select-Object -First 1
	} else {
		fNewestOfTag $Tag
	}

	if ($existing -and $existing.Stamp -ge $stampTime) {
		fNote "$Tag already current (held $($existing.Stamp.ToString($StampFormat)), src $stamp)"
		return
	}

	$dst = Join-Path $TargetDir "${DogfoodPrefix}_${stamp}_${Tag}.exe"
	if (Test-Path -LiteralPath $dst) {
		fNote "$Tag copy already present: $(Split-Path $dst -Leaf)"
		return
	}

	## Copies of one build do not agree on mtime - cicd dates the pool copy and the
	## synced copy separately, and Dropbox restamps what it syncs - so a build we
	## already hold keeps looking new and keeps getting copied in again. Settle it on
	## the bytes. The held copy keeps its own tag (it says what the build IS, which a
	## dfsync name cannot) and only takes the newer stamp, so the cheap test above
	## answers it next run without reading the whole binary.
	$twin = fHeldMatching -SrcPath $src -Remote:$remote
	if ($twin) {
		$restamped = Join-Path $TargetDir "${DogfoodPrefix}_${stamp}_$($twin.Tag).exe"
		if (Test-Path -LiteralPath $restamped) {
			fNote "$Tag same build as $($twin.Name); already held under that stamp"
			return
		}
		try {
			Move-Item -LiteralPath $twin.File.FullName -Destination $restamped -ErrorAction Stop
			fNote "$Tag same build as $($twin.Name) - restamped to $stamp"
		} catch {
			## A running image can refuse the rename; next run just tries again.
			fNote "$Tag same build as $($twin.Name), but the rename was refused"
		}
		return
	}

	## Copy to a temp name and rename it into place. A copy we abandon mid-transfer
	## (or one a dropped link kills) otherwise leaves a half-written .exe that later
	## reads as a perfectly good build and gets launched. '.partial' matches neither
	## the selection nor the age-prune name spec, so a leftover is inert either way;
	## fDeleteOldBuilds sweeps them.
	$tmp = "$dst.partial"
	Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue

	$copy = fRunBounded -Remote:$remote -TimeoutSec $NetCopyTimeoutSec -Arguments @($src, $tmp) -Script {
		param($SrcPath, $TmpPath)
		Copy-Item -LiteralPath $SrcPath -Destination $TmpPath -Force -ErrorAction Stop
	}

	if (-not $copy.Done) {
		## The abandoned copy may still hold the temp file open, so this delete is
		## best-effort - the next run's sweep gets what's left.
		Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
		fWarn -Gui "gave up copying $Tag build after $NetCopyTimeoutSec s (source stopped answering)"
		return
	}
	if ($copy.Error) {
		Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
		fWarn -Gui "couldn't copy $Tag build ($($copy.Error))"
		return
	}

	try {
		Move-Item -LiteralPath $tmp -Destination $dst -Force -ErrorAction Stop
		fNote "copied $Tag -> $(Split-Path $dst -Leaf)"
	} catch {
		Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
		fWarn -Gui "couldn't place $Tag build ($($_.Exception.Message))"
	}
}


## Pick the copy to run. Newest by stamp wins; if that newest is a local Windows
## build and the newest of each toolchain is within $CoinWindowMin of the other,
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

	## Neither of these has a sibling to weigh it against, so newest just wins.
	if ($latest.Tag -eq $TagB23 -or $latest.Tag -eq $TagSynced) {
		fNote "running newest ($($latest.Tag)): $($latest.Name)"
		return $latest.File.FullName
	}

	## Newest is a local Windows build - maybe coin-flip gnu vs msvc.
	$gnu  = $builds | Where-Object { $_.Tag -eq $TagLocalGnu }  | Sort-Object Stamp -Descending | Select-Object -First 1
	$msvc = $builds | Where-Object { $_.Tag -eq $TagLocalMsvc } | Sort-Object Stamp -Descending | Select-Object -First 1

	if ($gnu -and $msvc) {
		$gapMin = [math]::Abs(($gnu.Stamp - $msvc.Stamp).TotalMinutes)
		if ($gapMin -le $CoinWindowMin) {
			$pick = if ((Get-Random -Minimum 0 -Maximum 2) -eq 0) { $gnu } else { $msvc }
			fNote ("coin flip ($TagLocalGnu/$TagLocalMsvc within {0:N1} min) -> {1}: {2}" -f $gapMin, $pick.Tag, $pick.Name)
			return $pick.File.FullName
		}
	}

	fNote "running newest local ($($latest.Tag)): $($latest.Name)"
	return $latest.File.FullName
}


## All tagged copies as objects { File, Name, Tag, Stamp(DateTime) }. Only our own
## tags match, so a copy for some other target can never be selected to run here;
## adding a source means adding its tag above, nothing else.
function fTaggedBuilds {
	$known = ($TagB23, $TagLocalGnu, $TagLocalMsvc, $TagSynced | ForEach-Object { [regex]::Escape($_) }) -join "|"
	$rx    = "^$([regex]::Escape($DogfoodPrefix))_(?<stamp>\d{8}-\d{6})_(?<tag>$known)\.exe$"
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


## The held copy holding byte-for-byte the same build as $SrcPath, or $null. Size
## is the cheap discriminator - two builds almost never match on it - so the hash
## only runs when one does, and the read is bounded like every other remote step.
function fHeldMatching {
	param(
		[Parameter(Mandatory)][string]$SrcPath,
		[switch]$Remote
	)

	$stat = fRunBounded -Remote:$Remote -TimeoutSec $NetStatTimeoutSec -Arguments @($SrcPath) -Script {
		param($P)
		$i = Get-Item -LiteralPath $P -ErrorAction SilentlyContinue
		if ($i) { $i.Length } else { $null }
	}
	if (-not $stat.Done) { return $null }
	$srcSize = $stat.Value | Select-Object -First 1
	if (-not $srcSize) { return $null }

	$sameSize = @(fTaggedBuilds | Where-Object { $_.File.Length -eq $srcSize })
	if (-not $sameSize) { return $null }

	## Reading the source costs about what copying it would, and is bounded the same
	## way, so a link that dies here is no worse than one that dies mid-copy.
	$hash = fRunBounded -Remote:$Remote -TimeoutSec $NetCopyTimeoutSec -Arguments @($SrcPath) -Script {
		param($P)
		(Get-FileHash -LiteralPath $P -Algorithm SHA256 -ErrorAction SilentlyContinue).Hash
	}
	if (-not $hash.Done) { return $null }
	$srcHash = $hash.Value | Select-Object -First 1
	if (-not $srcHash) { return $null }

	foreach ($cand in $sameSize) {
		$h = (Get-FileHash -LiteralPath $cand.File.FullName -Algorithm SHA256 -ErrorAction SilentlyContinue).Hash
		if ($h -eq $srcHash) { return $cand }
	}
	return $null
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


## Is this a UNC path (\\host\share\...)? Only those get the bounded treatment
## below; a local path that stalls is a broken disk, not a network we can outwait.
## A mapped drive letter reads as local here - map by UNC to keep the bound.
function fIsUncPath {
	param([Parameter(Mandatory)][string]$Path)
	return ($Path -like "\\*")
}


## Is a UNC path's host answering on SMB (port 445) within $NetProbeTimeoutMs? A
## TCP connect settles a dead host in one round trip, where the redirector retries
## for ~21s first. Cached per host, so three sources on one host cost one probe.
## Anything unexpected reads as reachable - the bounded calls are the real backstop
## and a probe must never be the reason a live source is skipped.
function fUncHostReachable {
	param([Parameter(Mandatory)][string]$Path)

	if ($Path -notmatch '^\\\\(?<host>[^\\]+)\\') { return $true }
	$hostName = $Matches.host
	if ($script:NetProbed.ContainsKey($hostName)) { return $script:NetProbed[$hostName] }

	$reachable = $true
	try {
		$client = New-Object System.Net.Sockets.TcpClient
		try {
			$pending   = $client.BeginConnect($hostName, 445, $null, $null)
			$reachable = $pending.AsyncWaitHandle.WaitOne($NetProbeTimeoutMs)
			if ($reachable) {
				## Connected, refused, or no such name - only the first is reachable.
				try { $client.EndConnect($pending) } catch { $reachable = $false }
			}
		} finally { $client.Close() }
	} catch { $reachable = $true }

	$script:NetProbed[$hostName] = $reachable
	return $reachable
}


## Run a scriptblock under a wall-clock limit and report what happened, as
## @{ Done; Value; Error }. Done=$false means it was still going when the limit
## hit and has been abandoned - a wedged SMB call can't be interrupted, so we stop
## the runspace asynchronously and leave the thread to die on its own rather than
## block on the very thing we're timing out. -Remote is what asks for any of this;
## without it the block just runs inline (a runspace per local source is pure cost).
function fRunBounded {
	param(
		[Parameter(Mandatory)][scriptblock]$Script,
		[Parameter(Mandatory)][int]$TimeoutSec,
		[object[]]$Arguments = @(),
		[switch]$Remote
	)

	if (-not $Remote) {
		try   { return @{ Done = $true; Value = @(& $Script @Arguments); Error = $null } }
		catch { return @{ Done = $true; Value = @(); Error = $_.Exception.Message } }
	}

	$shell = [powershell]::Create()
	[void]$shell.AddScript($Script)
	foreach ($argument in $Arguments) { [void]$shell.AddArgument($argument) }

	$pending = $shell.BeginInvoke()
	if (-not $pending.AsyncWaitHandle.WaitOne([timespan]::FromSeconds($TimeoutSec))) {
		try { [void]$shell.BeginStop($null, $null) } catch { }
		return @{ Done = $false; Value = @(); Error = "gave up after $TimeoutSec s" }
	}

	try {
		return @{ Done = $true; Value = @($shell.EndInvoke($pending)); Error = $null }
	} catch {
		## EndInvoke rethrows the block's own error wrapped in its own; report the
		## innermost one, or a copy failure reads as a PowerShell plumbing failure.
		$reason = $_.Exception
		while ($reason.InnerException) { $reason = $reason.InnerException }
		return @{ Done = $true; Value = @(); Error = $reason.Message }
	} finally {
		$shell.Dispose()
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
## title tagged with the build's tag+stamp. Passed args come last so they win.
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

	## Picking a wallpaper here is disabled: the terminal rotates its own now, and a
	## wallpaper named on the command line pins it for the session - which would hide
	## exactly what we want to see. Uncomment (with fPickRandomWallpaper below) to go
	## back to choosing one here.
	$preArgs = @()
	#$wp = fPickRandomWallpaper
	#if ($wp) { $preArgs += "--wallpaper-file=$wp" }
	$preArgs += "--title=$title"

	$all = @($preArgs)
	if ($PassArgs) { $all += $PassArgs }

	## Start-Process joins -ArgumentList with spaces WITHOUT quoting, so an arg
	## whose value has a space (the title, or a path under a spaced folder) would
	## be split into separate argv entries by the target and rejected. Quote any
	## such arg ourselves.
	$quoted = @($all | ForEach-Object { fQuoteArg $_ })

	return fStartTerminal -Exe $Exe -ArgList $quoted
}


## Fall back to whatever terminal is on PATH, in $FallbackTerminals order. Our own
## silkterm keeps the tagged title (via fLaunchSilkTerm); generic terminals are
## launched plainly - silkterm's --title flag doesn't apply and its pass-through
## args likely don't either, so they get none. cmd.exe lives in System32 (always
## on PATH), so this effectively always finds something.
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


## Resolve SilkTerm's wallpaper dir the same way the app does: XDG_CONFIG_HOME,
## else HOME\.config, else APPDATA - then \silkterm\wallpaper. Unused while the
## pick in fLaunchSilkTerm is commented out; kept so re-enabling stays one line.
function fResolveWallpaperDir {
	$base = $null
	if ($env:XDG_CONFIG_HOME) { $base = $env:XDG_CONFIG_HOME }
	elseif ($env:HOME)        { $base = Join-Path $env:HOME ".config" }
	elseif ($env:APPDATA)     { $base = $env:APPDATA }
	if (-not $base) { return $null }
	return (Join-Path $base "silkterm\wallpaper")
}


## Pick a random image from the wallpaper dir, or $null if there are none. Unused;
## see fResolveWallpaperDir.
function fPickRandomWallpaper {
	$dir = fResolveWallpaperDir
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

## Per-host result of the SMB reachability probe, so each host is probed once a run.
$script:NetProbed = @{}

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
##		- 2026-08-24: Tell builds apart by their bytes, not their mtime - copies of
##		  one build disagreed on it, so the same binary kept getting copied in again
##		  under a second tag. A match keeps its own tag and takes the newer stamp.
##		  Never prune the newest copy, whatever its age.
##		- 2026-08-23: Added the synced dogfood dir as a fourth source ('dfsync'), so
##		  a build made on another box still reaches this one when b23 is off.
##		- 2026-08-06: Bound the wait on a network source (probe, stat, copy) instead
##		  of sitting through the SMB timeout when b23 is off or the link drops. Copy
##		  via a temp name so an abandoned one leaves no half-written build.
##		- 2026-08-02: Stop picking a wallpaper; the terminal rotates its own.
##		- 2026-08-01: Retag copies '<toolchain><built on><target><arch>', so a tag
##		  says what the binary IS: gnul -> gnulwi, gnuw -> gnuwwi, msvc -> msvcwwi.
##		  Each source re-copies once under its new name; old ones age out.
##		- 2026-07-22: Resolve the local clone root from a per-host candidate
##		  list (was hardcoded to one host's path, so gnuw/msvc never copied on
##		  the others).
##		- 2026-07-19: '--admin' now self-elevates the whole launcher (was only the
##		  launched terminal), so a non-elevated shortcut click copies the fresh build
##		  instead of silently launching a stale one. Report failures / skipped copies
##		  in a dialog for the shortcut case (console flashes shut); new '--gui' flag,
##		  auto-on when double-clicked.
##		- 2026-07-17: Strip a synced-on mark-of-the-web at startup so a later
##		  click under RemoteSigned isn't silently blocked.
##		- 2026-07-17: Log every run's per-source copy decision (and each note/
##		  warn) to n8runterm.log in the target dir, trimmed at 256KB.
##		- 2026-07-16: Age-prune stamped copies with any tag, not just the known
##		  three (one-off tags could never be deleted); selection still known-tags-only.
##		- 2026-07-15: Elevate only on '--admin' (consumed, not forwarded); default
##		  is the normal token.
##		- 2026-07-15: Launch elevated by default; fall back to silkterm on PATH /
##		  Windows Terminal / PyCmd / cmd.exe when no build or source is available.
##		- 2026-07-15: Target the local (non-synced) util dir, not the Dropbox one.
##		- 2026-07-15: Prune only files matching our own name spec (leave foreign
##		  files like cicd-win.ps1's fixed SilkTerm.exe alone).
##		- 2026-07-15: Reorder copy name to stamp-then-tag (slktrmdf_<stamp>_<tag>).
##		- 2026-07-15: Three tagged sources (gnul/gnuw/msvc); age-based delete;
##		  newest-by-stamp run with a gnuw/msvc coin flip when close in time.
##		- 2026-07-14: Return the launched Process so callers can target it by PID.
##		- 2026-07-14: Quote args with spaces (title/bg path) so they aren't split.
##		- 2026-07-14: Rotating stamped copies + prune idle ones (was fixed-name).
##		- 2026-07-14: Created (Windows port of the bash n8runterm).
