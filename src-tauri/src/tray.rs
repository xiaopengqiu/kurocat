use crate::config::AppSettings;
use crate::i18n::{platform_click_through_key, t, tf};
use crate::{
    apply_breed_and_save, apply_keyboard_layout_and_save, apply_keyboard_orientation_and_save,
    apply_scale_and_save, apply_tail_key_mode_and_save, is_pet_visible, reset_window_position,
    set_pet_always_on_top, set_pet_visible, shutdown_pet_windows, AppState,
};
use std::sync::{Mutex, OnceLock};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager, Wry};

#[derive(Clone)]
struct TrayCheckHandles {
    pause_guard: CheckMenuItem<Wry>,
    click_through: CheckMenuItem<Wry>,
    key_bubbles: CheckMenuItem<Wry>,
    privacy_guard: CheckMenuItem<Wry>,
    sound_toggle: CheckMenuItem<Wry>,
    always_on_top: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    obs_toggle: CheckMenuItem<Wry>,
    obs_hide_pet: CheckMenuItem<Wry>,
    mcp_toggle: CheckMenuItem<Wry>,
    fps_60: CheckMenuItem<Wry>,
    fps_30: CheckMenuItem<Wry>,
    lang_auto: CheckMenuItem<Wry>,
    lang_zh_cn: CheckMenuItem<Wry>,
    lang_zh_tw: CheckMenuItem<Wry>,
    lang_en_us: CheckMenuItem<Wry>,
    lang_ja_jp: CheckMenuItem<Wry>,
    lang_ko_kr: CheckMenuItem<Wry>,
}

fn active_handles() -> &'static Mutex<Option<TrayCheckHandles>> {
    static HANDLES: OnceLock<Mutex<Option<TrayCheckHandles>>> = OnceLock::new();
    HANDLES.get_or_init(|| Mutex::new(None))
}

fn default_tray_tooltip(lang: &str) -> String {
    #[cfg(windows)]
    {
        if crate::win_is_elevated() {
            return t(lang, "tray.tooltip_admin");
        }
    }
    t(lang, "tray.tooltip")
}

pub fn sync_checkmarks(s: &AppSettings) {
    if let Ok(guard) = active_handles().lock() {
        if let Some(h) = guard.as_ref() {
            let _ = h.click_through.set_checked(s.click_through_locked);
            let _ = h.key_bubbles.set_checked(s.show_key_bubbles);
            let _ = h.privacy_guard.set_checked(s.privacy_password_guard);
            let _ = h.sound_toggle.set_checked(s.sound_enabled);
            let _ = h.always_on_top.set_checked(s.always_on_top);
            let _ = h.autostart.set_checked(s.autostart);
            let _ = h.obs_toggle.set_checked(s.obs_enabled);
            let _ = h.obs_hide_pet.set_checked(s.obs_enabled && s.obs_hide_pet);
            let _ = h.mcp_toggle.set_checked(s.mcp_enabled);
            let _ = h.fps_30.set_checked(s.fps_cap == 30);
            let _ = h.fps_60.set_checked(s.fps_cap != 30);
            let _ = h.lang_auto.set_checked(s.language == "auto");
            let _ = h.lang_zh_cn.set_checked(s.language == "zh-CN");
            let _ = h.lang_zh_tw.set_checked(s.language == "zh-TW");
            let _ = h.lang_en_us.set_checked(s.language == "en-US");
            let _ = h.lang_ja_jp.set_checked(s.language == "ja-JP");
            let _ = h.lang_ko_kr.set_checked(s.language == "ko-KR");
        }
    }
}

