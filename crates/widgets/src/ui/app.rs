//! The root view of the showcase: one [`Session`] drawn through the widgets.
//!
//! This is the only place that owns GPUI entities (the text inputs) and the clock. Everything it draws comes from
//! `Session::view()`; everything a click does goes back through `Session::handle`.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::dock::{DockArea, DockLayout, DockPlacement, DockSkin, panel_handle};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::{ActiveTheme, Theme, TitleBar};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::ctx::{Inputs, TextBox, Ui};
use super::panels::{DetailsPanel, MainPanel, NavPanel};
use super::screens::{env, next, ticket, tickets};
use super::shell;
use super::theme::Pal;
use crate::session::Session;
use crate::vm::*;

gpui_kit::actions!(de_widgets, [OpenPaletteAction, CancelAction, Quit]);

/// What a text input needs: which field, single or multi-line, and its placeholder.
struct Need {
    field: Field,
    multiline: bool,
    placeholder: &'static str,
}

fn needs(vm: &AppVm) -> Vec<Need> {
    let mut out = Vec::new();
    let mut add = |field: Field, multiline: bool, placeholder: &'static str| {
        out.push(Need {
            field,
            multiline,
            placeholder,
        });
    };
    if let ScreenVm::Ticket { body, .. } = &vm.screen {
        match body.as_ref() {
            TicketBody::Overview(o) => {
                add(
                    Field::Comment(o.key.clone()),
                    true,
                    "Add a comment to Jira…",
                );
                add(
                    Field::Notes(o.key.clone()),
                    true,
                    "Private notes, saved locally…",
                );
            }
            TicketBody::Test(TestVm::Active { .. }) => {
                if let Route::Ticket { key, .. } = &vm.route {
                    add(Field::Checklist(key.clone()), false, "Add an item…");
                    add(
                        Field::Notes(key.clone()),
                        true,
                        "What you verified, what broke…",
                    );
                }
            }
            TicketBody::Ship(s) => {
                if let AnnounceBody::Draft { id, .. } = &s.announce {
                    add(
                        Field::Draft {
                            key: s.key.clone(),
                            id: id.clone(),
                        },
                        true,
                        "",
                    );
                }
            }
            TicketBody::Review(r) if r.composer.is_some() => {
                add(Field::Inline, true, "Comment on this line…");
            }
            _ => {}
        }
    }
    if matches!(vm.screen, ScreenVm::Next(_)) {
        add(Field::NextFilter, false, "Search suggestions…");
    }
    match &vm.sheet {
        Some(SheetVm::Confirm { preview, .. }) if preview.type_key.is_some() => {
            add(Field::TypedKey, false, "type the key");
        }
        Some(SheetVm::Compose { .. }) => add(Field::Compose, true, "Write the comment…"),
        Some(SheetVm::Palette { .. }) => add(Field::Palette, false, "Go to…"),
        _ => {}
    }
    out
}

/// A token that changes when a new sheet or composer opens, so its input is focused once.
fn focus_token(vm: &AppVm) -> Option<String> {
    match &vm.sheet {
        Some(SheetVm::Confirm { preview, .. }) if preview.type_key.is_some() => {
            Some(format!("confirm:{}", preview.title))
        }
        Some(SheetVm::Compose { title, .. }) => Some(format!("compose:{title}")),
        Some(SheetVm::Palette { .. }) => Some("palette".to_string()),
        _ => match &vm.screen {
            ScreenVm::Ticket { body, .. } => match body.as_ref() {
                TicketBody::Review(ReviewVm {
                    composer: Some((p, a)),
                    ..
                }) => Some(format!("inline:{p}:{a}")),
                _ => None,
            },
            _ => None,
        },
    }
}

/// `DE_SHOWCASE_ROUTE` (`next`, `tickets`, `uat`, `workspace`, `audit`, `settings`, `ticket:PROJ-142:review`) and
/// `DE_SHOWCASE_THEME` (`light`, `dark`) open the showcase somewhere specific; handy for screenshots.
fn apply_env(session: &mut Session) {
    if let Ok(theme) = std::env::var("DE_SHOWCASE_THEME") {
        match theme.as_str() {
            "light" => session.handle(Intent::SetTheme(ThemeChoice::Light)),
            "dark" => session.handle(Intent::SetTheme(ThemeChoice::Dark)),
            _ => {}
        }
    }
    let Ok(route) = std::env::var("DE_SHOWCASE_ROUTE") else {
        return;
    };
    let mut parts = route.split(':');
    let route = match parts.next() {
        Some("tickets") => Route::Tickets(Group::All),
        Some("uat") => Route::OnUat,
        Some("workspace") => Route::Workspace,
        Some("audit") => Route::Audit,
        Some("settings") => Route::Settings,
        Some("ticket") => {
            let key = parts.next().unwrap_or("PROJ-142");
            let tab = match parts.next() {
                Some("review") => TicketTab::Review,
                Some("test") => TicketTab::Test,
                Some("ship") => TicketTab::Ship,
                Some("timeline") => TicketTab::Timeline,
                _ => TicketTab::Overview,
            };
            Route::ticket(key, tab)
        }
        _ => Route::Next,
    };
    session.handle(Intent::Go(route));
}

