param(
    [Parameter(Mandatory = $true)]
    [string]$Version
)

$ErrorActionPreference = "Stop"
$cleanVersion = $Version.TrimStart('v', 'V')
$tag = "v$cleanVersion"

$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin;$env:PATH"

$cargoToml = Join-Path $PSScriptRoot "Cargo.toml"
$content = Get-Content $cargoToml -Raw
$updated = [regex]::Replace($content, '(?m)^version = "[^"]+"', "version = `"$cleanVersion`"", 1)
Set-Content -Path $cargoToml -Value $updated -NoNewline

Write-Host "Running tests for $tag..."
cargo test --lib
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "Building release bundles (.exe / .vst3 / .clap)..."
cargo xtask bundle phantom_voicer --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$src = Join-Path $PSScriptRoot "target\bundled"
$vst3Dest = "$env:LOCALAPPDATA\Programs\Common\VST3\PhantomVoicer"
$dlDest = "$env:USERPROFILE\Downloads\PhantomVoicer"
New-Item -ItemType Directory -Force -Path $vst3Dest, $dlDest | Out-Null
Copy-Item -Recurse -Force "$src\*" $vst3Dest
Copy-Item -Recurse -Force "$src\*" $dlDest

$staging = Join-Path $PSScriptRoot "release-staging"
Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $staging | Out-Null

$zipPath = Join-Path $staging "PhantomVoicer-win64.zip"
$manifestPath = Join-Path $staging "latest.json"

Compress-Archive -Path "$src\phantom_voicer.vst3", "$src\phantom_voicer.clap", "$src\phantom_voicer.exe" -DestinationPath $zipPath -Force

$manifest = @{
    version = $cleanVersion
    zip_url = "https://github.com/znagersu/phantom-voicer/releases/latest/download/PhantomVoicer-win64.zip"
} | ConvertTo-Json
Set-Content -Path $manifestPath -Value $manifest -Encoding UTF8

git add -A
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) {
    git commit -m "Release $tag"
}
git tag -f $tag
git push -u origin main
git push -f origin $tag

$existing = gh release view $tag 2>$null
if ($LASTEXITCODE -eq 0) {
    gh release upload $tag $zipPath $manifestPath --clobber
} else {
    gh release create $tag $zipPath $manifestPath --title "Phantom Voicer $tag" --notes "Persona 5 inspired instrument & chord voicer (VST3 / CLAP / Standalone EXE)."
}

Write-Host "Published $tag to GitHub Releases! Your friend's plugin will auto-detect it."
