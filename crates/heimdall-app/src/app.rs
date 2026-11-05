//! Top-level state machine: which screen is showing, the open session,
//! auto-lock (also when Windows locks or sleeps), autosave, clipboard
//! auto-clear and capture exclusion.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, OutputCommand};
use heimdall_core::content::{LineEnding, Profile, SecuritySettings};
use heimdall_core::Vault;
use zeroize::Zeroize;

use crate::screens::editor::EditorState;
use crate::screens::settings::Settings;
use crate::screens::unlock::{LockInfo, Unlock};
use crate::screens::welcome::Welcome;
use crate::screens::wizard::Wizard;
use crate::session::Session;
use crate::support::{self, clock, ClipboardGuard, RecentVaults};
use crate::usb::UsbKey;
use crate::{chrome, theme};

/// What a screen asks the app to do next.
pub enum Action {
    Welcome,
    PickFile,
    Open(PathBuf),
    Create,
    /// A vault was created, unlocked, or had its access reset. `usb` is the
    /// key it was opened with (or enrolled), if any.
    Opened {
        vault: Vault,
        path: PathBuf,
        usb: Option<UsbKey>,
    },
    /// Opened with the recovery code: a new password is mandatory (DESIGN §9 step 5).
    Recovered {
        vault: Vault,
        path: PathBuf,
    },
    Lock(LockReason),
    Settings,
    /// Settings, open on the keyboard shortcuts.
    Shortcuts,
    Notes,
}

#[derive(Clone, Copy)]
pub enum LockReason {
    Manual,
    Inactivity(u32),
    Minimized,
    UsbRemoved,
    WindowsLocked,
    Suspended,
}

enum Screen {
    Welcome(Welcome),
    Unlock(Box<Unlock>),
    Wizard(Box<Wizard>),
    Editor,
    Settings(Box<Settings>),
}

pub struct HeimdallApp {
    screen: Screen,
    session: Option<Session>,
    editor: EditorState,
    recent: RecentVaults,
    clipboard: ClipboardGuard,
    last_input: Instant,
    /// The native window, once known; and what was last applied to it.
    window: Option<isize>,
    capture_excluded: Option<bool>,
}

