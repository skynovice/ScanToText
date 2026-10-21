$ErrorActionPreference = 'Stop'

$rustProject = Join-Path $PSScriptRoot 'rust-app'
Push-Location $rustProject
try {
    & cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt --check failed' }

    & cargo test --all-targets --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo test failed' }

    & cargo clippy --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'cargo clippy failed' }
} finally {
    Pop-Location
}
