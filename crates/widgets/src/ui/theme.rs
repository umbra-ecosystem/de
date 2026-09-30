//! Colours: a tone from a view model becomes a theme colour here, and only here.

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use crate::vm::Tone;

#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: Hsla,
    pub panel: Hsla,
    pub raised: Hsla,
    pub border: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub ok: Hsla,
    pub warn: Hsla,
    pub bad: Hsla,
    pub hot: Hsla,
    pub add_bg: Hsla,
    pub del_bg: Hsla,
    pub hover: Hsla,
    pub dark: bool,
}

impl Pal {
    pub fn of(cx: &App) -> Self {
        let t = cx.theme();
        let dark = t.is_dark();
        Self {
            bg: t.background,
            panel: t.sidebar,
            raised: if dark {
                t.background.opacity(0.0).blend(t.secondary)
            } else {
                t.secondary
            },
            border: t.border,
            fg: t.foreground,
            muted: t.muted_foreground,
            faint: t.muted_foreground.opacity(0.65),
            accent: t.primary,
            ok: t.success,
            warn: t.warning,
            bad: t.danger,
            hot: hsla(0.03, 0.85, if dark { 0.62 } else { 0.5 }, 1.0),
            add_bg: t.success.opacity(if dark { 0.16 } else { 0.14 }),
            del_bg: t.danger.opacity(if dark { 0.16 } else { 0.12 }),
            hover: t.list_hover,
            dark,
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

    pub fn tone_bg(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Neutral => self.border.opacity(0.45),
            t => self.tone(t).opacity(0.15),
        }
    }
}

pub fn mono(cx: &App) -> SharedString {
    cx.theme().mono_font_family.clone()
}