pub struct AppView {
    pub session: Session,
    inputs: Inputs,
    focus: FocusHandle,
    focus_token: Option<String>,
    subs: Vec<Subscription>,
    area: Entity<DockArea>,
    weak: WeakEntity<AppView>,
    /// The frame being drawn: built once per render of this view, read by the dock panels.
    vm: Rc<AppVm>,
    right_open: bool,
    theme: ThemeChoice,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // One simulated minute passes per four real seconds; the session advances the fake world.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let alive = this.update(cx, |view, cx| {
                    view.session.tick(500);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        let mut session = Session::demo();
        apply_env(&mut session);
        let vm = Rc::new(session.view());

        // The dock: navigation on the left, details on the right, the screen in the middle.
        let (area, skin) = DockSkin::dock_area("de", None, window, cx);
        skin.set_toggle_button_visible(false, cx);
        skin.set_close_button_visible(false, cx);
        let app = cx.weak_entity();
        let nav = cx.new(|cx| NavPanel::new(app.clone(), cx));
        let main = cx.new(|cx| MainPanel::new(app.clone(), cx));
        let details = cx.new(|cx| DetailsPanel::new(app.clone(), cx));
        area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::tabs().panel_view(panel_handle(main), cx),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Left,
                DockLayout::tabs().panel_view(panel_handle(nav), cx),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Right,
                DockLayout::tabs().panel_view(panel_handle(details), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Left, px(210.0), window, cx);
            area.set_dock_size(DockPlacement::Right, px(300.0), window, cx);
        });

        Self {
            session,
            inputs: Inputs::default(),
            focus,
            focus_token: None,
            subs: Vec::new(),
            area,
            weak: app,
            vm,
            right_open: true,
            theme: ThemeChoice::System,
        }
    }

    fn ensure_input(&mut self, need: &Need, window: &mut Window, cx: &mut Context<Self>) {
        if self.inputs.map.contains_key(&need.field) {
            return;
        }
        let field = need.field.clone();
        let placeholder = need.placeholder;
        if need.multiline {
            let state = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(3, 10)
                    .placeholder(placeholder)
            });
            self.subs.push(cx.subscribe_in(
                &state,
                window,
                move |this: &mut Self, state, ev: &InputEvent, _window, cx| {
                    if matches!(ev, InputEvent::Change) {
                        let text = state.read(cx).value().to_string();
                        this.on_text(field.clone(), text, cx);
                    }
                },
            ));
            self.inputs
                .map
                .insert(need.field.clone(), TextBox::Area(state));
        } else {
            let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            self.subs.push(cx.subscribe_in(
                &state,
                window,
                move |this: &mut Self, state, ev: &InputEvent, _window, cx| match ev {
                    InputEvent::Change => {
                        let text = state.read(cx).value().to_string();
                        this.on_text(field.clone(), text, cx);
                    }
                    InputEvent::PressEnter { .. } => this.on_enter(&field, cx),
                    _ => {}
                },
            ));
            self.inputs
                .map
                .insert(need.field.clone(), TextBox::Line(state));
        }
    }

    fn on_text(&mut self, field: Field, text: String, cx: &mut Context<Self>) {
        if self.session.text(&field) != text {
            self.session.handle(Intent::SetText(field, text));
            cx.notify();
        }
    }

    fn on_enter(&mut self, field: &Field, cx: &mut Context<Self>) {
        match field {
            Field::TypedKey => self.session.handle(Intent::ConfirmSheet),
            Field::Checklist(_) => self.session.handle(Intent::Submit(field.clone())),
            Field::Palette => {
                if let Some(SheetVm::Palette { items, .. }) = self.session.view().sheet
                    && let Some(first) = items.first()
                {
                    self.session.handle(first.intent.clone());
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Create what is needed, mirror session text into the inputs, focus a newly opened one.
    fn sync_inputs(&mut self, vm: &AppVm, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = needs(vm);
        for n in &wanted {
            self.ensure_input(n, window, cx);
        }
        for n in &wanted {
            let want = self.session.text(&n.field);
            if let Some(b) = self.inputs.map.get(&n.field)
                && b.value(cx) != want
            {
                b.set(&want, window, cx);
            }
        }
        let token = focus_token(vm);
        if token != self.focus_token {
            if token.is_some() {
                let field = match &vm.sheet {
                    Some(SheetVm::Compose { .. }) => Some(Field::Compose),
                    Some(SheetVm::Palette { .. }) => Some(Field::Palette),
                    Some(SheetVm::Confirm { .. }) => Some(Field::TypedKey),
                    _ => Some(Field::Inline),
                };
                if let Some(b) = field.and_then(|f| self.inputs.map.get(&f)) {
                    b.focus(window, cx);
                }
            } else {
                window.focus(&self.focus, cx);
            }
            self.focus_token = token;
        }
    }

    fn screen(&self, ui: &Ui, cx: &App, vm: &ScreenVm) -> AnyElement {
        match vm {
            ScreenVm::Next(n) => next::next(ui, &self.inputs, n).into_any_element(),
            ScreenVm::Tickets(t) => tickets::tickets(ui, t).into_any_element(),
            ScreenVm::Ticket {
                head,
                tab,
                body,
                comment,
                checklist_add,
            } => ticket::ticket(
                ui,
                cx,
                &self.inputs,
                head,
                *tab,
                body,
                comment,
                checklist_add,
            )
            .into_any_element(),
            ScreenVm::OnUat(v) => env::on_uat(ui, v).into_any_element(),
            ScreenVm::Workspace(v) => env::workspace(ui, v).into_any_element(),
            ScreenVm::Audit(rows) => env::audit(ui, rows).into_any_element(),
            ScreenVm::Settings(v) => env::settings(ui, v).into_any_element(),
            ScreenVm::Missing(msg) => div().p_8().child(msg.clone()).into_any_element(),
        }
    }
}

impl AppView {
    fn ui(&self, cx: &App, weak: WeakEntity<AppView>) -> Ui {
        Ui {
            view: weak,
            pal: Pal::of(cx),
            mono: cx.theme().mono_font_family.clone(),
        }
    }

    /// The left dock: navigation.
    pub(crate) fn draw_nav(&self, cx: &App) -> AnyElement {
        let ui = self.ui(cx, self.weak.clone());
        shell::nav(&ui, &self.vm).into_any_element()
    }

    /// The center: the tab strip and the current screen.
    pub(crate) fn draw_main(&self, cx: &App) -> AnyElement {
        let ui = self.ui(cx, self.weak.clone());
        let screen = self.screen(&ui, cx, &self.vm.screen);
        // Lists own their scrolling (a uniform list needs a bounded height); every other screen scrolls as a page.
        let is_list = matches!(self.vm.screen, ScreenVm::Next(_) | ScreenVm::Tickets(_));
        let body = if is_list {
            div()
                .flex_1()
                .min_h_0()
                .px_4()
                .pt_4()
                .child(screen)
                .into_any_element()
        } else {
            div()
                .id("content")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_6()
                .py_5()
                .child(div().max_w(px(1180.0)).child(screen))
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ui.pal.bg)
            .child(shell::tabstrip(&ui, &self.vm))
            .child(body)
            .into_any_element()
    }

    /// The right dock: details of whatever is selected.
    pub(crate) fn draw_details(&self, cx: &App) -> AnyElement {
        let ui = self.ui(cx, self.weak.clone());
        div()
            .size_full()
            .child(shell::right_panel(
                &ui,
                &self.vm.right_title,
                &self.vm.right,
            ))
            .into_any_element()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = Rc::new(self.session.view());
        self.vm = vm.clone();
        self.sync_inputs(&vm, window, cx);
        // The right dock follows the session's flag when it changes (the title bar toggle).
        if vm.right_open != self.right_open {
            self.right_open = vm.right_open;
            self.area.update(cx, |area, cx| {
                area.toggle_dock(DockPlacement::Right, window, cx);
            });
        }
        if vm.theme != self.theme {
            self.theme = vm.theme;
            super::theme::apply_theme(vm.theme, window, cx);
        }
        let ui = self.ui(cx, cx.entity().downgrade());
        let pal = ui.pal;
        div()
            .key_context("Showcase")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &OpenPaletteAction, _, cx| {
                this.session.handle(Intent::OpenPalette);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CancelAction, _, cx| {
                this.session.handle(Intent::CancelSheet);
                cx.notify();
            }))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(pal.bg)
            .text_color(pal.fg)
            .text_sm()
            .child(TitleBar::new().child(shell::title_bar_content(&ui, &vm, vm.simulate.is_some())))
            .child(div().flex_1().min_h_0().child(self.area.clone()))
            .child(shell::status_bar(&ui, &vm.status))
            .when_some(vm.attention.as_ref(), |d, a| {
                d.child(shell::attention_panel(&ui, a))
            })
            .when_some(vm.simulate.as_ref(), |d, g| {
                d.child(shell::simulate_panel(&ui, g))
            })
            .when_some(vm.sheet.as_ref(), |d, s| {
                d.child(shell::sheet(&ui, cx, &self.inputs, s))
            })
            .child(shell::toasts(&ui, &vm.toasts))
    }
}

/// Open the showcase window and run until it closes.
pub fn run() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            Theme::sync_system_appearance(None, cx);
            // The app menu bar: "Go to…" lives here (with its ⌘K shortcut), not in the window.
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus(vec![
                Menu {
                    name: "de".into(),
                    items: vec![MenuItem::action("Quit de", Quit)],
                    disabled: false,
                },
                Menu {
                    name: "Go".into(),
                    items: vec![MenuItem::action("Go to…", OpenPaletteAction)],
                    disabled: false,
                },
            ]);
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-k", OpenPaletteAction, None),
                KeyBinding::new("ctrl-k", OpenPaletteAction, None),
                KeyBinding::new("escape", CancelAction, Some("Showcase")),
            ]);
            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            // A custom title bar: the window controls stay, and the app's own controls sit beside them.
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| AppView::new(window, cx))
            })
            .expect("failed to open the showcase window");
            cx.activate(true);
        });
}
