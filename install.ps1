#!/usr/bin/env pwsh

##	- Purpose: One-liner installer for a single-binary GitHub release. Detects the
##	  OS and CPU, works out which release asset that is, verifies its sha256
##	  against the release's checksums file, and installs it. Idempotent: states
##	  its plan, asks before touching anything, and does nothing when the
##	  installed binary is already current.
##	- Reusable: everything project-specific lives in the settings block below.
##	- Runs on Windows PowerShell 5.1 and on PowerShell 7+ (pwsh) on any platform
##	  it supports - Windows, Linux and macOS.
##	- Syntax:
##	  irm https://raw.githubusercontent.com/jim-collier/silkterm/main/install.ps1 | iex
##	  or, to pass options:
##	  & ([scriptblock]::Create((irm 'https://raw.githubusercontent.com/jim-collier/silkterm/main/install.ps1'))) -Release dev
##	- Options: -Release stable|dev, -Target user|system, -Yes, -Version, -Help.
##	  The OS, the CPU architecture and the asset name are all detected.
##	- History:
##	  - 20260723 JC: Created.
##	  - 20260806 JC: Made project-agnostic; dropped -Arch for autodetection;
##	                 added -Version; runs on Windows PowerShell 5.1 as well as 7+;
##	                 permission and lock failures now explain themselves.
##	  - 20260807 JC: Safe to run from `irm | iex`, where the text executes in the
##	                 caller's own shell: no `exit` (it would close their window),
##	                 no $script: scope (absent in a script block), and StrictMode
##	                 plus the preference variables scoped to the run.

##	Copyright © 2026 Jim Collier (CryptogID: ѳ6ᴚ℈𐀘𐇦ɛ𐊁¥Mﾏb϶Δ𐌞)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT

[CmdletBinding()]
param(
	[ValidateSet('stable', 'dev')] [string]$Release = 'stable',
	[ValidateSet('user', 'system')] [string]$Target = 'user',
	[switch]$Yes,
	[switch]$Help,
	[switch]$Version
)


##	•••••••••••••••••••  Per-project settings - edit only these  ••••••••••••••••••

$installerVersion = '1.1.0'
$ownerRepo        = 'jim-collier/silkterm'
$appName          = 'SilkTerm'
$exeName          = 'silkterm'
$appComment       = 'Smooth-scrolling GPU terminal with split panes'

##	Release asset names. {exe} {version} {os} {arch} {ext} are substituted; {ext}
##	is ".exe" on Windows and empty elsewhere. {os} is windows/linux/macos, {arch}
##	is x86_64/arm64 - match whatever the release actually publishes.
$assetPattern     = '{exe}-{version}-{os}-{arch}{ext}'
$sumsPattern      = '{exe}-{version}-sha256sums.txt'

##	Windows Start Menu shortcut, and Linux freedesktop launcher. 0 for a non-GUI
##	program (the binary is still installed and still put on PATH).
$menuEntry        = 1
$desktopGenericName = 'Terminal'
$desktopIcon      = 'utilities-terminal'
$desktopCategories = 'System;TerminalEmulator;'
$desktopKeywords  = 'terminal;shell;prompt;command;'

##	••••••••••••••••••••••••  End per-project settings  ••••••••••••••••••••••••••

$apiBase = "https://api.github.com/repos/$ownerRepo"
$dlBase  = "https://github.com/$ownerRepo/releases/download"
$rawBase = "https://raw.githubusercontent.com/$ownerRepo/main"

##	`exit` is only safe when this really IS its own process. The advertised
##	one-liner runs the downloaded text inside the USER'S shell, where an `exit`
##	closes their window instead of ending the install - so failures travel as an
##	exception and only a genuine script file turns that into an exit code.
$runningAsScriptFile = -not [string]::IsNullOrEmpty($MyInvocation.MyCommand.Path)

##	5.0 and older lack Get-FileHash, so there is no verifying a download there.
if ($PSVersionTable.PSVersion.Major -lt 5) {
	Write-Host "Error: this installer needs PowerShell 5.1 or newer (you have $($PSVersionTable.PSVersion))." -ForegroundColor Red
	Write-Host '  Install PowerShell 7: https://aka.ms/powershell'
	Write-Host ''
	if ($runningAsScriptFile) { exit 1 }
	return
}

