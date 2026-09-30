//! The three dock panels of the window: navigation (left), the screen (center) and details (right).
//!
//! A panel holds no state. It asks the app view to draw its part of the current frame, so the dock can
//! move, resize and collapse them while everything they show still comes from the session.

use gpui_kit::component::dock::{BasePanel, Panel, PanelControl, PanelEvent};
use gpui_kit::*;

use super::app::AppView;

macro_rules! panel {
    ($name:ident, $id:literal, $title:literal, $draw:ident) => {
        pub struct $name {
            app: WeakEntity<AppView>,
            focus: FocusHandle,
        }

        impl $name {
            pub fn new(app: WeakEntity<AppView>, cx: &mut Context<Self>) -> Self {
                Self {
                    app,
                    focus: cx.focus_handle(),
                }
            }
        }

        impl BasePanel for $name {
            fn panel_name(&self) -> &'static str {
                $id
            }
            fn closable(&self, _: &App) -> bool {
                false
            }
            fn zoomable(&self, _: &App) -> bool {
                false
            }
        }

        impl Panel for $name {
            fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                $title
            }
            fn zoom_control(&self, _: &App) -> Option<PanelControl> {
                None
            }
            fn inner_padding(&self, _: &App) -> bool {
                false
            }
            fn title_bar(&self, _: &App) -> bool {
                false
            }
        }

        impl EventEmitter<PanelEvent> for $name {}

        impl Focusable for $name {
            fn focus_handle(&self, _: &App) -> FocusHandle {
                self.focus.clone()
            }
        }

        impl Render for $name {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.app
                    .read_with(&*cx, |app, cx| app.$draw(cx))
                    .unwrap_or_else(|_| div().into_any_element())
            }
        }
    };
}

panel!(NavPanel, "de.nav", "Navigate", draw_nav);
panel!(MainPanel, "de.main", "Screen", draw_main);
panel!(DetailsPanel, "de.details", "Details", draw_details);
