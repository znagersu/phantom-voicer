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
    UpToDate { stale_copies: usize },
    Available {
        version: String,
        zip_url: String,
        stale_copies: usize,
    },
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
            let stale_copies = count_duplicate_plugin_copies();
            let next = match fetch_latest_manifest() {
                Ok(manifest) => {
                    if is_newer_version(&manifest.version, CURRENT_VERSION) {
                        UpdateStatus::Available {
                            version: manifest.version,
                            zip_url: manifest.zip_url,
                            stale_copies,
                        }
                    } else {
                        UpdateStatus::UpToDate { stale_copies }
                    }
                }
                Err(err) => {
                    if stale_copies > 0 {
                        UpdateStatus::UpToDate { stale_copies }
                    } else {
                        UpdateStatus::Error(err)
                    }
                }
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

    /// Cleanses all duplicate / stale plugin copies on disk using the currently running install.
    pub fn cleanse_async(&self) {
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
            *guard = UpdateStatus::Installing("Cleanse".to_string());
        }

        let state = self.state.clone();
        std::thread::spawn(move || {
            let result = launch_cleanse_only();
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

fn detect_active_target() -> Result<(PathBuf, PathBuf), String> {
    let mod_path = current_module_path().ok_or("Could not locate plugin path")?;

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

    let parent = mod_path
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Missing install directory".to_string())?;
    Ok((mod_path, parent))
}

fn collect_scan_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for var in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "LOCALAPPDATA",
        "USERPROFILE",
    ] {
        if let Ok(val) = std::env::var(var) {
            let base = PathBuf::from(val);
            match var {
                "ProgramFiles" | "ProgramFiles(x86)" => {
                    roots.push(base.join("Common Files\\VST3"));
                    roots.push(base.join("Common Files\\CLAP"));
                    roots.push(base.join("VST3"));
                    roots.push(base.join("Steinberg\\VstPlugins"));
                }
                "LOCALAPPDATA" => {
                    roots.push(base.join("Programs\\Common\\VST3"));
                    roots.push(base.join("Programs\\Common\\CLAP"));
                }
                "USERPROFILE" => {
                    roots.push(base.join("Documents\\Image-Line"));
                }
                _ => {}
            }
        }
    }
    roots
}

fn scan_dir_for_plugins(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 || !dir.is_dir() {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_ascii_lowercase();
            if lower == "phantom_voicer.vst3" && p.is_dir() {
                out.push(p);
                continue; // Do not recurse inside bundle (avoids counting nested legacy dirs separately)
            }
            if lower == "phantom_voicer.clap" && p.is_file() {
                out.push(p);
                continue;
            }
        }
        if p.is_dir() {
            scan_dir_for_plugins(&p, depth - 1, out);
        }
    }
}

/// Counts any extra/stale plugin copies in VST3/CLAP folders beyond the single active copy.
pub fn count_duplicate_plugin_copies() -> usize {
    let active = match detect_active_target() {
        Ok((t, _)) => t,
        Err(_) => return 0,
    };
    let active_canon = std::fs::canonicalize(&active).unwrap_or(active);

    let mut found = Vec::new();
    for root in collect_scan_roots() {
        scan_dir_for_plugins(&root, 4, &mut found);
    }

    let mut seen_canon = Vec::new();
    let mut extra = 0usize;
    for path in found {
        let canon = std::fs::canonicalize(&path).unwrap_or(path.clone());
        if seen_canon.contains(&canon) {
            continue;
        }
        seen_canon.push(canon.clone());
        if canon != active_canon {
            extra += 1;
        } else {
            // Also flag if active bundle itself contains a legacy nested phantom_voicer.vst3\phantom_voicer.vst3
            if path.is_dir() && path.join("phantom_voicer.vst3").exists() {
                extra += 1;
            }
        }
    }
    extra
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

    spawn_updater_gui(version, &extract_dir, &active_target, &install_dir, false)?;
    Ok(format!("Reload plugin -> v{version}"))
}

fn launch_cleanse_only() -> Result<String, String> {
    let (active_target, install_dir) = detect_active_target()?;
    let temp_root = std::env::temp_dir().join(format!(
        "PhantomVoicerCleanse-{}-{}",
        std::process::id(),
        unix_millis()
    ));
    std::fs::create_dir_all(&temp_root).map_err(|e| format!("Temp dir error: {e}"))?;
    let stage_dir = temp_root.join("stage");
    std::fs::create_dir_all(&stage_dir).map_err(|e| format!("Stage dir error: {e}"))?;

    // Stage the active healthy bundle so cleanse can safely rebuild canonical path & delete all dupes
    let active_lower = active_target.to_string_lossy().to_ascii_lowercase();
    if active_lower.ends_with(".vst3") && active_target.is_dir() {
        let copy = hidden_command("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Copy-Item -Recurse -Force '{}' '{}'",
                    active_target.to_string_lossy().replace('\'', "''"),
                    stage_dir.to_string_lossy().replace('\'', "''")
                ),
            ])
            .output()
            .map_err(|e| format!("Stage failed: {e}"))?;
        if !copy.status.success() {
            return Err("Could not stage active VST3".to_string());
        }
    }

    spawn_updater_gui(CURRENT_VERSION, &stage_dir, &active_target, &install_dir, true)?;
    Ok("Finish cleanse in popup".to_string())
}

