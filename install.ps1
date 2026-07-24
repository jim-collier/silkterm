#!/usr/bin/env pwsh

##	- Purpose: One-liner installer for SilkTerm via PowerShell 7+ (Windows,
##	  Linux; macOS builds are not published yet). Downloads the latest release
##	  binary from GitHub, verifies its sha256 against the release's checksums
##	  file, and installs it. Idempotent: states its plan, asks before touching
##	  anything, and does nothing when the installed binary is already current.
##	- Syntax:
##	  & ([scriptblock]::Create((irm 'https://raw.githubusercontent.com/jim-collier/silkterm/main/install.ps1'))) [-Release stable|dev] [-Target user|system] [-Arch x64|amd64|arm64] [-Yes]
##	- Options:
##	  -Release  stable (default) = latest full release; dev = newest release
##	            including pre-releases. With no full release published yet,
##	            stable falls back to dev with a note.
##	  -Target   user (default) or system (needs admin/root)
##	  -Arch     override the detected CPU architecture
##	  -Yes      skip the confirmation prompt
##	- Install locations:
##	  Windows user:   %LOCALAPPDATA%\Programs\SilkTerm\silkterm.exe (+ user PATH + Start Menu shortcut)
##	  Windows system: C:\Program Files\SilkTerm\silkterm.exe (+ machine PATH + common Start Menu)
##	  Linux user:     ~/.local/bin/silkterm (+ ~/.local/share/applications launcher)
##	  Linux system:   /usr/local/bin/silkterm (run with sudo)
##	- History:
##	  - 20260723 JC: Created.

##	Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT

[CmdletBinding()]
param(
	[ValidateSet('stable', 'dev')] [string]$Release = 'stable',
	[ValidateSet('user', 'system')] [string]$Target = 'user',
	[ValidateSet('x64', 'amd64', 'arm64', 'x86_64', 'aarch64')] [string]$Arch = '',
	[switch]$Yes
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ownerRepo = 'jim-collier/silkterm'
$exeName = 'silkterm'
$apiBase = "https://api.github.com/repos/$ownerRepo"
$dlBase = "https://github.com/$ownerRepo/releases/download"

function fFail([string]$msg) { Write-Host "Error: $msg" -ForegroundColor Red; Write-Host ''; exit 1 }

if ($PSVersionTable.PSVersion.Major -lt 7) { fFail 'this installer needs PowerShell 7+ (pwsh)' }

Write-Host ''

## Detect OS + architecture
$onWindows = $IsWindows
if ($IsMacOS) { fFail "macOS builds are not published yet - please build from source: https://github.com/$ownerRepo#building-from-source" }
if (-not $Arch) {
	$Arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLower()) {
		'x64' { 'x86_64' }
		'arm64' { 'arm64' }
		default { fFail "unsupported architecture: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)" }
	}
}
$Arch = switch ($Arch.ToLower()) { 'x64' { 'x86_64' } 'amd64' { 'x86_64' } 'x86_64' { 'x86_64' } default { 'arm64' } }
$osArch = if ($onWindows) { "windows-$Arch" } else { "linux-$Arch" }

## Resolve the release tag
Write-Host "Looking up the latest $Release release of SilkTerm ..."
$tag = $null
if ($Release -eq 'stable') {
	try { $tag = (Invoke-RestMethod "$apiBase/releases/latest").tag_name } catch {
		Write-Host 'No full release published yet; using the newest pre-release instead.'
		$Release = 'dev'
	}
}
if ($Release -eq 'dev' -and -not $tag) {
	try { $rels = @(Invoke-RestMethod "$apiBase/releases?per_page=5"); if ($rels.Count) { $tag = $rels[0].tag_name } } catch {}
}
if (-not $tag) { fFail "no releases found at github.com/$ownerRepo (releases may not be published yet - see the README for building from source)" }
$version = $tag -replace '^v', ''

## Work out names, paths, and the plan
$ext = if ($onWindows) { '.exe' } else { '' }
$asset = "$exeName-$version-$osArch$ext"
$sums = "$exeName-$version-sha256sums.txt"
if ($onWindows) {
	if ($Target -eq 'user') {
		$destDir = Join-Path $env:LOCALAPPDATA 'Programs\SilkTerm'
		$menuDir = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
	} else {
		$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
		if (-not $isAdmin) { fFail 'system install needs an elevated (administrator) PowerShell' }
		$destDir = Join-Path $env:ProgramFiles 'SilkTerm'
		$menuDir = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs'
	}
	$destFile = Join-Path $destDir "$exeName.exe"
} else {
	if ($Target -eq 'user') {
		$destDir = Join-Path $HOME '.local/bin'
	} else {
		if ((id -u) -ne '0') { fFail 'system install on Linux needs root - run under sudo, or use -Target user' }
		$destDir = '/usr/local/bin'
	}
	$destFile = Join-Path $destDir $exeName
}

