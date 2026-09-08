$ErrorActionPreference = "Continue"
. "$PSScriptRoot\_env.ps1"

# G6: the test process reads the live user config, so this reflects whichever
# account ssh logged in as, not a clean machine.
Push-Location (Join-Path $RepoDir "source")
cargo test 2>&1 | ForEach-Object { $_.ToString() }
$code = $LASTEXITCODE
Pop-Location
"test exit=$code"
exit $code
