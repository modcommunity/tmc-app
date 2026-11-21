//! The system tray, "close to tray", and notification cards. Desktop only.
//!
//! Three opt-in behaviours, all OFF until the user turns them on under
//! Settings → Advanced:
//!
//!   * **Close to tray** (`minimise_to_tray`): closing the window hides it and
//!     leaves the app running, with a tray icon to bring it back or quit.
//!   * **Desktop notifications** (`tray_notifications`): while the app is alive,
//!     poll the account's notifications and show each new one as a small card
//!     in the bottom-right corner of the screen.
//!
//! The card is a real, borderless, always-on-top window rendering the app's own
//! bundle (`components/notification-toast`), not an OS notification, so it looks
//! the same on every platform and can carry a per-kind design.
//!
//! TRUST: the card's text came from the network. It reaches the page as a JSON
//! literal in an initialisation script and is rendered as text. What a click can
//! do is decided HERE, against the notice this process stored for that window —
//! the webview never supplies a URL — and only for a link on the site's own
//! host (`tmc_core::notify::safe_open_url`).

use std::sync::Mutex;
use std::time::Duration;

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
};

use tmc_core::api::Method;
use tmc_core::audit;
use tmc_core::notify::{self, Cursor, Notice};

use crate::state::AppState;

const TRAY_ID: &str = "gamespice-tray";
const TOAST_PREFIX: &str = "toast-";
const MENU_LABEL: &str = "traymenu";

/// Logical size of a card, and the gap to the screen edge and between cards.
const CARD_W: f64 = 380.0;
const CARD_H: f64 = 108.0;
const MARGIN: f64 = 16.0;
/// A card closes itself after this long even if the page never asks to.
const FAILSAFE_CLOSE: Duration = Duration::from_secs(45);
/// At most this many cards on screen; an older one makes way for a new one.
const MAX_ON_SCREEN: usize = 4;

#[derive(Default)]
pub struct Notifier {
    cursor: Mutex<Cursor>,
    /// Live cards, oldest first, by window label, with what a click opens.
    cards: Mutex<Vec<(String, Option<String>)>>,
    serial: std::sync::atomic::AtomicU64,
    /// The signed-in user the cursor belongs to, so an account switch starts
    /// from a fresh baseline instead of announcing the other account's backlog.
    account: Mutex<Option<String>>,
}

/// Show the main window and give it focus.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Create or remove the tray icon to match the settings. Safe to call any
/// number of times. Returns whether a tray icon exists afterwards.
pub fn apply(app: &AppHandle) -> bool {
    let wanted = app.state::<AppState>().settings.get().minimise_to_tray;

    if !wanted {
        let _ = app.remove_tray_by_id(TRAY_ID);

        return false;
    }

    if app.tray_by_id(TRAY_ID).is_some() {
        return true;
    }

    match build_tray(app) {
        Ok(()) => true,
        Err(e) => {
            // No tray (a Linux desktop without a status-notifier host, say).
            // The caller must then NOT hide the window: an app with no window
            // and no tray is unreachable.
            tracing::warn!("could not create the tray icon: {e}");

            false
        }
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    // No native menu: a right-click opens our own small window instead (see
    // `open_tray_menu`), which is the only way to get the GameSpice look.
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("GameSpice App")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                let app = tray.app_handle().clone();

                match button {
                    MouseButton::Left => show_main(&app),
                    MouseButton::Right => {
                        // Tray events arrive on the event-loop thread, and
                        // building a window there deadlocks on Windows.
                        tauri::async_runtime::spawn(async move {
                            open_tray_menu(&app, position);
                        });
                    }
                    _ => {}
                }
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;

    Ok(())
}

/// Logical size of the tray menu window.
const MENU_W: f64 = 232.0;
const MENU_H: f64 = 262.0;

/// Open (or toggle off) the tray menu next to the cursor, kept inside the
/// work area of whichever monitor the click was on.
fn open_tray_menu(app: &AppHandle, at: tauri::PhysicalPosition<f64>) {
    if let Some(existing) = app.get_webview_window(MENU_LABEL) {
        let _ = existing.close();

        return;
    }

    let script = format!(
        "window.__GS_TRAY__ = {{ version: {:?} }};",
        env!("CARGO_PKG_VERSION")
    );

    let built = WebviewWindowBuilder::new(app, MENU_LABEL, WebviewUrl::App("index.html".into()))
        .title("GameSpice menu")
        .inner_size(MENU_W, MENU_H)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(true)
        .visible(false)
        .focused(true)
        .initialization_script(script.as_str())
        .build();

    let window = match built {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("could not open the tray menu: {e}");

            return;
        }
    };

    let monitor = app
        .monitor_from_point(at.x, at.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());

    let scale = monitor.as_ref().map_or(1.0, |m| m.scale_factor());
    let w = MENU_W * scale;
    let h = MENU_H * scale;

    // The menu grows up and to the left of the click, as the taskbar is
    // usually at the bottom right.
    let mut x = at.x - w;
    let mut y = at.y - h;

    if let Some(m) = &monitor {
        let area = m.work_area();
        let left = f64::from(area.position.x);
        let top = f64::from(area.position.y);
        let right = left + f64::from(area.size.width);
        let bottom = top + f64::from(area.size.height);

        x = x.clamp(left, (right - w).max(left));
        y = y.clamp(top, (bottom - h).max(top));
    }

    let _ = window.set_position(PhysicalPosition::new(x as i32, y as i32));

    // Like any menu, it goes away when focus leaves it.
    let closing = window.clone();

    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Focused(false)) {
            let _ = closing.close();
        }
    });

    let _ = window.show();
    let _ = window.set_focus();
}

