//! Small stateless widgets. Each takes plain view-model data and the [`Ui`]; none reads or writes state.

use std::hash::{Hash, Hasher};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::ctx::Ui;
use super::theme::{Pal, mono};
use crate::vm::*;

fn hash_id(prefix: &str, what: &impl std::fmt::Debug) -> SharedString {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{what:?}").hash(&mut h);
    format!("{prefix}-{:x}", h.finish()).into()
}

/* ------------------------------ atoms ------------------------------ */

/// A small rounded label in a tone: status, priority, deploy state.
pub fn pill(pal: &Pal, badge: &Badge) -> Div {
    let c = pal.tone(badge.tone);
    div()
        .flex_none()
        .px_1p5()
        .py_0p5()
        .rounded_md()
        .text_xs()
        .text_color(if badge.tone == Tone::Neutral {
            pal.muted
        } else {
            c
        })
        .bg(pal.tone_bg(badge.tone))
        .whitespace_nowrap()
        .child(badge.text.clone())
}

pub fn pills(pal: &Pal, badges: &[Badge]) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap_1()
        .children(badges.iter().map(|b| pill(pal, b)))
}

pub fn hotfix_pill(pal: &Pal) -> Div {
    pill(pal, &Badge::new("HOTFIX", Tone::Hot))
}

/// A button for a view-model [`Btn`]. `ghost` makes it quiet (dismiss, snooze).
pub fn button(ui: &Ui, a: &Btn) -> Button {
    let b = Button::new(hash_id("btn", &(&a.label, &a.intent)))
        .label(a.label.clone())
        .small()
        .on_click(ui.on_click(a.intent.clone()));
    let b = if a.primary { b.primary() } else { b };
    let b = match &a.hint {
        Some(h) => b.tooltip(h.clone()),
        None => b,
    };
    b.disabled(!a.enabled)
}

pub fn button_ghost(ui: &Ui, a: &Btn) -> Button {
    button(ui, a).ghost()
}

pub fn buttons(ui: &Ui, actions: &[Btn]) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .children(actions.iter().map(|a| button(ui, a)))
}

/// A ticket key that navigates to the ticket when clicked.
pub fn key_link(ui: &Ui, key: &str, intent: Intent) -> Stateful<Div> {
    div()
        .id(hash_id("key", &(key, &intent)))
        .flex_none()
        .font_family(ui.mono.clone())
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ui.pal.accent)
        .cursor_pointer()
        .hover(|s| s.underline())
        .on_click(ui.on_click(intent))
        .child(key.to_string())
}

pub fn mono_text(cx: &App, text: impl Into<SharedString>) -> Div {
    div().font_family(mono(cx)).text_sm().child(text.into())
}

pub fn key_text(ui: &Ui, key: &str) -> Div {
    div()
        .flex_none()
        .font_family(ui.mono.clone())
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ui.pal.accent)
        .child(key.to_string())
}

pub fn muted(pal: &Pal, text: impl Into<SharedString>) -> Div {
    div().text_sm().text_color(pal.muted).child(text.into())
}

pub fn faint(pal: &Pal, text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(pal.faint).child(text.into())
}

pub fn avatar(pal: &Pal, initials: &str) -> Div {
    div()
        .flex_none()
        .size_5()
        .rounded_full()
        .bg(pal.accent.opacity(0.22))
        .text_color(pal.accent)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(initials.to_string())
}

pub fn dot(color: Hsla) -> Div {
    div().flex_none().size_2().rounded_full().bg(color)
}

/* ------------------------------ layout ------------------------------ */

pub fn card(pal: &Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(pal.border)
        .bg(pal.raised)
}

/// A titled block of a screen.
pub fn section(pal: &Pal, title: impl Into<SharedString>) -> Div {
    div().flex().flex_col().gap_2().child(
        div()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(pal.muted)
            .child(title.into().to_uppercase()),
    )
}

pub fn heading(title: impl Into<SharedString>) -> Div {
    div()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.into())
}

pub fn empty_state(pal: &Pal, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .p_8()
        .rounded_lg()
        .border_1()
        .border_color(pal.border)
        .text_color(pal.muted)
        .child(text.into())
}

pub fn warn_box(pal: &Pal, tone: Tone, children: impl IntoIterator<Item = AnyElement>) -> Div {
    let c = pal.tone(tone);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(c.opacity(0.5))
        .bg(c.opacity(0.1))
        .text_sm()
        .children(children)
}

/// A risk/uat/pr banner: a title, some lines and its actions.
pub fn banner(ui: &Ui, b: &BannerVm) -> Div {
    let pal = &ui.pal;
    let c = pal.tone(b.tone);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(c.opacity(0.55))
        .bg(c.opacity(0.1))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c)
                .child(b.title.clone()),
        )
        .children(
            b.lines
                .iter()
                .map(|l| div().text_sm().text_color(pal.fg).child(l.clone())),
        )
        .when(!b.actions.is_empty(), |d| {
            d.child(div().pt_1().child(buttons(ui, &b.actions)))
        })
}

pub fn kv(pal: &Pal, k: &Kv) -> Div {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap_3()
        .text_sm()
        .child(div().flex_none().text_color(pal.muted).child(k.key.clone()))
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap_1()
                .children(k.value.iter().map(|b| {
                    if b.tone == Tone::Neutral && k.value.len() == 1 {
                        div().text_color(pal.fg).child(b.text.clone())
                    } else {
                        pill(pal, b)
                    }
                })),
        )
}

