$ErrorActionPreference = "Continue"
. "$PSScriptRoot\_env.ps1"

$src = Join-Path $RepoDir "source"
Push-Location $src
$sw = [Diagnostics.Stopwatch]::StartNew()
cargo build --release 2>&1 | ForEach-Object { $_.ToString() }
$code = $LASTEXITCODE
$sw.Stop()
Pop-Location
"build exit=$code in $([math]::Round($sw.Elapsed.TotalSeconds))s"
$exe = Join-Path $RepoDir "target\release\silkterm.exe"
if (Test-Path $exe) { "binary = $exe  " + (Get-Item $exe).Length + " bytes" }
exit $code
