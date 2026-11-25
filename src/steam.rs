//! Everything that touches Steam: finding it, reading the saved accounts, and switching.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

use crate::vdf::{self, Value};

const STEAMID64_BASE: u64 = 76_561_197_960_265_728;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Processes that must be gone before Steam's files can be edited safely.
const STEAM_PROCESSES: [&str; 4] = [
    "steam.exe",
    "steamwebhelper.exe",
    "gameoverlayui.exe",
    "steamerrorreporter.exe",
];

#[derive(Clone, Debug)]
pub struct Account {
    pub id64: u64,
    pub account_name: String,
    pub persona_name: String,
    /// Last sign-in on this PC, in Unix seconds.
    pub timestamp: Option<i64>,
}

impl Account {
    /// The 32-bit account ID Steam uses in the registry.
    pub fn account_id(&self) -> u32 {
        self.id64.wrapping_sub(STEAMID64_BASE) as u32
    }

    /// Profile name, or the login name if the profile name is blank.
    pub fn name(&self) -> &str {
        if self.persona_name.trim().is_empty() {
            &self.account_name
        } else {
            &self.persona_name
        }
    }
}

/// Finds the Steam folder (the one with steam.exe in it).
pub fn find_steam() -> Option<PathBuf> {
    let read = |root: RegKey, path: &str, name: &str| -> Option<PathBuf> {
        let value: String = root.open_subkey(path).ok()?.get_value(name).ok()?;
        Some(PathBuf::from(value.replace('/', "\\")))
    };
    [
        read(
            RegKey::predef(HKEY_CURRENT_USER),
            r"Software\Valve\Steam",
            "SteamPath",
        ),
        read(
            RegKey::predef(HKEY_LOCAL_MACHINE),
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        ),
        Some(PathBuf::from(r"C:\Program Files (x86)\Steam")),
    ]
    .into_iter()
    .flatten()
    .find(|dir| dir.join("steam.exe").is_file())
}

pub fn loginusers_path(steam: &Path) -> PathBuf {
    steam.join("config").join("loginusers.vdf")
}

/// Reads the accounts Steam has saved on this PC, sorted by name.
pub fn load_accounts(steam: &Path) -> Result<Vec<Account>, String> {
    let path = loginusers_path(steam);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("can't read loginusers.vdf ({e})"))?;
    let root = vdf::parse(&text).map_err(|e| format!("can't parse loginusers.vdf ({e})"))?;
    let Some(Value::Obj(users)) = vdf::find(&root, "users") else {
        return Ok(Vec::new());
    };
    let mut accounts: Vec<Account> = users
        .iter()
        .filter_map(|(key, user)| {
            let account_name = user.get_str("AccountName")?.trim().to_owned();
            if account_name.is_empty() {
                return None;
            }
            Some(Account {
                id64: key.parse().ok()?,
                account_name,
                persona_name: user.get_str("PersonaName").unwrap_or_default().to_owned(),
                timestamp: user
                    .get_str("Timestamp")
                    .and_then(|t| t.trim().parse().ok()),
            })
        })
        .collect();
    accounts.sort_by_key(|a| a.name().to_lowercase());
    Ok(accounts)
}

/// Steam keeps a copy of each saved account's profile picture (184x184 PNG).
pub fn load_avatar(steam: &Path, id64: u64) -> Option<image::RgbaImage> {
    let path = steam
        .join("config")
        .join("avatarcache")
        .join(format!("{id64}.png"));
    let bytes = fs::read(path).ok()?;
    Some(image::load_from_memory(&bytes).ok()?.to_rgba8())
}

pub fn is_steam_running() -> bool {
    !find_processes(&["steam.exe"]).is_empty()
}

/// The account Steam is signed in to right now, if any.
pub fn active_account_id() -> Option<u32> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam\ActiveProcess")
        .ok()?;
    let id: u32 = key.get_value("ActiveUser").ok()?;
    (id != 0).then_some(id)
}

/// The login name Steam signs in to by itself on its next start. `None` means it will ask.
pub fn auto_login_name() -> Option<String> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Valve\Steam")
        .ok()?;
    let name: String = key.get_value("AutoLoginUser").ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Restarts Steam signed in to `target`. With `None`, Steam opens its sign-in screen instead.
pub fn switch_to(steam: &Path, target: Option<&Account>) -> Result<(), String> {
    close_steam(steam)?;
    write_loginusers(steam, target.map(|a| a.id64))?;
    write_registry(target.map_or("", |a| a.account_name.as_str()))?;
    Command::new(steam.join("steam.exe"))
        .current_dir(steam)
        .spawn()
        .map_err(|e| format!("couldn't start Steam ({e})"))?;
    Ok(())
}

fn close_steam(steam: &Path) -> Result<(), String> {
    if is_steam_running() {
        // Ask Steam to exit cleanly first. Closing its window would only hide it in the tray.
        let _ = Command::new(steam.join("steam.exe"))
            .arg("-shutdown")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        let deadline = Instant::now() + Duration::from_secs(20);
        while is_steam_running() && Instant::now() < deadline {
            sleep(Duration::from_millis(250));
        }
    }
    let leftovers = find_processes(&STEAM_PROCESSES);
    if !leftovers.is_empty() {
        kill(&leftovers);
        sleep(Duration::from_millis(500));
    }
    if is_steam_running() {
        return Err("Steam didn't close. Is it running as administrator?".into());
    }
    Ok(())
}

