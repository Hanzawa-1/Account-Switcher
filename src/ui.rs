//! The window, drawn with plain Win32 and GDI in VS Code's "Dark Modern" colors.
//! Nothing here touches the GPU, so no graphics driver gets loaded into the process.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect, GdiFlush,
    GetDC, GetMonitorInfoW, GetStockObject, GetTextExtentPoint32W, IntersectClipRect,
    InvalidateRect, MonitorFromPoint, ReleaseDC, ScreenToClient, SelectClipRgn, SelectObject,
    SetBkMode, SetTextColor, UpdateWindow, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY,
    DEFAULT_CHARSET, DEFAULT_GUI_FONT, DIB_RGB_COLORS, DT_BOTTOM, DT_CALCRECT, DT_CENTER,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_TOP, DT_VCENTER,
    DT_WORDBREAK, FW_NORMAL, HBITMAP, HDC, HFONT, HGDIOBJ, LOGFONTW, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow, GetSystemMetricsForDpi,
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_RETURN, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetCursorPos, GetMessageW,
    KillTimer, LoadCursorW, LoadImageW, PostMessageW, PostQuitMessage, RegisterClassExW, SetTimer,
    SetWindowPos, ShowWindow, TranslateMessage, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, IMAGE_ICON, LR_DEFAULTCOLOR, MA_NOACTIVATE, MINMAXINFO,
    MSG, SM_CXICON, SM_CXSMICON, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW,
    WM_APP, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCHITTEST, WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW, WS_POPUP,
};

use crate::steam::{self, Account};

/// GDI colors are 0x00BBGGRR.
const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

// Colors from VS Code's "Dark Modern" theme.
const BG: u32 = rgb(0x18, 0x18, 0x18);
const FG: u32 = rgb(0xCC, 0xCC, 0xCC);
const MUTED: u32 = rgb(0x9D, 0x9D, 0x9D);
const WHITE: u32 = rgb(0xFF, 0xFF, 0xFF);
const HOVER: u32 = rgb(0x2A, 0x2D, 0x2E);
const SELECTED: u32 = rgb(0x04, 0x39, 0x5E);
const FOCUS: u32 = rgb(0x00, 0x78, 0xD4);
const BUTTON_HOVER: u32 = rgb(0x2D, 0x2E, 0x2E);
const SCROLL_THUMB: u32 = rgb(0x42, 0x42, 0x42);
const STATUS: u32 = rgb(0x00, 0x7A, 0xCC);
const STATUS_BUSY: u32 = rgb(0xCC, 0x66, 0x33);
const STATUS_ERROR: u32 = rgb(0xC7, 0x2E, 0x2E);
const ONLINE: u32 = rgb(0x3F, 0xB9, 0x50);
const OFFLINE: u32 = rgb(0x8B, 0x8B, 0x8B);
const VAC_RED: u32 = rgb(0xF1, 0x4C, 0x4C);
const TIP_BG: u32 = rgb(0x20, 0x20, 0x20);
const TIP_BORDER: u32 = rgb(0x45, 0x45, 0x45);

// Layout in logical pixels (scaled by the monitor's DPI).
const WIDTH: i32 = 380;
const HEIGHT: i32 = 420;
const MIN_WIDTH: i32 = 300;
const MIN_HEIGHT: i32 = 200;
const HEADER: i32 = 32;
const ROW: i32 = 46;
const AVATAR: i32 = 32;
const STATUS_BAR: i32 = 22;
const PAD: i32 = 14;

// Glyphs from Segoe MDL2 Assets, the icon font built into Windows 10 and 11.
const ICON_ADD: &str = "\u{E710}";
const ICON_REFRESH: &str = "\u{E72C}";
const ICON_GAME: &str = "\u{E7FC}";

const WM_SWITCHED: u32 = WM_APP + 1;
const WM_VAC_CHECKED: u32 = WM_APP + 2;
/// Lives in windows-sys's Controls module; defined here to avoid that whole feature.
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER_POLL: usize = 1;
const TIMER_TOOLTIP: usize = 2;
const TIMER_MESSAGE: usize = 3;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TIP: RefCell<TipText> = const {
        RefCell::new(TipText { text: Vec::new(), font: null_mut(), pad_x: 0, pad_y: 0 })
    };
}

/// Runs `f` on the app state. Returns `None` before the window exists, or when a Win32 call
/// made inside another handler re-enters the window procedure.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut app = cell.try_borrow_mut().ok()?;
        app.as_mut().map(f)
    })
}

