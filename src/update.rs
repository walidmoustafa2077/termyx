//! Checking GitHub Releases for a newer Termyx and applying it from the
//! running instance.
//!
//! The update is the release's setup package: it downloads into a temp
//! staging folder, then a detached helper waits for this process to exit
//! before running the installer, so the running executable is never
//! overwritten in place. The network side shells out to Windows PowerShell,
//! which keeps an HTTP client out of the dependency tree.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The releases API the check reads; PowerShell parses the JSON and prints
/// only the tag.
const RELEASES_API: &str = "https://api.github.com/repos/walidmoustafa2077/termyx/releases/latest";

/// Where release assets live, one `v`-prefixed segment per version tag.
const RELEASES_DOWNLOAD: &str = "https://github.com/walidmoustafa2077/termyx/releases/download";

/// The staging folder under the temp directory; cleared before every update.
const STAGING_NAME: &str = "termyx-update";

/// Waits for the running instance to exit, then installs the staged package
/// and starts the new build. `-Yes` keeps setup unattended, and the helper's
/// null stdin keeps it off the animated path that waits for a keypress.
const HELPER: &str = "\
param([string]$Dir, [int]$WaitId)
Wait-Process -Id $WaitId -ErrorAction SilentlyContinue
& (Join-Path $Dir 'setup.ps1') -Yes -Launch
exit $LASTEXITCODE
";

/// What a check or a stage failed with.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("powershell exited with {status}: {message}")]
    PowerShell { status: i32, message: String },
    #[error("the update check returned no release tag")]
    NoTag,
    #[error("{path} could not be prepared: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the downloaded update has no setup.ps1 next to it")]
    MissingSetup,
}

/// True when `latest` names a strictly newer version than `current`.
///
/// Parts compare as numbers, so `0.10.0` is newer than `0.1.9`, and a
/// missing part counts as zero so `0.2` still beats `0.1.9`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(version: &str) -> Vec<u64> {
        version
            .trim()
            .trim_start_matches('v')
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    }
    let latest = parts(latest);
    let current = parts(current);
    let len = latest.len().max(current.len());
    (0..len).any(|index| {
        let left = latest.get(index).copied().unwrap_or(0);
        let right = current.get(index).copied().unwrap_or(0);
        left > right
            && (0..index).all(|earlier| {
                latest.get(earlier).copied().unwrap_or(0)
                    == current.get(earlier).copied().unwrap_or(0)
            })
    })
}

/// The version an API reply names, without its leading `v`.
///
/// Empty output is `None`: an error page must not read as a release tag.
pub fn tag_from(output: &str) -> Option<String> {
    let tag = output.trim();
    if tag.is_empty() {
        return None;
    }
    Some(tag.trim_start_matches('v').to_string())
}

/// The setup asset a release ships for `version`.
pub fn asset_name(version: &str) -> String {
    format!("Termyx-{version}-setup.zip")
}

/// Where that asset downloads from; tags carry a `v`, asset names do not.
pub fn download_url(version: &str) -> String {
    format!("{RELEASES_DOWNLOAD}/v{version}/{}", asset_name(version))
}

/// Asks GitHub for the latest release and reports it when it is newer than
/// the running build.
pub fn check() -> Result<Option<String>, UpdateError> {
    let command = format!(
        "(Invoke-RestMethod -Uri '{RELEASES_API}' -TimeoutSec 30 \
         -Headers @{{'User-Agent'='termyx'}}).tag_name"
    );
    let tag = tag_from(&powershell(&command)?).ok_or(UpdateError::NoTag)?;
    Ok(is_newer(&tag, env!("CARGO_PKG_VERSION")).then_some(tag))
}