fn write_loginusers(steam: &Path, target: Option<u64>) -> Result<(), String> {
    let path = loginusers_path(steam);
    if !path.exists() {
        // Nothing saved yet, which is fine when adding the first account.
        return Ok(());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("can't read loginusers.vdf ({e})"))?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut root = vdf::parse(&text).map_err(|e| format!("can't parse loginusers.vdf ({e})"))?;
    set_auto_login(&mut root, target)?;
    fs::copy(&path, path.with_extension("vdf.bak"))
        .map_err(|e| format!("can't back up loginusers.vdf ({e})"))?;
    let tmp = path.with_extension("vdf.tmp");
    fs::write(&tmp, vdf::write(&root, newline))
        .map_err(|e| format!("can't write loginusers.vdf ({e})"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("can't replace loginusers.vdf ({e})"))
}

/// Marks `target` as the account Steam signs in to on its next start, and every other account as not.
fn set_auto_login(root: &mut [(String, Value)], target: Option<u64>) -> Result<(), String> {
    let Some(Value::Obj(users)) = vdf::find_mut(root, "users") else {
        return Err("loginusers.vdf has no users".into());
    };
    let target = target.map(|id| id.to_string());
    for (id, user) in users.iter_mut() {
        let is_target = target.as_deref() == Some(id.as_str());
        let flag = if is_target { "1" } else { "0" };
        user.set_str("AutoLogin", flag);
        // Older Steam versions use these fields instead.
        if user.get("MostRecent").is_some() {
            user.set_str("MostRecent", flag);
        }
        if is_target {
            user.set_str("RememberPassword", "1");
            if user.get("AllowAutoLogin").is_some() {
                user.set_str("AllowAutoLogin", "1");
            }
        }
    }
    Ok(())
}

fn write_registry(account_name: &str) -> Result<(), String> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(r"Software\Valve\Steam")
        .map_err(|e| format!("can't open Steam's registry key ({e})"))?;
    key.set_value("AutoLoginUser", &account_name.to_owned())
        .and_then(|_| key.set_value("RememberPassword", &1u32))
        .map_err(|e| format!("can't write Steam's registry key ({e})"))
}

/// Process IDs of running programs whose exe name matches one of `names`.
fn find_processes(names: &[&str]) -> Vec<u32> {
    let mut pids = Vec::new();
    // SAFETY: a plain Win32 process snapshot walk; the handle is closed before returning.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return pids;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if names.iter().any(|n| n.eq_ignore_ascii_case(&exe)) {
                pids.push(entry.th32ProcessID);
            }
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    pids
}

fn kill(pids: &[u32]) {
    for &pid in pids {
        // SAFETY: the handle is checked before use and closed after.
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !handle.is_null() {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
}

/// Reads `vacBanned` from the account's public Steam profile. `None` if Steam can't be reached.
pub fn vac_banned(id64: u64) -> Option<bool> {
    let url = format!("https://steamcommunity.com/profiles/{id64}?xml=1");
    let response = minreq::get(url).with_timeout(10).send().ok()?;
    let body = response.as_str().ok()?;
    let start = body.find("<vacBanned>")? + "<vacBanned>".len();
    let end = start + body[start..].find('<')?;
    Some(body[start..end].trim() == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT_STEAM: &str = r#""users" { "1" { "AccountName" "a" "AutoLogin" "1" "RememberPassword" "1" "timestamp" "5" } "2" { "AccountName" "b" "AutoLogin" "0" "RememberPassword" "1" "timestamp" "6" } }"#;
    const OLDER_STEAM: &str = r#""users" { "1" { "AccountName" "a" "MostRecent" "1" "AllowAutoLogin" "1" } "2" { "AccountName" "b" "MostRecent" "0" "AllowAutoLogin" "0" } }"#;

    fn switched(text: &str, target: Option<u64>) -> Vec<(String, Value)> {
        let mut root = vdf::parse(text).unwrap();
        set_auto_login(&mut root, target).unwrap();
        root
    }

    fn field<'a>(root: &'a [(String, Value)], id: &str, key: &str) -> Option<&'a str> {
        vdf::find(root, "users")?.get(id)?.get_str(key)
    }

    #[test]
    fn marks_only_the_target() {
        let root = switched(CURRENT_STEAM, Some(2));
        assert_eq!(field(&root, "1", "AutoLogin"), Some("0"));
        assert_eq!(field(&root, "2", "AutoLogin"), Some("1"));
        assert_eq!(
            field(&root, "1", "RememberPassword"),
            Some("1"),
            "other accounts stay remembered"
        );
        assert_eq!(field(&root, "2", "timestamp"), Some("6"));
    }

    #[test]
    fn handles_older_steam_fields() {
        let root = switched(OLDER_STEAM, Some(2));
        assert_eq!(field(&root, "1", "MostRecent"), Some("0"));
        assert_eq!(field(&root, "2", "MostRecent"), Some("1"));
        assert_eq!(field(&root, "2", "AllowAutoLogin"), Some("1"));
    }

    #[test]
    fn no_target_clears_auto_login() {
        let root = switched(CURRENT_STEAM, None);
        assert_eq!(field(&root, "1", "AutoLogin"), Some("0"));
        assert_eq!(field(&root, "2", "AutoLogin"), Some("0"));
    }
}