##	Nothing here sets StrictMode or a preference variable. Run by `irm ... | iex`
##	this text executes in the user's OWN shell, so a change at this level would
##	outlive the install and quietly alter their session. The entry point at the
##	bottom sets all of it inside a script block, which scopes it to the run and
##	needs no restoring - assigning a preference from a child scope only makes a
##	local copy of it anyway, so "save it and put it back" does not work here.


##	Output helpers

##	fFail <message> [hint ...] - one error line, then any hints, then abort.
function fFail {
	param([string]$Message, [string[]]$Hints = @())
	Write-Host ''
	Write-Host "Error: $Message" -ForegroundColor Red
	foreach ($hint in $Hints) { Write-Host "  $hint" }
	Write-Host ''
	##	Carries no message worth printing - fFail has already said everything.
	throw (New-Object System.OperationCanceledException 'installer-abort')
}

function fHelp {
	Write-Host ''
	Write-Host "$appName installer $installerVersion"
	Write-Host ''
	Write-Host "Downloads the newest $appName release from GitHub, checks its sha256, and"
	Write-Host 'installs it. It prints what it is about to do and asks first, and it does'
	Write-Host 'nothing at all when the installed copy is already current.'
	Write-Host ''
	Write-Host 'Usage:'
	Write-Host "  & ([scriptblock]::Create((irm '$rawBase/install.ps1'))) [options]"
	Write-Host ''
	Write-Host 'Options:'
	Write-Host '  -Release stable|dev   stable (default): newest full release'
	Write-Host '                        dev:              newest release, pre-releases included'
	Write-Host '  -Target  user|system  user (default):   just for you, no elevation needed'
	Write-Host '                        system:           for everyone (needs admin / root)'
	Write-Host '  -Yes                  skip the confirmation prompt'
	Write-Host '  -Version              print this installer''s version and exit'
	Write-Host '  -Help                 this text'
	Write-Host ''
	Write-Host 'The operating system, the CPU architecture and the matching release asset are'
	Write-Host 'all detected - there is nothing to pass for them.'
	Write-Host ''
}

##	The message buried in an exception chain is the one worth showing; the outer
##	one is usually just "Exception calling ...".
function fInnerMessage {
	param($ErrorRecord)
	$ex = $ErrorRecord.Exception
	while ($ex.InnerException) { $ex = $ex.InnerException }
	return $ex.Message
}

##	Turn a filesystem failure into something actionable. Access-denied and
##	file-in-use are the two that actually happen, and they need opposite advice.
function fFileError {
	param($ErrorRecord, [string]$What, [string]$Path)
	$ex = $ErrorRecord.Exception
	while ($ex.InnerException) { $ex = $ex.InnerException }
	$msg = $ex.Message
	if ($ex -is [System.UnauthorizedAccessException]) {
		$hints = @("Windows/your OS refused write access to: $Path")
		if ($onWindows) {
			if ($Target -eq 'system') { $hints += 'Re-run from an elevated PowerShell (right-click -> Run as administrator).' }
			else { $hints += 'Check that the folder is not read-only, and that antivirus is not blocking it.' }
			$hints += 'Or use -Target user to install under your own profile instead.'
		} else {
			if ($Target -eq 'system') { $hints += 'Re-run under sudo, or use -Target user to install under $HOME instead.' }
			else { $hints += "Check who owns it:  ls -ld $Path" }
		}
		fFail "$What - permission denied" $hints
	}
	if ($msg -match 'being used by another process|used by another process|text file busy') {
		fFail "$What - the file is in use" @(
			"Another copy of $appName is still running and is holding $Path open.",
			"Close every $appName window (or end its task) and run this again."
		)
	}
	fFail "$What - $msg" @("Path: $Path")
}


##	Environment

##	5.1 is Windows-only and defines no $IsWindows/$IsLinux/$IsMacOS at all, so
##	those cannot simply be read - under StrictMode that is a hard error.
if ($PSVersionTable.PSVersion.Major -ge 6) {
	$onWindows = $IsWindows
	$onMac     = $IsMacOS
} else {
	$onWindows = $true
	$onMac     = $false
}

