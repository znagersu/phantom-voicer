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
$clapUser = "$env:LOCALAPPDATA\Programs\Common\CLAP\phantom_voicer.clap"
New-Item -ItemType Directory -Force -Path $vst3Dest, $dlDest | Out-Null

$vst3Old = Join-Path $vst3Dest "phantom_voicer.vst3"
$dlOld = Join-Path $dlDest "phantom_voicer.vst3"
if (Test-Path $vst3Old) { Remove-Item -Recurse -Force $vst3Old }
if (Test-Path $dlOld) { Remove-Item -Recurse -Force $dlOld }

Copy-Item -Recurse -Force "$src\*" $vst3Dest
Copy-Item -Recurse -Force "$src\*" $dlDest
if (Test-Path $clapUser) {
    Copy-Item -Force (Join-Path $src "phantom_voicer.clap") $clapUser
}

$staging = Join-Path $PSScriptRoot "release-staging"
Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $staging | Out-Null

$zipPath = Join-Path $staging "PhantomVoicer-win64.zip"
$manifestPath = Join-Path $staging "latest.json"

Compress-Archive -Path "$src\phantom_voicer.vst3", "$src\phantom_voicer.clap", "$src\phantom_voicer.exe" -DestinationPath $zipPath -Force

# Pin zip_url directly to the exact release tag so CDN caches never mix versions
$manifest = @{
    version = $cleanVersion
    zip_url = "https://github.com/znagersu/phantom-voicer/releases/download/$tag/PhantomVoicer-win64.zip"
} | ConvertTo-Json
[System.IO.File]::WriteAllText($manifestPath, $manifest, [System.Text.UTF8Encoding]::new($false))

git add -A
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) {
    git commit -m "Release $tag"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
git tag -f $tag
git push -u origin main
git push -f origin $tag

$ErrorActionPreference = "Continue"
gh release view $tag *>$null
$releaseExists = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = "Stop"

if ($releaseExists) {
    gh release upload $tag $zipPath $manifestPath --clobber
} else {
    gh release create $tag $zipPath $manifestPath --title "Phantom Voicer $tag" --notes "Persona 5 inspired instrument & chord voicer (VST3 / CLAP / Standalone EXE)."
}

Write-Host "Published $tag to GitHub Releases! Your friend's plugin will auto-detect it."