Write-Host ''
Write-Host 'Plan:'
Write-Host "  Release:  $tag ($Release)"
Write-Host "  Download: $dlBase/$tag/$asset"
Write-Host "  Verify:   sha256 against $sums"
Write-Host "  Install:  $destFile"
if ($onWindows) { Write-Host "  Shortcut: $menuDir\SilkTerm.lnk (+ add $destDir to PATH)" }
Write-Host ''
if (-not $Yes) {
	$answer = Read-Host 'Proceed? [y/N]'
	if ($answer -notmatch '^(y|yes)$') { Write-Host 'Aborted - nothing was touched.'; Write-Host ''; exit 0 }
	Write-Host ''
}

## Download + verify
$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "silkterm-install-$PID"
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
try {
	Write-Host "Downloading $asset ..."
	try { Invoke-WebRequest "$dlBase/$tag/$asset" -OutFile (Join-Path $tmpDir $asset) } catch { fFail "download failed (no $osArch build in release $tag?)" }
	try { Invoke-WebRequest "$dlBase/$tag/$sums" -OutFile (Join-Path $tmpDir $sums) } catch { fFail "checksums file missing from release $tag" }
	$wantSha = $null
	foreach ($line in Get-Content (Join-Path $tmpDir $sums)) {
		$parts = $line -split '\s+', 2
		if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $asset) { $wantSha = $parts[0].ToLower(); break }
	}
	if (-not $wantSha) { fFail "no checksum entry for $asset in $sums" }
	$haveSha = (Get-FileHash -Algorithm SHA256 (Join-Path $tmpDir $asset)).Hash.ToLower()
	if ($haveSha -ne $wantSha) { fFail "checksum mismatch (expected $wantSha, got $haveSha) - not installing" }
	Write-Host 'Checksum OK.'

	## Idempotence: already current?
	if ((Test-Path $destFile) -and ((Get-FileHash -Algorithm SHA256 $destFile).Hash.ToLower() -eq $wantSha)) {
		Write-Host ''
		Write-Host "Already up to date: $destFile is $tag. Nothing to do."
		Write-Host ''
		exit 0
	}

	## Install
	Write-Host ''
	Write-Host 'Installing ...'
	New-Item -ItemType Directory -Force -Path $destDir | Out-Null
	Copy-Item (Join-Path $tmpDir $asset) $destFile -Force
	if ($onWindows) {
		## Start Menu shortcut
		$shell = New-Object -ComObject WScript.Shell
		$lnk = $shell.CreateShortcut((Join-Path $menuDir 'SilkTerm.lnk'))
		$lnk.TargetPath = $destFile
		$lnk.WorkingDirectory = $destDir
		$lnk.Description = 'Smooth-scrolling GPU terminal with split panes'
		$lnk.Save()
		## PATH (skip when already present)
		$scope = if ($Target -eq 'user') { 'User' } else { 'Machine' }
		$path = [Environment]::GetEnvironmentVariable('Path', $scope)
		if (($path -split ';') -notcontains $destDir) {
			[Environment]::SetEnvironmentVariable('Path', ($path.TrimEnd(';') + ';' + $destDir), $scope)
			Write-Host "Added $destDir to the $scope PATH (new shells will pick it up)."
		}
	} else {
		chmod 0755 $destFile
		## Desktop launcher (user target only; system installs get it from the .deb/.rpm)
		if ($Target -eq 'user') {
			$appDir = Join-Path $HOME '.local/share/applications'
			New-Item -ItemType Directory -Force -Path $appDir | Out-Null
			@(
				'[Desktop Entry]', 'Type=Application', 'Name=SilkTerm', 'GenericName=Terminal',
				'Comment=Smooth-scrolling GPU terminal with split panes', "Exec=$destFile",
				'Icon=utilities-terminal', 'Terminal=false', 'Categories=System;TerminalEmulator;',
				'Keywords=terminal;shell;prompt;command;', 'StartupNotify=true'
			) | Set-Content (Join-Path $appDir "$exeName.desktop")
		}
	}
	Write-Host "Installed $tag to $destFile"
	Write-Host ''
} finally {
	Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
}
