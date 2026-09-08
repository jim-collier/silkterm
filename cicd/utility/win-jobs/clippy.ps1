$ErrorActionPreference = "Continue"
. "$PSScriptRoot\_env.ps1"

# Separate target dir, the way cicd does it - clippy and build otherwise
# invalidate each other's artifacts every run.
$env:CARGO_TARGET_DIR = Join-Path $RepoDir "source\target\lint"
Push-Location (Join-Path $RepoDir "source")
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | ForEach-Object { $_.ToString() }
$code = $LASTEXITCODE
Pop-Location
"clippy exit=$code"
exit $code
