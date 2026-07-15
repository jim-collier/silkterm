##	Purpose:
##		- Windows-native CI/CD pipeline for SilkTerm. A PowerShell port of the
##		  Linux cicd.bash, doing as much of the same work as Windows allows -
##		  including the parts cicd.bash farms out to helper scripts (the profiler
##		  run and the git backup/publish). Does NOT touch cicd.bash (that stays the
##		  Linux/cross pipeline).
##		- Stages (fail-fast; any error aborts before the next stage):
##		   1. format         (cargo fmt)
##		   2. debug build    (cargo build)
##		   3. tests + lints  (cargo test; clippy + cargo-deny are ADVISORY here)
##		   4. profiler       (flamegraph SVG; best-effort, non-gating - env-skips
##		                      when python/pprof isn't available, same as Linux)
##		   5. release builds  x86_64 msvc AND gnu (always both), + ARM64 when its
##		                      toolchain is present (auto-detected, else warn-skip)
##		   6. packages       (NSIS installer .exe per built arch, if makensis found)
##		   7. dogfood        (copy the best x86_64 build to <dogfood>\SilkTerm.exe)
##		   8. publish        (stash -> pull -> add -> commit -> push, current branch)
##		- What Windows can't do (dropped vs cicd.bash): the headless scroll harness /
##		  screenshots / demo (need Xvfb), .deb/.rpm packages (Linux), and the rar
##		  version-archive step of publish (skipped by request). clippy is advisory,
##		  not gating: the Unix-gated ctl code emits dead_code warnings here, so
##		  -D warnings can't pass. The profiler is best-effort: pprof samples via a
##		  Unix SIGPROF path, so on Windows a profiling-build or run failure warns and
##		  skips instead of aborting.
##		- Dogfood pick: prefer the msvc build IF it's self-contained (statically
##		  linked, no VCRUNTIME140/MSVCP140 dependency); else the gnu build; else
##		  whichever single build exists. Installs the fixed name SilkTerm.exe into
##		  the same folder n8runterm.ps1 uses, alongside (never colliding with) its
##		  stamped dogfood copies.
##		- Syntax:
##		  pwsh cicd/cicd-win.ps1 [options]
##		  Options:
##		   -Yes            run unattended (no confirm / message prompt)
##		   -Quiet          quiet + unattended (implies -Yes); publish runs quiet too
##		   -Quick          skip the slow stages (profiler + ARM builds + packages)
##		   -Gate           merge gate only: fmt --check + clippy + tests, then exit
##		   -NoFmt          skip the formatter stage
##		   -NoProfile      skip the profiler stage
##		   -NoArm          skip the ARM64 release builds + their packages
##		   -NoPackage      skip the packages stage (NSIS installers)
##		   -NoDogfood      skip the dogfood install
##		   -NoPublish      skip the git publish stage
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
	[switch]$NoProfile,
	[switch]$NoArm,
	[switch]$NoPackage,
	[switch]$NoDogfood,
	[switch]$NoPublish,
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

## Profiler stage (mirrors cicd.bash / config.bash). Builds an optimized+symbols
## binary and runs the app under pprof's in-process sampler against a heavy python
## workload, writing a flamegraph SVG. Best-effort on Windows: pprof's SIGPROF
## sampler is a Unix path, so a profiling-build or run failure here warns + skips.
## Its own artifact dir so the Linux profiling run can't clobber it (or v.v.).
$ProfileSecs           = 8
$ProfileFeature        = "profiling"
$ProfileProfile        = "profiling"
$ProfileWorkloadScript = Join-Path $Root "cicd\utility\n8output-random-unicode.py"
$ProfileWorkloadArgs   = "600 0"      # <duration_s> <delay_s>; duration >> ProfileSecs, no delay = max output
$ProfileOutDir         = Join-Path $Root "cicd\artifacts\profiling-win"

## Dogfood: the fixed-name copy for hand-launching. SAME folder n8runterm.ps1
## pulls from, so both coexist - n8runterm manages its stamped slktrmdf_* copies
## and prunes only those; this SilkTerm.exe is left alone.
$DogfoodDir      = "C:\opt\0-0\common\exec\synced\util\mswin\gui\by-self\win64"
$DogfoodFixedExe = "SilkTerm.exe"