impl HeimdallApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply_style(&cc.egui_ctx);
        let recent = RecentVaults::load();
        Self {
            screen: Screen::Welcome(Welcome::new(&recent)),
            session: None,
            editor: EditorState::default(),
            recent,
            clipboard: ClipboardGuard::default(),
            last_input: Instant::now(),
            window: None,
            capture_excluded: None,
        }
    }

    /// Finds the native window on the first frame and starts watching for
    /// Windows locking or going to sleep (DESIGN §12).
    fn attach_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.window.is_some() {
            return;
        }
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = frame.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(h) = handle.as_raw() else {
            return;
        };
        let hwnd = h.hwnd.get();
        self.window = Some(hwnd);
        let ctx = ctx.clone();
        heimdall_platform::watch_session(hwnd, move || ctx.request_repaint());
    }

    /// Hidden from screen capture unless an open Basic vault turned it off.
    /// With no vault open (Welcome, Unlock, the wizard) it is always on.
    ///
    /// Debug builds leave the window capturable when `HEIMDALL_ALLOW_CAPTURE`
    /// is set, so UI tests can take screenshots.
    fn apply_capture_exclusion(&mut self) {
        let Some(hwnd) = self.window else {
            return;
        };
        #[cfg(debug_assertions)]
        if std::env::var_os("HEIMDALL_ALLOW_CAPTURE").is_some() {
            return;
        }
        let on = self
            .session
            .as_ref()
            .is_none_or(|s| s.vault.content.security.exclude_from_capture);
        if self.capture_excluded != Some(on) && heimdall_platform::set_capture_excluded(hwnd, on) {
            self.capture_excluded = Some(on);
        }
    }

    /// Sleep always locks; Win+L too, unless a Basic vault turned it off.
    fn session_event_lock(&self) -> Option<LockReason> {
        let event = heimdall_platform::take_session_event()?;
        let security = &self.session.as_ref()?.vault.content.security;
        match event {
            heimdall_platform::SessionEvent::Suspending => Some(LockReason::Suspended),
            heimdall_platform::SessionEvent::Locked => (security.profile != Profile::Basic
                || security.lock_on_windows_lock)
                .then_some(LockReason::WindowsLocked),
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Welcome => self.screen = Screen::Welcome(Welcome::new(&self.recent)),
            Action::PickFile => {
                let picked = rfd::FileDialog::new()
                    .set_title("Open vault")
                    .add_filter("Heimdall vault", &["heim"])
                    .pick_file();
                if let Some(path) = picked {
                    self.screen = Screen::Unlock(Box::new(Unlock::new(path, None)));
                }
            }
            Action::Open(path) => self.screen = Screen::Unlock(Box::new(Unlock::new(path, None))),
            Action::Create => self.screen = Screen::Wizard(Box::new(Wizard::create())),
            Action::Opened { vault, path, usb } => self.opened(ctx, vault, path, usb),
            Action::Recovered { vault, path } => {
                self.screen = Screen::Wizard(Box::new(Wizard::reset(vault, path)))
            }
            Action::Lock(reason) => self.lock(ctx, reason),
            Action::Settings => self.screen = Screen::Settings(Box::default()),
            Action::Shortcuts => self.screen = Screen::Settings(Box::new(Settings::shortcuts())),
            Action::Notes => self.screen = Screen::Editor,
        }
    }

    fn opened(&mut self, ctx: &egui::Context, mut vault: Vault, path: PathBuf, usb: Option<UsbKey>) {
        // The password fields' text and undo history live in egui's memory.
        wipe_ui_memory(ctx);
        let purged = vault.content.purge_expired_trash(heimdall_core::content::now());
        if vault.content.security.remember_recent {
            self.recent.touch(&path);
        } else {
            self.recent.remove(&path);
        }
        let mut session = Session::new(ctx, vault, path, usb);
        if purged > 0 {
            session.touch(None);
        }
        self.editor = EditorState::new(&session);
        self.session = Some(session);
        self.screen = Screen::Editor;
        self.last_input = Instant::now();
    }

    /// Saves pending changes, wipes the vault from memory and shows the
    /// lock screen (DESIGN §9 step 3). If saving fails the vault stays open,
    /// so no work is lost; the error is shown in the status bar. The one
    /// exception is a removed USB key: presence mode always locks (§8.3).
    fn lock(&mut self, ctx: &egui::Context, reason: LockReason) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let must_lock = matches!(reason, LockReason::UsbRemoved);
        if session.has_unsaved() && !session.save() && !must_lock {
            self.screen = Screen::Editor;
            return;
        }
        let saved_at = match (&session.save_error, session.last_saved) {
            (Some(e), _) if must_lock => Some(format!("Your last changes couldn't be saved: {e}")),
            (_, Some(t)) => Some(format!("Your changes were saved at {}.", clock(t))),
            _ => None,
        };
        let path = session.path.clone();
        self.session = None; // drops the Vault: MK and note text are zeroized
        self.clipboard.clear_now();
        self.editor = EditorState::default();
        wipe_ui_memory(ctx);

        let reason = match reason {
            LockReason::Manual => "Locked".to_owned(),
            LockReason::Inactivity(s) => {
                format!("Locked after {} of inactivity", crate::support::duration_label(s))
            }
            LockReason::Minimized => "Locked when the window was minimized".to_owned(),
            LockReason::UsbRemoved => "Locked because the USB key was removed".to_owned(),
            LockReason::WindowsLocked => "Locked when Windows was locked".to_owned(),
            LockReason::Suspended => "Locked before the computer went to sleep".to_owned(),
        };
        self.screen = Screen::Unlock(Box::new(Unlock::new(path, Some(LockInfo { reason, saved_at }))));
    }

    fn auto_lock(&mut self, ctx: &egui::Context) -> Option<LockReason> {
        let session = self.session.as_ref()?;
        if session.presence.as_ref().is_some_and(|p| p.removed()) {
            return Some(LockReason::UsbRemoved);
        }
        // Not while a new recovery code is on screen or keys are being
        // changed: locking then would lose the code (it is already saved).
        if matches!(&self.screen, Screen::Settings(page) if page.holds_lock()) {
            return None;
        }
        let security = &session.vault.content.security;
        if security.lock_on_minimize && ctx.input(|i| i.viewport().minimized == Some(true)) {
            return Some(LockReason::Minimized);
        }
        let secs = security.lock_after_s?;
        (self.last_input.elapsed() >= Duration::from_secs(secs as u64))
            .then_some(LockReason::Inactivity(secs))
    }

    /// Every copy: CRLF conversion, kept out of the clipboard history, and
    /// auto-cleared (DESIGN §10.4, §11.2).
    fn handle_copies(&mut self, ctx: &egui::Context) {
        let crlf = self
            .session
            .as_ref()
            .is_some_and(|s| s.vault.content.editor.line_ending == LineEnding::Crlf);
        let timer = ctx.data_mut(|d| d.remove_temp::<u32>(support::clear_after_id()));
        let clear_after = timer
            .or(self
                .session
                .as_ref()
                .map(|s| s.vault.content.security.clipboard_clear_s))
            .unwrap_or(SecuritySettings::defaults_for(Profile::Basic).clipboard_clear_s);
        // A copy that includes a masked secret: its own, shorter timer (§11.2).
        let secret_after = self
            .session
            .as_ref()
            .map_or(clear_after, |s| s.vault.content.security.secret_clear_s)
            .min(clear_after);
        ctx.output_mut(|o| {
            o.commands.retain_mut(|cmd| {
                let OutputCommand::CopyText(text) = cmd else {
                    return true;
                };
                if crlf {
                    let converted = text.replace("\r\n", "\n").replace('\n', "\r\n");
                    text.zeroize();
                    *text = converted;
                }
                let after = if heimdall_core::secret::contains_secret(text) {
                    secret_after
                } else {
                    clear_after
                };
                self.clipboard.copied(text, after);
                if !support::set_clipboard(text) {
                    // Let egui try; then the text isn't kept out of the history.
                    return true;
                }
                text.zeroize();
                false
            });
        });
    }
}

