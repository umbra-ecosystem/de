//! Small stateless widgets. Each takes plain view-model data and the [`Ui`]; none reads or writes state.

use std::hash::{Hash, Hasher};

use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyTitle};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Disableable, IconName, Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::ctx::Ui;
use super::theme::{Pal, mono};
use crate::vm::*;

pub fn hash_id(prefix: &str, what: &impl std::fmt::Debug) -> SharedString {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{what:?}").hash(&mut h);
    format!("{prefix}-{:x}", h.finish()).into()
}

/* ------------------------------ atoms ------------------------------ */

/// A small label in a tone: status, priority, deploy state. The kit's `Tag`, coloured from the theme.
pub fn pill(pal: &Pal, badge: &Badge) -> Tag {
    let tag = if badge.tone == Tone::Neutral {
        Tag::secondary()
    } else {
        let c = pal.tone(badge.tone);
        Tag::custom(c.opacity(0.15), c, c.opacity(0.35))
    };
    tag.small().child(badge.text.clone())
}

pub fn pills(pal: &Pal, badges: &[Badge]) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap_1()
        .children(badges.iter().map(|b| pill(pal, b)))
}

pub fn hotfix_pill(pal: &Pal) -> Tag {
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
        .text_color(ui.pal.accent)
        .child(key.to_string())
}

/// A quiet icon button that copies `value` to the clipboard.
pub fn copy_button(ui: &Ui, value: impl Into<String>) -> Button {
    let value: String = value.into();
    Button::new(hash_id("copy", &value))
        .ghost()
        .small()
        .icon(IconName::Copy)
        .tooltip("Copy")
        .on_click(ui.on_click(Intent::Copy(value)))
}

/// A quiet icon button that opens `url` in the browser.
pub fn open_button(ui: &Ui, url: impl Into<String>, tip: &str) -> Button {
    let url: String = url.into();
    Button::new(hash_id("open", &url))
        .ghost()
        .small()
        .icon(IconName::ExternalLink)
        .tooltip(tip.to_string())
        .on_click(ui.on_click(Intent::OpenExternal(url)))
}

/// Mono text with a copy button beside it (branch, SHA, command). The button is
/// small and quiet; it copies `copy` (defaults to what is shown).
pub fn copyable_mono(ui: &Ui, shown: impl Into<String>, copy: Option<String>) -> Div {
    let shown: String = shown.into();
    let value = copy.unwrap_or_else(|| shown.clone());
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .font_family(ui.mono.clone())
                .text_sm()
                .child(shown),
        )
        .child(copy_button(ui, value))
}

pub fn muted(pal: &Pal, text: impl Into<SharedString>) -> Div {
    div().text_sm().text_color(pal.muted).child(text.into())
}

pub fn faint(pal: &Pal, text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(pal.faint).child(text.into())
}

pub fn avatar(name: &str) -> Avatar {
    Avatar::new().name(name.to_string()).small()
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
}

/// A full-width block of a screen: a label, then its content. A hairline runs edge to edge above every block but
/// the first. The label keeps the page margin; the content decides its own (text is padded with [`pad`], tables and
/// lists run flush so their row lines reach the edges).
pub fn band(
    pal: &Pal,
    first: bool,
    title: impl Into<SharedString>,
    content: impl IntoElement,
) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .when(!first, |d| {
            d.border_t_1().border_color(pal.border.opacity(0.6))
        })
        .child(
            div()
                .px_4()
                .pt_4()
                .pb_2()
                .text_xs()
                .text_color(pal.muted)
                .child(title.into()),
        )
        .child(content)
}

/// Page margin around text content inside a [`band`].
pub fn pad(content: impl IntoElement) -> Div {
    div().px_4().pb_4().child(content)
}

/// A titled block of a screen.
pub fn section(pal: &Pal, title: impl Into<SharedString>) -> Div {
    let title: SharedString = title.into();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .when(!title.is_empty(), |d| {
            d.child(div().text_xs().text_color(pal.muted).child(title.clone()))
        })
}

pub fn heading(title: impl Into<SharedString>) -> Div {
    div().text_lg().child(title.into())
}