fn spawn_updater_gui(
    version: &str,
    src_dir: &Path,
    active_target: &Path,
    install_dir: &Path,
    cleanse_only: bool,
) -> Result<(), String> {
    let script_path = src_dir
        .parent()
        .unwrap_or(src_dir)
        .join("apply_update.ps1");

    let install_str = install_dir.to_string_lossy().replace('\'', "''");
    let active_str = active_target.to_string_lossy().replace('\'', "''");
    let extract_str = src_dir.to_string_lossy().replace('\'', "''");
    let ver_str = version.replace('\'', "''");
    let cleanse_flag = if cleanse_only { "$true" } else { "$false" };

    // TopMost visible WinForms updater that:
    // 1) Discovers ALL phantom_voicer.* copies (standard VST3, CLAP, VST2, Image-Line, Downloads, Desktop).
    // 2) Auto-elevates via UAC if any location requires Admin.
    // 3) NEVER forces the user to close FL Studio first: if a DLL/EXE is locked in RAM, NTFS Move-Item
    //    moves the locked file/bundle out of the VST3/CLAP folder into a temp trash folder immediately,
    //    then installs the fresh verified vX.Y.Z bundle into the canonical path!
    // 4) Verifies SHA-256 hash before declaring success.
    let ps_script = format!(
        r#"param([switch]$Elevated)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

try {{
    $src = '{extract_str}'
    $dst = '{install_str}'
    $activeTarget = '{active_str}'
    $ver = '{ver_str}'
    $cleanseOnly = {cleanse_flag}

    $srcVst3 = Join-Path $src 'phantom_voicer.vst3'
    $srcClap = Join-Path $src 'phantom_voicer.clap'
    $srcExe  = Join-Path $src 'phantom_voicer.exe'
    $srcVst3Hash = if (Test-Path (Join-Path $srcVst3 'Contents\x86_64-win\phantom_voicer.vst3')) {{ (Get-FileHash (Join-Path $srcVst3 'Contents\x86_64-win\phantom_voicer.vst3')).Hash }} else {{ $null }}
    $srcClapHash = if (Test-Path $srcClap) {{ (Get-FileHash $srcClap).Hash }} else {{ $null }}
    $srcExeHash  = if (Test-Path $srcExe)  {{ (Get-FileHash $srcExe).Hash }}  else {{ $null }}

    $canonicalVst3 = if ($activeTarget -match '\.vst3$') {{ $activeTarget }} else {{ Join-Path "$env:LOCALAPPDATA\Programs\Common\VST3" 'phantom_voicer.vst3' }}
    $downloadsFolder = Join-Path "$env:USERPROFILE\Downloads" 'PhantomVoicer'

    $allVst3 = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)
    $allClap = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)
    $allExe  = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)
    $allBak  = New-Object System.Collections.Generic.HashSet[string]([StringComparer]::OrdinalIgnoreCase)

    if ($activeTarget -match '\.vst3$') {{ [void]$allVst3.Add($activeTarget) }}
    elseif ($activeTarget -match '\.clap$') {{ [void]$allClap.Add($activeTarget) }}
    elseif ($activeTarget -match '\.exe$') {{ [void]$allExe.Add($activeTarget) }}

    $roots = @(
        $dst,
        "$env:ProgramFiles\Common Files\VST3",
        "${{env:ProgramFiles(x86)}}\Common Files\VST3",
        "$env:ProgramFiles\VST3",
        "$env:ProgramFiles\Steinberg\VstPlugins",
        "$env:ProgramFiles\Common Files\CLAP",
        "$env:LOCALAPPDATA\Programs\Common\VST3",
        "$env:LOCALAPPDATA\Programs\Common\CLAP",
        "$env:USERPROFILE\Documents\Image-Line",
        "$env:USERPROFILE\Downloads",
        "$env:USERPROFILE\Desktop"
    ) | Where-Object {{ $_ -and (Test-Path $_) }} | Select-Object -Unique

    foreach ($root in $roots) {{
        Get-ChildItem -Path $root -Filter 'phantom_voicer.vst3' -Directory -Recurse -Depth 3 -ErrorAction SilentlyContinue | ForEach-Object {{
            if ($_.Parent.Name -ne 'phantom_voicer.vst3') {{ [void]$allVst3.Add($_.FullName) }}
        }}
        Get-ChildItem -Path $root -Filter 'phantom_voicer.vst3.bak' -Directory -Recurse -Depth 3 -ErrorAction SilentlyContinue | ForEach-Object {{
            [void]$allBak.Add($_.FullName)
        }}
        Get-ChildItem -Path $root -Filter 'phantom_voicer.clap' -File -Recurse -Depth 3 -ErrorAction SilentlyContinue | ForEach-Object {{
            [void]$allClap.Add($_.FullName)
        }}
        Get-ChildItem -Path $root -Filter 'phantom_voicer.exe' -File -Recurse -Depth 3 -ErrorAction SilentlyContinue | ForEach-Object {{
            [void]$allExe.Add($_.FullName)
        }}
    }}

    function Test-DirWritable([string]$dir) {{
        if (-not (Test-Path $dir)) {{ return $true }}
        $probe = Join-Path $dir ('.pv_w_' + [guid]::NewGuid().ToString('N'))
        try {{
            [System.IO.File]::WriteAllText($probe, 'ok')
            Remove-Item -Force $probe -ErrorAction SilentlyContinue
            return $true
        }} catch {{
            return $false
        }}
    }}

    $needsElevation = -not (Test-DirWritable (Split-Path $canonicalVst3 -Parent))
    foreach ($t in $allVst3) {{ if (-not (Test-DirWritable (Split-Path $t -Parent))) {{ $needsElevation = $true }} }}
    foreach ($t in $allClap) {{ if (-not (Test-DirWritable (Split-Path $t -Parent))) {{ $needsElevation = $true }} }}
    foreach ($t in $allExe)  {{ if (-not (Test-DirWritable (Split-Path $t -Parent))) {{ $needsElevation = $true }} }}

    if ($needsElevation) {{
        if ($Elevated) {{
            throw "Administrator rights were not sufficient to modify protected plugin folders."
        }}
        try {{
            Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -ArgumentList @(
                '-NoProfile', '-ExecutionPolicy', 'Bypass',
                '-File', ('"{{0}}"' -f $PSCommandPath), '-Elevated'
            )
            exit 0
        }} catch {{
            [System.Windows.Forms.MessageBox]::Show(
                "Administrator permission was canceled. Cannot update/cleanse protected plugin files.",
                "Phantom Voicer Updater",
                [System.Windows.Forms.MessageBoxButtons]::OK,
                [System.Windows.Forms.MessageBoxIcon]::Warning,
                [System.Windows.Forms.MessageBoxDefaultButton]::Button1,
                [System.Windows.Forms.MessageBoxOptions]::DefaultDesktopOnly
            ) | Out-Null
            exit 0
        }}
    }}

    # Best-effort cleanup of old trash folders from prior hot-swaps
    Get-ChildItem -Path $env:TEMP -Filter 'PhantomVoicerTrash-*' -Directory -ErrorAction SilentlyContinue | ForEach-Object {{
        Remove-Item -Recurse -Force $_.FullName -ErrorAction SilentlyContinue
    }}

    $trashRoot = Join-Path $env:TEMP ('PhantomVoicerTrash-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force -Path $trashRoot | Out-Null

    function Remove-OrMoveLocked([string]$targetPath) {{
        if (-not (Test-Path $targetPath)) {{ return $false }}
        try {{
            Remove-Item -Recurse -Force $targetPath -ErrorAction Stop
            return $true
        }} catch {{
            # Windows locks delete on loaded DLLs, but allows moving them out of the VST3/CLAP folder on the same volume!
            $srcRoot = [System.IO.Path]::GetPathRoot($targetPath)
            $tmpRoot = [System.IO.Path]::GetPathRoot($trashRoot)
            $destDir = if ($srcRoot -ieq $tmpRoot) {{
                $trashRoot
            }} else {{
                $alt = Join-Path $srcRoot 'PhantomVoicerTrash'
                New-Item -ItemType Directory -Force -Path $alt | Out-Null
                $alt
            }}
            $destName = (Split-Path $targetPath -Leaf) + '.' + [guid]::NewGuid().ToString('N').Substring(0, 6)
            Move-Item -Path $targetPath -Destination (Join-Path $destDir $destName) -Force -ErrorAction Stop
            return $true
        }}
    }}

    $cleansed = 0
    $hotSwapped = $false

    foreach ($b in $allBak) {{
        if (Remove-OrMoveLocked $b) {{ $cleansed++ }}
    }}

    foreach ($oldV in $allVst3) {{
        if (-not (Test-Path $oldV)) {{ continue }}
        if (-not $srcVst3Hash -and $oldV.Equals($canonicalVst3, [StringComparison]::OrdinalIgnoreCase)) {{
            $nested = Join-Path $oldV 'phantom_voicer.vst3'
            if (Test-Path $nested) {{
                if (Remove-OrMoveLocked $nested) {{ $cleansed++ }}
            }}
            continue
        }}
        $wasLocked = $false
        $dll = Join-Path $oldV 'Contents\x86_64-win\phantom_voicer.vst3'
        if (Test-Path $dll) {{
            try {{
                $fs = [System.IO.File]::Open($dll, 'Open', 'ReadWrite', 'None')
                $fs.Close()
            }} catch {{
                $wasLocked = $true
                $hotSwapped = $true
            }}
        }}
        if (Remove-OrMoveLocked $oldV) {{
            if (-not $oldV.Equals($canonicalVst3, [StringComparison]::OrdinalIgnoreCase)) {{
                $cleansed++
            }}
        }}
    }}

    foreach ($oldC in $allClap) {{
        if (Remove-OrMoveLocked $oldC) {{ $cleansed++ }}
    }}

    foreach ($oldE in $allExe) {{
        $dlExe = Join-Path $downloadsFolder 'phantom_voicer.exe'
        if (-not $oldE.Equals($dlExe, [StringComparison]::OrdinalIgnoreCase)) {{
            if (Remove-OrMoveLocked $oldE) {{ $cleansed++ }}
        }}
    }}

    if ($srcVst3Hash) {{
        $canonParent = Split-Path $canonicalVst3 -Parent
        New-Item -ItemType Directory -Force -Path $canonParent | Out-Null
        Copy-Item -Recurse -Force $srcVst3 $canonParent
        $placed = Join-Path $canonParent (Split-Path $srcVst3 -Leaf)
        if ($placed -ne $canonicalVst3) {{
            if (Test-Path $canonicalVst3) {{ [void](Remove-OrMoveLocked $canonicalVst3) }}
            Rename-Item -Path $placed -NewName (Split-Path $canonicalVst3 -Leaf)
        }}
        $dstDll = Join-Path $canonicalVst3 'Contents\x86_64-win\phantom_voicer.vst3'
        $nested = Join-Path $canonicalVst3 'phantom_voicer.vst3'
        if (Test-Path $nested) {{ [void](Remove-OrMoveLocked $nested); $cleansed++ }}
        if (-not (Test-Path $dstDll) -or ((Get-FileHash $dstDll).Hash -ne $srcVst3Hash)) {{
            throw "SHA-256 verification failed for $dstDll"
        }}
    }}

    if ($activeTarget -match '\.exe$' -and $srcExeHash) {{
        [void](Remove-OrMoveLocked $activeTarget)
        Copy-Item -Force $srcExe $activeTarget
        if ((Get-FileHash $activeTarget).Hash -ne $srcExeHash) {{ throw "SHA-256 failed for $activeTarget" }}
    }} elseif ((Test-Path $downloadsFolder) -and $srcExeHash -and -not $cleanseOnly) {{
        $dlExe = Join-Path $downloadsFolder 'phantom_voicer.exe'
        [void](Remove-OrMoveLocked $dlExe)
        Copy-Item -Force $srcExe $dlExe
        if ($srcVst3Hash) {{
            $dlVst3 = Join-Path $downloadsFolder 'phantom_voicer.vst3'
            if (Test-Path $dlVst3) {{ [void](Remove-OrMoveLocked $dlVst3) }}
            Copy-Item -Recurse -Force $srcVst3 $downloadsFolder
        }}
        if ($srcClapHash) {{
            Copy-Item -Force $srcClap (Join-Path $downloadsFolder 'phantom_voicer.clap')
        }}
    }}

    $headline = if ($cleanseOnly) {{ "Phantom Voicer Cleanse Complete!" }} else {{ "Phantom Voicer Updated to v$ver!" }}
    $reloadHint = if ($hotSwapped) {{
        "Delete & re-add Phantom Voicer in FL Studio (if FL still caches the old DLL in RAM, restart FL once)."
    }} else {{
        "Delete & re-add Phantom Voicer in FL Studio to load v$ver!"
    }}

    [System.Windows.Forms.MessageBox]::Show(
        "$headline`r`n`r`nInstalled: $canonicalVst3`r`nCleansed old/duplicate copies: $cleansed`r`n`r`n$reloadHint",
        "Phantom Voicer Updater",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Information,
        [System.Windows.Forms.MessageBoxDefaultButton]::Button1,
        [System.Windows.Forms.MessageBoxOptions]::DefaultDesktopOnly
    ) | Out-Null
}} catch {{
    [System.Windows.Forms.MessageBox]::Show(
        "Phantom Voicer updater failed:`n$($_.Exception.Message)",
        "Phantom Voicer Update Error",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Error,
        [System.Windows.Forms.MessageBoxDefaultButton]::Button1,
        [System.Windows.Forms.MessageBoxOptions]::DefaultDesktopOnly
    ) | Out-Null
    exit 1
}}
"#
    );

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

    Ok(())
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