##	5.1 inherits .NET Framework's TLS default, which on older Windows is still
##	TLS 1.0 - and github.com has refused that for years. The symptom is an
##	unhelpful "underlying connection was closed", so set it before any request.
if (-not $onWindows -or $PSVersionTable.PSVersion.Major -lt 6) {
	try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072 } catch {}
}


##	-UseBasicParsing keeps 5.1 off the Internet Explorer engine, which throws
##	outright when IE has never been launched on the machine. 7 dropped the
##	parameter's meaning but still accepts it, so only 5.1 needs it passed.
$webArgs = @{}
if ($PSVersionTable.PSVersion.Major -lt 6) { $webArgs['UseBasicParsing'] = $true }

##	Only the API is rate-limited per IP; release downloads are not, so they
##	stay anonymous.
function fApi {
	param([string]$Url)
	$headers = @{ 'Accept' = 'application/vnd.github+json' }
	if ($env:GITHUB_TOKEN) { $headers['Authorization'] = "Bearer $($env:GITHUB_TOKEN)" }
	return Invoke-RestMethod -Uri $Url -Headers $headers @webArgs
}


function fMain {

	Write-Host ''

	##	A local copy, because the stable -> dev fallback below rewrites it. Note
	##	nothing in here uses a $script:-qualified variable: that scope does not
	##	exist when this text is run as a script BLOCK, which is one of the three
	##	ways the one-liner reaches a user.
	$release = $Release

	##	Detect the platform. Windows reports the CPU through the environment,
	##	because [RuntimeInformation] needs .NET 4.7.1 and 5.1 predates that.
	$osToken = ''; $archToken = ''; $exeExt = ''; $osProblem = ''
	if ($onWindows) {
		$osToken = 'windows'; $exeExt = '.exe'
		$rawArch = $env:PROCESSOR_ARCHITEW6432
		if (-not $rawArch) { $rawArch = $env:PROCESSOR_ARCHITECTURE }
	} elseif ($onMac) {
		$osToken = 'macos'
		$rawArch = (& uname -m)
	} else {
		$osToken = 'linux'
		$rawArch = (& uname -m)
	}
	##	Every `break` is load-bearing: a PowerShell switch runs EVERY matching
	##	branch, so without them "arm64" matches the arm64 arm and then the
	##	32-bit-ARM arm, and the last one wins.
	switch -Regex ("$rawArch".ToLower()) {
		'^(amd64|x64|x86_64)$'  { $archToken = 'x86_64'; break }
		'^(arm64|aarch64)$'     { $archToken = 'arm64'; break }
		'^(x86|i[3-6]86)$'      { $osProblem = '32-bit x86 is not supported'; break }
		'^arm'                  { $osProblem = '32-bit ARM is not supported'; break }
		default                 { $osProblem = "unrecognized CPU architecture: $rawArch" }
	}
	if ($osProblem) {
		fFail $osProblem @(
			"No $appName build is published for this platform.",
			"Building from source: https://github.com/$ownerRepo#build-it-yourself"
		)
	}

	##	Resolve the release tag. "latest" deliberately EXCLUDES pre-releases, so
	##	stable asks for it first and only then falls back to the newest of any
	##	kind - which is also what makes a project with only betas installable.
	Write-Host "Looking up the newest $release release of $appName ..."
	$tag = $null
	$apiError = ''
	if ($release -eq 'stable') {
		try { $tag = (fApi "$apiBase/releases/latest").tag_name }
		catch {
			$apiError = fInnerMessage $_
			Write-Host 'No full release published yet; using the newest pre-release instead.'
			$release = 'dev'
		}
	}
	if ($release -eq 'dev' -and -not $tag) {
		try {
			$rels = @(fApi "$apiBase/releases?per_page=10")
			if ($rels.Count -gt 0) { $tag = $rels[0].tag_name }
		} catch { $apiError = fInnerMessage $_ }
	}
	if (-not $tag) {
		if ($apiError -match 'rate limit') {
			fFail "GitHub's API rate limit is exhausted for this IP" @(
				'Wait an hour, or set $env:GITHUB_TOKEN to a personal access token and re-run.'
			)
		}
		fFail "could not find a release at github.com/$ownerRepo" @(
			'Either none is published yet (build it from source - see the README),',
			'or github.com is unreachable from here (check your network or proxy).',
			"Detail: $apiError"
		)
	}
	$version = $tag -replace '^v', ''

	##	Work out the asset name for this platform
	$asset = $assetPattern -replace '\{exe\}', $exeName -replace '\{version\}', $version `
		-replace '\{os\}', $osToken -replace '\{arch\}', $archToken -replace '\{ext\}', $exeExt
	$sums = $sumsPattern -replace '\{exe\}', $exeName -replace '\{version\}', $version

	##	Pull the checksums first: it is small, it says which platforms this
	##	release actually carries, and its hash lets an already-current install
	##	finish without downloading the binary at all.
	$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "$exeName-install-$PID"
	New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
	try {
		$sumsPath = Join-Path $tmpDir $sums
		try { Invoke-WebRequest -Uri "$dlBase/$tag/$sums" -OutFile $sumsPath @webArgs }
		catch {
			fFail "release $tag has no checksums file ($sums)" @(
				'Nothing can be verified without it, so nothing will be installed.',
				"Release page: https://github.com/$ownerRepo/releases/tag/$tag"
			)
		}

		$wantSha = $null
		$published = @()
		foreach ($line in (Get-Content -LiteralPath $sumsPath)) {
			$parts = $line -split '\s+', 2
			if ($parts.Count -ne 2) { continue }
			$name = $parts[1].Trim().TrimStart('*')
			$published += $name
			if ($name -eq $asset) { $wantSha = $parts[0].ToLower() }
		}
		if (-not $wantSha) {
			fFail "release $tag has no build for $osToken-$archToken" (@(
				"Expected asset: $asset",
				'What it does carry:'
			) + ($published | ForEach-Object { "  $_" }) + @(
				"Building from source: https://github.com/$ownerRepo#build-it-yourself"
			))
		}

		##	Destination
		$menuDir = ''
		$appDir = ''
		if ($onWindows) {
			if ($Target -eq 'user') {
				$destDir = Join-Path $env:LOCALAPPDATA "Programs\$appName"
				$menuDir = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
				$pathScope = 'User'
			} else {
				$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
				if (-not $admin) {
					fFail 'a system-wide install needs an elevated PowerShell' @(
						'Right-click PowerShell -> Run as administrator, and run this again,',
						'or drop -Target system to install just for you (no elevation needed).'
					)
				}
				$destDir = Join-Path $env:ProgramFiles $appName
				$menuDir = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs'
				$pathScope = 'Machine'
			}
			$destFile = Join-Path $destDir "$exeName.exe"
		} else {
			if ($Target -eq 'user') {
				$destDir = Join-Path $HOME '.local/bin'
				$appDir = Join-Path $HOME '.local/share/applications'
			} else {
				if ((& id -u) -ne '0') {
					fFail 'a system-wide install needs root' @(
						'Re-run it under sudo, or drop -Target system to install under $HOME instead.'
					)
				}
				$destDir = '/usr/local/bin'
				$appDir = '/usr/local/share/applications'
			}
			$destFile = Join-Path $destDir $exeName
		}
		if ($menuEntry -ne 1 -or $onMac) { $menuDir = ''; $appDir = '' }

		##	Already current? Then say so and stop - no prompt, no download.
		##	A file that cannot be READ (locked by a running copy) must not throw
		##	here: fall through unresolved and let the copy fail with a message
		##	that actually says what to do about it.
		$installedSha = ''
		if (Test-Path -LiteralPath $destFile) {
			try { $installedSha = (Get-FileHash -Algorithm SHA256 -LiteralPath $destFile).Hash.ToLower() }
			catch { $installedSha = '' }
		}
		if ($installedSha -eq $wantSha) {
			Write-Host ''
			Write-Host "Already up to date: $destFile is $tag. Nothing to do."
			Write-Host ''
			return
		}

		##	The plan
		Write-Host ''
		Write-Host 'Plan:'
		Write-Host "  Program:  $appName $tag ($release)"
		Write-Host "  Platform: $osToken-$archToken"
		Write-Host "  Download: $dlBase/$tag/$asset"
		Write-Host "  Verify:   sha256 against $sums"
		Write-Host "  Install:  $destFile"
		if ($menuDir) { Write-Host "  Shortcut: $menuDir\$appName.lnk" }
		if ($appDir)  { Write-Host "  Launcher: $appDir/$exeName.desktop" }
		if ($onWindows) { Write-Host "  PATH:     $destDir added to the $pathScope PATH" }
		Write-Host ''
		if (-not $Yes) {
			##	Read-Host hands back an empty COLLECTION at end-of-input, and
			##	`@() -notmatch ...` is itself an empty collection - which is
			##	falsy, so an `if (... -notmatch ...)` abort branch silently does
			##	not run and the thing installs unasked. Cast to a string, then
			##	test for an explicit yes, so anything unexpected declines.
			##	[Environment]::UserInteractive is no help here: it stays True
			##	with stdin redirected from nowhere.
			##	"$(...)" flattens $null and an empty collection alike to ''; a
			##	plain [string] cast of the latter still comes back $null, and
			##	.Trim() on that throws.
			$answer = ''
			try { $answer = "$(Read-Host 'Proceed? [y/N]')".Trim().ToLowerInvariant() } catch { $answer = '' }
			if ($answer -ne 'y' -and $answer -ne 'yes') {
				if ($answer -eq '' -and [Console]::IsInputRedirected) {
					fFail 'there is no terminal here to ask for confirmation' @(
						'Re-run with -Yes to install without being asked.'
					)
				}
				Write-Host 'Aborted - nothing was touched.'
				Write-Host ''
				return
			}
			Write-Host ''
		}

		##	Download + verify
		Write-Host "Downloading $asset ..."
		$assetPath = Join-Path $tmpDir $asset
		try { Invoke-WebRequest -Uri "$dlBase/$tag/$asset" -OutFile $assetPath @webArgs }
		catch {
			fFail 'download failed' @(
				'The release lists this asset, so this is most likely a network problem.',
				"URL: $dlBase/$tag/$asset",
				"Detail: $(fInnerMessage $_)"
			)
		}
		$haveSha = (Get-FileHash -Algorithm SHA256 -LiteralPath $assetPath).Hash.ToLower()
		if ($haveSha -ne $wantSha) {
			fFail 'checksum mismatch - NOT installing' @(
				"expected $wantSha",
				"got      $haveSha",
				'The download was corrupted or tampered with. Try again; if it repeats, report it.'
			)
		}
		Write-Host 'Checksum OK.'

		##	Install
		Write-Host ''
		Write-Host 'Installing ...'
		try { New-Item -ItemType Directory -Force -Path $destDir | Out-Null }
		catch { fFileError $_ "could not create $destDir" $destDir }
		try { Copy-Item -LiteralPath $assetPath -Destination $destFile -Force }
		catch { fFileError $_ "could not write $destFile" $destFile }
		if (-not $onWindows) { & chmod 0755 $destFile }

		##	Start Menu shortcut (Windows)
		if ($menuDir) {
			try {
				$shell = New-Object -ComObject WScript.Shell
				$lnk = $shell.CreateShortcut((Join-Path $menuDir "$appName.lnk"))
				$lnk.TargetPath = $destFile
				$lnk.WorkingDirectory = $destDir
				$lnk.Description = $appComment
				$lnk.Save()
			} catch {
				Write-Host "Note: could not create the Start Menu shortcut ($(fInnerMessage $_)) - $appName itself installed fine."
			}
		}

		##	Freedesktop launcher (Linux)
		if ($appDir) {
			try {
				New-Item -ItemType Directory -Force -Path $appDir | Out-Null
				@(
					'[Desktop Entry]', 'Type=Application', "Name=$appName",
					"GenericName=$desktopGenericName", "Comment=$appComment", "Exec=$destFile",
					"Icon=$desktopIcon", 'Terminal=false', "Categories=$desktopCategories",
					"Keywords=$desktopKeywords", 'StartupNotify=true'
				) | Set-Content -LiteralPath (Join-Path $appDir "$exeName.desktop")
			} catch {
				Write-Host "Note: could not write the desktop launcher to $appDir - $appName itself installed fine."
			}
		}

		##	PATH
		if ($onWindows) {
			fAddToWindowsPath $destDir $pathScope
		} else {
			if (":$($env:PATH):" -notlike "*:$destDir`:*") {
				Write-Host ''
				Write-Host "Note: $destDir is not on your PATH, so '$exeName' won't be found by name yet."
				Write-Host "  Add it with:  echo 'export PATH=`"$destDir`:`$PATH`"' >> ~/.profile"
				Write-Host "  Until then, run it in full:  $destFile"
			}
		}

		Write-Host ''
		Write-Host "Installed $appName $tag to $destFile"
		Write-Host ''
	} finally {
		Remove-Item -Recurse -Force -LiteralPath $tmpDir -ErrorAction SilentlyContinue
	}
}

##	Append to the persistent PATH via the registry rather than via
##	[Environment]::SetEnvironmentVariable, which rewrites a REG_EXPAND_SZ PATH as
##	a plain REG_SZ - that silently kills every %VAR% already in it. Reading and
##	writing the raw value keeps whatever kind it already was.
function fAddToWindowsPath {
	param([string]$Dir, [string]$Scope)
	try {
		if ($Scope -eq 'Machine') {
			$root = [Microsoft.Win32.Registry]::LocalMachine
			$sub = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
		} else {
			$root = [Microsoft.Win32.Registry]::CurrentUser
			$sub = 'Environment'
		}
		$key = $root.OpenSubKey($sub, $true)
		if (-not $key) { throw "cannot open HKEY\$sub for writing" }
		try {
			$raw = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
			$kind = if ($raw) { $key.GetValueKind('Path') } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
			if (($raw -split ';') -contains $Dir) { return }
			$new = if ($raw) { $raw.TrimEnd(';') + ';' + $Dir } else { $Dir }
			##	A User PATH over ~2047 chars gets truncated by parts of Windows.
			##	Losing an existing PATH is far worse than not being on it.
			if ($Scope -eq 'User' -and $new.Length -gt 2047) {
				Write-Host "Note: your user PATH is already near the length limit, so $Dir was NOT added (nothing was changed)."
				Write-Host "  Run $appName in full:  $(Join-Path $Dir "$exeName.exe")"
				return
			}
			$key.SetValue('Path', $new, $kind)
			$env:PATH = "$env:PATH;$Dir"
			Write-Host "Added $Dir to the $Scope PATH - already-open shells need a restart to see it."
		} finally { $key.Close() }
	} catch {
		Write-Host "Note: could not update the $Scope PATH ($(fInnerMessage $_)) - $appName itself installed fine."
		Write-Host "  Run it in full:  $(Join-Path $Dir "$exeName.exe")"
	}
}


##	Script entry point. The `& { }` is what keeps StrictMode off the caller's
##	shell - it applies to this block and everything it calls, and lapses here.
##	A hashtable rather than the block's return value: it is a reference, so the
##	child scope can set it directly, and a stray line of pipeline output from
##	anything in here cannot turn the answer into an array.
$state = @{ failed = $false }
& {
	Set-StrictMode -Version 2.0
	$ErrorActionPreference = 'Stop'
	##	On 5.1 the progress bar makes Invoke-WebRequest an order of magnitude
	##	slower. Both of these lapse with the block, so nothing needs restoring.
	$ProgressPreference = 'SilentlyContinue'
	try {
		if ($Help) { fHelp }
		elseif ($Version) { Write-Host ''; Write-Host "$appName installer $installerVersion"; Write-Host '' }
		else { fMain }
	} catch [System.OperationCanceledException] {
		##	A handled failure; fFail already printed what went wrong and why.
		$state.failed = $true
	} catch {
		##	Anything unforeseen still reads as a sentence, not a stack trace.
		Write-Host ''
		Write-Host "Error: $(fInnerMessage $_)" -ForegroundColor Red
		Write-Host ''
		$state.failed = $true
	}
} | Out-Null
if ($state.failed -and $runningAsScriptFile) { exit 1 }
