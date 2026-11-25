$ErrorActionPreference = "Stop"

$cargoToml = Get-Content -Raw -Path (Join-Path $PSScriptRoot "..\Cargo.toml")
if ($cargoToml -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
    throw "Could not parse version from Cargo.toml"
}
$ver = $Matches[1]
$exeName = "Boogie-v$ver.exe"
$zipName = "Boogie-v$ver-Windows-x64.zip"

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $root
try {
    Write-Host "Building Boogie v$ver (release)..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force $dist | Out-Null

    $exeDst = Join-Path $dist $exeName
    Copy-Item "target\release\boogie.exe" $exeDst -Force

    $zipStage = Join-Path $dist "zip_stage"
    if (Test-Path $zipStage) { Remove-Item -Recurse -Force $zipStage }
    New-Item -ItemType Directory -Force (Join-Path $zipStage "config") | Out-Null
    Copy-Item $exeDst (Join-Path $zipStage $exeName)
    Copy-Item "config\api_keys.example.json" (Join-Path $zipStage "config\api_keys.example.json")
    Copy-Item "README.md" (Join-Path $zipStage "README.md")
    Copy-Item "LICENSE" (Join-Path $zipStage "LICENSE")

    $zipDst = Join-Path $dist $zipName
    if (Test-Path $zipDst) { Remove-Item -Force $zipDst }
    Compress-Archive -Path "$zipStage\*" -DestinationPath $zipDst -Force
    Remove-Item -Recurse -Force $zipStage

    $shaDst = Join-Path $dist "SHA256SUMS.txt"
    Get-FileHash $exeDst, $zipDst -Algorithm SHA256 | ForEach-Object {
        "$($_.Hash.ToLowerInvariant())  $(Split-Path -Leaf $_.Path)"
    } | Set-Content -Encoding ascii $shaDst

    Write-Host "Packaged:"
    Write-Host "  - $exeDst"
    Write-Host "  - $zipDst"
    Write-Host "  - $shaDst"
} finally {
    Pop-Location
}
