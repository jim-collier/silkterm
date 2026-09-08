#!/usr/bin/env pwsh

##	Purpose:
##		- Drive n8runterm.ps1 in a sandboxed HOME and check what it does to files
##		  it did not create. install.bash puts a release build at the same path
##		  the launcher wants for its symlink, and the launcher used to delete it.
##		- Nothing here touches the real home directory or the real pool.
##	History: At bottom of script.

##	Copyright © 2026 Bubbles (ID: XଌฅრX۳ᛟԃლፀƅꓩหδლც)
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Launcher = Join-Path (Split-Path $PSScriptRoot -Parent | Split-Path -Parent | Split-Path -Parent) "utility/n8runterm.ps1"
if (-not (Test-Path -LiteralPath $Launcher)) { throw "no launcher at $Launcher" }

$script:Failures = 0
function fCheck { param([string]$What, [bool]$Ok)
	if ($Ok) { Write-Host "  ok   $What" }
	else     { Write-Host "  FAIL $What"; $script:Failures++ }
}

## A sandbox home with a source dir holding one "build".
function fSandbox {
	$root = Join-Path ([System.IO.Path]::GetTempPath()) "silkterm-launcher-test-$PID-$(Get-Random)"
	$src  = Join-Path $root "synced/0-0/common/exec/app/linux"
	New-Item -ItemType Directory -Path $src -Force | Out-Null
	Set-Content -LiteralPath (Join-Path $src "silkterm") -Value "dogfood build" -NoNewline
	Set-Content -LiteralPath (Join-Path $src "silkterm.tag") -Value "gnulli" -NoNewline
	if (-not $IsWindows) { chmod +x (Join-Path $src "silkterm") }
	return $root
}

function fRun { param([string]$Home_)
	$env:HOME = $Home_
	$env:USERPROFILE = $Home_
	& pwsh -NoProfile -File $Launcher --install-only 2>&1 | Out-Null
}

$realHome = $env:HOME
try {
	Write-Host "a release build already installed at the symlink path"
	$root = fSandbox
	$bin = Join-Path $root ".local/bin"
	New-Item -ItemType Directory -Path $bin -Force | Out-Null
	$installed = Join-Path $bin "silkterm"
	Set-Content -LiteralPath $installed -Value "a release build somebody installed" -NoNewline
	fRun $root
	fCheck "the installed build is still there" (Test-Path -LiteralPath $installed)
	fCheck "and is still itself" ((Get-Content -LiteralPath $installed -Raw) -eq "a release build somebody installed")
	Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue

	Write-Host "nothing installed at the symlink path"
	$root = fSandbox
	fRun $root
	$link = Join-Path $root ".local/bin/silkterm"
	fCheck "the launcher put its own silkterm there" (Test-Path -LiteralPath $link)
	Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
} finally {
	$env:HOME = $realHome
}

if ($script:Failures -gt 0) { Write-Host "$($script:Failures) failed"; exit 1 }
Write-Host "all passed"

##	History:
##		- 2026-09-08: Created.
