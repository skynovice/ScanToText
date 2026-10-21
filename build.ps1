$ErrorActionPreference = 'Stop'

& (Join-Path $PSScriptRoot 'test.ps1')

$rustProject = Join-Path $PSScriptRoot 'rust-app'
$releaseExe = Join-Path $rustProject 'target\release\roi-ocr-helper.exe'
$dist = Join-Path $PSScriptRoot 'dist'
$outputExe = Join-Path $dist 'ROI-OCR-Helper.exe'

Push-Location $rustProject
try {
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed' }
} finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item -LiteralPath $releaseExe -Destination $outputExe -Force
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $outputExe).Hash
Write-Host "Build สำเร็จ: $outputExe"
Write-Host "SHA256: $hash"
