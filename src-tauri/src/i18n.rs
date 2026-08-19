//! Bilingual (zh/en) strings for the native shell: tray, native menu bar
//! (macOS), OS notifications, and the handful of `Result<_, String>` errors
//! that surface directly in the boot/error screen or the plugin-install log
//! (see `ui/app.js`'s own `STRINGS`/`t()` for the frontend half of this same
//! split — it detects locale independently, client-side, since none of this
//! reaches the WebView). Deliberately *not* used for the free-form
//! diagnostic log tail (`push_log`/`get_log_tail`): translating every
//! interpolated log line is a much bigger, lower-value undertaking than the
//! primary UI text this covers, and that log is debug scrollback the user
//! opts into, not primary UI.
//!
//! Locale is detected fresh at each call site (cheap OS call, no caching
//! needed) using the same "positively-detected English, else Chinese" rule
//! `@deepseek-ai/dsh-client-locale` uses for the harness web UI itself (see
//! GitHub issue #23's own analysis of that package). Deliberately independent
//! of that package's persisted `locale.preference` setting: the harness page
//! is a plain remote page with zero Tauri IPC access (see lib.rs's module doc
//! comment), so there is no in-repo way to read its setting, and no reason to
//! invent a side channel just for this.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

pub fn detect() -> Lang {
    match sys_locale::get_locale() {
        Some(locale) if locale.to_lowercase().starts_with("en") => Lang::En,
        _ => Lang::Zh,
    }
}

/// `en` on a detected English locale, `zh` otherwise — see the module doc
/// comment for why "otherwise" (not just "on detected Chinese") is correct.
/// For strings that need a value interpolated somewhere other than a plain
/// trailing position (surrounding punctuation/word order genuinely differs
/// between the two languages), call sites branch on `Lang` directly instead
/// of trying to force this helper's shape onto them.
pub fn tr(lang: Lang, zh: &'static str, en: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Zh => zh,
    }
}
