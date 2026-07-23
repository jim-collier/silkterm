##	Purpose:
##		- Windows-native CI/CD pipeline for SilkTerm. A PowerShell port of the
##		  Linux cicd.bash, doing as much of the same work as Windows allows -
##		  including the parts cicd.bash farms out to helper scripts (the git
##		  backup/publish). Does NOT touch cicd.bash (that stays the
##		  Linux/cross pipeline).
##		- Stages (fail-fast; any error aborts before the next stage):
##		   0. remote sync    (fetch; fast-forward if safely behind; abort if diverged)
##		   1. format         (cargo fmt)
##		   2. debug build    (cargo build)
##		   3. tests + lints  (cargo test; clippy + cargo-deny are ADVISORY here)
##		   4. release builds  x86_64 msvc AND gnu (always both), + ARM64 when its
##		                      toolchain is present (auto-detected, else warn-skip)
##		   5. packages       (NSIS installer .exe per built arch, if makensis found)
##		   6. dogfood        (copy the best x86_64 build to <dogfood>\SilkTerm.exe)
##		   7. publish        (stash -> pull -> add -> commit -> push, current branch)
##		- What Windows can't do (dropped vs cicd.bash): the profiler (pprof's
##		  SIGPROF sampler is Unix-only - the profiling feature can't even compile
##		  for a Windows target), the headless scroll harness / screenshots / demo
##		  (need Xvfb), .deb/.rpm packages (Linux), and the rar version-archive step
##		  of publish (skipped by request). clippy is advisory, not gating: the
##		  Unix-gated ctl code emits dead_code warnings here, so -D warnings can't
##		  pass.
##		- Dogfood pick: prefer the msvc build IF it's self-contained (statically
##		  linked, no VCRUNTIME140/MSVCP140 dependency); else the gnu build; else
##		  whichever single build exists. The fixed SilkTerm.exe goes to the SYNCED
##		  util dir; n8runterm.ps1 manages its own stamped pool in its LOCAL dir
##		  (the two stay separate dirs on purpose).
##		- Syntax:
##		  pwsh cicd/cicd-win.ps1 [options]
##		  Options:
##		   -Yes            run unattended (no confirm / message prompt)
##		   -Quiet          quiet + unattended (implies -Yes); publish runs quiet too
##		   -Quick          skip the slow stages (ARM builds + packages)
##		   -Gate           merge gate only: fmt --check + clippy + tests, then exit
##		   -NoFmt          skip the formatter stage
##		   -NoArm          skip the ARM64 release builds + their packages
##		   -NoPackage      skip the packages stage (NSIS installers)
##		   -NoDogfood      skip the dogfood install
##		   -NoPublish      skip the git publish stage
##		   -NoSync         skip the remote sync check (stage 0)
##		   -Message MSG    publish hands-off with this commit message (no editor)
##		   -Help           show this help
##	History: At bottom of script.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT

[CmdletBinding()]
param(
	[switch]$Yes,
	[switch]$Quiet,
	[switch]$Quick,
	[switch]$Gate,
	[switch]$NoFmt,
	[switch]$NoArm,
	[switch]$NoPackage,
	[switch]$NoDogfood,
	[switch]$NoPublish,
	[switch]$NoSync,
	[string]$Message = "",
	[switch]$Help
)

## Requires PowerShell 7+ (pwsh): this script uses $IsWindows and PS7 semantics.
## Windows PowerShell 5.1 has no $IsWindows, so the StrictMode guard below would
## throw a cryptic error instead. Bail early with a clear pointer.
if ($PSVersionTable.PSVersion.Major -lt 6) {
	Write-Error "cicd-win.ps1 needs PowerShell 7+ (pwsh); you're on Windows PowerShell $($PSVersionTable.PSVersion). Run: pwsh -File cicd/cicd-win.ps1"
	exit 1
}

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
## We drive native tools (git, cargo) by hand and read $LASTEXITCODE - several
## probes (git diff --quiet, clippy) return non-zero ON PURPOSE. Keep a non-zero
## native exit from throwing so those reads work regardless of the caller's shell.
$PSNativeCommandUseErrorActionPreference = $false

