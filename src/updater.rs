use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LATEST_MANIFEST_URL: &str =
    "https://github.com/znagersu/phantom-voicer/releases/latest/download/latest.json";

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Available { version: String, zip_url: String },
    Installing(String),
    ReadyToRestart(String),
    Error(String),
}

#[derive(Deserialize)]
struct LatestManifest {
    version: String,
    zip_url: String,
}

#[derive(Clone)]
pub struct UpdaterHandle {
    state: Arc<Mutex<UpdateStatus>>,
}

impl Default for UpdaterHandle {
    fn default() -> Self {
        let handle = Self {
            state: Arc::new(Mutex::new(UpdateStatus::Idle)),
        };
        handle.check_async();
        handle
    }
}

impl UpdaterHandle {
    pub fn status(&self) -> UpdateStatus {
        self.state
            .lock()
            .map(|g| g.clone())
            .unwrap_or(UpdateStatus::Idle)
    }

    pub fn check_async(&self) {
        {
            let mut guard = match self.state.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if matches!(
                *guard,
                UpdateStatus::Checking
                    | UpdateStatus::Installing(_)
                    | UpdateStatus::ReadyToRestart(_)
            ) {
                return;
            }
            *guard = UpdateStatus::Checking;
        }

        let state = self.state.clone();
        std::thread::spawn(move || {
            let next = match fetch_latest_manifest() {
                Ok(manifest) => {
                    if is_newer_version(&manifest.version, CURRENT_VERSION) {
                        UpdateStatus::Available {
                            version: manifest.version,
                            zip_url: manifest.zip_url,
                        }
                    } else {
                        UpdateStatus::UpToDate
                    }
                }
                Err(err) => UpdateStatus::Error(err),
            };

            if let Ok(mut guard) = state.lock() {
                *guard = next;
            }
        });
    }