/// A row of the tray menu was chosen. Only the menu window may ask, and the
/// action is a fixed word, never a path the page chose.
#[tauri::command]
pub async fn tray_action(app: AppHandle, window: Window, action: String) {
    if window.label() != MENU_LABEL {
        return;
    }

    let _ = window.close();

    let route = match action.as_str() {
        "open" => None,
        "play" => Some("/play"),
        "downloads" => Some("/downloads"),
        "settings" => Some("/settings"),
        "quit" => {
            app.exit(0);

            return;
        }
        _ => return,
    };

    show_main(&app);

    if let Some(route) = route {
        let _ = app.emit_to("main", "tray://navigate", route);
    }
}

/// The window's close button. Returns `true` when the close was turned into a
/// hide, in which case the caller must `prevent_close`.
pub fn close_to_tray(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();

    // Only a cheap, non-blocking check here: this runs on the event-loop thread
    // inside the close request, so anything slow or that waits on that same
    // thread (building a tray icon, hiding the window, a database write for the
    // audit line) freezes the whole window. The tray is built at start-up and
    // whenever the setting changes, so "no tray yet" means close normally.
    if !state.settings.get().minimise_to_tray || app.tray_by_id(TRAY_ID).is_none() {
        return false;
    }

    let handle = app.clone();

    tauri::async_runtime::spawn(async move {
        if let Some(w) = handle.get_webview_window("main") {
            let _ = w.hide();
        }

        audit!(
            handle.state::<AppState>().audit,
            Info,
            App,
            "app.tray",
            "Window closed to the system tray"
        );
    });

    true
}

/// Start the polling loop. It reads the settings on every tick, so switching
/// the feature on or off, or changing the interval, needs no restart.
pub fn spawn_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut waited = Duration::ZERO;

        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;

            waited += Duration::from_secs(2);

            let state = app.state::<AppState>();
            let settings = state.settings.get();

            if !settings.tray_notifications {
                // Off: forget the baseline, so turning it on later does not
                // announce everything that arrived while it was off.
                app.state::<Notifier>().cursor.lock().unwrap().reset();

                waited = Duration::from_secs(u64::from(settings.notification_poll_secs));

                continue;
            }

            if waited < Duration::from_secs(u64::from(settings.notification_poll_secs)) {
                continue;
            }

            waited = Duration::ZERO;

            if let Err(e) = poll_once(&app).await {
                tracing::debug!("notification poll failed: {e}");
            }
        }
    });
}

async fn poll_once(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let this = app.state::<Notifier>();

    // Not signed in: nothing to ask for, and no request to make.
    let Some(user) = state.auth.user() else {
        this.cursor.lock().unwrap().reset();

        return Ok(());
    };

    let who = user.id.clone();

    {
        let mut account = this.account.lock().unwrap();

        if account.as_deref() != Some(who.as_str()) {
            *account = Some(who);

            this.cursor.lock().unwrap().reset();
        }
    }

    let since = this.cursor.lock().unwrap().since().map(str::to_string);

    let mut path = format!("/notifications?limit={}", notify::POLL_LIMIT);

    if let Some(since) = since.filter(|s| !s.is_empty()) {
        path.push_str("&since=");
        path.push_str(&since.replace(['&', '#', ' '], ""));
    }

    let data = state
        .api
        .request(Method::GET, &path, None, true)
        .await
        .map_err(|e| e.detail().to_string())?;

    let (unread, items) = notify::parse(&data);

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let tip = match unread {
            0 => "GameSpice App".to_string(),
            n => format!("GameSpice App — {n} unread"),
        };

        let _ = tray.set_tooltip(Some(tip));
    }

    let fresh = this.cursor.lock().unwrap().advance(items);

    if fresh.is_empty() || window_in_front(app) {
        return Ok(());
    }

    for notice in collapse(fresh) {
        show_card(app, notice);
    }

    Ok(())
}

/// The user is looking at the app, so a card would only repeat the bell.
fn window_in_front(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false)
}

/// A burst becomes one summary card rather than a stack of windows.
fn collapse(fresh: Vec<Notice>) -> Vec<Notice> {
    if fresh.len() <= notify::MAX_CARDS_PER_POLL {
        return fresh;
    }

    let newest = fresh.last().cloned();

    vec![Notice {
        id: "summary".into(),
        kind: "SYSTEM".into(),
        title: format!("{} new notifications", fresh.len()),
        body: newest.map(|n| n.title).unwrap_or_default(),
        url: Some("/notifications".into()),
        created_at: String::new(),
        actor: None,
    }]
}