fn build_tray_menu(
    app: &AppHandle,
    initial: &AppSettings,
    paused: bool,
) -> tauri::Result<(Menu<Wry>, TrayCheckHandles)> {
    let lang = initial.language.as_str();

    #[cfg(windows)]
    let display_menu = crate::windows_display::create_menu(app)?;
    #[cfg(target_os = "macos")]
    let display_menu = crate::macos_window::create_display_menu(app)?;

    let toggle_show = MenuItem::with_id(app, "toggle_show", t(lang, "tray.toggle_show"), true, None::<&str>)?;
    let pause_guard = CheckMenuItem::with_id(
        app,
        "pause_guard",
        t(lang, "tray.pause_guard"),
        true,
        paused,
        None::<&str>,
    )?;
    #[cfg(windows)]
    let restart_as_admin = {
        let elevated = crate::win_is_elevated();
        let label = if elevated {
            t(lang, "tray.admin_ready")
        } else {
            t(lang, "tray.admin_restart")
        };
        MenuItem::with_id(app, "restart_as_admin", label, !elevated, None::<&str>)?
    };

    let click_through = CheckMenuItem::with_id(
        app,
        "click_through",
        t(lang, platform_click_through_key()),
        true,
        initial.click_through_locked,
        None::<&str>,
    )?;
    let key_bubbles = CheckMenuItem::with_id(
        app,
        "key_bubbles",
        t(lang, "tray.key_bubbles"),
        true,
        initial.show_key_bubbles,
        None::<&str>,
    )?;
    let privacy_guard = CheckMenuItem::with_id(
        app,
        "privacy_guard",
        t(lang, "tray.privacy_guard"),
        true,
        initial.privacy_password_guard,
        None::<&str>,
    )?;
    let sound_toggle = CheckMenuItem::with_id(
        app,
        "sound_toggle",
        t(lang, "tray.sound_toggle"),
        true,
        initial.sound_enabled,
        None::<&str>,
    )?;
    let always_on_top = CheckMenuItem::with_id(
        app,
        "always_on_top",
        t(lang, "tray.always_on_top"),
        true,
        initial.always_on_top,
        None::<&str>,
    )?;
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        t(lang, "tray.autostart"),
        true,
        initial.autostart,
        None::<&str>,
    )?;
    let mcp_toggle = CheckMenuItem::with_id(
        app,
        "mcp_toggle",
        t(lang, "tray.mcp_toggle"),
        true,
        initial.mcp_enabled,
        None::<&str>,
    )?;

    let obs_port = if initial.obs_port == 0 { 27421 } else { initial.obs_port };
    let obs_toggle = CheckMenuItem::with_id(
        app,
        "obs_toggle",
        tf(lang, "tray.obs_toggle", &[("port", obs_port.to_string())]),
        true,
        initial.obs_enabled,
        None::<&str>,
    )?;
    let obs_hide_pet = CheckMenuItem::with_id(
        app,
        "obs_hide_pet",
        t(lang, "tray.obs_hide_pet"),
        true,
        initial.obs_enabled && initial.obs_hide_pet,
        None::<&str>,
    )?;
    let obs_open_browser = MenuItem::with_id(
        app,
        "obs_open_browser",
        t(lang, "tray.obs_open_browser"),
        true,
        None::<&str>,
    )?;
    let obs_menu = Submenu::with_items(
        app,
        t(lang, "tray.obs_menu"),
        true,
        &[&obs_toggle, &obs_hide_pet, &obs_open_browser],
    )?;

    let breed_black = MenuItem::with_id(app, "breed_black", t(lang, "breeds.black"), true, None::<&str>)?;
    let breed_ragdoll = MenuItem::with_id(app, "breed_ragdoll", t(lang, "breeds.ragdoll"), true, None::<&str>)?;
    let breed_orange = MenuItem::with_id(app, "breed_orange", t(lang, "breeds.orange-tabby"), true, None::<&str>)?;
    let breed_calico = MenuItem::with_id(app, "breed_calico", t(lang, "breeds.calico"), true, None::<&str>)?;
    let breed_tuxedo = MenuItem::with_id(app, "breed_tuxedo", t(lang, "breeds.tuxedo"), true, None::<&str>)?;
    let breed_siamese = MenuItem::with_id(app, "breed_siamese", t(lang, "breeds.siamese"), true, None::<&str>)?;
    let breed_dafeiyu = MenuItem::with_id(app, "breed_dafeiyu", t(lang, "breeds.dafeiyu"), true, None::<&str>)?;
    let breed_menu = Submenu::with_items(
        app,
        t(lang, "tray.breed_menu"),
        true,
        &[
            &breed_black,
            &breed_ragdoll,
            &breed_orange,
            &breed_calico,
            &breed_tuxedo,
            &breed_siamese,
            &breed_dafeiyu,
        ],
    )?;

    let scale_25 = MenuItem::with_id(app, "scale_25", t(lang, "tray.scale_25"), true, None::<&str>)?;
    let scale_50 = MenuItem::with_id(app, "scale_50", t(lang, "tray.scale_50"), true, None::<&str>)?;
    let scale_75 = MenuItem::with_id(app, "scale_75", t(lang, "tray.scale_75"), true, None::<&str>)?;
    let scale_100 = MenuItem::with_id(app, "scale_100", t(lang, "tray.scale_100"), true, None::<&str>)?;
    let scale_125 = MenuItem::with_id(app, "scale_125", t(lang, "tray.scale_125"), true, None::<&str>)?;
    let scale_150 = MenuItem::with_id(app, "scale_150", t(lang, "tray.scale_150"), true, None::<&str>)?;
    let scale_menu = Submenu::with_items(
        app,
        t(lang, "tray.scale_menu"),
        true,
        &[&scale_25, &scale_50, &scale_75, &scale_100, &scale_125, &scale_150],
    )?;

    let kb_65 = MenuItem::with_id(app, "kb_65", t(lang, "tray.kb_65"), true, None::<&str>)?;
    let kb_108 = MenuItem::with_id(app, "kb_108", t(lang, "tray.kb_108"), true, None::<&str>)?;
    let kb_sep = PredefinedMenuItem::separator(app)?;
    let kb_cat = MenuItem::with_id(app, "kb_cat", t(lang, "tray.kb_cat"), true, None::<&str>)?;
    let kb_viewer = MenuItem::with_id(app, "kb_viewer", t(lang, "tray.kb_viewer"), true, None::<&str>)?;
    let kb_menu = Submenu::with_items(
        app,
        t(lang, "tray.kb_menu"),
        true,
        &[&kb_65, &kb_108, &kb_sep, &kb_cat, &kb_viewer],
    )?;

    let tail_smart = MenuItem::with_id(app, "tail_smart", t(lang, "tray.tail_smart"), true, None::<&str>)?;
    let tail_custom = MenuItem::with_id(app, "tail_custom", t(lang, "tray.tail_custom"), true, None::<&str>)?;
    let tail_edge = MenuItem::with_id(app, "tail_edge", t(lang, "tray.tail_edge"), true, None::<&str>)?;
    let tail_off = MenuItem::with_id(app, "tail_off", t(lang, "tray.tail_off"), true, None::<&str>)?;
    let tail_sep = PredefinedMenuItem::separator(app)?;
    let tail_pick = MenuItem::with_id(app, "tail_pick_keys", t(lang, "tray.tail_pick"), true, None::<&str>)?;
    let tail_menu = Submenu::with_items(
        app,
        t(lang, "tray.tail_menu"),
        true,
        &[
            &tail_smart,
            &tail_custom,
            &tail_edge,
            &tail_off,
            &tail_sep,
            &tail_pick,
        ],
    )?;

    let fps_60 = CheckMenuItem::with_id(app, "fps_60", t(lang, "tray.fps_60"), true, initial.fps_cap != 30, None::<&str>)?;
    let fps_30 = CheckMenuItem::with_id(app, "fps_30", t(lang, "tray.fps_30"), true, initial.fps_cap == 30, None::<&str>)?;
    let fps_menu = Submenu::with_items(app, t(lang, "tray.fps_menu"), true, &[&fps_60, &fps_30])?;

    let lang_auto = CheckMenuItem::with_id(app, "lang_auto", t(lang, "tray.lang_auto"), true, initial.language == "auto", None::<&str>)?;
    let lang_sep = PredefinedMenuItem::separator(app)?;
    let lang_zh_cn = CheckMenuItem::with_id(app, "lang_zh-CN", "简体中文", true, initial.language == "zh-CN", None::<&str>)?;
    let lang_zh_tw = CheckMenuItem::with_id(app, "lang_zh-TW", "繁體中文", true, initial.language == "zh-TW", None::<&str>)?;
    let lang_en_us = CheckMenuItem::with_id(app, "lang_en-US", "English", true, initial.language == "en-US", None::<&str>)?;
    let lang_ja_jp = CheckMenuItem::with_id(app, "lang_ja-JP", "日本語", true, initial.language == "ja-JP", None::<&str>)?;
    let lang_ko_kr = CheckMenuItem::with_id(app, "lang_ko-KR", "한국어", true, initial.language == "ko-KR", None::<&str>)?;
    let lang_menu = Submenu::with_items(
        app,
        t(lang, "tray.lang_menu"),
        true,
        &[
            &lang_auto,
            &lang_sep,
            &lang_zh_cn,
            &lang_zh_tw,
            &lang_en_us,
            &lang_ja_jp,
            &lang_ko_kr,
        ],
    )?;

    let reset_pos = MenuItem::with_id(app, "reset_pos", t(lang, "tray.reset_pos"), true, None::<&str>)?;
    #[cfg(windows)]
    let desktop_shortcut =
        MenuItem::with_id(app, "desktop_shortcut", t(lang, "tray.desktop_shortcut"), true, None::<&str>)?;
    #[cfg(target_os = "macos")]
    let mac_input_menu = {
        let mac_reset_perms = MenuItem::with_id(
            app,
            "mac_reset_permissions",
            t(lang, "tray.mac_reset_perms"),
            true,
            None::<&str>,
        )?;
        let mac_reconnect = MenuItem::with_id(
            app,
            "mac_reconnect_input",
            t(lang, "tray.mac_reconnect"),
            true,
            None::<&str>,
        )?;
        let mac_diag = MenuItem::with_id(
            app,
            "mac_diagnose_input",
            t(lang, "tray.mac_diag"),
            true,
            None::<&str>,
        )?;
        let mac_sep = PredefinedMenuItem::separator(app)?;
        let mac_open_listen = MenuItem::with_id(
            app,
            "mac_open_input_monitoring",
            t(lang, "tray.mac_open_listen"),
            true,
            None::<&str>,
        )?;
        let mac_open_ax = MenuItem::with_id(
            app,
            "mac_open_accessibility",
            t(lang, "tray.mac_open_ax"),
            true,
            None::<&str>,
        )?;
        Submenu::with_items(
            app,
            t(lang, "tray.mac_input_menu"),
            true,
            &[
                &mac_reset_perms,
                &mac_reconnect,
                &mac_diag,
                &mac_sep,
                &mac_open_listen,
                &mac_open_ax,
            ],
        )?
    };
    let check_update = MenuItem::with_id(
        app,
        "check_update",
        tf(
            lang,
            "tray.check_update",
            &[("ver", crate::updater::CURRENT_VERSION.to_string())],
        ),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", t(lang, "tray.quit"), true, None::<&str>)?;

    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    #[cfg(windows)]
    let menu = Menu::with_items(
        app,
        &[
            &toggle_show,
            &pause_guard,
            &restart_as_admin,
            &sep1,
            &click_through,
            &privacy_guard,
            &key_bubbles,
            &sound_toggle,
            &always_on_top,
            &breed_menu,
            &kb_menu,
            &tail_menu,
            &scale_menu,
            &fps_menu,
            &lang_menu,
            &desktop_shortcut,
            &display_menu,
            &reset_pos,
            &autostart,
            &obs_menu,
            &mcp_toggle,
            &check_update,
            &sep2,
            &quit,
        ],
    )?;

    #[cfg(target_os = "macos")]
    let menu = Menu::with_items(
        app,
        &[
            &toggle_show,
            &pause_guard,
            &sep1,
            &click_through,
            &privacy_guard,
            &key_bubbles,
            &sound_toggle,
            &always_on_top,
            &breed_menu,
            &kb_menu,
            &tail_menu,
            &scale_menu,
            &fps_menu,
            &lang_menu,
            &mac_input_menu,
            &display_menu,
            &reset_pos,
            &autostart,
            &obs_menu,
            &mcp_toggle,
            &check_update,
            &sep2,
            &quit,
        ],
    )?;

    #[cfg(all(not(windows), not(target_os = "macos")))]
    let menu = Menu::with_items(
        app,
        &[
            &toggle_show,
            &pause_guard,
            &sep1,
            &click_through,
            &privacy_guard,
            &key_bubbles,
            &sound_toggle,
            &always_on_top,
            &breed_menu,
            &kb_menu,
            &tail_menu,
            &scale_menu,
            &fps_menu,
            &lang_menu,
            &reset_pos,
            &autostart,
            &obs_menu,
            &mcp_toggle,
            &check_update,
            &sep2,
            &quit,
        ],
    )?;

    let handles = TrayCheckHandles {
        pause_guard,
        click_through,
        key_bubbles,
        privacy_guard,
        sound_toggle,
        always_on_top,
        autostart,
        obs_toggle,
        obs_hide_pet,
        mcp_toggle,
        fps_60,
        fps_30,
        lang_auto,
        lang_zh_cn,
        lang_zh_tw,
        lang_en_us,
        lang_ja_jp,
        lang_ko_kr,
    };

    Ok((menu, handles))
}