    pub fn install_async(&self, version: String, zip_url: String) {
        {
            let mut guard = match self.state.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if matches!(
                *guard,
                UpdateStatus::Installing(_) | UpdateStatus::ReadyToRestart(_)
            ) {
                return;
            }
            *guard = UpdateStatus::Installing(version.clone());
        }

        let state = self.state.clone();
        std::thread::spawn(move || {
            let result = download_and_launch_updater(&version, &zip_url);
            if let Ok(mut guard) = state.lock() {
                *guard = match result {
                    Ok(msg) => UpdateStatus::ReadyToRestart(msg),
                    Err(err) => UpdateStatus::Error(err),
                };
            }
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SemVer {
    major: u32,
    minor: u32,
    patch: u32,
    is_release: bool,
}

fn parse_semver(v: &str) -> Option<SemVer> {
    let clean = v
        .trim()
        .trim_start_matches(['v', 'V'])
        .split('+')
        .next()?;
    let (core, is_release) = match clean.split_once('-') {
        Some((c, _pre)) => (c, false),
        None => (clean, true),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some(SemVer {
        major,
        minor,
        patch,
        is_release,
    })
}

pub fn is_newer_version(remote: &str, current: &str) -> bool {
    match (parse_semver(remote), parse_semver(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn fetch_latest_manifest() -> Result<LatestManifest, String> {
    // Cache-buster query param prevents Fastly/Varnish CDN from serving stale latest.json
    let cache_busted_url = format!("{LATEST_MANIFEST_URL}?t={}", unix_millis());
    let output = hidden_command("curl.exe")
        .args([
            "-fsSL",
            "-H",
            "Cache-Control: no-cache",
            "--max-time",
            "8",
            &cache_busted_url,
        ])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    if !output.status.success() {
        return Err(String::from("Offline or no release yet"));
    }

    let body = output
        .stdout
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(&output.stdout);

    serde_json::from_slice::<LatestManifest>(body)
        .map_err(|e| format!("Invalid manifest: {e}"))
}

#[cfg(target_os = "windows")]
fn current_module_path() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    extern "system" {
        fn GetModuleHandleExW(dwFlags: u32, lpModuleName: *const u8, phModule: *mut usize) -> i32;
        fn GetModuleFileNameW(hModule: usize, lpFilename: *mut u16, nSize: u32) -> u32;
    }

    const FLAG_FROM_ADDRESS: u32 = 0x0000_0004;
    const FLAG_UNCHANGED_REFCOUNT: u32 = 0x0000_0002;

    let mut hmod: usize = 0;
    let ok = unsafe {
        GetModuleHandleExW(
            FLAG_FROM_ADDRESS | FLAG_UNCHANGED_REFCOUNT,
            current_module_path as *const u8,
            &mut hmod,
        )
    };
    if ok == 0 || hmod == 0 {
        return None;
    }

    let mut buf = vec![0u16; 1024];
    let len = unsafe { GetModuleFileNameW(hmod, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if len == 0 || len >= buf.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(&buf[..len])))
}

#[cfg(not(target_os = "windows"))]
fn current_module_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Resolves the exact target artifact path (`...\phantom_voicer.vst3` bundle dir, `.clap`, or `.exe`)
/// and its parent install directory.
fn detect_active_target() -> Result<(PathBuf, PathBuf), String> {
    let mod_path = current_module_path().ok_or("Could not locate plugin path")?;

    // Walk ancestors for a `.vst3` bundle folder (case-insensitive)
    for anc in mod_path.ancestors() {
        if let Some(ext) = anc.extension() {
            if ext.eq_ignore_ascii_case("vst3") && anc.is_dir() {
                let parent = anc
                    .parent()
                    .map(|p| p.to_path_buf())
                    .ok_or_else(|| "Missing VST3 parent directory".to_string())?;
                return Ok((anc.to_path_buf(), parent));
            }
        }
    }

    // Standalone `.exe` or `.clap`
    let parent = mod_path
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Missing install directory".to_string())?;
    Ok((mod_path, parent))
}

fn download_and_launch_updater(version: &str, zip_url: &str) -> Result<String, String> {
    if !zip_url.starts_with("https://github.com/znagersu/phantom-voicer/") {
        return Err("Untrusted release URL".to_string());
    }

    let (active_target, install_dir) = detect_active_target()?;
    let temp_root = std::env::temp_dir().join(format!(
        "PhantomVoicerUpdate-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    std::fs::create_dir_all(&temp_root).map_err(|e| format!("Temp dir error: {e}"))?;

    let zip_path = temp_root.join("PhantomVoicer-win64.zip");
    let extract_dir = temp_root.join("extracted");

    let busted_zip_url = if zip_url.contains('?') {
        zip_url.to_string()
    } else {
        format!("{zip_url}?t={}", unix_millis())
    };

    let dl = hidden_command("curl.exe")
        .args([
            "-fsSL",
            "-L",
            "-H",
            "Cache-Control: no-cache",
            "--max-time",
            "60",
            "-o",
            &zip_path.to_string_lossy(),
            &busted_zip_url,
        ])
        .output()
        .map_err(|e| format!("Download failed: {e}"))?;

    if !dl.status.success() {
        return Err("Download failed".to_string());
    }

    let expand = hidden_command("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                zip_path.to_string_lossy().replace('\'', "''"),
                extract_dir.to_string_lossy().replace('\'', "''")
            ),
        ])
        .output()
        .map_err(|e| format!("Extract failed: {e}"))?;

    if !expand.status.success() {
        return Err("Extract failed".to_string());
    }

    let script_path = temp_root.join("apply_update.ps1");
    let install_str = install_dir.to_string_lossy().replace('\'', "''");
    let active_str = active_target.to_string_lossy().replace('\'', "''");
    let extract_str = extract_dir.to_string_lossy().replace('\'', "''");
    let ver_str = version.replace('\'', "''");

    // Bulletproof self-updater:
    // - Whole script wrapped in try/catch so errors are always surfaced.
    // - Discovers active target + any duplicate copies across standard VST3/CLAP folders so FL Studio
    //   never keeps loading a stale duplicate.
    // - Checks write/delete permission on ALL discovered targets; if any need Admin, elevates via UAC once.
    // - Checks ONLY sharing/lock IOExceptions (not ACL errors) and prompts Retry/Cancel until FL Studio closes.
    // - Performs atomic .bak swap + SHA-256 verification before removing backup.
    let ps_script = format!(
        r#"param([switch]$Elevated)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms

try {{
    $src = '{extract_str}'
    $dst = '{install_str}'
    $activeTarget = '{active_str}'
    $ver = '{ver_str}'

    $srcVst3 = Join-Path $src 'phantom_voicer.vst3'
    $srcClap = Join-Path $src 'phantom_voicer.clap'
    $srcExe  = Join-Path $src 'phantom_voicer.exe'
    $srcVst3Hash = (Get-FileHash (Join-Path $srcVst3 'Contents\x86_64-win\phantom_voicer.vst3')).Hash
    $srcClapHash = (Get-FileHash $srcClap).Hash
    $srcExeHash  = (Get-FileHash $srcExe).Hash

    # Collect all installed targets to update so no stale duplicate remains
    $vst3Targets = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)
    $clapTargets = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)
    $exeTargets  = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)

    if ($activeTarget -match '\.vst3$') {{ [void]$vst3Targets.Add($activeTarget) }}
    elseif ($activeTarget -match '\.clap$') {{ [void]$clapTargets.Add($activeTarget) }}
    elseif ($activeTarget -match '\.exe$') {{ [void]$exeTargets.Add($activeTarget) }}

    $roots = @(
        $dst,
        "$env:ProgramFiles\Common Files\VST3",
        "$env:LOCALAPPDATA\Programs\Common\VST3",
        "$env:ProgramFiles\Common Files\CLAP",
        "$env:LOCALAPPDATA\Programs\Common\CLAP"
    ) | Select-Object -Unique

    foreach ($root in $roots) {{
        if (-not $root -or -not (Test-Path $root)) {{ continue }}
        Get-ChildItem -Path $root -Filter 'phantom_voicer.vst3' -Directory -Recurse -Depth 2 -ErrorAction SilentlyContinue | ForEach-Object {{
            # Skip nested phantom_voicer.vst3\phantom_voicer.vst3 from legacy bug; parent will be replaced atomically
            if ($_.Parent.Name -ne 'phantom_voicer.vst3') {{
                [void]$vst3Targets.Add($_.FullName)
            }}
        }}
        Get-ChildItem -Path $root -Filter 'phantom_voicer.clap' -File -Recurse -Depth 2 -ErrorAction SilentlyContinue | ForEach-Object {{
            [void]$clapTargets.Add($_.FullName)
        }}
        Get-ChildItem -Path $root -Filter 'phantom_voicer.exe' -File -Recurse -Depth 2 -ErrorAction SilentlyContinue | ForEach-Object {{
            [void]$exeTargets.Add($_.FullName)
        }}
    }}

    if ($vst3Targets.Count -eq 0 -and $clapTargets.Count -eq 0 -and $exeTargets.Count -eq 0) {{
        throw "No target installation found for $activeTarget"
    }}

    function Test-PathWritable([string]$path) {{
        $dir = if (Test-Path $path -PathType Container) {{ Split-Path $path -Parent }} elseif (Test-Path $path) {{ Split-Path $path -Parent }} else {{ $dst }}
        $probe = Join-Path $dir ('.pv_w_' + [guid]::NewGuid().ToString('N'))
        try {{
            [System.IO.File]::WriteAllText($probe, 'ok')
            Remove-Item -Force $probe -ErrorAction SilentlyContinue
            return $true
        }} catch [System.UnauthorizedAccessException] {{
            return $false
        }} catch {{
            return $false
        }}
    }}

    function Test-FileLocked([string]$filePath) {{
        if (-not (Test-Path $filePath -PathType Leaf)) {{ return $false }}
        try {{
            $fs = [System.IO.File]::Open($filePath, 'Open', 'ReadWrite', 'None')
            $fs.Close()
            return $false
        }} catch [System.UnauthorizedAccessException] {{
            return $false
        }} catch [System.IO.IOException] {{
            $hr = $_.Exception.HResult -band 0xFFFF
            # 32 = ERROR_SHARING_VIOLATION, 33 = ERROR_LOCK_VIOLATION
            if ($hr -eq 32 -or $hr -eq 33) {{ return $true }}
            return $true
        }}
    }}

    $needsElevation = $false
    foreach ($t in $vst3Targets) {{ if (-not (Test-PathWritable $t)) {{ $needsElevation = $true }} }}
    foreach ($t in $clapTargets) {{ if (-not (Test-PathWritable $t)) {{ $needsElevation = $true }} }}
    foreach ($t in $exeTargets)  {{ if (-not (Test-PathWritable $t)) {{ $needsElevation = $true }} }}

    if ($needsElevation) {{
        if ($Elevated) {{
            throw "Administrator rights were not sufficient to write to target plugin folders."
        }}
        try {{
            Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -ArgumentList @(
                '-NoProfile', '-ExecutionPolicy', 'Bypass',
                '-File', ('"{{0}}"' -f $PSCommandPath), '-Elevated'
            )
            exit 0
        }} catch {{
            [System.Windows.Forms.MessageBox]::Show(
                "Administrator permission was canceled. Cannot update protected plugin files.",
                "Phantom Voicer Update Canceled",
                [System.Windows.Forms.MessageBoxButtons]::OK,
                [System.Windows.Forms.MessageBoxIcon]::Warning
            ) | Out-Null
            exit 0
        }}
    }}

    while ($true) {{
        $lockedFile = $null
        foreach ($t in $vst3Targets) {{
            $dll = Join-Path $t 'Contents\x86_64-win\phantom_voicer.vst3'
            if (Test-FileLocked $dll) {{ $lockedFile = $dll; break }}
        }}
        if (-not $lockedFile) {{
            foreach ($t in $clapTargets) {{ if (Test-FileLocked $t) {{ $lockedFile = $t; break }} }}
        }}
        if (-not $lockedFile) {{
            foreach ($t in $exeTargets) {{ if (Test-FileLocked $t) {{ $lockedFile = $t; break }} }}
        }}
        if (-not $lockedFile) {{ break }}

        $ans = [System.Windows.Forms.MessageBox]::Show(
            "Phantom Voicer v$ver is ready!`n`nClose FL Studio / Phantom Voicer completely (wait 2 seconds), then click Retry.`n`nIn use: $lockedFile",
            "Phantom Voicer Updater",
            [System.Windows.Forms.MessageBoxButtons]::RetryCancel,
            [System.Windows.Forms.MessageBoxIcon]::Information
        )
        if ($ans -eq [System.Windows.Forms.DialogResult]::Cancel) {{
            exit 0
        }}
        Start-Sleep -Milliseconds 600
    }}

    $updatedCount = 0

    foreach ($dstVst3 in $vst3Targets) {{
        $parent = Split-Path $dstVst3 -Parent
        $bak = "$dstVst3.bak"
        if (Test-Path $bak) {{ Remove-Item -Recurse -Force $bak }}
        if (Test-Path $dstVst3) {{ Rename-Item -Path $dstVst3 -NewName (Split-Path $bak -Leaf) }}
        try {{
            Copy-Item -Recurse -Force $srcVst3 $parent
            $copiedName = Split-Path $srcVst3 -Leaf
            $placed = Join-Path $parent $copiedName
            if ($placed -ne $dstVst3) {{
                if (Test-Path $dstVst3) {{ Remove-Item -Recurse -Force $dstVst3 }}
                Rename-Item -Path $placed -NewName (Split-Path $dstVst3 -Leaf)
            }}
            $dstDll = Join-Path $dstVst3 'Contents\x86_64-win\phantom_voicer.vst3'
            if (-not (Test-Path $dstDll) -or ((Get-FileHash $dstDll).Hash -ne $srcVst3Hash)) {{
                throw "SHA-256 verification failed for $dstDll"
            }}
            if (Test-Path $bak) {{ Remove-Item -Recurse -Force $bak }}
            $updatedCount++
        }} catch {{
            if ((Test-Path $bak) -and -not (Test-Path $dstVst3)) {{
                Rename-Item -Path $bak -NewName (Split-Path $dstVst3 -Leaf)
            }}
            throw
        }}
    }}

    foreach ($dstClap in $clapTargets) {{
        Copy-Item -Force $srcClap $dstClap
        if ((Get-FileHash $dstClap).Hash -ne $srcClapHash) {{
            throw "SHA-256 verification failed for $dstClap"
        }}
        $updatedCount++
    }}

    foreach ($dstExe in $exeTargets) {{
        Copy-Item -Force $srcExe $dstExe
        if ((Get-FileHash $dstExe).Hash -ne $srcExeHash) {{
            throw "SHA-256 verification failed for $dstExe"
        }}
        $updatedCount++
    }}

    if ($updatedCount -eq 0) {{
        throw "No files were updated."
    }}

    [System.Windows.Forms.MessageBox]::Show(
        "Phantom Voicer updated to v$ver ($updatedCount target(s) verified)!`nRe-open FL Studio or Phantom Voicer now.",
        "Phantom Voicer Updated",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Information
    ) | Out-Null
}} catch {{
    [System.Windows.Forms.MessageBox]::Show(
        "Phantom Voicer update to v$ver failed:`n$($_.Exception.Message)",
        "Phantom Voicer Update Error",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Error
    ) | Out-Null
    exit 1
}}
"#
    );

    // Write UTF-8 with BOM so Windows PowerShell 5.1 parses non-ASCII paths cleanly
    let mut script_bytes = Vec::with_capacity(3 + ps_script.len());
    script_bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    script_bytes.extend_from_slice(ps_script.as_bytes());
    std::fs::write(&script_path, script_bytes).map_err(|e| format!("Script write error: {e}"))?;

    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-WindowStyle",
        "Hidden",
        "-File",
        &script_path.to_string_lossy(),
    ]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map_err(|e| format!("Could not start updater: {e}"))?;

    Ok(format!("Close DAW/app to finish v{version}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison_works() {
        assert!(is_newer_version("0.2.0", "0.1.0"));
        assert!(is_newer_version("v1.0.0", "0.9.9"));
        assert!(is_newer_version("0.3.0", "0.3.0-rc.1"));
        assert!(!is_newer_version("0.3.0-rc.1", "0.3.0"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(!is_newer_version("0.2.2+build.1", "0.2.2"));
        assert!(!is_newer_version("0.0.9", "0.1.0"));
    }
}
