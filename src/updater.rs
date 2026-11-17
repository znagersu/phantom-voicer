use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

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
            if matches!(*guard, UpdateStatus::Checking | UpdateStatus::Installing(_)) {
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
            if matches!(*guard, UpdateStatus::Installing(_)) {
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

fn parse_semver(v: &str) -> Option<(u32, u32, u32)> {
    let clean = v.trim().trim_start_matches(['v', 'V']);
    let mut parts = clean.split('.');
    let maj = parts.next()?.parse().ok()?;
    let min = parts.next().unwrap_or("0").parse().ok()?;
    let pat = parts
        .next()
        .unwrap_or("0")
        .split('-')
        .next()?
        .parse()
        .ok()?;
    Some((maj, min, pat))
}

pub fn is_newer_version(remote: &str, current: &str) -> bool {
    match (parse_semver(remote), parse_semver(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
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
    let output = hidden_command("curl.exe")
        .args(["-fsSL", "--max-time", "8", LATEST_MANIFEST_URL])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    if !output.status.success() {
        return Err(String::from("Offline or no release yet"));
    }

    serde_json::from_slice::<LatestManifest>(&output.stdout)
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

/// Resolves the directory containing the installed `phantom_voicer.vst3` / `.exe` / `.clap`.
fn detect_install_dir() -> Result<PathBuf, String> {
    let mod_path = current_module_path().ok_or("Could not locate plugin path")?;
    let s = mod_path.to_string_lossy().to_ascii_lowercase();

    // If running inside `...\phantom_voicer.vst3\Contents\x86_64-win\phantom_voicer.vst3`
    if s.contains(".vst3\\contents\\") || s.contains(".vst3/contents/") {
        let mut p: &Path = &mod_path;
        for _ in 0..3 {
            p = p.parent().ok_or("Unexpected VST3 bundle layout")?;
        }
        return p
            .parent()
            .map(|d| d.to_path_buf())
            .ok_or_else(|| "Missing VST3 parent dir".to_string());
    }

    // Running as standalone `.exe` or `.clap`
    mod_path
        .parent()
        .map(|d| d.to_path_buf())
        .ok_or_else(|| "Missing install directory".to_string())
}

fn download_and_launch_updater(version: &str, zip_url: &str) -> Result<String, String> {
    if !zip_url.starts_with("https://github.com/znagersu/phantom-voicer/") {
        return Err("Untrusted release URL".to_string());
    }

    let install_dir = detect_install_dir()?;
    let temp_root = std::env::temp_dir().join("PhantomVoicerUpdate");
    let _ = std::fs::remove_dir_all(&temp_root);
    std::fs::create_dir_all(&temp_root).map_err(|e| format!("Temp dir error: {e}"))?;

    let zip_path = temp_root.join("PhantomVoicer-win64.zip");
    let extract_dir = temp_root.join("extracted");

    let dl = hidden_command("curl.exe")
        .args([
            "-fsSL",
            "-L",
            "--max-time",
            "60",
            "-o",
            &zip_path.to_string_lossy(),
            zip_url,
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
    let extract_str = extract_dir.to_string_lossy().replace('\'', "''");
    let ver_str = version.replace('\'', "''");

    // Helper script waits up to 90s for FL Studio / Phantom Voicer to release file locks,
    // elevates via UAC if installing into Program Files, copies files, and notifies user.
    let ps_script = format!(
        r#"$ErrorActionPreference = 'Stop'
$src = '{extract_str}'
$dst = '{install_str}'
$ver = '{ver_str}'

function Test-Locked($dir) {{
    $targets = @(
        (Join-Path $dir 'phantom_voicer.vst3\Contents\x86_64-win\phantom_voicer.vst3'),
        (Join-Path $dir 'phantom_voicer.exe')
    )
    foreach ($t in $targets) {{
        if (Test-Path $t) {{
            try {{
                $fs = [System.IO.File]::Open($t, 'Open', 'ReadWrite', 'None')
                $fs.Close()
            }} catch {{
                return $true
            }}
        }}
    }}
    return $false
}}

Add-Type -AssemblyName System.Windows.Forms
$warned = $false
for ($i = 0; $i -lt 90; $i++) {{
    if (-not (Test-Locked $dst)) {{ break }}
    if (-not $warned) {{
        $warned = $true
        [System.Windows.Forms.MessageBox]::Show(
            "Phantom Voicer v$ver is ready! Please close FL Studio / Phantom Voicer now, then click OK to finish installing.",
            "Phantom Voicer Updater",
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Information
        ) | Out-Null
    }}
    Start-Sleep -Seconds 1
}}

try {{
    if (Test-Path (Join-Path $src 'phantom_voicer.vst3')) {{
        Copy-Item -Recurse -Force (Join-Path $src 'phantom_voicer.vst3') (Join-Path $dst 'phantom_voicer.vst3')
    }}
    if (Test-Path (Join-Path $src 'phantom_voicer.clap')) {{
        Copy-Item -Force (Join-Path $src 'phantom_voicer.clap') (Join-Path $dst 'phantom_voicer.clap')
    }}
    if (Test-Path (Join-Path $src 'phantom_voicer.exe')) {{
        Copy-Item -Force (Join-Path $src 'phantom_voicer.exe') (Join-Path $dst 'phantom_voicer.exe')
    }}
    [System.Windows.Forms.MessageBox]::Show(
        "Phantom Voicer updated to v$ver! Re-open FL Studio or Phantom Voicer to use the new version.",
        "Phantom Voicer Updated",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Information
    ) | Out-Null
}} catch {{
    [System.Windows.Forms.MessageBox]::Show(
        "Could not overwrite files in $dst.`nClose FL Studio and run as Administrator if installed in Program Files.`nError: $($_.Exception.Message)",
        "Phantom Voicer Update Error",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Error
    ) | Out-Null
}}
"#
    );

    std::fs::write(&script_path, ps_script).map_err(|e| format!("Script write error: {e}"))?;

    // Launch detached PowerShell helper so it survives if user closes FL Studio / standalone exe
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
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(!is_newer_version("0.0.9", "0.1.0"));
    }
}