if ($Help) {
	## Print only the leading Purpose..History header block (mirrors cicd.bash's
	## `sed -n '/Purpose:/,/History:/p'`), not every top-level ## comment.
	$inBlock = $false
	foreach ($line in (Get-Content -LiteralPath $PSCommandPath)) {
		if ($line -match '^##\tPurpose:') { $inBlock = $true }
		if ($inBlock) {
			if ($line -match '^##\tHistory:') { break }
			$line -replace '^##\t?', ''
		}
	}
	exit 0
}

## Windows-only: this pipeline shells out to makensis, reads PE imports, and
## writes into a Windows dogfood dir. Refuse to run anywhere else.
if (-not $IsWindows) {
	Write-Error "cicd-win.ps1: this pipeline only runs on Windows (use cicd/cicd.bash on Linux)."
	exit 1
}

## -Quiet implies unattended; both suppress the preflight prompt.
$Unattended = ($Yes -or $Quiet)


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Configuration

## Repo root = the parent of this script's cicd/ dir. All cargo commands run here.
$Root    = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$AppName = "SilkTerm"
$ExeName = "silkterm"

## The single version source (first `version = "..."` line).
$VersionManifest = Join-Path $Root "source\Cargo.toml"

## Release build matrix. x86_64 msvc + gnu build every run (owner rule: always
## build both). ARM64 rows are attempted only when their toolchain is detected
## (see fArmSkipReason); otherwise they warn-skip. os-arch feeds the artifact
## name (<exe>-<version>-<os-arch>.exe).
$Targets = @(
	[pscustomobject]@{ Arch="x86_64"; Tk="msvc";    Triple="x86_64-pc-windows-msvc";     OsArch="windows-x86_64-msvc";    Builder="build";    Arm=$false }
	[pscustomobject]@{ Arch="x86_64"; Tk="gnu";     Triple="x86_64-pc-windows-gnu";      OsArch="windows-x86_64-gnu";     Builder="build";    Arm=$false }
	[pscustomobject]@{ Arch="arm64";  Tk="msvc";    Triple="aarch64-pc-windows-msvc";    OsArch="windows-arm64-msvc";     Builder="build";    Arm=$true  }
	[pscustomobject]@{ Arch="arm64";  Tk="gnullvm"; Triple="aarch64-pc-windows-gnullvm"; OsArch="windows-arm64-gnullvm";  Builder="zigbuild"; Arm=$true  }
)

## Collected release binaries + checksums land here (its own dir so the Linux
## pipeline's cicd/artifacts/release wipe can't nuke Windows artifacts, or v.v.).
$ReleaseArtifactDir = Join-Path $Root "cicd\artifacts\release-win"

## NSIS installer template (shared with the Linux pipeline).
$NsisTemplate = Join-Path $Root "cicd\packaging\windows\installer.nsi.in"

## Full-run transcript (gitignored, alongside the Linux lint logs' sibling).
$LogDir = Join-Path $Root "cicd\artifacts\lint-win"

## Dogfood: the fixed-name copy for hand-launching, into the SYNCED util dir so it
## rides Dropbox and any box can grab it. Deliberately a SEPARATE dir from
## n8runterm.ps1's LOCAL dir - n8runterm manages its own machine-local stamped
## slktrmdf_* pool and launches from there; the two never share a folder.
$DogfoodDir      = "C:\opt\0-0\common\exec\synced\util\mswin\gui\by-self\win64"
$DogfoodFixedExe = "SilkTerm.exe"

## Pinned helper-tool versions (the Windows-relevant subset of config.bash's
## TOOL_PINS). Warn (non-gating) when an installed tool has drifted, so a box
## update can't silently change results. "name|version|command args...".
$ToolPins = @(
	"cargo-zigbuild|0.23.0|cargo-zigbuild --version"
	"cargo-deny|0.19.9|cargo-deny --version"
	"makensis|3.12|MAKENSIS"          # special-cased: resolve via fFindMakensis
)

## Cap compile/test parallelism to half the cores so a run stays usable.
$Cores       = [Environment]::ProcessorCount
$CicdMaxJobs = [Math]::Max(1, [Math]::Floor($Cores / 2))

## Where the toolchains live. cargo/rustup first, then the mingw linker for the
## gnu target (matches the memory'd build setup).
$CargoBin  = Join-Path $env:USERPROFILE ".cargo\bin"
$MingwBin  = "C:\ProgramData\mingw64\mingw64\bin"


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Output helpers (mirror cicd.bash: fEcho / fEcho_Clean / fSection)