## Pinned helper-tool versions (the Windows-relevant subset of config.bash's
## TOOL_PINS). Warn (non-gating) when an installed tool has drifted, so a box
## update can't silently change results. "name|version|command args...".
$ToolPins = @(
	"cargo-zigbuild|0.23.0|cargo-zigbuild --version"
	"cargo-deny|0.19.9|cargo-deny --version"
	"makensis|3.11|MAKENSIS"          # special-cased: resolve via fFindMakensis
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

## Locate a python interpreter as a bare PATH command name (so the app's
## word-split --shell string stays space-free). $null when none is installed.
function fFindPython {
	foreach ($c in @("python3", "python", "py")) {
		if (Get-Command $c -ErrorAction SilentlyContinue) { return $c }
	}
	return $null
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
		$verLine = $null
		try {
			if ($cmd -eq "MAKENSIS") {
				$mk = fFindMakensis
				if ($mk) { $verLine = (& $mk -VERSION 2>$null | Select-Object -First 1) }
			} else {
				$exe  = ($cmd -split '\s+')[0]
				$rest = ($cmd -split '\s+') | Select-Object -Skip 1
				if (Get-Command $exe -ErrorAction SilentlyContinue) {
					$verLine = (& $exe @rest 2>$null | Select-Object -First 1)
				}
			}
		} catch { $verLine = $null }
		if (-not $verLine) { fWarn "$name not found (pinned $want)"; continue }
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

	fSection "5  Release build: $($Target.OsArch)"
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

## Profiler stage: build the profiling binary, run the app under the sampler
## against the python workload, write a flamegraph SVG. Best-effort / non-gating
## on Windows (env miss or pprof Unix-path failure -> warn + skip, never abort).
function fRunProfiler {
	param([Parameter(Mandatory)][string]$Stamp)
	if ($NoProfile) { fNote "profiler disabled (-NoProfile)"; return }
	if ($Quick)     { fNote "profiler skipped (-Quick)";     return }

	## Environmental prerequisites (not the app's fault) -> skip with a warning.
	$py = fFindPython
	if (-not $py)                                        { fWarn "profiler skipped: python not found";                 return }
	if (-not (Test-Path -LiteralPath $ProfileWorkloadScript)) { fWarn "profiler skipped: workload missing ($ProfileWorkloadScript)"; return }

	## Build the profiling binary. On Windows this can fail (pprof samples via a
	## Unix SIGPROF path), so a build failure warns + skips rather than aborting.
	fNote "building $ProfileProfile/$ExeName (cargo --profile $ProfileProfile --features $ProfileFeature)"
	& cargo build --profile $ProfileProfile --features $ProfileFeature
	if ($LASTEXITCODE -ne 0) { fWarn "profiler skipped: profiling build failed (pprof likely unsupported on Windows)"; return }
	$profExe = Join-Path $Root "target\$ProfileProfile\$ExeName.exe"
	if (-not (Test-Path -LiteralPath $profExe)) { fWarn "profiler skipped: $profExe not produced"; return }

	New-Item -ItemType Directory -Path $ProfileOutDir -Force | Out-Null
	$out = Join-Path $ProfileOutDir "flame_$Stamp.svg"

	## The app samples itself and writes the SVG on exit (it quits after
	## SILK_PROFILE_SECS). Unlike Linux there's no Xvfb, so a real window opens for
	## the duration. Start-Process -Wait because the GUI-subsystem exe doesn't block
	## the caller; the --shell value is pre-quoted (Start-Process space-joins
	## ArgumentList WITHOUT quoting - the n8runterm title-arg gotcha).
	fNote "running app ${ProfileSecs}s under sampler (a window opens briefly) ..."
	$env:SILK_PROFILE_OUT  = $out
	$env:SILK_PROFILE_SECS = "$ProfileSecs"
	try {
		$shellVal = "`"$py $ProfileWorkloadScript $ProfileWorkloadArgs`""
		$p = Start-Process -FilePath $profExe -ArgumentList @("--shell", $shellVal) -Wait -PassThru
	} finally {
		Remove-Item Env:\SILK_PROFILE_OUT, Env:\SILK_PROFILE_SECS -ErrorAction SilentlyContinue
	}
	if ($p.ExitCode -ne 0) { fWarn "profiler run exited $($p.ExitCode) (non-gating)"; return }
	if (-not (Test-Path -LiteralPath $out) -or (Get-Item -LiteralPath $out).Length -eq 0) {
		fWarn "profiler produced no SVG (non-gating)"; return
	}
	fEcho "OK: flamegraph: $out"

	## Keep the profiling dir from growing without bound (newest ~15).
	Get-ChildItem -LiteralPath $ProfileOutDir -Filter 'flame_*.svg' -ErrorAction SilentlyContinue |
		Sort-Object LastWriteTime -Descending | Select-Object -Skip 15 |
		Remove-Item -Force -ErrorAction SilentlyContinue

	## Hot-spot summary into the log (non-fatal).
	$report = Join-Path $Root "cicd\utility\flame-report.py"
	if (Test-Path -LiteralPath $report) { & $py $report --dir $ProfileOutDir 2>$null }
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
		if ($LASTEXITCODE -ne 0) { fWarn "cargo-deny reported findings (advisory)" }
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
	fEcho_Clean "Format ......: $(if ($NoFmt) { '(skipped)' } else { 'cargo fmt' })"
	fEcho_Clean "Profiler ....: $(if ($NoProfile -or $Quick) { '(skipped)' } else { "${ProfileSecs}s run -> flamegraph SVG (best-effort)" })"
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
		if ($m) { $publishMsg = $m }
	}

	## Start the transcript once past the preflight.
	New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
	try { Start-Transcript -LiteralPath (Join-Path $LogDir "run_$stamp.log") | Out-Null } catch {}

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

	## Stage 4: profiler (best-effort; env-skips when python/pprof unavailable).
	fSection "4  Profiler"
	fRunProfiler -Stamp $stamp

	## Stage 5: release builds (x86_64 msvc + gnu always; ARM64 when ready).
	$built = @()
	foreach ($t in $Targets) {
		$r = fBuildTarget $t
		if ($r) { $built += $r }
	}
	if (-not $built) { fDie "no release binaries were produced" }
	$ver = fVersion
	fCollectArtifacts -Built $built -Ver $ver

	## Stage 6: packages.
	fSection "6  Packages"
	if ($NoPackage -or $Quick) { fNote "packages skipped" }
	else { fBuildPackages -Built $built -Ver $ver }

	## Stage 7: dogfood.
	fSection "7  Dogfood"
	if ($NoDogfood) { fNote "dogfood skipped" }
	else { fDogfood -Built $built }

	## Stage 8: publish.
	fSection "8  Publish"
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
