//! What a view needs besides its view model: the palette, a way to emit intents, and the text inputs.

use std::collections::BTreeMap;

use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::*;

use super::app::AppView;
use super::theme::Pal;
use crate::vm::{Field, Intent};

/// Handed to every view function. Cheap to clone.
#[derive(Clone)]
pub struct Ui {
    pub view: WeakEntity<AppView>,
    pub pal: Pal,
    pub mono: SharedString,
}

impl Ui {
    /// Send an intent to the session and redraw.
    pub fn send(&self, intent: Intent, cx: &mut App) {
        self.view
            .update(cx, move |v, cx| {
                v.session.handle(intent);
                cx.notify();
            })
            .ok();
    }

    /// A click handler that emits `intent`.
    pub fn on_click(
        &self,
        intent: Intent,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let ui = self.clone();
        move |_, _, cx| ui.send(intent.clone(), cx)
    }

    /// The same for handlers that receive a checkbox state.
    pub fn on_toggle(&self, intent: Intent) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
        let ui = self.clone();
        move |_, _, cx| ui.send(intent.clone(), cx)
    }
}

pub enum TextBox {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl TextBox {
    pub fn value(&self, cx: &App) -> String {
        match self {
            TextBox::Line(e) => e.read(cx).value().to_string(),
            TextBox::Area(e) => e.read(cx).value().to_string(),
        }
    }

    pub fn set(&self, text: &str, window: &mut Window, cx: &mut App) {
        match self {
            TextBox::Line(e) => e.update(cx, |s, cx| s.set_value(text.to_string(), window, cx)),
            TextBox::Area(e) => e.update(cx, |s, cx| s.set_value(text.to_string(), window, cx)),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        let handle = match self {
            TextBox::Line(e) => e.read(cx).focus_handle(cx),
            TextBox::Area(e) => e.read(cx).focus_handle(cx),
        };
        handle.focus(window, cx);
    }
}

/// The live text inputs, by field. Created by the app view; the screens only look them up.
#[derive(Default)]
pub struct Inputs {
    pub map: BTreeMap<Field, TextBox>,
}

impl Inputs {
    /// A single-line input for `field`, or an empty element if it was not created.
    pub fn line(&self, field: &Field) -> AnyElement {
        match self.map.get(field) {
            Some(TextBox::Line(e)) => Input::new(e).into_any_element(),
            _ => div().into_any_element(),
        }
    }

    /// A single-line input with no border or fill and right-aligned text, so a settings row keeps the look of a
    /// plain value (label left, value right) and is still editable in place.
    pub fn flat(&self, field: &Field) -> AnyElement {
        match self.map.get(field) {
            Some(TextBox::Line(e)) => Input::new(e)
                .appearance(false)
                .text_right()
                .h(px(28.0))
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    /// The search field of a list strip: a search icon, a little taller than a plain input. The strip is 48px tall
    /// so it has some air; the caller caps its width.
    pub fn search(&self, field: &Field) -> AnyElement {
        match self.map.get(field) {
            Some(TextBox::Line(e)) => Input::new(e)
                .h(px(36.0))
                .prefix(Icon::new(IconName::Search).small())
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    pub fn area(&self, field: &Field) -> AnyElement {
        match self.map.get(field) {
            Some(TextBox::Area(e)) => Textarea::new(e).into_any_element(),
            _ => div().into_any_element(),
        }
    }
}
