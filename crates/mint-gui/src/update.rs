//! Self-updater: checks the GitHub releases of the project and replaces the
//! running executables with the ones from the newest release.
//!
//! Releases are identified by the short commit hash at the end of the release
//! name ("MINT - 1.1.1 - ... - 56a329"). Only `mint-gui` (and `mint-cli`
//! if it sits next to it) are extracted, using HTTP range requests, so even a
//! release zip that bundles a whole build folder is not downloaded in full.

use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "InsanityLabs/MINT_Moc3_INochi_Translator";
pub const COMMIT: &str = env!("MINT_COMMIT");
const COMMIT_TIME: &str = env!("MINT_COMMIT_TIME");
const BLOCK: u64 = 1 << 20;

/// "windows-x64", "linux-x64", "macos-arm64"... as used in release asset names.
pub fn platform_tag() -> String {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        _ => "linux",
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

fn gui_file() -> String {
    format!("mint-gui{}", std::env::consts::EXE_SUFFIX)
}

fn cli_file() -> String {
    format!("mint-cli{}", std::env::consts::EXE_SUFFIX)
}

/// Does `head` look like a program for this OS (PE / ELF / Mach-O)?
fn looks_like_program(head: &[u8]) -> bool {
    match std::env::consts::OS {
        "windows" => head.starts_with(b"MZ"),
        "macos" => matches!(head.get(..4), Some([0xCF, 0xFA, 0xED, 0xFE]) | Some([0xCA, 0xFE, 0xBA, 0xBE]) | Some([0xFE, 0xED, 0xFA, 0xCF])),
        _ => head.starts_with(b"\x7fELF"),
    }
}

/// Pick this platform's zip. Releases made before multi-platform builds had a
/// single untagged zip, which is Windows-only.
fn pick_asset<'a>(assets: &'a [Value], tag: &str) -> Option<&'a Value> {
    let zips = || assets.iter().filter(|a| a["name"].as_str().is_some_and(|n| n.to_lowercase().ends_with(".zip")));
    zips()
        .find(|a| a["name"].as_str().is_some_and(|n| n.to_lowercase().contains(tag)))
        .or_else(|| {
            let tagged = |a: &&Value| a["name"].as_str().is_some_and(|n| ["windows-", "linux-", "macos-"].iter().any(|t| n.to_lowercase().contains(t)));
            (std::env::consts::OS == "windows").then(|| zips().filter(|a| !tagged(a)).min_by_key(|a| a["size"].as_u64().unwrap_or(u64::MAX))).flatten()
        })
}

/// Which releases to follow: tagged releases, or the rolling `nightly` pre-release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Channel {
    #[default]
    Stable,
    Nightly,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub name: String,
    pub commit: String,
    asset_url: String,
    asset_size: u64,
}

#[derive(Clone, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { done: u64, total: u64 },
    Ready,
    Failed(String),
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(concat!("MINT/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build()
}

/// Seconds since the epoch for "YYYY-MM-DDTHH:MM:SSZ".
fn iso_to_unix(s: &str) -> Option<i64> {
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146097 + doe - 719468) * 86400 + hh * 3600 + mm * 60 + ss)
}