pub fn refresh_tray_language(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let paused = state.paused_guard.load(std::sync::atomic::Ordering::SeqCst);
    if let Ok((new_menu, new_handles)) = build_tray_menu(app, &settings, paused) {
        if let Ok(mut guard) = active_handles().lock() {
            *guard = Some(new_handles);
        }
        if let Some(tray) = app.tray_by_id("kuro-tray") {
            let _ = tray.set_menu(Some(new_menu));
            let tip = if paused {
                format!("KuroCat ({})", t(&settings.language, "tray.pause_guard"))
            } else {
                default_tray_tooltip(&settings.language)
            };
            let _ = tray.set_tooltip(Some(tip));
        }
    }
}

fn apply_language_and_save(app: &AppHandle, raw_lang: &str) {
    let state = app.state::<AppState>();
    let updated = {
        let mut s = state.settings.lock().unwrap();
        s.language = crate::config::normalize_language(raw_lang);
        crate::config::save_settings(&s);
        s.clone()
    };
    refresh_tray_language(app);
    let _ = app.emit("kuro-settings", updated);
}

fn apply_fps_cap_and_save(app: &AppHandle, cap: u16) {
    let state = app.state::<AppState>();
    let updated = {
        let mut s = state.settings.lock().unwrap();
        s.fps_cap = crate::config::normalize_fps_cap(cap);
        crate::config::save_settings(&s);
        s.clone()
    };
    sync_checkmarks(&updated);
    let _ = app.emit("kuro-settings", updated);
}