$script:WasLastEchoBlank = $false
$script:Letterbox = "•" * 73

function fEcho_Clean {
	param([string]$Msg = "")
	if ($Msg) { Write-Host $Msg; $script:WasLastEchoBlank = $false }
	elseif (-not $script:WasLastEchoBlank) { Write-Host ""; $script:WasLastEchoBlank = $true }
}
function fEcho     { param([string]$Msg = ""); if ($Msg) { fEcho_Clean "[ $Msg ]" } else { fEcho_Clean } }
function fSection  { param([string]$Msg);      fEcho_Clean; fEcho_Clean $script:Letterbox; fEcho $Msg }
function fNote     { param([string]$Msg); fEcho_Clean $Msg }
function fWarn     { param([string]$Msg); fEcho "WARNING: $Msg" }
function fDie      { param([string]$Msg); fEcho "FAILED: $Msg"; exit 1 }


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Functions

## Run a native command from the repo root; abort (fail-fast) on a non-zero exit.
function fExec {
	param(
		[Parameter(Mandatory)][string]$What,
		[Parameter(Mandatory)][string]$File,
		[string[]]$CmdArgs = @()
	)
	& $File @CmdArgs
	if ($LASTEXITCODE -ne 0) { fDie "$What failed (exit $LASTEXITCODE): $File $($CmdArgs -join ' ')" }
}

## First `version = "x"` from the manifest.
function fVersion {
	$line = Select-String -LiteralPath $VersionManifest -Pattern '^\s*version\s*=\s*"([^"]+)"' |
		Select-Object -First 1
	if (-not $line) { fDie "no version found in $VersionManifest" }
	return $line.Matches[0].Groups[1].Value
}

## True if the exe is self-contained: no dynamic dependency on the VC runtime
## (VCRUNTIME140 / MSVCP140). Scans the PE for those import names - present only
## when msvc links the CRT dynamically (i.e. without +crt-static). The gnu build
## is always static (see .cargo/config.toml), so it reads standalone too.
function fExeIsStandalone {
	param([Parameter(Mandatory)][string]$Path)
	$bytes = [System.IO.File]::ReadAllBytes($Path)
	$ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
	foreach ($dep in @("VCRUNTIME140", "MSVCP140")) {
		if ($ascii -match [regex]::Escape($dep)) { return $false }
	}
	return $true
}

## Locate makensis (not on PATH by default). $null if NSIS isn't installed.
function fFindMakensis {
	$cmd = Get-Command makensis -ErrorAction SilentlyContinue
	if ($cmd) { return $cmd.Source }
	foreach ($p in @(
		"C:\Program Files (x86)\NSIS\makensis.exe",
		"C:\Program Files\NSIS\makensis.exe",
		"C:\ProgramData\chocolatey\bin\makensis.exe")) {
		if (Test-Path -LiteralPath $p) { return $p }
	}
	return $null
}

## Warn (non-gating) when a pinned helper tool is missing or has drifted from its
## pin. Mirrors cicd.bash's TOOL_PINS loop. makensis is special-cased because it
## isn't on PATH by default (resolved via fFindMakensis).
function fCheckToolPins {
	foreach ($pin in $ToolPins) {
		$parts   = $pin -split '\|', 3
		$name    = $parts[0]; $want = $parts[1]; $cmd = $parts[2]
		$found   = $false; $verLine = $null
		try {
			if ($cmd -eq "MAKENSIS") {
				$mk = fFindMakensis
				if ($mk) { $found = $true; $out = & $mk -VERSION 2>$null; $verLine = $out | Select-Object -First 1 }
			} else {
				$exe  = ($cmd -split '\s+')[0]
				$rest = @(($cmd -split '\s+') | Select-Object -Skip 1)
				if (Get-Command $exe -ErrorAction SilentlyContinue) {
					## Collect the whole output FIRST, then take the first line. Piping a
					## native command straight into `Select-Object -First 1` races: the
					## early upstream-stop can kill the tool mid-print (exit 101) and drop
					## its version line, which read as a false "not found".
					$found = $true; $out = & $exe @rest 2>$null; $verLine = $out | Select-Object -First 1
				}
			}
		} catch { $verLine = $null }
		if (-not $found)   { fWarn "$name not found (pinned $want)"; continue }
		if (-not $verLine) { fWarn "$name present but version unreadable (pinned $want)"; continue }
		$m = [regex]::Match($verLine, '[0-9]+(\.[0-9]+)+')
		$have = if ($m.Success) { $m.Value } else { "$verLine".Trim() }
		if ($have -ne $want) { fWarn "$name is $have, pinned $want (update the pin or the tool)" }
	}
}

