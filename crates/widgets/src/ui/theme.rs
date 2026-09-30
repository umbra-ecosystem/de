//! Colours. Everything comes from the built-in theme (`gpui-component`'s `Theme`), so switching light, dark or the
//! system appearance restyles the whole window; nothing here is a colour of our own.
//!
//! A tone from a view model becomes a theme colour here, and only here.

use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::*;

use crate::vm::{ThemeChoice, Tone};

#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: Hsla,
    /// Side panels and bars: the theme's sidebar colour.
    pub panel: Hsla,
    /// A quiet grouped area (the theme's group box).
    pub surface: Hsla,
    pub border: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub ok: Hsla,
    pub warn: Hsla,
    pub bad: Hsla,
    /// Hotfixes: the theme's magenta.
    pub hot: Hsla,
    pub add_bg: Hsla,
    pub del_bg: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub dark: bool,
}

impl Pal {
    pub fn of(cx: &App) -> Self {
        let t = cx.theme();
        Self {
            bg: t.background,
            panel: t.sidebar,
            surface: t.group_box,
            border: t.border,
            fg: t.foreground,
            muted: t.muted_foreground,
            faint: t.muted_foreground.opacity(0.65),
            accent: t.primary,
            ok: t.success,
            warn: t.warning,
            bad: t.danger,
            hot: t.magenta,
            add_bg: t.success.opacity(0.16),
            del_bg: t.danger.opacity(0.16),
            hover: t.list_hover,
            selected: t.list_active,
            dark: t.is_dark(),
        }
    }

    pub fn tone(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Neutral => self.muted,
            Tone::Accent => self.accent,
            Tone::Ok => self.ok,
            Tone::Warn => self.warn,
            Tone::Bad => self.bad,
            Tone::Hot => self.hot,
        }
    }
}

pub fn mono(cx: &App) -> SharedString {
    cx.theme().mono_font_family.clone()
}

/// Apply the user's choice through the built-in theme system.
pub fn apply_theme(choice: ThemeChoice, window: &mut Window, cx: &mut App) {
    match choice {
        ThemeChoice::System => Theme::sync_system_appearance(Some(window), cx),
        ThemeChoice::Light => Theme::change(ThemeMode::Light, Some(window), cx),
        ThemeChoice::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
    }
}