/// egui keeps text-edit state (including undo history) in its memory.
/// Reset it on lock so no note text lingers there.
fn wipe_ui_memory(ctx: &egui::Context) {
    ctx.memory_mut(|m| *m = Default::default());
    theme::apply_style(ctx);
}

impl eframe::App for HeimdallApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(theme::BG).to_array()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        chrome::handle_resize(ctx);
        self.attach_window(ctx, frame);

        if ctx.input(|i| !i.events.is_empty() || i.pointer.is_moving()) {
            self.last_input = Instant::now();
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            if let Some(s) = self.session.as_mut() {
                if s.has_unsaved() {
                    s.save();
                }
            }
            self.clipboard.clear_now();
        }

        if let Some(reason) = self.session_event_lock().or_else(|| self.auto_lock(ctx)) {
            self.lock(ctx, reason);
        }
        if let Some(s) = self.session.as_mut() {
            if s.autosave_due() {
                s.save();
            }
        }

        let action = match &mut self.screen {
            Screen::Welcome(w) => w.ui(ctx),
            Screen::Unlock(u) => u.ui(ctx),
            Screen::Wizard(w) => w.ui(ctx),
            Screen::Editor => match self.session.as_mut() {
                Some(s) => crate::screens::editor::ui(ctx, s, &mut self.editor, self.last_input),
                None => Some(Action::Welcome),
            },
            Screen::Settings(page) => match self.session.as_mut() {
                Some(s) => page.ui(ctx, s, &mut self.recent),
                None => Some(Action::Welcome),
            },
        };
        if let Some(action) = action {
            self.apply(ctx, action);
        }

        self.handle_copies(ctx);
        self.apply_capture_exclusion();
        let clip_due = self.clipboard.tick();
        chrome::paint_border(ctx);

        // Wake up for the lock countdown, autosave and clipboard clearing.
        let mut next = Duration::from_secs(1);
        if let Some(d) = clip_due {
            next = next.min(d);
        }
        if self.session.is_some() || clip_due.is_some() {
            ctx.request_repaint_after(next);
        }
    }
}
