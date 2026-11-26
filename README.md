# Slim Steam Switch

A small, portable Steam account switcher for Windows that looks like VS Code.

Download `slim-steam-switch.exe` from the [latest release](https://github.com/Hanzawa-1/Account-Switcher/releases/latest) and run it. The exe isn't signed, so Windows may warn about it the first time: choose **More info**, then **Run anyway**.

- Lists the Steam accounts saved on this PC, with their profile pictures and last login time.
- Double-click an account (or select it and press Enter) to switch. Steam closes and starts again signed in to that account.
- `+` opens Steam's sign-in screen so you can add another account.
- A red frame around a profile picture means the account has a VAC ban.
- A green dot marks the account that's signed in. When Steam is closed, a gray dot marks the account it will sign in to next.

It's a single `.exe` under 1 MB for Windows 10 and 11: no installer, no admin rights, no settings files. The window is drawn with plain Win32, without the GPU, so it uses about 20 MB of memory. Profile pictures and last login times come from Steam's own files on your PC. VAC status is the only thing it looks up online, from each account's public Steam profile.

Switching only works for accounts you signed in to with "Remember me" ticked. Before each switch, the app backs up `Steam\config\loginusers.vdf` to `loginusers.vdf.bak`.

## Build

Needs Rust with the MSVC toolchain:

```powershell
cargo build --release
```

The exe is `target\release\slim-steam-switch.exe`.