## True if a rustup target is installed.
function fTargetInstalled {
	param([Parameter(Mandatory)][string]$Triple)
	return ((& rustup target list --installed) -contains $Triple)
}

## True if the ARM64 msvc libraries are present (the VS "MSVC ARM64 build tools"
## component installs a lib\arm64 under the MSVC tools dir).
function fHasArm64VcLibs {
	$base = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC"
	if (-not (Test-Path -LiteralPath $base)) { return $false }
	return [bool](Get-ChildItem -LiteralPath $base -Directory -ErrorAction SilentlyContinue |
		ForEach-Object { Test-Path -LiteralPath (Join-Path $_.FullName "lib\arm64") } |
		Where-Object { $_ })
}

## Decide whether an ARM64 target can build here; returns a reason string when it
## can't (for a clear warn-skip), or $null when it's good to go.
function fArmSkipReason {
	param([Parameter(Mandatory)]$Target)
	if (-not (fTargetInstalled $Target.Triple)) { return "rustup target $($Target.Triple) not installed" }
	if ($Target.Builder -eq "zigbuild") {
		if (-not (Get-Command cargo-zigbuild -ErrorAction SilentlyContinue)) { return "cargo-zigbuild not found" }
		if (-not (Get-Command zig -ErrorAction SilentlyContinue))            { return "zig not found" }
	}
	if ($Target.Builder -eq "build" -and $Target.Arch -eq "arm64") {
		if (-not (fHasArm64VcLibs)) { return "VS MSVC ARM64 build tools not installed" }
	}
	return $null
}

## Build one release target. Returns a result object on success, or $null when an
## ARM target is skipped (x86_64 failures abort - owner rule: always build both).
function fBuildTarget {
	param([Parameter(Mandatory)]$Target)

	if ($Target.Arm) {
		if ($NoArm)  { fNote "skip $($Target.OsArch): -NoArm";  return $null }
		if ($Quick)  { fNote "skip $($Target.OsArch): -Quick";  return $null }
		$reason = fArmSkipReason $Target
		if ($reason) { fWarn "$($Target.OsArch) skipped: $reason"; return $null }
	}

	fSection "4  Release build: $($Target.OsArch)"
	$exe = Join-Path $Root "target\$($Target.Triple)\release\$ExeName.exe"

	$cargoArgs = if ($Target.Builder -eq "zigbuild") {
		@("zigbuild", "--release", "--target", $Target.Triple)
	} else {
		@("build", "--release", "--target", $Target.Triple)
	}

	if ($Target.Arm) {
		## Non-gating: an ARM toolchain hiccup warns and skips, never aborts.
		& cargo @cargoArgs
		if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $exe)) {
			fWarn "$($Target.OsArch) build failed (non-gating)"; return $null
		}
	} else {
		fExec "release build ($($Target.OsArch))" "cargo" $cargoArgs
		if (-not (Test-Path -LiteralPath $exe)) { fDie "missing artifact for $($Target.OsArch): $exe" }
	}

	$size = "{0:N1} MB" -f ((Get-Item -LiteralPath $exe).Length / 1MB)
	fEcho "OK: $($Target.OsArch): $exe ($size)"
	return [pscustomobject]@{ Arch=$Target.Arch; Tk=$Target.Tk; OsArch=$Target.OsArch; Exe=$exe }
}

## Copy the built binaries into the artifact dir under versioned names and write a
## sha256sums file over them (parallels cicd.bash's write_sums).
function fCollectArtifacts {
	param([Parameter(Mandatory)][array]$Built, [Parameter(Mandatory)][string]$Ver)
	if (Test-Path -LiteralPath $ReleaseArtifactDir) { Remove-Item -LiteralPath $ReleaseArtifactDir -Recurse -Force }
	New-Item -ItemType Directory -Path $ReleaseArtifactDir -Force | Out-Null
	foreach ($b in $Built) {
		Copy-Item -LiteralPath $b.Exe -Destination (Join-Path $ReleaseArtifactDir "$ExeName-$Ver-$($b.OsArch).exe") -Force
	}
	fWriteSums $Ver
	fEcho "OK: $($Built.Count) release artifact(s) -> $ReleaseArtifactDir"
}