fn is_hash(s: &str) -> bool {
    (6..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Short commit hash a release was built from (last token of its name or tag).
fn release_commit(name: &str, tag: &str) -> Option<String> {
    let from_name = name.split(|c: char| c.is_whitespace() || c == '-').filter(|t| !t.is_empty()).last();
    let from_tag = tag.rsplit('-').next();
    [from_name, from_tag].into_iter().flatten().find(|t| is_hash(t)).map(|t| t.to_lowercase())
}

fn same_commit(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    a.starts_with(&b) || b.starts_with(&a)
}

/// Newest nightly pre-release in a `releases` listing (GitHub returns newest first).
fn pick_nightly(list: &Value) -> Option<&Value> {
    list.as_array()?.iter().find(|r| {
        r["prerelease"].as_bool() == Some(true)
            && r["draft"].as_bool() != Some(true)
            && r["tag_name"].as_str().is_some_and(|t| t.starts_with("nightly-"))
    })
}

/// Is a newer release than this build published? `Ok(None)` = up to date.
pub fn check(channel: Channel) -> Result<Option<Release>, String> {
    check_with(COMMIT, COMMIT_TIME.parse().unwrap_or(0), channel)
}

pub fn check_with(local_commit: &str, local_time: i64, channel: Channel) -> Result<Option<Release>, String> {
    if local_commit == "unknown" {
        return Ok(None);
    }
    // `latest` skips pre-releases, so nightly builds never reach stable users.
    let base = format!("https://api.github.com/repos/{REPO}/releases");
    let get = |url: &str| -> Result<Option<Value>, String> {
        match agent().get(url).set("Accept", "application/vnd.github+json").call() {
            Ok(r) => r.into_json().map(Some).map_err(|e| format!("bad update response: {e}")),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(format!("update check failed: {e}")),
        }
    };
    let found = match channel {
        Channel::Stable => get(&format!("{base}/latest"))?,
        Channel::Nightly => get(&format!("{base}?per_page=30"))?.and_then(|list| pick_nightly(&list).cloned()),
    };
    let Some(v) = found else { return Ok(None) };
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let (name, tag) = (s("name"), s("tag_name"));
    let Some(commit) = release_commit(&name, &tag) else {
        return Err(format!("can't tell which commit release \"{name}\" is built from"));
    };
    let published = iso_to_unix(&s("published_at")).unwrap_or(0);
    if same_commit(local_commit, &commit) || published <= local_time {
        return Ok(None);
    }
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let tag = platform_tag();
    let asset = pick_asset(&assets, &tag).ok_or_else(|| format!("the newest release has no build for {tag}"))?;
    Ok(Some(Release {
        name,
        commit,
        asset_url: asset["browser_download_url"].as_str().unwrap_or("").to_string(),
        asset_size: asset["size"].as_u64().unwrap_or(0),
    }))
}

/// Seekable reader over an HTTP resource using range requests.
struct Remote {
    agent: ureq::Agent,
    url: String,
    origin: String,
    size: u64,
    pos: u64,
    block: Option<(u64, Vec<u8>)>,
}

impl Remote {
    fn open(agent: ureq::Agent, origin: &str, size: u64) -> Result<Self, String> {
        let mut r = Remote { agent, url: origin.to_string(), origin: origin.to_string(), size, pos: 0, block: None };
        r.resolve()?;
        Ok(r)
    }

    /// Follow the redirect once so later range requests hit the final URL.
    fn resolve(&mut self) -> Result<(), String> {
        let resp = self.agent.get(&self.origin).set("Range", "bytes=0-0").call().map_err(|e| format!("download failed: {e}"))?;
        self.url = resp.get_url().to_string();
        if let Some(total) = resp.header("Content-Range").and_then(|h| h.rsplit('/').next()).and_then(|t| t.parse().ok()) {
            self.size = total;
        }
        Ok(())
    }

    fn fetch(&mut self, start: u64) -> std::io::Result<Vec<u8>> {
        let end = (start + BLOCK).min(self.size) - 1;
        let mut last = None;
        for attempt in 0..3 {
            let res = self.agent.get(&self.url).set("Range", &format!("bytes={start}-{end}")).call();
            match res {
                Ok(resp) => {
                    let mut buf = Vec::with_capacity((end - start + 1) as usize);
                    resp.into_reader().read_to_end(&mut buf)?;
                    return Ok(buf);
                }
                Err(e) => {
                    last = Some(e.to_string());
                    if attempt == 0 {
                        let _ = self.resolve();
                    }
                }
            }
        }
        Err(std::io::Error::other(last.unwrap_or_default()))
    }
}

impl Read for Remote {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.size || out.is_empty() {
            return Ok(0);
        }
        let start = self.pos / BLOCK * BLOCK;
        if self.block.as_ref().map(|b| b.0) != Some(start) {
            let data = self.fetch(start)?;
            self.block = Some((start, data));
        }
        let (bs, data) = self.block.as_ref().unwrap();
        let off = (self.pos - bs) as usize;
        let n = out.len().min(data.len().saturating_sub(off));
        out[..n].copy_from_slice(&data[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Remote {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let p = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.size as i64 + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if p < 0 {
            return Err(std::io::Error::other("seek before start"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

/// Extract the first zip entry whose file name is `file` (not from `deps/`).
fn extract(zip: &mut zip::ZipArchive<Remote>, file: &str, dest: &Path, progress: &dyn Fn(u64)) -> Result<bool, String> {
    let idx = (0..zip.len())
        .filter_map(|i| zip.name_for_index(i).map(|n| (i, n.to_string())))
        .filter(|(_, n)| n.rsplit('/').next() == Some(file) && !n.contains("/deps/"))
        .min_by_key(|(_, n)| n.len())
        .map(|(i, _)| i);
    let Some(i) = idx else { return Ok(false) };
    let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
    let mut out = std::fs::File::create(dest).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = entry.read(&mut buf).map_err(|e| format!("download failed: {e}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut out, &buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done);
    }
    drop(out);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755));
    }
    let head = std::fs::read(dest).map_err(|e| e.to_string())?;
    if head.len() < 1_000_000 || !looks_like_program(&head) {
        let _ = std::fs::remove_file(dest);
        return Err(format!("{file} in the release does not look like a valid program"));
    }
    Ok(true)
}

/// Replace `target` with `new_file`. A running exe can be renamed on Windows,
/// so the old one is moved to `<name>.old` and removed on the next start.
fn swap(target: &Path, new_file: &Path) -> Result<(), String> {
    let old = old_path(target);
    let _ = std::fs::remove_file(&old);
    if target.exists() {
        std::fs::rename(target, &old).map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
    }
    if let Err(e) = std::fs::rename(new_file, target) {
        let _ = std::fs::rename(&old, target);
        return Err(format!("cannot install the update: {e}"));
    }
    Ok(())
}

fn old_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Download the release and swap it in for `target` (and the CLI beside it).
pub fn install(rel: &Release, target: &Path, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    if !rel.asset_url.starts_with("https://github.com/") {
        return Err("refusing to download from an unexpected address".into());
    }
    let remote = Remote::open(agent(), &rel.asset_url, rel.asset_size)?;
    let mut zip = zip::ZipArchive::new(remote).map_err(|e| format!("cannot read the release archive: {e}"))?;
    let dir = target.parent().ok_or("no install folder")?;
    let gui_name = target.file_name().and_then(|n| n.to_str()).unwrap_or(&gui_file()).to_string();
    let total = (0..zip.len())
        .filter_map(|i| zip.by_index_raw(i).ok().map(|e| (e.name().to_string(), e.size())))
        .filter(|(n, _)| n.rsplit('/').next() == Some(gui_name.as_str()) && !n.contains("/deps/"))
        .map(|(_, s)| s)
        .next()
        .unwrap_or(rel.asset_size);

    let gui_new = dir.join(format!("{gui_name}.new"));
    if !extract(&mut zip, &gui_name, &gui_new, &|d| progress(d, total))? {
        return Err(format!("{gui_name} is not in the release archive"));
    }
    let cli = dir.join(cli_file());
    let cli_new = dir.join(format!("{}.new", cli_file()));
    let cli_updated = cli.exists() && extract(&mut zip, &cli_file(), &cli_new, &|_| {}).unwrap_or(false);
    swap(target, &gui_new)?;
    if cli_updated {
        let _ = swap(&cli, &cli_new);
    }
    Ok(())
}

/// Remove leftovers of a previous update (best effort).
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
        if let Some(dir) = exe.parent() {
            let _ = std::fs::remove_file(old_path(&dir.join(cli_file())));
        }
    }
}

fn settings_path() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = match std::env::consts::OS {
        "windows" => std::env::var_os("APPDATA").map(PathBuf::from),
        "macos" => home().map(|h| h.join("Library").join("Application Support")),
        _ => std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config"))),
    };
    base.unwrap_or_else(std::env::temp_dir).join("MINT").join("settings.json")
}

fn read_settings() -> Value {
    std::fs::read(settings_path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or_else(|| serde_json::json!({}))
}

fn write_setting(key: &str, value: Value) {
    let mut v = read_settings();
    v[key] = value;
    let p = settings_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, v.to_string());
}

pub fn auto_check_enabled() -> bool {
    read_settings()["check_updates"].as_bool().unwrap_or(true)
}

pub fn set_auto_check(on: bool) {
    write_setting("check_updates", Value::Bool(on));
}

pub fn channel() -> Channel {
    if read_settings()["channel"].as_str() == Some("nightly") { Channel::Nightly } else { Channel::Stable }
}

pub fn set_channel(c: Channel) {
    write_setting("channel", Value::String(if c == Channel::Nightly { "nightly" } else { "stable" }.into()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_names() {
        assert_eq!(release_commit("MINT - 1.1.1 -1112026212 - 56a329", "MINT-1.1.156a329").as_deref(), Some("56a329"));
        assert_eq!(release_commit("MINT - 0.1.1 - 1112026146 - 7676af8", "MINT-1.1-7676af8").as_deref(), Some("7676af8"));
        assert!(same_commit("56a329a", "56a329"));
        assert!(!same_commit("56a329a", "7676af8"));
    }

    #[test]
    fn picks_newest_nightly_only() {
        let list = serde_json::json!([
            { "tag_name": "v1.2.0", "prerelease": false, "draft": false },
            { "tag_name": "nightly-20261103-0600-bbbbbbb", "prerelease": true, "draft": false },
            { "tag_name": "nightly-20261102-0600-aaaaaaa", "prerelease": true, "draft": false },
            { "tag_name": "v1.3.0-rc1", "prerelease": true, "draft": false }
        ]);
        assert_eq!(pick_nightly(&list).unwrap()["tag_name"], "nightly-20261103-0600-bbbbbbb");
        assert!(pick_nightly(&serde_json::json!([{ "tag_name": "v1.0.0", "prerelease": false }])).is_none());
    }

    #[test]
    fn picks_platform_asset() {
        let assets: Vec<Value> = serde_json::from_str(
            r#"[{"name":"MINT-linux-x64-abc.zip","size":5},{"name":"MINT-macos-arm64-abc.zip","size":6},
                {"name":"MINT-windows-x64-abc.zip","size":7},{"name":"MINT-windows-x64-abc.zip.sha256","size":1}]"#,
        )
        .unwrap();
        assert_eq!(pick_asset(&assets, "macos-arm64").unwrap()["size"], 6);
        assert_eq!(pick_asset(&assets, "linux-x64").unwrap()["size"], 5);
        assert!(pick_asset(&assets, "macos-x64").is_none() || std::env::consts::OS == "windows");
        // legacy single untagged zip counts as the Windows build
        let legacy: Vec<Value> = serde_json::from_str(r#"[{"name":"MINT.-.1.1.1.zip","size":9}]"#).unwrap();
        assert_eq!(pick_asset(&legacy, "windows-x64").is_some(), std::env::consts::OS == "windows");
    }

    #[test]
    fn parses_iso_time() {
        assert_eq!(iso_to_unix("1970-01-02T00:00:00Z"), Some(86400));
        assert_eq!(iso_to_unix("2026-11-01T07:15:35Z"), Some(1793517335));
    }

    /// Network test against the real releases: `cargo test -p mint-gui -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_check_and_install() {
        // Same commit as the newest release: nothing to do.
        if platform_tag() != "windows-x64" {
            return; // the existing release only has a Windows build
        }
        assert!(check_with("56a329a", 1793517000, Channel::Stable).unwrap().is_none());
        // An older build: the newest release is offered.
        let rel = check_with("0000000", 1, Channel::Stable).unwrap().expect("an update");
        println!("offered: {} ({})", rel.name, rel.commit);
        let dir = std::env::temp_dir().join("mint_update_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join(gui_file());
        std::fs::write(&target, b"old build").unwrap();
        let last = std::cell::Cell::new(0u64);
        install(&rel, &target, &|d, _| last.set(d)).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        assert!(looks_like_program(&bytes));
        assert!(bytes.len() > 1_000_000);
        assert_eq!(std::fs::read(old_path(&target)).unwrap(), b"old build");
        println!("installed {} bytes", bytes.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