pub fn run() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let system_dpi = GetDpiForSystem();
        let icon = |metric| {
            let size = GetSystemMetricsForDpi(metric, system_dpi);
            // Icon resource 1 is embedded by build.rs.
            LoadImageW(
                instance,
                make_int_resource(1),
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            )
        };

        let class_name = wide("SlimSteamSwitch");
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hIcon: icon(SM_CXICON),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class_name.as_ptr(),
            hIconSm: icon(SM_CXSMICON),
            ..zeroed()
        };
        RegisterClassExW(&class);
        let tip_class_name = wide("SlimSteamSwitchTip");
        let tip_class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(tip_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: tip_class_name.as_ptr(),
            ..zeroed()
        };
        RegisterClassExW(&tip_class);

        let mut frame = RECT {
            left: 0,
            top: 0,
            right: scale(WIDTH, system_dpi),
            bottom: scale(HEIGHT, system_dpi),
        };
        AdjustWindowRectExForDpi(&mut frame, WS_OVERLAPPEDWINDOW, 0, 0, system_dpi);
        let title = wide("Slim Steam Switch");
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            frame.right - frame.left,
            frame.bottom - frame.top,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        let dark: i32 = 1;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as _,
            &dark as *const i32 as *const c_void,
            size_of::<i32>() as u32,
        );
        let tip = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            tip_class_name.as_ptr(),
            null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );

        let dpi = GetDpiForWindow(hwnd);
        APP.with(|cell| *cell.borrow_mut() = Some(App::new(hwnd, tip, dpi)));
        with_app(App::start);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let low = (lparam & 0xFFFF) as u16;
    let high = ((lparam >> 16) & 0xFFFF) as u16;
    let (x, y) = (low as i16 as i32, high as i16 as i32);
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            with_app(|app| app.paint(dc));
            EndPaint(hwnd, &ps);
            0
        }
        // Everything is painted in WM_PAINT; skipping the erase avoids flicker.
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            with_app(|app| app.resize(low as i32, high as i32));
            0
        }
        WM_GETMINMAXINFO => {
            let dpi = GetDpiForWindow(hwnd);
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: scale(MIN_WIDTH, dpi),
                bottom: scale(MIN_HEIGHT, dpi),
            };
            AdjustWindowRectExForDpi(&mut frame, WS_OVERLAPPEDWINDOW, 0, 0, dpi);
            let info = &mut *(lparam as *mut MINMAXINFO);
            info.ptMinTrackSize = POINT {
                x: frame.right - frame.left,
                y: frame.bottom - frame.top,
            };
            0
        }
        WM_DPICHANGED => {
            with_app(|app| app.set_dpi(((wparam >> 16) & 0xFFFF) as u32));
            let suggested = &*(lparam as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            0
        }
        WM_MOUSEMOVE => {
            with_app(|app| app.on_mouse_move(x, y));
            0
        }
        WM_MOUSELEAVE => {
            with_app(App::on_mouse_leave);
            0
        }
        WM_LBUTTONDOWN => {
            with_app(|app| app.on_mouse_down(x, y));
            0
        }
        WM_LBUTTONUP => {
            with_app(|app| app.on_mouse_up(x, y));
            0
        }
        WM_LBUTTONDBLCLK => {
            with_app(|app| app.on_double_click(x, y));
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
            with_app(|app| app.on_wheel(delta));
            0
        }
        WM_KEYDOWN => {
            with_app(|app| app.on_key(wparam as u16));
            0
        }
        WM_TIMER => {
            with_app(|app| app.on_timer(wparam));
            0
        }
        WM_SWITCHED => {
            with_app(App::on_switched);
            0
        }
        WM_VAC_CHECKED => {
            with_app(|app| app.invalidate());
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Row(usize),
    Add,
    Refresh,
}

struct App {
    hwnd: HWND,
    tip: HWND,
    dpi: u32,
    fonts: Fonts,
    width: i32,
    height: i32,
    back: Option<BackBuffer>,
    steam: Option<PathBuf>,
    accounts: Vec<Account>,
    load_error: Option<String>,
    avatars: HashMap<u64, Avatar>,
    /// VAC status per SteamID64, filled in by a background thread.
    vac: Arc<Mutex<HashMap<u64, bool>>>,
    selected: Option<u64>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    tracking_mouse: bool,
    scroll: i32,
    /// Status text while a switch runs in the background.
    switching: Option<String>,
    switch_result: Arc<Mutex<Option<Result<(), String>>>>,
    /// A short-lived status bar message and whether it's an error.
    message: Option<(String, bool)>,
    steam_running: bool,
    active_id: Option<u32>,
    /// Login name Steam signs in to on its next start.
    auto_login: Option<String>,
    loginusers_modified: Option<SystemTime>,
}

impl App {
    fn new(hwnd: HWND, tip: HWND, dpi: u32) -> Self {
        let mut client: RECT = unsafe { zeroed() };
        unsafe { GetClientRect(hwnd, &mut client) };
        Self {
            hwnd,
            tip,
            dpi,
            fonts: Fonts::new(dpi),
            width: client.right,
            height: client.bottom,
            back: None,
            steam: steam::find_steam(),
            accounts: Vec::new(),
            load_error: None,
            avatars: HashMap::new(),
            vac: Arc::default(),
            selected: None,
            hover: None,
            pressed: None,
            tracking_mouse: false,
            scroll: 0,
            switching: None,
            switch_result: Arc::default(),
            message: None,
            steam_running: false,
            active_id: None,
            auto_login: None,
            loginusers_modified: None,
        }
    }

    fn start(&mut self) {
        self.reload();
        self.selected = self
            .active_account()
            .or(self.next_account())
            .map(|a| a.id64);
        unsafe { SetTimer(self.hwnd, TIMER_POLL, 2000, None) };
    }

    fn px(&self, value: i32) -> i32 {
        scale(value, self.dpi)
    }

    fn px_f(&self, value: f32) -> f32 {
        value * self.dpi as f32 / 96.0
    }

    fn invalidate(&self) {
        unsafe { InvalidateRect(self.hwnd, null(), 0) };
    }

    /// Re-reads the accounts and their profile pictures from Steam's files.
    fn reload(&mut self) {
        self.accounts.clear();
        self.load_error = None;
        self.hover = None;
        self.pressed = None;
        if let Some(dir) = &self.steam {
            match steam::load_accounts(dir) {
                Ok(accounts) => self.accounts = accounts,
                Err(e) => self.load_error = Some(e),
            }
            self.loginusers_modified = modified(&steam::loginusers_path(dir));
        }
        self.load_avatars();
        self.check_vac();
        self.poll_steam();
        self.clamp_scroll();
    }

    fn load_avatars(&mut self) {
        self.avatars.clear();
        let size = self.px(AVATAR);
        let Some(dir) = &self.steam else { return };
        for acc in &self.accounts {
            if let Some(avatar) = load_avatar(dir, acc.id64, size) {
                self.avatars.insert(acc.id64, avatar);
            }
        }
    }

    /// Looks up VAC status online for accounts that haven't been checked yet.
    fn check_vac(&self) {
        let ids: Vec<u64> = {
            let known = self.vac.lock().unwrap();
            self.accounts
                .iter()
                .map(|a| a.id64)
                .filter(|id| !known.contains_key(id))
                .collect()
        };
        if ids.is_empty() {
            return;
        }
        let vac = Arc::clone(&self.vac);
        let hwnd = self.hwnd as isize;
        std::thread::spawn(move || {
            for id in ids {
                if let Some(banned) = steam::vac_banned(id) {
                    vac.lock().unwrap().insert(id, banned);
                    unsafe { PostMessageW(hwnd as HWND, WM_VAC_CHECKED, 0, 0) };
                }
            }
        });
    }

    fn poll_steam(&mut self) {
        self.steam_running = steam::is_steam_running();
        self.active_id = if self.steam_running {
            steam::active_account_id()
        } else {
            None
        };
        self.auto_login = steam::auto_login_name();
    }

    fn active_account(&self) -> Option<&Account> {
        let id = self.active_id?;
        self.accounts.iter().find(|a| a.account_id() == id)
    }

    /// The account Steam will sign in to the next time it starts.
    fn next_account(&self) -> Option<&Account> {
        let name = self.auto_login.as_deref()?;
        self.accounts
            .iter()
            .find(|a| a.account_name.eq_ignore_ascii_case(name))
    }

    /// Green dot: signed in right now. Gray dot: Steam is closed and will sign in to this account next.
    fn presence(&self, acc: &Account) -> Option<u32> {
        if self.steam_running {
            (self.active_id == Some(acc.account_id())).then_some(ONLINE)
        } else {
            (self.next_account().map(|a| a.id64) == Some(acc.id64)).then_some(OFFLINE)
        }
    }

    fn switch(&mut self, target: Option<Account>) {
        if self.switching.is_some() {
            return;
        }
        let Some(dir) = self.steam.clone() else {
            return;
        };
        if let Some(acc) = &target {
            if self.active_id == Some(acc.account_id()) {
                self.show_message(format!("Already signed in as {}", acc.name()), false);
                return;
            }
        }
        self.switching = Some(match &target {
            Some(acc) => format!("Switching to {}…", acc.name()),
            None => "Opening Steam sign-in…".to_owned(),
        });
        self.message = None;
        let result = Arc::clone(&self.switch_result);
        let hwnd = self.hwnd as isize;
        std::thread::spawn(move || {
            *result.lock().unwrap() = Some(steam::switch_to(&dir, target.as_ref()));
            unsafe { PostMessageW(hwnd as HWND, WM_SWITCHED, 0, 0) };
        });
        self.invalidate();
    }

    fn on_switched(&mut self) {
        let result = self.switch_result.lock().unwrap().take();
        self.switching = None;
        if let Some(Err(e)) = result {
            self.show_message(format!("Couldn't switch: {e}"), true);
        }
        self.poll_steam();
        self.invalidate();
    }

    fn show_message(&mut self, text: String, is_error: bool) {
        self.message = Some((text, is_error));
        unsafe { SetTimer(self.hwnd, TIMER_MESSAGE, 6000, None) };
        self.invalidate();
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_POLL => {
                if self.switching.is_some() {
                    return;
                }
                self.poll_steam();
                let changed = self.steam.as_ref().is_some_and(|dir| {
                    modified(&steam::loginusers_path(dir)) != self.loginusers_modified
                });
                if changed {
                    self.reload();
                }
                // Repaint anyway: Steam's state and the "x min ago" times may have changed.
                self.invalidate();
            }
            TIMER_TOOLTIP => {
                unsafe { KillTimer(self.hwnd, TIMER_TOOLTIP) };
                self.show_tooltip();
            }
            TIMER_MESSAGE => {
                unsafe { KillTimer(self.hwnd, TIMER_MESSAGE) };
                self.message = None;
                self.invalidate();
            }
            _ => {}
        }
    }

    fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
        self.fonts = Fonts::new(dpi);
        self.load_avatars();
        self.hide_tooltip();
    }

    fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        self.clamp_scroll();
        self.hide_tooltip();
        self.invalidate();
    }

    // ---- layout and hit testing ----

    fn list_bounds(&self) -> (i32, i32) {
        (self.px(HEADER), self.height - self.px(STATUS_BAR))
    }

    fn shows_rows(&self) -> bool {
        self.steam.is_some() && self.load_error.is_none() && !self.accounts.is_empty()
    }

    fn max_scroll(&self) -> i32 {
        let (top, bottom) = self.list_bounds();
        (self.accounts.len() as i32 * self.px(ROW) - (bottom - top)).max(0)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0, self.max_scroll());
    }

    fn row_rect(&self, index: usize) -> RECT {
        let (top, _) = self.list_bounds();
        let y = top + index as i32 * self.px(ROW) - self.scroll;
        RECT {
            left: 0,
            top: y,
            right: self.width,
            bottom: y + self.px(ROW),
        }
    }

    fn buttons(&self) -> [(Hit, &'static str, RECT); 2] {
        let size = self.px(22);
        let center_y = self.px(HEADER) / 2;
        let square = |center_x: i32| RECT {
            left: center_x - size / 2,
            top: center_y - size / 2,
            right: center_x - size / 2 + size,
            bottom: center_y - size / 2 + size,
        };
        let refresh_x = self.width - self.px(22);
        [
            (Hit::Add, ICON_ADD, square(refresh_x - self.px(26))),
            (Hit::Refresh, ICON_REFRESH, square(refresh_x)),
        ]
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<Hit> {
        let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        if let Some((hit, _, _)) = self.buttons().iter().find(|(_, _, r)| inside(r)) {
            return Some(*hit);
        }
        let (top, bottom) = self.list_bounds();
        if !self.shows_rows() || y < top || y >= bottom {
            return None;
        }
        let index = ((y - top + self.scroll) / self.px(ROW)) as usize;
        (index < self.accounts.len()).then_some(Hit::Row(index))
    }

    fn ensure_visible(&mut self, index: usize) {
        let (top, bottom) = self.list_bounds();
        let row_top = index as i32 * self.px(ROW);
        let row_bottom = row_top + self.px(ROW);
        if row_top < self.scroll {
            self.scroll = row_top;
        } else if row_bottom > self.scroll + (bottom - top) {
            self.scroll = row_bottom - (bottom - top);
        }
        self.clamp_scroll();
    }

    // ---- input ----

    fn on_mouse_move(&mut self, x: i32, y: i32) {
        if !self.tracking_mouse {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            unsafe { TrackMouseEvent(&mut track) };
            self.tracking_mouse = true;
        }
        self.set_hover(self.hit_test(x, y));
    }

    fn on_mouse_leave(&mut self) {
        self.tracking_mouse = false;
        self.set_hover(None);
    }

    fn set_hover(&mut self, hit: Option<Hit>) {
        if hit == self.hover {
            return;
        }
        self.hover = hit;
        self.hide_tooltip();
        unsafe {
            if hit.is_some() {
                SetTimer(self.hwnd, TIMER_TOOLTIP, 600, None);
            } else {
                KillTimer(self.hwnd, TIMER_TOOLTIP);
            }
        }
        self.invalidate();
    }

    fn on_mouse_down(&mut self, x: i32, y: i32) {
        let hit = self.hit_test(x, y);
        self.pressed = hit;
        self.hide_tooltip();
        if let Some(Hit::Row(i)) = hit {
            self.selected = Some(self.accounts[i].id64);
            self.invalidate();
        }
    }

    fn on_mouse_up(&mut self, x: i32, y: i32) {
        let hit = self.hit_test(x, y);
        if hit != self.pressed.take() {
            return;
        }
        match hit {
            Some(Hit::Add) => self.switch(None),
            Some(Hit::Refresh) => {
                self.vac.lock().unwrap().clear();
                self.reload();
                self.invalidate();
            }
            _ => {}
        }
    }

    fn on_double_click(&mut self, x: i32, y: i32) {
        if let Some(Hit::Row(i)) = self.hit_test(x, y) {
            let acc = self.accounts[i].clone();
            self.selected = Some(acc.id64);
            self.switch(Some(acc));
        }
    }

    fn on_wheel(&mut self, delta: i32) {
        self.scroll -= delta * self.px(ROW) / 120;
        self.clamp_scroll();
        // The rows moved under the cursor.
        let mut cursor = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut cursor);
            ScreenToClient(self.hwnd, &mut cursor);
        }
        self.set_hover(self.hit_test(cursor.x, cursor.y));
        self.invalidate();
    }

    fn on_key(&mut self, key: u16) {
        if !self.shows_rows() {
            return;
        }
        let index = self
            .selected
            .and_then(|id| self.accounts.iter().position(|a| a.id64 == id));
        match key {
            VK_DOWN | VK_UP => {
                let last = self.accounts.len() - 1;
                let next = match index {
                    None => 0,
                    Some(i) if key == VK_DOWN => (i + 1).min(last),
                    Some(i) => i.saturating_sub(1),
                };
                self.selected = Some(self.accounts[next].id64);
                self.ensure_visible(next);
                self.invalidate();
            }
            VK_RETURN => {
                if let Some(i) = index {
                    let acc = self.accounts[i].clone();
                    self.switch(Some(acc));
                }
            }
            _ => {}
        }
    }

    // ---- tooltip ----

    fn show_tooltip(&self) {
        let text = match self.hover {
            Some(Hit::Add) => "Add account".to_owned(),
            Some(Hit::Refresh) => "Refresh".to_owned(),
            Some(Hit::Row(i)) => match self.accounts.get(i) {
                Some(acc) => {
                    let vac = self.vac.lock().unwrap().get(&acc.id64).copied();
                    tooltip_text(acc, vac)
                }
                None => return,
            },
            None => return,
        };
        let text: Vec<u16> = text.encode_utf16().collect();
        let (pad_x, pad_y) = (self.px(8), self.px(4));
        let font = self.fonts.name;
        unsafe {
            let dc = GetDC(null_mut());
            let old = SelectObject(dc, font);
            let mut size = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            DrawTextW(
                dc,
                text.as_ptr(),
                text.len() as i32,
                &mut size,
                DT_CALCRECT | DT_NOPREFIX,
            );
            SelectObject(dc, old);
            ReleaseDC(null_mut(), dc);
            let (w, h) = (size.right + 2 * pad_x + 2, size.bottom + 2 * pad_y + 2);

            // Below the cursor, kept inside the monitor.
            let mut cursor = POINT { x: 0, y: 0 };
            GetCursorPos(&mut cursor);
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(
                MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            );
            let work = monitor.rcWork;
            let x = cursor.x.min(work.right - w).max(work.left);
            let mut y = cursor.y + self.px(20);
            if y + h > work.bottom {
                y = cursor.y - h - self.px(4);
            }

            TIP.with(|tip| {
                *tip.borrow_mut() = TipText {
                    text,
                    font,
                    pad_x,
                    pad_y,
                }
            });
            SetWindowPos(
                self.tip,
                HWND_TOPMOST,
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.tip, null(), 1);
        }
    }

    fn hide_tooltip(&self) {
        unsafe { ShowWindow(self.tip, SW_HIDE) };
    }

    // ---- painting ----

    fn paint(&mut self, window_dc: HDC) {
        let (w, h) = (self.width, self.height);
        if w <= 0 || h <= 0 {
            return;
        }
        let mut back = match self.back.take() {
            Some(back) if back.width == w && back.height == h => back,
            _ => match unsafe { BackBuffer::new(window_dc, w, h) } {
                Some(back) => back,
                None => return,
            },
        };
        let dc = back.dc;
        unsafe {
            fill(
                dc,
                RECT {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                },
                BG,
            );
            self.paint_header(dc);
            self.paint_list(&mut back);
            self.paint_status(dc);
            BitBlt(window_dc, 0, 0, w, h, dc, 0, 0, SRCCOPY);
            // Don't keep our fonts selected, so they can be deleted on a DPI change.
            SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
        }
        self.back = Some(back);
    }

    unsafe fn paint_header(&self, dc: HDC) {
        let header = RECT {
            left: self.px(PAD),
            top: 0,
            right: self.width,
            bottom: self.px(HEADER),
        };
        draw_text(
            dc,
            "ACCOUNTS",
            header,
            self.fonts.small,
            FG,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        for (hit, glyph, rect) in self.buttons() {
            if self.hover == Some(hit) {
                fill(dc, rect, BUTTON_HOVER);
            }
            draw_text(
                dc,
                glyph,
                rect,
                self.fonts.icons,
                FG,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }
    }

    unsafe fn paint_list(&self, back: &mut BackBuffer) {
        let dc = back.dc;
        let (top, bottom) = self.list_bounds();
        let note = if self.steam.is_none() {
            Some("Steam isn't installed on this PC.".to_owned())
        } else if let Some(e) = &self.load_error {
            Some(format!("Couldn't read Steam's account list: {e}"))
        } else if self.accounts.is_empty() {
            Some("No saved Steam accounts yet. Use + to add one.".to_owned())
        } else {
            None
        };
        if let Some(note) = note {
            let area = RECT {
                left: self.px(PAD),
                top: top + self.px(24),
                right: self.width - self.px(PAD),
                bottom,
            };
            draw_text(
                dc,
                &note,
                area,
                self.fonts.name,
                MUTED,
                DT_CENTER | DT_WORDBREAK,
            );
            return;
        }

        IntersectClipRect(dc, 0, top, self.width, bottom);
        let vac = self.vac.lock().unwrap().clone();
        let first = (self.scroll / self.px(ROW)) as usize;
        for (index, acc) in self.accounts.iter().enumerate().skip(first) {
            let row = self.row_rect(index);
            if row.top >= bottom {
                break;
            }
            let banned = vac.get(&acc.id64) == Some(&true);
            self.paint_row(back, acc, index, row, banned, (top, bottom));
        }
        // A thin scrollbar thumb when the list doesn't fit.
        let view = bottom - top;
        let content = self.accounts.len() as i32 * self.px(ROW);
        if content > view {
            let thumb = (view * view / content).max(self.px(20));
            let thumb_top = top + self.scroll * (view - thumb) / (content - view);
            let rect = RECT {
                left: self.width - self.px(6),
                top: thumb_top,
                right: self.width,
                bottom: thumb_top + thumb,
            };
            fill(dc, rect, SCROLL_THUMB);
        }
        SelectClipRgn(dc, null_mut());
    }

    unsafe fn paint_row(
        &self,
        back: &mut BackBuffer,
        acc: &Account,
        index: usize,
        row: RECT,
        banned: bool,
        clip: (i32, i32),
    ) {
        let dc = back.dc;
        let selected = self.selected == Some(acc.id64);
        let background = if selected {
            SELECTED
        } else if self.hover == Some(Hit::Row(index)) {
            HOVER
        } else {
            BG
        };
        fill(dc, row, background);
        if selected {
            frame(dc, row, FOCUS);
        }

        let size = self.px(AVATAR);
        let center_y = (row.top + row.bottom) / 2;
        let picture = RECT {
            left: self.px(PAD),
            top: center_y - size / 2,
            right: self.px(PAD) + size,
            bottom: center_y - size / 2 + size,
        };
        if banned {
            let t = self.px(2);
            let border = RECT {
                left: picture.left - t,
                top: picture.top - t,
                right: picture.right + t,
                bottom: picture.bottom + t,
            };
            fill(dc, border, VAC_RED);
        }
        match self.avatars.get(&acc.id64) {
            Some(avatar) => blit(back, avatar, picture.left, picture.top, clip),
            None => {
                fill(dc, picture, placeholder_color(acc.id64));
                draw_text(
                    dc,
                    &initial(acc.name()),
                    picture,
                    self.fonts.initial,
                    WHITE,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
        if let Some(color) = self.presence(acc) {
            let cx = picture.right as f32 - self.px_f(1.0);
            let cy = picture.bottom as f32 - self.px_f(1.0);
            fill_circle(back, cx, cy, self.px_f(5.5), background, clip);
            fill_circle(back, cx, cy, self.px_f(3.5), color, clip);
        }

        let mut right = row.right - self.px(PAD);
        if let Some(ts) = acc.timestamp {
            let when = relative_time(ts);
            let width = text_width(dc, &when, self.fonts.small);
            let rect = RECT {
                left: right - width,
                top: row.top,
                right,
                bottom: row.bottom,
            };
            draw_text(
                dc,
                &when,
                rect,
                self.fonts.small,
                MUTED,
                DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
            );
            right -= width + self.px(10);
        }
        let x = picture.right + self.px(12);
        let name = RECT {
            left: x,
            top: row.top,
            right,
            bottom: center_y,
        };
        draw_text(
            dc,
            acc.name(),
            name,
            self.fonts.name,
            FG,
            DT_LEFT | DT_BOTTOM | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        let login = RECT {
            left: x,
            top: center_y + self.px(1),
            right,
            bottom: row.bottom,
        };
        draw_text(
            dc,
            &acc.account_name,
            login,
            self.fonts.small,
            MUTED,
            DT_LEFT | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }

    unsafe fn paint_status(&self, dc: HDC) {
        let (w, h) = (self.width, self.height);
        let top = h - self.px(STATUS_BAR);
        let (color, text) = self.status();
        fill(
            dc,
            RECT {
                left: 0,
                top,
                right: w,
                bottom: h,
            },
            color,
        );
        let left = self.px(10);
        let bar = |left: i32, right: i32| RECT {
            left,
            top,
            right,
            bottom: h,
        };
        let flags = DT_VCENTER | DT_SINGLELINE;
        draw_text(
            dc,
            ICON_GAME,
            bar(left, w),
            self.fonts.status_icon,
            WHITE,
            DT_LEFT | flags,
        );
        let text_left = left + text_width(dc, ICON_GAME, self.fonts.status_icon) + self.px(6);
        let mut right = w - self.px(10);
        if self.shows_rows() {
            let hint = "Double-click to switch";
            let width = text_width(dc, hint, self.fonts.status);
            draw_text(
                dc,
                hint,
                bar(right - width, right),
                self.fonts.status,
                WHITE,
                DT_RIGHT | flags,
            );
            right -= width + self.px(12);
        }
        draw_text(
            dc,
            &text,
            bar(text_left, right),
            self.fonts.status,
            WHITE,
            DT_LEFT | DT_END_ELLIPSIS | flags,
        );
    }

    fn status(&self) -> (u32, String) {
        if let Some(label) = &self.switching {
            return (STATUS_BUSY, label.clone());
        }
        if let Some((text, is_error)) = &self.message {
            return (if *is_error { STATUS_ERROR } else { STATUS }, text.clone());
        }
        if self.steam.is_none() {
            return (STATUS_ERROR, "Steam not found".to_owned());
        }
        if !self.steam_running {
            return match self.next_account() {
                Some(acc) => (
                    STATUS,
                    format!("Steam not running · will sign in as {}", acc.name()),
                ),
                None => (STATUS, "Steam not running".to_owned()),
            };
        }
        match self.active_account() {
            Some(acc) => (STATUS, format!("Steam running · {}", acc.name())),
            None => (STATUS, "Steam running · not signed in".to_owned()),
        }
    }
}

/// The fonts the window uses, created for one DPI.
struct Fonts {
    /// 13 px: account names, tooltips, notes.
    name: HFONT,
    /// 11 px: login names, times, the header.
    small: HFONT,
    /// 12 px: status bar text.
    status: HFONT,
    /// 14 px: the letter on an account without a picture.
    initial: HFONT,
    /// 14 px icon font: header buttons.
    icons: HFONT,
    /// 13 px icon font: the status bar icon.
    status_icon: HFONT,
}

impl Fonts {
    fn new(dpi: u32) -> Self {
        Self {
            name: create_font("Segoe UI", scale(13, dpi)),
            small: create_font("Segoe UI", scale(11, dpi)),
            status: create_font("Segoe UI", scale(12, dpi)),
            initial: create_font("Segoe UI", scale(14, dpi)),
            icons: create_font("Segoe MDL2 Assets", scale(14, dpi)),
            status_icon: create_font("Segoe MDL2 Assets", scale(13, dpi)),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for font in [
            self.name,
            self.small,
            self.status,
            self.initial,
            self.icons,
            self.status_icon,
        ] {
            unsafe { DeleteObject(font) };
        }
    }
}

/// Windows substitutes a fallback font (Yu Gothic, Malgun Gothic, …) for characters Segoe UI lacks.
fn create_font(face: &str, pixels: i32) -> HFONT {
    let mut font: LOGFONTW = unsafe { zeroed() };
    font.lfHeight = -pixels;
    font.lfWeight = FW_NORMAL as i32;
    font.lfCharSet = DEFAULT_CHARSET;
    font.lfQuality = CLEARTYPE_QUALITY;
    for (dst, src) in font.lfFaceName.iter_mut().zip(face.encode_utf16().take(31)) {
        *dst = src;
    }
    unsafe { CreateFontIndirectW(&font) }
}

/// An off-screen 32-bit bitmap the whole window is drawn into before it's copied to the screen.
struct BackBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    pixels: *mut u32,
    width: i32,
    height: i32,
}

impl BackBuffer {
    unsafe fn new(window_dc: HDC, width: i32, height: i32) -> Option<Self> {
        let dc = CreateCompatibleDC(window_dc);
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        let mut bits: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, bitmap);
        Some(Self {
            dc,
            bitmap,
            old,
            pixels: bits as *mut u32,
            width,
            height,
        })
    }

    /// The pixels as 0x00RRGGBB, for drawing GDI can't do (pictures, smooth dots).
    fn pixels_mut(&mut self) -> &mut [u32] {
        unsafe {
            // Let GDI finish drawing before touching the pixels directly.
            GdiFlush();
            std::slice::from_raw_parts_mut(self.pixels, (self.width * self.height) as usize)
        }
    }
}

impl Drop for BackBuffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// A profile picture scaled for the current DPI, as 0x00RRGGBB pixels.
struct Avatar {
    size: i32,
    pixels: Vec<u32>,
}

fn load_avatar(steam: &Path, id64: u64, size: i32) -> Option<Avatar> {
    let img = steam::load_avatar(steam, id64)?;
    let img = image::imageops::resize(
        &img,
        size as u32,
        size as u32,
        image::imageops::FilterType::CatmullRom,
    );
    let bg = to_pixel(BG);
    let pixels = img
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            blend(bg, (r as u32) << 16 | (g as u32) << 8 | b as u32, a as u32)
        })
        .collect();
    Some(Avatar { size, pixels })
}

/// The text the tooltip window paints.
struct TipText {
    text: Vec<u16>,
    font: HFONT,
    pad_x: i32,
    pad_y: i32,
}

unsafe extern "system" fn tip_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);
            fill(dc, rect, TIP_BG);
            frame(dc, rect, TIP_BORDER);
            TIP.with(|tip| {
                let tip = tip.borrow();
                let mut text_rect = RECT {
                    left: rect.left + tip.pad_x + 1,
                    top: rect.top + tip.pad_y + 1,
                    right: rect.right - tip.pad_x - 1,
                    bottom: rect.bottom - tip.pad_y - 1,
                };
                SelectObject(dc, tip.font);
                SetTextColor(dc, FG);
                SetBkMode(dc, TRANSPARENT as _);
                DrawTextW(
                    dc,
                    tip.text.as_ptr(),
                    tip.text.len() as i32,
                    &mut text_rect,
                    DT_LEFT | DT_NOPREFIX,
                );
            });
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        // Let the mouse pass through to the main window, and never take focus.
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---- drawing helpers ----

unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    let brush = CreateSolidBrush(color);
    FillRect(dc, &rect, brush);
    DeleteObject(brush);
}

unsafe fn frame(dc: HDC, rect: RECT, color: u32) {
    let brush = CreateSolidBrush(color);
    FrameRect(dc, &rect, brush);
    DeleteObject(brush);
}

unsafe fn draw_text(dc: HDC, text: &str, rect: RECT, font: HFONT, color: u32, flags: u32) {
    let text: Vec<u16> = text.encode_utf16().collect();
    let mut rect = rect;
    SelectObject(dc, font);
    SetTextColor(dc, color);
    SetBkMode(dc, TRANSPARENT as _);
    DrawTextW(
        dc,
        text.as_ptr(),
        text.len() as i32,
        &mut rect,
        flags | DT_NOPREFIX,
    );
}

unsafe fn text_width(dc: HDC, text: &str, font: HFONT) -> i32 {
    let text: Vec<u16> = text.encode_utf16().collect();
    SelectObject(dc, font);
    let mut size = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
    size.cx
}

/// Copies a profile picture into the back buffer, skipping rows outside `clip`.
fn blit(back: &mut BackBuffer, avatar: &Avatar, left: i32, top: i32, clip: (i32, i32)) {
    let (width, height) = (back.width, back.height);
    let pixels = back.pixels_mut();
    for row in 0..avatar.size {
        let y = top + row;
        if y < clip.0.max(0) || y >= clip.1.min(height) {
            continue;
        }
        for col in 0..avatar.size {
            let x = left + col;
            if x >= 0 && x < width {
                pixels[(y * width + x) as usize] =
                    avatar.pixels[(row * avatar.size + col) as usize];
            }
        }
    }
}

/// Draws a smooth (anti-aliased) filled circle straight into the back buffer.
fn fill_circle(back: &mut BackBuffer, cx: f32, cy: f32, radius: f32, color: u32, clip: (i32, i32)) {
    let (width, height) = (back.width, back.height);
    let color = to_pixel(color);
    let pixels = back.pixels_mut();
    let top = ((cy - radius - 1.0).floor() as i32).max(clip.0).max(0);
    let bottom = ((cy + radius + 1.0).ceil() as i32).min(clip.1).min(height);
    let left = ((cx - radius - 1.0).floor() as i32).max(0);
    let right = ((cx + radius + 1.0).ceil() as i32).min(width);
    for y in top..bottom {
        for x in left..right {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            if coverage > 0.0 {
                let i = (y * width + x) as usize;
                pixels[i] = blend(pixels[i], color, (coverage * 255.0).round() as u32);
            }
        }
    }
}

/// Mixes `src` over `dst` (both 0x00RRGGBB) with `alpha` from 0 to 255.
fn blend(dst: u32, src: u32, alpha: u32) -> u32 {
    let channel = |shift: u32| {
        let d = (dst >> shift) & 0xFF;
        let s = (src >> shift) & 0xFF;
        ((s * alpha + d * (255 - alpha) + 127) / 255) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// A GDI color (0x00BBGGRR) as a bitmap pixel (0x00RRGGBB).
fn to_pixel(color: u32) -> u32 {
    (color & 0xFF) << 16 | (color & 0xFF00) | (color >> 16) & 0xFF
}

fn scale(value: i32, dpi: u32) -> i32 {
    (value as f32 * dpi as f32 / 96.0).round() as i32
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Windows' MAKEINTRESOURCEW: refers to a resource by its number instead of a name.
fn make_int_resource(id: u16) -> *const u16 {
    id as usize as *const u16
}

// ---- text ----

fn tooltip_text(account: &Account, vac: Option<bool>) -> String {
    let last_login = account.timestamp.and_then(exact_time).map_or_else(
        || "Last login unknown".to_owned(),
        |t| format!("Last login {t}"),
    );
    let vac = match vac {
        Some(true) => "VAC banned",
        Some(false) => "No VAC bans",
        None => "VAC status unknown",
    };
    format!("{last_login}\n{vac}")
}

fn relative_time(timestamp: i64) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match (now - timestamp).max(0) {
        s if s < MIN => "just now".to_owned(),
        s if s < HOUR => format!("{} min ago", s / MIN),
        s if s < DAY => format!("{} h ago", s / HOUR),
        s if s < 2 * DAY => "yesterday".to_owned(),
        s if s < 14 * DAY => format!("{} days ago", s / DAY),
        s if s < 60 * DAY => format!("{} weeks ago", s / (7 * DAY)),
        s if s < 365 * DAY => format!("{} months ago", s / (30 * DAY)),
        s if s < 730 * DAY => "1 year ago".to_owned(),
        s => format!("{} years ago", s / (365 * DAY)),
    }
}

fn exact_time(timestamp: i64) -> Option<String> {
    let local = chrono::DateTime::from_timestamp(timestamp, 0)?.with_timezone(&chrono::Local);
    Some(local.format("%-d %b %Y, %H:%M").to_string())
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn placeholder_color(id64: u64) -> u32 {
    const COLORS: [u32; 5] = [
        rgb(0x7F, 0x77, 0xDD),
        rgb(0x1D, 0x9E, 0x75),
        rgb(0xD8, 0x5A, 0x30),
        rgb(0x37, 0x8A, 0xDD),
        rgb(0xD4, 0x53, 0x7E),
    ];
    COLORS[(id64 % COLORS.len() as u64) as usize]
}

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default()
}