/// Downloads the setup package for `version` into the temp staging folder
/// and arms a detached helper to install it once this process exits.
pub fn stage(version: &str) -> Result<(), UpdateError> {
    let dir = staging_dir();
    reset(&dir)?;
    let zip = dir.join(asset_name(version));
    let url = download_url(version);
    let command = format!(
        "Invoke-WebRequest -Uri '{url}' -OutFile '{}' -UseBasicParsing -TimeoutSec 300; \
         Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip.display(),
        zip.display(),
        dir.display()
    );
    powershell(&command)?;
    if !dir.join("setup.ps1").is_file() {
        return Err(UpdateError::MissingSetup);
    }
    arm_helper(&dir)
}

/// The temp folder every update stages into.
fn staging_dir() -> PathBuf {
    std::env::temp_dir().join(STAGING_NAME)
}

/// Starts from an empty staging folder so a failed earlier attempt cannot
/// shadow the fresh download.
fn reset(dir: &Path) -> Result<(), UpdateError> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|source| UpdateError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    std::fs::create_dir_all(dir).map_err(|source| UpdateError::Io {
        path: dir.to_path_buf(),
        source,
    })
}

/// Writes the wait-and-install script into the staging folder and starts it
/// with no window of its own, its output captured for the log beside it.
fn arm_helper(dir: &Path) -> Result<(), UpdateError> {
    let helper = dir.join("update-helper.ps1");
    std::fs::write(&helper, HELPER).map_err(|source| UpdateError::Io {
        path: helper.clone(),
        source,
    })?;
    let log_path = dir.join("update.log");
    let log = std::fs::File::create(&log_path).map_err(|source| UpdateError::Io {
        path: log_path.clone(),
        source,
    })?;
    let err_log = log.try_clone().map_err(|source| UpdateError::Io {
        path: log_path.clone(),
        source,
    })?;
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&helper)
        .arg("-Dir")
        .arg(dir)
        .arg("-WaitId")
        .arg(std::process::id().to_string())
        // Null stdin reports input as redirected, which is what keeps the
        // installer off the animated path: that path ends on a keypress this
        // windowless helper would never receive.
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err_log));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes while the helper waits.
        command.creation_flags(0x0800_0000);
    }
    command.spawn().map_err(|source| UpdateError::Io {
        path: helper,
        source,
    })?;
    Ok(())
}

/// Runs a PowerShell command and returns its standard output.
fn powershell(command: &str) -> Result<String, UpdateError> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command,
        ])
        .output()
        .map_err(|source| UpdateError::Io {
            path: PathBuf::from("powershell.exe"),
            source,
        })?;
    if !output.status.success() {
        let status = output.status.code().unwrap_or(-1);
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(UpdateError::PowerShell { status, message });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strictly_newer_tag_wins_and_equal_or_older_does_not() {
        assert!(is_newer("0.1.3", "0.1.2"));
        assert!(is_newer("0.10.0", "0.1.9"), "parts compare as numbers");
        assert!(is_newer("0.2", "0.1.9"), "a missing part counts as zero");
        assert!(is_newer("v0.1.3", "0.1.2"), "the tag prefix is ignored");
        assert!(!is_newer("0.1.2", "0.1.2"));
        assert!(!is_newer("0.1.1", "0.1.2"));
        assert!(
            !is_newer("0.1.2", "0.1.2.1"),
            "extra current parts beat missing ones"
        );
    }

    #[test]
    fn the_release_tag_is_read_without_its_prefix() {
        assert_eq!(tag_from("v0.1.3\n").as_deref(), Some("0.1.3"));
        assert_eq!(tag_from("0.1.3").as_deref(), Some("0.1.3"));
        assert_eq!(tag_from(""), None);
        assert_eq!(tag_from("  \n"), None);
    }

    #[test]
    fn the_setup_asset_names_the_version_it_stages() {
        assert_eq!(asset_name("0.1.3"), "Termyx-0.1.3-setup.zip");
        assert_eq!(
            download_url("0.1.3"),
            "https://github.com/walidmoustafa2077/termyx/releases/download/v0.1.3/Termyx-0.1.3-setup.zip"
        );
    }
}