/// An empty screen that helps: a title, what the place is for, and the next useful thing to do.
pub fn empty_panel(ui: &Ui, e: &EmptyVm) -> Empty {
    let pal = &ui.pal;
    let mut empty = Empty::new().header(
        EmptyHeader::new()
            .title(EmptyTitle::new().child(e.title.clone()))
            .description(EmptyDescription::new().child(e.hint.clone())),
    );
    if !e.actions.is_empty() {
        empty = empty.content(
            EmptyContent::new().child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap_2()
                    .text_color(pal.fg)
                    .children(e.actions.iter().map(|a| button(ui, a))),
            ),
        );
    }
    empty
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
                .text_color(c)
                .when(b.tone == Tone::Bad, |d| d.font_weight(FontWeight::BOLD))
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
                        div()
                            .text_color(pal.fg)
                            .child(b.text.clone())
                            .into_any_element()
                    } else {
                        pill(pal, b).into_any_element()
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

/// A row of tabs (the kit's `TabBar`). `items` are `(label, marker dot)`; clicking tab `i` emits `intents[i]`.
pub fn tab_bar(
    ui: &Ui,
    id: &'static str,
    items: Vec<(String, bool)>,
    selected: usize,
    intents: Vec<Intent>,
) -> TabBar {
    let pal = ui.pal;
    let ui = ui.clone();
    TabBar::new(id)
        .underline()
        // The bar runs edge to edge; this keeps the first tab in line with the content's margin.
        .prefix(div().w_4())
        .selected_index(selected)
        .children(items.into_iter().map(move |(label, dot_on)| {
            let t = Tab::new().label(label);
            if dot_on { t.suffix(dot(pal.accent)) } else { t }
        }))
        .on_click(move |ix, _, cx| {
            if let Some(i) = intents.get(*ix) {
                ui.send(i.clone(), cx);
            }
        })
}

/// A toggle-style filter button (ticket groups, diff mode, theme).
pub fn chip(ui: &Ui, label: impl Into<SharedString>, on: bool, intent: Intent) -> Button {
    let label: SharedString = label.into();
    Button::new(hash_id("chip", &(&label, &intent)))
        .label(label)
        .small()
        .ghost()
        .selected(on)
        .on_click(ui.on_click(intent))
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
                    .child(inline_text(pal, cx, x))
                    .into_any_element(),
                Block::Heading(x) => div().text_base().pt_2().child(x.clone()).into_any_element(),
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
                            .child(div().child(inline_text(pal, cx, i)))
                    }))
                    .into_any_element(),
                Block::Code(x) => code_block(pal, cx, x.clone()).into_any_element(),
                Block::Numbered(items) => div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .children(items.iter().enumerate().map(|(n, i)| {
                        div()
                            .flex()
                            .gap_2()
                            .text_sm()
                            .child(
                                div()
                                    .flex_none()
                                    .min_w_4()
                                    .text_color(pal.muted)
                                    .child(format!("{}.", n + 1)),
                            )
                            .child(div().child(inline_text(pal, cx, i)))
                    }))
                    .into_any_element(),
                Block::Quote(x) => div()
                    .pl_3()
                    .border_l_2()
                    .border_color(pal.border)
                    .text_sm()
                    .text_color(pal.muted)
                    .child(inline_text(pal, cx, x))
                    .into_any_element(),
            }
        }))
}

/// Text with `@[name]` mentions highlighted. Words are separate flex items so it wraps like a paragraph.
pub fn inline_text(pal: &Pal, cx: &App, text: &str) -> Div {
    let mut row = div().flex().flex_wrap().items_baseline().gap_x_1();
    let mut rest = text;
    let word = |w: &str| div().child(w.to_string());
    // Two markers ride in the text: `@[Name|id]` mentions, and `![name]` references to attached
    // files, which render as a chip naming the file where the body references it. The file itself
    // stays in the Attachments band only when nothing references it.
    loop {
        let mention = rest.find("@[").map(|s| (s, false));
        let file = rest.find("![").map(|s| (s, true));
        let Some((start, is_file)) = mention.into_iter().chain(file).min_by_key(|(s, _)| *s)
        else {
            break;
        };
        let Some(len) = rest[start..].find(']') else {
            break;
        };
        for w in rest[..start].split_whitespace() {
            row = row.child(word(w));
        }
        if is_file {
            let name = &rest[start + 2..start + len];
            row = row.child(
                div()
                    .px_1()
                    .rounded_sm()
                    .font_family(mono(cx))
                    .text_color(pal.fg)
                    .bg(pal.border.opacity(0.4))
                    .child(name.to_string()),
            );
        } else {
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
        }
        rest = &rest[start + len + 1..];
    }
    for w in rest.split_whitespace() {
        row = row.child(word(w));
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
        .flex_wrap()
        .items_center()
        .gap_1()
        .children(s.steps.iter().enumerate().map(|(i, (label, st))| {
            // Colour lives on the dot only; the words stay neutral (current step brightest).
            let (dot_colour, text) = match st {
                StepState::Done => (pal.ok, pal.muted),
                StepState::Now => (pal.accent, pal.fg),
                StepState::Bad => (pal.bad, pal.fg),
                StepState::Todo => (pal.faint, pal.faint),
            };
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(i > 0, |d| d.child(div().w_3().h_px().bg(pal.border)))
                .child(dot(dot_colour))
                .child(div().text_xs().text_color(text).child(label.clone()))
        }))
}