pub fn setup_tray(app: &AppHandle, initial: &AppSettings) -> tauri::Result<()> {
    let (menu, handles) = build_tray_menu(app, initial, false)?;
    if let Ok(mut guard) = active_handles().lock() {
        *guard = Some(handles);
    }

    let icon = Image::from_bytes(include_bytes!("../icons/tray-icon.png"))?;

    // Keep tray checkmarks synchronized when settings change from right-click menu, toast, or MCP.
    app.listen("kuro-settings", move |ev| {
        if let Ok(s) = serde_json::from_str::<AppSettings>(ev.payload()) {
            sync_checkmarks(&s);
        }
    });

    TrayIconBuilder::with_id("kuro-tray")
        .icon(icon)
        .tooltip(default_tray_tooltip(&initial.language))
        .menu(&menu)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let state = app.state::<AppState>();
                if state.paused_guard.load(std::sync::atomic::Ordering::SeqCst) {
                    state
                        .paused_guard
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    crate::input_hook::set_hooks_paused(false);
                    if let Ok(guard) = active_handles().lock() {
                        if let Some(h) = guard.as_ref() {
                            let _ = h.pause_guard.set_checked(false);
                        }
                    }
                    let lang = state.settings.lock().unwrap().language.clone();
                    let _ = tray.set_tooltip(Some(default_tray_tooltip(&lang)));
                    set_pet_visible(app, true);
                } else {
                    let vis = is_pet_visible(app);
                    set_pet_visible(app, !vis);
                }
            }
        })
        .on_menu_event(move |app, event| {
            let state = app.state::<AppState>();
            match event.id().as_ref() {
                "toggle_show" => {
                    if state.paused_guard.load(std::sync::atomic::Ordering::SeqCst) {
                        state
                            .paused_guard
                            .store(false, std::sync::atomic::Ordering::SeqCst);
                        crate::input_hook::set_hooks_paused(false);
                        if let Ok(guard) = active_handles().lock() {
                            if let Some(h) = guard.as_ref() {
                                let _ = h.pause_guard.set_checked(false);
                            }
                        }
                        let lang = state.settings.lock().unwrap().language.clone();
                        if let Some(tray) = app.tray_by_id("kuro-tray") {
                            let _ = tray.set_tooltip(Some(default_tray_tooltip(&lang)));
                        }
                        set_pet_visible(app, true);
                    } else {
                        let next_vis = !is_pet_visible(app);
                        if next_vis {
                            let mut s = state.settings.lock().unwrap();
                            if s.obs_hide_pet {
                                s.obs_hide_pet = false;
                                crate::config::save_settings(&s);
                                sync_checkmarks(&s);
                                let _ = app.emit("kuro-settings", s.clone());
                            }
                        }
                        set_pet_visible(app, next_vis);
                    }
                }
                "pause_guard" => {
                    let next = !state
                        .paused_guard
                        .load(std::sync::atomic::Ordering::SeqCst);
                    state
                        .paused_guard
                        .store(next, std::sync::atomic::Ordering::SeqCst);
                    crate::input_hook::set_hooks_paused(next);
                    if let Ok(guard) = active_handles().lock() {
                        if let Some(h) = guard.as_ref() {
                            let _ = h.pause_guard.set_checked(next);
                        }
                    }
                    let lang = state.settings.lock().unwrap().language.clone();
                    if let Some(tray) = app.tray_by_id("kuro-tray") {
                        let tip = if next {
                            format!("KuroCat ({})", t(&lang, "tray.pause_guard"))
                        } else {
                            default_tray_tooltip(&lang)
                        };
                        let _ = tray.set_tooltip(Some(tip));
                    }
                    set_pet_visible(app, !next);
                }
                #[cfg(windows)]
                "restart_as_admin" => {
                    if !crate::win_is_elevated() {
                        crate::win_restart_as_admin(app.clone());
                    }
                }
                "click_through" => {
                    let updated = {
                        let mut s = state.settings.lock().unwrap();
                        s.click_through_locked = !s.click_through_locked;
                        crate::config::save_settings(&s);
                        s.clone()
                    };
                    sync_checkmarks(&updated);
                    let _ = app.emit("kuro-settings", updated);
                }
                "key_bubbles" => {
                    let updated = {
                        let mut s = state.settings.lock().unwrap();
                        s.show_key_bubbles = !s.show_key_bubbles;
                        crate::config::save_settings(&s);
                        s.clone()
                    };
                    sync_checkmarks(&updated);
                    let _ = app.emit("kuro-settings", updated);
                }
                "privacy_guard" => {
                    let updated = {
                        let mut s = state.settings.lock().unwrap();
                        s.privacy_password_guard = !s.privacy_password_guard;
                        crate::config::save_settings(&s);
                        s.clone()
                    };
                    sync_checkmarks(&updated);
                    let _ = app.emit("kuro-settings", updated);
                }
                "sound_toggle" => {
                    let updated = {
                        let mut s = state.settings.lock().unwrap();
                        s.sound_enabled = !s.sound_enabled;
                        crate::config::save_settings(&s);
                        s.clone()
                    };
                    sync_checkmarks(&updated);
                    let _ = app.emit("kuro-settings", updated);
                }
                "always_on_top" => {
                    let (on_top, updated) = {
                        let mut s = state.settings.lock().unwrap();
                        s.always_on_top = !s.always_on_top;
                        let on_top = s.always_on_top;
                        crate::config::save_settings(&s);
                        (on_top, s.clone())
                    };
                    sync_checkmarks(&updated);
                    set_pet_always_on_top(app, on_top);
                    let _ = app.emit("kuro-settings", updated);
                }
                "autostart" => {
                    let mut s = state.settings.lock().unwrap();
                    let next = !s.autostart;
                    if crate::config::set_windows_autostart(next).is_ok() {
                        s.autostart = next;
                        crate::config::save_settings(&s);
                        let updated = s.clone();
                        drop(s);
                        sync_checkmarks(&updated);
                        let _ = app.emit("kuro-settings", updated);
                    }
                }
                "obs_toggle" => {
                    let (enabled, port, updated) = {
                        let mut s = state.settings.lock().unwrap();
                        s.obs_enabled = !s.obs_enabled;
                        if !s.obs_enabled {
                            s.obs_hide_pet = false;
                        }
                        if s.obs_port == 0 {
                            s.obs_port = 27421;
                        }
                        crate::config::save_settings(&s);
                        (s.obs_enabled, s.obs_port, s.clone())
                    };
                    crate::obs::sync_obs_server(app, enabled, port);
                    sync_checkmarks(&updated);
                    if !enabled && !state.paused_guard.load(std::sync::atomic::Ordering::Relaxed) {
                        set_pet_visible(app, true);
                    }
                    let _ = app.emit("kuro-settings", updated);
                }
                "obs_hide_pet" => {
                    let (hide_now, port, updated) = {
                        let mut s = state.settings.lock().unwrap();
                        let next_hide = !(s.obs_enabled && s.obs_hide_pet);
                        if next_hide && !s.obs_enabled {
                            s.obs_enabled = true;
                        }
                        if s.obs_port == 0 {
                            s.obs_port = 27421;
                        }
                        s.obs_hide_pet = next_hide;
                        crate::config::save_settings(&s);
                        (next_hide, s.obs_port, s.clone())
                    };
                    crate::obs::sync_obs_server(app, updated.obs_enabled, port);
                    sync_checkmarks(&updated);
                    if !state.paused_guard.load(std::sync::atomic::Ordering::Relaxed) {
                        set_pet_visible(app, !hide_now);
                    }
                    let _ = app.emit("kuro-settings", updated);
                }
                "obs_open_browser" => {
                    let (port, enabled_changed, updated) = {
                        let mut s = state.settings.lock().unwrap();
                        if s.obs_port == 0 {
                            s.obs_port = 27421;
                        }
                        let changed = !s.obs_enabled;
                        if changed {
                            s.obs_enabled = true;
                            crate::config::save_settings(&s);
                        }
                        (s.obs_port, changed, s.clone())
                    };
                    crate::obs::sync_obs_server(app, true, port);
                    if enabled_changed {
                        sync_checkmarks(&updated);
                        let _ = app.emit("kuro-settings", updated);
                    }
                    crate::updater::open_url_in_browser(&crate::obs::obs_url(port));
                }
                "mcp_toggle" => {
                    let updated = {
                        let mut s = state.settings.lock().unwrap();
                        s.mcp_enabled = !s.mcp_enabled;
                        let enabled = s.mcp_enabled;
                        state
                            .mcp_enabled
                            .store(enabled, std::sync::atomic::Ordering::Relaxed);
                        crate::config::save_settings(&s);
                        s.clone()
                    };
                    sync_checkmarks(&updated);
                    let _ = app.emit("kuro-settings", updated);
                }
                "breed_black" => apply_breed_and_save(app, "black"),
                "breed_ragdoll" => apply_breed_and_save(app, "ragdoll"),
                "breed_orange" => apply_breed_and_save(app, "orange-tabby"),
                "breed_calico" => apply_breed_and_save(app, "calico"),
                "breed_tuxedo" => apply_breed_and_save(app, "tuxedo"),
                "breed_siamese" => apply_breed_and_save(app, "siamese"),
                "breed_dafeiyu" => apply_breed_and_save(app, "dafeiyu"),
                "scale_25" => apply_scale_and_save(app, 0.25),
                "scale_50" => apply_scale_and_save(app, 0.5),
                "scale_75" => apply_scale_and_save(app, 0.75),
                "scale_100" => apply_scale_and_save(app, 1.0),
                "scale_125" => apply_scale_and_save(app, 1.25),
                "scale_150" => apply_scale_and_save(app, 1.5),
                "kb_65" => apply_keyboard_layout_and_save(app, "65"),
                "kb_108" => apply_keyboard_layout_and_save(app, "108"),
                "kb_cat" => apply_keyboard_orientation_and_save(app, "cat"),
                "kb_viewer" => apply_keyboard_orientation_and_save(app, "viewer"),
                "tail_smart" => apply_tail_key_mode_and_save(app, "smart"),
                "tail_custom" => apply_tail_key_mode_and_save(app, "custom"),
                "tail_edge" => apply_tail_key_mode_and_save(app, "edge"),
                "tail_off" => apply_tail_key_mode_and_save(app, "off"),
                "tail_pick_keys" => {
                    set_pet_visible(app, true);
                    let _ = app.emit("kuro-open-tail-picker", ());
                }
                "fps_60" => apply_fps_cap_and_save(app, 0),
                "fps_30" => apply_fps_cap_and_save(app, 30),
                "lang_auto" => apply_language_and_save(app, "auto"),
                "lang_zh-CN" => apply_language_and_save(app, "zh-CN"),
                "lang_zh-TW" => apply_language_and_save(app, "zh-TW"),
                "lang_en-US" => apply_language_and_save(app, "en-US"),
                "lang_ja-JP" => apply_language_and_save(app, "ja-JP"),
                "lang_ko-KR" => apply_language_and_save(app, "ko-KR"),
                "desktop_shortcut" => {
                    let _ = crate::config::create_desktop_shortcut();
                }
                #[cfg(target_os = "macos")]
                "mac_reset_permissions" => {
                    crate::input_hook::macos_hook::reset_and_request_macos_permissions();
                }
                #[cfg(target_os = "macos")]
                "mac_reconnect_input" => {
                    crate::input_hook::macos_hook::trigger_macos_reconnect();
                }
                #[cfg(target_os = "macos")]
                "mac_diagnose_input" => {
                    crate::input_hook::macos_hook::show_macos_diagnostics_dialog();
                }
                #[cfg(target_os = "macos")]
                "mac_open_input_monitoring" => {
                    crate::input_hook::macos_hook::open_macos_privacy_pane("input_monitoring");
                }
                #[cfg(target_os = "macos")]
                "mac_open_accessibility" => {
                    crate::input_hook::macos_hook::open_macos_privacy_pane("accessibility");
                }
                "check_update" => {
                    let app_clone = app.clone();
                    let lang = state.settings.lock().unwrap().language.clone();
                    set_pet_visible(app, true);
                    if crate::updater::emit_check_status(
                        &app_clone,
                        "checking",
                        t(&lang, "toast.checking"),
                        None,
                    ) {
                        return;
                    }
                    std::thread::spawn(move || match crate::updater::check_latest_update() {
                        Ok(info) => {
                            let (stage, msg) = if info.has_update {
                                (
                                    "available",
                                    tf(&lang, "toast.available", &[("ver", info.latest_version.clone())]),
                                )
                            } else {
                                (
                                    "up_to_date",
                                    tf(&lang, "toast.up_to_date", &[("ver", info.current_version.clone())]),
                                )
                            };
                            crate::updater::emit_check_status(
                                &app_clone,
                                stage,
                                msg,
                                Some(info),
                            );
                        }
                        Err(err) => {
                            crate::updater::emit_check_status(&app_clone, "error", err, None);
                        }
                    });
                }
                "reset_pos" => reset_window_position(app),
                "quit" => {
                    shutdown_pet_windows();
                    app.exit(0);
                }
                _ => {
                    #[cfg(windows)]
                    crate::windows_display::handle_menu(app, event.id().as_ref());
                    #[cfg(target_os = "macos")]
                    crate::macos_window::handle_display_menu(app, event.id().as_ref());
                }
            }
        })
        .build(app)?;

    Ok(())
}