/// Open one card in the bottom-right corner, above any already showing.
pub fn show_card(app: &AppHandle, notice: Notice) {
    let this = app.state::<Notifier>();

    let n = this
        .serial
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let label = format!("{TOAST_PREFIX}{n}");

    // Make room: the oldest card goes before a fifth would appear.
    let evict = {
        let cards = this.cards.lock().unwrap();

        (cards.len() >= MAX_ON_SCREEN).then(|| cards[0].0.clone())
    };

    if let Some(old) = evict {
        if let Some(w) = app.get_webview_window(&old) {
            let _ = w.close();
        }
    }

    let click_url = notice
        .url
        .as_deref()
        .and_then(|u| notify::safe_open_url(u, tmc_core::api::api_base()));

    let payload = serde_json::json!({
        "notice": {
            "id": notice.id,
            "title": notice.title,
            "body": notice.body,
            "actor": notice.actor,
            "kind": notice.kind,
            "clickable": true,
        },
        "style": notify::style_for(&notice.kind),
    });

    // `</` cannot appear in a JSON literal inside a script and end it, because
    // this is an initialisation script and not markup — but escaping it costs
    // nothing and keeps that true if the injection point ever changes.
    let script = format!(
        "window.__GS_TOAST__ = {};",
        payload.to_string().replace("</", "<\\/")
    );

    this.cards.lock().unwrap().push((label.clone(), click_url));

    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("GameSpice notification")
        .inner_size(CARD_W, CARD_H)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .shadow(true)
        .visible(false)
        .initialization_script(script.as_str())
        .build();

    let window = match built {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("could not open a notification card: {e}");

            this.cards.lock().unwrap().retain(|(l, _)| l != &label);

            return;
        }
    };

    let handle = app.clone();
    let closed = label.clone();

    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            handle
                .state::<Notifier>()
                .cards
                .lock()
                .unwrap()
                .retain(|(l, _)| l != &closed);

            reflow(&handle);
        }
    });

    reflow(app);

    let _ = window.show();

    let failsafe = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FAILSAFE_CLOSE).await;

        if let Some(w) = failsafe.get_webview_window(&label) {
            let _ = w.close();
        }
    });
}

/// Lay the live cards out from the bottom-right of the primary monitor's work
/// area (clear of the taskbar), newest at the bottom.
fn reflow(app: &AppHandle) {
    let labels: Vec<String> = app
        .state::<Notifier>()
        .cards
        .lock()
        .unwrap()
        .iter()
        .map(|(l, _)| l.clone())
        .collect();

    let Ok(Some(monitor)) = app.primary_monitor() else {
        return;
    };

    let scale = monitor.scale_factor();
    let area = monitor.work_area();

    let w = CARD_W * scale;
    let h = CARD_H * scale;
    let gap = MARGIN * scale;

    let right = f64::from(area.position.x) + f64::from(area.size.width);
    let bottom = f64::from(area.position.y) + f64::from(area.size.height);

    for (i, label) in labels.iter().rev().enumerate() {
        let Some(win) = app.get_webview_window(label) else {
            continue;
        };

        let x = right - w - gap;
        let y = bottom - gap - (i as f64 + 1.0) * h - i as f64 * (gap / 2.0);

        let _ = win.set_position(PhysicalPosition::new(x as i32, y as i32));
    }
}

/// Which card is asking. Only a card this process opened may use the two
/// commands below.
fn card_label(window: &Window) -> Option<String> {
    window
        .label()
        .starts_with(TOAST_PREFIX)
        .then(|| window.label().to_string())
}

/// The card was clicked: bring the app forward and open what it linked to.
#[tauri::command]
pub fn toast_activate(app: AppHandle, window: Window) {
    let Some(label) = card_label(&window) else {
        return;
    };

    let url: Option<String> = app
        .state::<Notifier>()
        .cards
        .lock()
        .unwrap()
        .iter()
        .find(|(l, _)| *l == label)
        .and_then(|(_, u)| u.clone());

    show_main(&app);

    if let Some(url) = url {
        use tauri_plugin_opener::OpenerExt;

        let _ = app.opener().open_url(url, None::<&str>);
    }

    let _ = window.close();
}

/// The card timed out, or its close button was pressed.
#[tauri::command]
pub fn toast_dismiss(window: Window) {
    if card_label(&window).is_some() {
        let _ = window.close();
    }
}

/// For tests and the "send a test notification" button: show a card without
/// the network.
///
/// `async` on purpose: Tauri runs a synchronous command on the main thread, and
/// building a window from there deadlocks on Windows, so the button did nothing.
#[tauri::command]
pub async fn notification_test(app: AppHandle) {
    show_card(
        &app,
        Notice {
            id: "test".into(),
            kind: "SYSTEM".into(),
            title: "Notifications are on".into(),
            body: "This is what a GameSpice notification looks like.".into(),
            url: None,
            created_at: String::new(),
            actor: None,
        },
    );
}