## (Re)write the checksums file over every artifact in the dir except itself.
function fWriteSums {
	param([Parameter(Mandatory)][string]$Ver)
	$sumsName = "$ExeName-$Ver-sha256sums.txt"
	$sumsPath = Join-Path $ReleaseArtifactDir $sumsName
	$lines = Get-ChildItem -LiteralPath $ReleaseArtifactDir -File |
		Where-Object { $_.Name -ne $sumsName } |
		ForEach-Object { "{0}  {1}" -f (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name }
	if ($lines) { Set-Content -LiteralPath $sumsPath -Value $lines -Encoding ascii }
}

## Build a self-contained NSIS installer per built arch (upgrades in place). A
## missing makensis warns and skips, never aborts.
function fBuildPackages {
	param([Parameter(Mandatory)][array]$Built, [Parameter(Mandatory)][string]$Ver)
	$makensis = fFindMakensis
	if (-not $makensis) { fWarn "makensis not found; installers skipped"; return }
	if (-not (Test-Path -LiteralPath $NsisTemplate)) { fWarn "NSIS template missing; installers skipped"; return }

	$made = 0
	foreach ($b in $Built) {
		$out = Join-Path $ReleaseArtifactDir "$ExeName-$Ver-$($b.OsArch)-setup.exe"
		$nsi = [System.IO.Path]::GetTempFileName() + ".nsi"
		(Get-Content -Raw -LiteralPath $NsisTemplate).
			Replace("@VERSION@", $Ver).
			Replace("@ARCH@",    $b.OsArch).
			Replace("@SRCEXE@",  $b.Exe).
			Replace("@OUTFILE@", $out) | Set-Content -LiteralPath $nsi -Encoding utf8
		& $makensis -V2 $nsi | Out-Null
		$rc = $LASTEXITCODE
		Remove-Item -LiteralPath $nsi -Force -ErrorAction SilentlyContinue
		if ($rc -eq 0 -and (Test-Path -LiteralPath $out)) { fEcho "OK: installer ($($b.OsArch))"; $made++ }
		else { fWarn "NSIS installer failed ($($b.OsArch))" }
	}
	if ($made) { fWriteSums $Ver }
	fEcho "OK: $made installer(s) -> $ReleaseArtifactDir"
}

## Install the fixed-name dogfood copy. Prefer the standalone msvc build, else
## gnu, else whichever single x86_64 build exists (this box runs x64).
function fDogfood {
	param([Parameter(Mandatory)][array]$Built)
	$x64  = @($Built | Where-Object { $_.Arch -eq "x86_64" })
	$msvc = @($x64 | Where-Object { $_.Tk -eq "msvc" }) | Select-Object -First 1
	$gnu  = @($x64 | Where-Object { $_.Tk -eq "gnu"  }) | Select-Object -First 1

	$pick = $null; $why = ""
	if ($msvc -and $gnu) {
		if (fExeIsStandalone $msvc.Exe) { $pick = $msvc; $why = "msvc is standalone" }
		else                            { $pick = $gnu;  $why = "msvc has a VC-runtime dependency" }
	} elseif ($msvc) { $pick = $msvc; $why = "only msvc built" }
	elseif   ($gnu)  { $pick = $gnu;  $why = "only gnu built" }

	if (-not $pick) { fWarn "no x86_64 build to dogfood; skipping"; return }

	if (-not (Test-Path -LiteralPath $DogfoodDir)) {
		New-Item -ItemType Directory -Path $DogfoodDir -Force | Out-Null
	}
	$dst = Join-Path $DogfoodDir $DogfoodFixedExe
	Copy-Item -LiteralPath $pick.Exe -Destination $dst -Force
	fEcho "OK: dogfood ($($pick.Tk); $why) -> $dst"
}

## Stage 0: make sure the local branch can be safely refreshed from its upstream
## BEFORE spending the build - what stage 7 pushes should be what got built and
## tested here, not an untested post-build merge. Behind-only is safe (fast-
## forward, stash-wrapped for a dirty tree); diverged aborts now rather than at
## publish. Offline just warns - a local build shouldn't need the net.
function fRemoteSync {
	& git rev-parse --abbrev-ref '@{u}' 2>$null | Out-Null
	if ($LASTEXITCODE -ne 0) {
		$branch = (& git rev-parse --abbrev-ref HEAD).Trim()
		fNote "no upstream for ${branch}; nothing to sync"
		return
	}
	& git fetch --quiet 2>$null
	if ($LASTEXITCODE -ne 0) { fWarn "git fetch failed (offline?); continuing with the local tree"; return }
	$ahead  = [int](& git rev-list --count '@{u}..HEAD')
	$behind = [int](& git rev-list --count 'HEAD..@{u}')
	if ($behind -eq 0) {
		if ($ahead) { fEcho "OK: up to date with upstream ($ahead ahead)" }
		else        { fEcho "OK: up to date with upstream" }
		return
	}
	if ($ahead -gt 0) { fDie "diverged from upstream ($ahead ahead, $behind behind) - reconcile first, or rerun with -NoSync" }
	## Behind only: a fast-forward can't lose anything. Same stash dance as
	## fPublish so a dirty tree can't block the pull.
	& git diff --quiet;          $dirtyTracked = ($LASTEXITCODE -ne 0)
	& git diff --cached --quiet; $dirtyStaged  = ($LASTEXITCODE -ne 0)
	$untracked = (& git ls-files --others --exclude-standard)
	$didStash = $false
	if ($dirtyTracked -or $dirtyStaged -or $untracked) {
		$before = @(& git stash list).Count
		fEcho_Clean "git stash push --include-untracked ..."
		fExec "git stash" "git" @("stash", "push", "--include-untracked", "-m", "auto-stash")
		$after = @(& git stash list).Count
		$didStash = ($after -gt $before)
	}
	fEcho_Clean "git pull --ff-only ..."
	fExec "git pull" "git" @("pull", "--ff-only")
	if ($didStash) {
		fEcho_Clean "git stash pop ..."
		fExec "git stash pop" "git" @("stash", "pop")
	}
	fEcho "OK: fast-forwarded $behind commit(s) from upstream"
}

## Publish: a native port of n8git_backup-and-publish MINUS the rar archive step.
## stash (if dirty) -> pull --no-ff (if upstream) -> pop -> add -> commit -> push.
## $Msg empty means "let git open its editor" (git uses core.editor / EDITOR).
function fPublish {
	param([Parameter(Mandatory)][AllowEmptyString()][string]$Msg)
	$branch = (& git rev-parse --abbrev-ref HEAD).Trim()
	fNote "branch: $branch"

	## Stash local changes (tracked + untracked) before syncing with upstream.
	& git diff --quiet;        $dirtyTracked = ($LASTEXITCODE -ne 0)
	& git diff --cached --quiet; $dirtyStaged = ($LASTEXITCODE -ne 0)
	$untracked = (& git ls-files --others --exclude-standard)
	$didStash = $false
	if ($dirtyTracked -or $dirtyStaged -or $untracked) {
		$before = @(& git stash list).Count
		fEcho_Clean "git stash push --include-untracked ..."
		fExec "git stash" "git" @("stash", "push", "--include-untracked", "-m", "auto-stash")
		$after = @(& git stash list).Count
		$didStash = ($after -gt $before)
	}

	## Sync with this branch's upstream if it has one (a brand-new local branch has
	## nothing to pull; the push below sets its upstream on first publish). --no-edit
	## keeps an unattended run from blocking on a merge-commit editor.
	& git rev-parse --abbrev-ref '@{u}' 2>$null | Out-Null
	$hasUpstream = ($LASTEXITCODE -eq 0)
	if ($hasUpstream) {
		fEcho_Clean "git pull --no-ff ..."
		fExec "git pull" "git" @("pull", "--no-ff", "--no-edit")
	}
	if ($didStash) {
		fEcho_Clean "git stash pop ..."
		fExec "git stash pop" "git" @("stash", "pop")
	}

	fEcho_Clean "git add --all ..."
	fExec "git add" "git" @("add", "--all")

	& git diff --cached --quiet; $hasStaged = ($LASTEXITCODE -ne 0)
	if ($hasStaged) {
		if ($Msg) {
			fExec "git commit" "git" @("commit", "-m", $Msg)
			fEcho "OK: committed (`"$Msg`")"
		} else {
			## No message -> let git open the configured editor (core.editor / EDITOR).
			& git commit
			if ($LASTEXITCODE -ne 0) { fDie "git commit failed or was aborted (empty message?)" }
			fEcho "OK: committed (via editor)"
		}
	} else {
		fNote "nothing to commit"
	}

	## Push: set upstream on first publish, else push only when ahead.
	if (-not $hasUpstream) {
		fEcho_Clean "git push -u origin HEAD ..."
		fExec "git push" "git" @("push", "-u", "origin", "HEAD")
		fEcho "OK: pushed $branch (upstream set)"
	} else {
		$ahead = (& git log '@{u}..' --oneline)
		if ($ahead) {
			fEcho_Clean "git push origin ..."
			fExec "git push" "git" @("push", "origin")
			fEcho "OK: pushed $branch"
		} else {
			fNote "up to date with upstream; nothing to push"
		}
	}
}

## Advisory lint pass: clippy can't gate on Windows (Unix-gated ctl code emits
## dead_code, so -D warnings never passes), so run it plain and just report.
function fLintAdvisory {
	if (Get-Command cargo-clippy -ErrorAction SilentlyContinue) {
		$saved = $env:CARGO_TARGET_DIR
		$env:CARGO_TARGET_DIR = "target/lint"   ## don't invalidate the build cache
		try {
			& cargo clippy --workspace --all-targets
			if ($LASTEXITCODE -ne 0) { fWarn "clippy reported findings (advisory on Windows)" }
			else { fEcho "OK: clippy clean" }
		} finally { $env:CARGO_TARGET_DIR = $saved }
	} else { fNote "clippy skipped (component not installed)" }

	if (Get-Command cargo-deny -ErrorAction SilentlyContinue) {
		& cargo deny check
		## The OK line also keeps the next section's spacing right: raw deny output
		## bypasses the blank counter, so end the stage with our own line.
		if ($LASTEXITCODE -ne 0) { fWarn "cargo-deny reported findings (advisory)" }
		else { fEcho "OK: deps clean (cargo-deny)" }
	} else { fNote "cargo-deny skipped (not installed)" }
}


#••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
# Entry point

function fMain {
	## Toolchain PATH: rustup (cross targets, edition 2024) then the mingw linker.
	$env:PATH = "$CargoBin;$MingwBin;$env:PATH"
	## Cap parallelism.
	$env:CARGO_BUILD_JOBS  = "$CicdMaxJobs"
	$env:RUST_TEST_THREADS = "$CicdMaxJobs"

	Set-Location -LiteralPath $Root
	$stamp = Get-Date -Format "yyyyMMdd-HHmmss"

	## Gate mode: fmt --check + clippy (advisory) + tests, then exit. Fast local
	## stand-in for a hosted CI check; nothing is mutated or published.
	if ($Gate) {
		fSection "Gate 1/3  Format check"
		fExec "format check" "cargo" @("fmt", "--check")
		fEcho "OK: formatting clean"
		fSection "Gate 2/3  Lints (advisory)"
		fLintAdvisory
		fSection "Gate 3/3  Tests"
		fExec "tests" "cargo" @("test")
		fEcho "OK: tests passed"
		fSection "$AppName gate: PASSED."
		fEcho_Clean
		return
	}

	## Warn (non-gating) on any drifted/missing pinned helper tool.
	fCheckToolPins

	## Resolve the publish commit message: -Message wins, then an auto stamp when
	## unattended; interactive runs capture it at the preflight prompt below. An
	## empty message at commit time means "let git open its editor".
	$publishMsg = ""
	if     ($Message)     { $publishMsg = $Message }
	elseif ($Unattended)  { $publishMsg = "$AppName CI/CD $stamp" }

	## Preflight summary.
	fEcho_Clean
	fEcho_Clean "$AppName Windows CI/CD"
	fEcho_Clean
	fEcho_Clean "Repo root ...: $Root"
	fEcho_Clean "Jobs ........: $CicdMaxJobs of $Cores cores"
	fEcho_Clean "Remote sync .: $(if ($NoSync) { '(skipped)' } else { 'fetch + fast-forward check' })"
	fEcho_Clean "Format ......: $(if ($NoFmt) { '(skipped)' } else { 'cargo fmt' })"
	fEcho_Clean "Release .....: x86_64 msvc + gnu$(if ($NoArm -or $Quick) { '' } else { ' + ARM64 (if toolchain present)' })"
	fEcho_Clean "Packages ....: $(if ($NoPackage -or $Quick) { '(skipped)' } else { 'NSIS installers (if makensis present)' })"
	fEcho_Clean "Dogfood .....: $(if ($NoDogfood) { '(skipped)' } else { "$DogfoodDir\$DogfoodFixedExe" })"
	if ($NoPublish)          { fEcho_Clean "Publish .....: (skipped)" }
	elseif ($publishMsg)     { fEcho_Clean "Publish .....: commit + push current branch (hands-off: `"$publishMsg`")" }
	else                     { fEcho_Clean "Publish .....: commit + push current branch (will prompt; blank = editor)" }
	fEcho_Clean
	fEcho_Clean "Fail-fast: any error aborts before the next stage."
	fEcho_Clean

	## Capture the commit message up front so the run finishes unattended. This is
	## the natural place to bail on the common (publish) path - Ctrl+C aborts.
	if (-not $Unattended -and -not $NoPublish -and -not $publishMsg) {
		$m = Read-Host "Publish commit message (blank = editor; Ctrl+C aborts)"
		## Read-Host bypasses the blank counter; reset it so the next section's
		## leading blank isn't swallowed (the prompt line is now the last output).
		$script:WasLastEchoBlank = $false
		if ($m) { $publishMsg = $m }
	}

	## Start the transcript once past the preflight.
	New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
	try { Start-Transcript -LiteralPath (Join-Path $LogDir "run_$stamp.log") | Out-Null } catch {}

	## Stage 0: remote sync.
	fSection "0  Remote sync"
	if ($NoSync) { fNote "remote sync skipped" }
	else { fRemoteSync }

	## Stage 1: format.
	fSection "1  Format"
	if ($NoFmt) { fNote "format skipped" }
	else { fExec "format" "cargo" @("fmt"); fEcho "OK: formatted" }

	## Stage 2: debug build.
	fSection "2  Debug build"
	fExec "debug build" "cargo" @("build")
	fEcho "OK: debug build"

	## Stage 3: tests + advisory lints.
	fSection "3  Tests"
	fExec "tests" "cargo" @("test")
	fEcho "OK: tests passed"
	fLintAdvisory

	## Stage 4: release builds (x86_64 msvc + gnu always; ARM64 when ready).
	## (No profiler stage here: pprof's SIGPROF sampler is Unix-only - the
	## profiling feature can't even compile for a Windows target.)
	$built = @()
	foreach ($t in $Targets) {
		$r = fBuildTarget $t
		if ($r) { $built += $r }
	}
	if (-not $built) { fDie "no release binaries were produced" }
	$ver = fVersion
	fCollectArtifacts -Built $built -Ver $ver

	## Stage 5: packages.
	fSection "5  Packages"
	if ($NoPackage -or $Quick) { fNote "packages skipped" }
	else { fBuildPackages -Built $built -Ver $ver }

	## Stage 6: dogfood.
	fSection "6  Dogfood"
	if ($NoDogfood) { fNote "dogfood skipped" }
	else { fDogfood -Built $built }

	## Stage 7: publish.
	fSection "7  Publish"
	if ($NoPublish) { fNote "publish skipped" }
	else { fPublish -Msg $publishMsg }

	fSection "$AppName Windows CI/CD: done."
	fEcho_Clean
}

try {
	fMain
} finally {
	try { Stop-Transcript | Out-Null } catch {}
}


##	History:
##		- 2026-07-15 JC: Created (Windows-native port of cicd.bash: build/test/
##		  package/dogfood/publish; msvc + gnu always, ARM64 when ready).
##		- 2026-07-15 JC: Parity pass - profiler stage, full stash/pull/commit/push
##		  publish (rar skipped), preflight message prompt + -Quiet, -Quick skips
##		  the profiler, tool-pin drift warnings.
##		- 2026-07-22 JC: Stage 0 remote sync - fetch, fast-forward if safely
##		  behind, abort if diverged.
##		- 2026-07-22 JC: Dropped the profiler stage (pprof is Unix-only: SIGPROF
##		  sampling, can't compile or run on Windows); stages renumbered. Fixed
##		  the missing blank line after the commit-message prompt.