pub fn check_row(
    ui: &Ui,
    id: &str,
    label: &str,
    checked: bool,
    intent: Intent,
) -> impl IntoElement {
    Checkbox::new(hash_id(id, &(label, &intent)))
        .label(label.to_string())
        .checked(checked)
        .on_click(ui.on_toggle(intent))
}

/// A segmented row of tabs (ticket tabs, ticket groups, diff mode).
pub fn seg_tab(
    ui: &Ui,
    label: impl Into<SharedString>,
    on: bool,
    dot_on: bool,
    intent: Intent,
) -> Stateful<Div> {
    let pal = &ui.pal;
    let label: SharedString = label.into();
    div()
        .id(hash_id("seg", &(&label, &intent)))
        .flex()
        .items_center()
        .gap_1()
        .px_3()
        .py_1()
        .text_sm()
        .cursor_pointer()
        .border_b_2()
        .border_color(if on {
            pal.accent
        } else {
            gpui_kit::transparent_black()
        })
        .text_color(if on { pal.fg } else { pal.muted })
        .font_weight(if on {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .hover(|s| s.text_color(pal.fg))
        .on_click(ui.on_click(intent))
        .child(label)
        .when(dot_on, |d| d.child(dot(pal.accent)))
}

pub fn chip(ui: &Ui, label: impl Into<SharedString>, on: bool, intent: Intent) -> Stateful<Div> {
    let pal = &ui.pal;
    let label: SharedString = label.into();
    div()
        .id(hash_id("chip", &(&label, &intent)))
        .px_2()
        .py_0p5()
        .rounded_full()
        .text_xs()
        .cursor_pointer()
        .border_1()
        .border_color(if on { pal.accent } else { pal.border })
        .bg(if on {
            pal.accent.opacity(0.18)
        } else {
            gpui_kit::transparent_black()
        })
        .text_color(if on { pal.accent } else { pal.muted })
        .on_click(ui.on_click(intent))
        .child(label)
}

/// A monospace block: commands, logs, drafts.
pub fn code_block(pal: &Pal, cx: &App, text: impl Into<SharedString>) -> Div {
    div()
        .p_2()
        .rounded_md()
        .bg(pal.border.opacity(0.35))
        .font_family(mono(cx))
        .text_xs()
        .whitespace_normal()
        .child(text.into())
}

/// Jira's rich text: paragraphs, headings, lists, code, `@[you]` mentions.
pub fn blocks(pal: &Pal, cx: &App, items: &[Block]) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(items.iter().map(|b| {
            match b {
                Block::Para(x) => div()
                    .text_sm()
                    .child(inline_text(pal, x))
                    .into_any_element(),
                Block::Heading(x) => div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .pt_1()
                    .child(x.clone())
                    .into_any_element(),
                Block::List(items) => div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .children(items.iter().map(|i| {
                        div()
                            .flex()
                            .gap_2()
                            .text_sm()
                            .child(div().text_color(pal.muted).child("•"))
                            .child(div().child(inline_text(pal, i)))
                    }))
                    .into_any_element(),
                Block::Code(x) => code_block(pal, cx, x.clone()).into_any_element(),
            }
        }))
}

/// Text with `@[name]` mentions highlighted.
pub fn inline_text(pal: &Pal, text: &str) -> Div {
    let mut row = div().flex().flex_wrap().items_baseline();
    let mut rest = text;
    while let Some(start) = rest.find("@[") {
        let Some(len) = rest[start..].find(']') else {
            break;
        };
        if start > 0 {
            row = row.child(rest[..start].to_string());
        }
        let name = &rest[start + 2..start + len];
        let me = name.eq_ignore_ascii_case("you");
        row = row.child(
            div()
                .px_1()
                .rounded_sm()
                .text_color(if me { pal.accent } else { pal.fg })
                .bg(if me {
                    pal.accent.opacity(0.18)
                } else {
                    pal.border.opacity(0.4)
                })
                .child(format!("@{}", if me { "you" } else { name })),
        );
        rest = &rest[start + len + 1..];
    }
    if !rest.is_empty() {
        row = row.child(rest.to_string());
    }
    row
}

pub fn deploy_chip(pal: &Pal, c: &DeployChip) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(pill(pal, &Badge::new(c.text.clone(), c.tone)))
        .child(faint(
            pal,
            match &c.step {
                Some(s) => format!("#{} · {s}", c.run),
                None => format!("#{}", c.run),
            },
        ))
}

pub fn step_badge(pal: &Pal, n: usize, state: StepState) -> Div {
    let (c, text) = match state {
        StepState::Done => (pal.ok, "✓".to_string()),
        StepState::Now => (pal.accent, n.to_string()),
        StepState::Bad => (pal.bad, "✕".to_string()),
        StepState::Todo => (pal.faint, n.to_string()),
    };
    div()
        .flex_none()
        .size_6()
        .rounded_full()
        .border_1()
        .border_color(c)
        .text_color(c)
        .text_sm()
        .flex()
        .items_center()
        .justify_center()
        .child(text)
}

/// The nine-step progress of a ticket, on every ticket screen.
pub fn stepper(pal: &Pal, s: &StepperVm) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(s.steps.iter().enumerate().map(|(i, (label, st))| {
            let (c, w) = match st {
                StepState::Done => (pal.ok, FontWeight::NORMAL),
                StepState::Now => (pal.accent, FontWeight::SEMIBOLD),
                _ => (pal.faint, FontWeight::NORMAL),
            };
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(i > 0, |d| d.child(div().w_3().h_px().bg(pal.border)))
                .child(dot(c))
                .child(
                    div()
                        .text_xs()
                        .text_color(c)
                        .font_weight(w)
                        .child(label.clone()),
                )
        }))
}
