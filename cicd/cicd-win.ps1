##	Purpose:
##		- Windows-native CI/CD pipeline for SilkTerm. A PowerShell port of the
##		  Linux cicd.bash, trimmed to what actually runs on Windows: dev + release
##		  builds, tests, lints, packages, dogfood install, and publish. Does NOT
##		  touch cicd.bash (that stays the Linux/cross pipeline).
##		- Stages (fail-fast; any error aborts before the next stage):
##		   1. format         (cargo fmt)
##		   2. debug build    (cargo build)
##		   3. tests + lints  (cargo test; clippy + cargo-deny are ADVISORY here)
##		   4. release builds  x86_64 msvc AND gnu (always both), + ARM64 when its
##		                      toolchain is present (auto-detected, else warn-skip)
##		   5. packages       (NSIS installer .exe per built arch, if makensis found)
##		   6. dogfood        (copy the best x86_64 build to <dogfood>\SilkTerm.exe)
##		   7. publish        (git add + commit + push the current branch)
##		- What Windows can't do (dropped vs cicd.bash): the profiler (Linux SIGPROF
##		  sampler), the headless scroll harness / screenshots / demo (need Xvfb),
##		  and .deb/.rpm packages (Linux). clippy is advisory, not gating: the
##		  Unix-gated ctl code emits dead_code warnings here, so -D warnings can't pass.
##		- Dogfood pick: prefer the msvc build IF it's self-contained (statically
##		  linked, no VCRUNTIME140/MSVCP140 dependency); else the gnu build; else
##		  whichever single build exists. Installs the fixed name SilkTerm.exe into
##		  the same folder n8runterm.ps1 uses, alongside (never colliding with) its
##		  stamped dogfood copies.
##		- Syntax:
##		  pwsh cicd/cicd-win.ps1 [options]
##		  Options:
##		   -Yes            run unattended (no confirm / publish prompt)
##		   -Quick          skip the slow stages (ARM builds + packages)
##		   -Gate           merge gate only: fmt --check + clippy + tests, then exit
##		   -NoFmt          skip the formatter stage
##		   -NoArm          skip the ARM64 release builds + their packages
##		   -NoPackage      skip the packages stage (NSIS installers)
##		   -NoDogfood      skip the dogfood install
##		   -NoPublish      skip the git publish stage
##		   -Message MSG    publish hands-off with this commit message
##		   -Help           show this help
##	History: At bottom of script.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT

[CmdletBinding()]
param(
	[switch]$Yes,
	[switch]$Quick,
	[switch]$Gate,
	[switch]$NoFmt,
	[switch]$NoArm,
	[switch]$NoPackage,
	[switch]$NoDogfood,
	[switch]$NoPublish,
	[string]$Message = "",
	[switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($Help) {
	## Print the Purpose/Syntax block and exit (mirrors cicd.bash --help).
	Get-Content -LiteralPath $PSCommandPath |
		Where-Object { $_ -match '^##' } |
		ForEach-Object { $_ -replace '^##\t?', '' }
	exit 0
}

## Windows-only: this pipeline shells out to makensis, reads PE imports, and
## writes into a Windows dogfood dir. Refuse to run anywhere else.
if (-not $IsWindows) {
	Write-Error "cicd-win.ps1: this pipeline only runs on Windows (use cicd/cicd.bash on Linux)."
	exit 1
}


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
## (see fArmTargetReady); otherwise they warn-skip. os-arch feeds the artifact
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

## Dogfood: the fixed-name copy for hand-launching. SAME folder n8runterm.ps1
## pulls from, so both coexist - n8runterm manages its stamped slktrmdf_* copies
## and prunes only those; this SilkTerm.exe is left alone.
$DogfoodDir      = "C:\opt\0-0\common\exec\synced\util\mswin\gui\by-self\win64"
$DogfoodFixedExe = "SilkTerm.exe"

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
		if ($NoArm)  { fNote "skip $($Target.OsArch): --NoArm";  return $null }
		if ($Quick)  { fNote "skip $($Target.OsArch): --Quick";  return $null }
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

## Publish: stage everything, commit (message from -Message, else auto when -Yes,
## else prompt), and push the current branch. Mirrors cicd.bash stage 8.
function fPublish {
	param([Parameter(Mandatory)][string]$Stamp)
	$branch = (& git rev-parse --abbrev-ref HEAD).Trim()
	fNote "branch: $branch"

	& git add -A
	if ($LASTEXITCODE -ne 0) { fDie "git add failed" }

	## Nothing staged -> still push (branch may be ahead), don't commit an empty.
	& git diff --cached --quiet
	$hasChanges = ($LASTEXITCODE -ne 0)

	if ($hasChanges) {
		$msg = $Message
		if (-not $msg) {
			if ($Yes) { $msg = "$AppName CI/CD $Stamp" }
			else {
				$msg = Read-Host "Publish commit message (blank = 'auto')"
				if (-not $msg) { $msg = "$AppName CI/CD $Stamp" }
			}
		}
		fExec "git commit" "git" @("commit", "-m", $msg)
		fEcho "OK: committed (`"$msg`")"
	} else {
		fNote "nothing to commit"
	}
	fExec "git push" "git" @("push")
	fEcho "OK: pushed $branch"
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

	## Preflight summary.
	fEcho_Clean
	fEcho_Clean "$AppName Windows CI/CD"
	fEcho_Clean
	fEcho_Clean "Repo root ...: $Root"
	fEcho_Clean "Jobs ........: $CicdMaxJobs of $Cores cores"
	fEcho_Clean "Format ......: $(if ($NoFmt) { '(skipped)' } else { 'cargo fmt' })"
	fEcho_Clean "Release .....: x86_64 msvc + gnu$(if ($NoArm -or $Quick) { '' } else { ' + ARM64 (if toolchain present)' })"
	fEcho_Clean "Packages ....: $(if ($NoPackage -or $Quick) { '(skipped)' } else { 'NSIS installers (if makensis present)' })"
	fEcho_Clean "Dogfood .....: $(if ($NoDogfood) { '(skipped)' } else { "$DogfoodDir\$DogfoodFixedExe" })"
	fEcho_Clean "Publish .....: $(if ($NoPublish) { '(skipped)' } else { 'git commit + push current branch' })"
	fEcho_Clean

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

	## Stage 4: release builds (x86_64 msvc + gnu always; ARM64 when ready).
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
	else { fPublish -Stamp $stamp }

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
