//! **The Settings page itself: ONE screen, four groups** (owner's redesign mockup).
//!
//! It used to be a menu of four door-cards (Select server, Quick connect, Video & playback, Sign
//! out) with the playback knobs one level in, on a page of their own. The mockup draws the whole
//! thing on one page — Servidor, Reproducción, Audio y subtítulos, Interfaz — as a two-column grid
//! of compact cards, so the knobs a person changes live beside the doors they walk through, and
//! reaching the quality picker no longer costs a push. The field/picker machinery of the retired
//! `screens::preferences` (the confirmed values, the optimistically-flipped switches, the Force
//! acknowledgement) is this page's own now: a picker is still the page's submenu over the row it
//! edits, sliding on the family's one push spring.
//!
//! **A row is ONE triple — the row it draws, the action its press runs and the value it shows —
//! built in the order the table draws them** (the #398 fix, kept): a row's element is its
//! FLATTENED index across the sections, so the vector `activate` reads has to be in paint order.
//! Two parallel vectors is exactly how every press from the first switch down ran somebody else's
//! field.
//!
//! The groups are the mockup's, and the ROW ORDER inside them is the order a viewer reads. The
//! sections are what the two-column grid pairs inside, so a group never borrows a neighbour's
//! cell: `TableView::grid` restarts its columns at every `Section`.

use std::borrow::Cow;
use std::sync::mpsc::{self, Receiver};

use crate::route::{DirectPlayMode, Quality};
use crate::ui::decision_prompt::{DecisionPrompt, PromptStep};
use crate::ui::frame::Budget;
use crate::ui::machine::{
    Canon, Cx, Delivery, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputEvent,
    InputKind, InstanceId, Key, LogicalState, Machine, MachineId, NavOp,
};
use crate::ui::route_screen::{RouteLayout, RoutePush};
use crate::ui::screen::{
    At, Dir, DrawFrame, Enter, FocusSource, FocusTarget, Focusable, GroupSpec, HitSource, Placed,
    RenderStrategy, Screen, ScreenEvent, Step,
};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::table_screen::{Header, TableScreen};
use crate::ui::{theme, Painter, Rect};

use super::super::family::{table_focus, InnerHost, SettingsPage, ALERT_GROUP};
use super::super::registry::{word, AppFx, PreferenceCmd, ALERT};
use super::preference_name;

/// The page's recorded shape. It was `PreferencesV2` when this was *Video & playback*; the page
/// now carries the server and interface rows as well, and which page the surface's stack has at
/// its root is a different fact about the tree than it was — so the shape moves and the replay
/// fixtures are re-recorded rather than silently re-graded against a page that changed meaning.
pub(crate) const SHAPE: &str = "SettingsHomeV1{picker:u8,selection:u32,busy:bool,status:str,quality:u8,direct_play:u8,confirm:bool,affirm:bool,alert_scroll:u32,values:[str]}";

/// One knob with its own picker. The variant index rides the page's logical state (`picker as
/// u8`), so a field is APPENDED and never reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Quality,
    DirectPlay,
    LipSync,
    UpNext,
    UpNextWindow,
    SubtitleSize,
    SubtitlePosition,
    SubtitleBackground,
    /// #370: ask or auto-skip for server-marked intro/credits segments.
    SkipMarkers,
}

impl Field {
    fn title(self) -> &'static str {
        match self {
            Self::Quality => crate::i18n::msg::settings_playback_quality(),
            Self::DirectPlay => crate::i18n::msg::settings_playback_direct_play(),
            Self::LipSync => crate::i18n::msg::settings_playback_lip_sync(),
            Self::UpNext => crate::i18n::msg::settings_playback_up_next(),
            Self::UpNextWindow => crate::i18n::msg::settings_playback_up_next_window(),
            Self::SubtitleSize => crate::i18n::msg::settings_playback_subtitle_size(),
            Self::SubtitlePosition => crate::i18n::msg::settings_playback_subtitle_position(),
            Self::SubtitleBackground => crate::i18n::msg::settings_playback_subtitle_background(),
            Self::SkipMarkers => crate::i18n::msg::settings_playback_skip_markers(),
        }
    }

    /// The mockup's second line: one short sentence saying what the knob does.
    fn detail(self) -> &'static str {
        match self {
            Self::Quality => crate::i18n::msg::settings_playback_quality_detail(),
            Self::DirectPlay => crate::i18n::msg::settings_playback_direct_play_detail(),
            Self::LipSync => crate::i18n::msg::settings_playback_lip_sync_detail(),
            Self::UpNext => crate::i18n::msg::settings_playback_up_next_detail(),
            Self::UpNextWindow => crate::i18n::msg::settings_playback_up_next_window_detail(),
            Self::SubtitleSize => crate::i18n::msg::settings_playback_subtitle_size_detail(),
            Self::SubtitlePosition => crate::i18n::msg::settings_playback_subtitle_position_detail(),
            Self::SubtitleBackground => {
                crate::i18n::msg::settings_playback_subtitle_background_detail()
            }
            Self::SkipMarkers => crate::i18n::msg::settings_playback_skip_markers_detail(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Quality(Quality),
    DirectPlay(DirectPlayMode),
    LipSync(i64),
    UpNext(crate::server::session::UpNextPolicy),
    Window(i64),
    SubtitleSize(u8),
    SubtitlePosition(u8),
    SubtitleBackground(u8),
    Markers(crate::server::session::MarkerPolicy),
}

#[derive(Clone)]
enum Action {
    /// Open one of the page's own pickers.
    Open(Field),
    /// Choose one option in the open picker.
    Pick(Value),
    /// Flip the Library's Recently Added shelf in place (#297).
    ToggleRecentlyAdded,
    /// Flip the idle screen in place.
    ToggleIdleScreen,
    /// Walk to the server picker page.
    Servers,
    /// Start the quick-connect add-server flow.
    AddServer,
    /// Close this server's session.
    SignOut,
    /// Walk to the interface language page.
    Language,
    /// Walk to the Legal index.
    Legal,
    /// Walk to About Butaca.
    About,
}

struct State {
    picker: Option<Field>,
    /// The selected row of the level focus is on: the field list, or the open picker.
    selected: i32,
    busy: bool,
    status: String,
    quality: Quality,
    direct_play: DirectPlayMode,
    values: Vec<String>,
    confirming: bool,
    affirmative: bool,
    alert_scroll: u32,
}

impl LogicalState for State {
    fn write(&self, c: &mut Canon) {
        c.u8(self.picker.map_or(0, |p| p as u8 + 1))
            .u32(self.selected.max(0) as u32)
            .bool(self.busy)
            .str(&self.status)
            .u8(self.quality as u8)
            .u8(self.direct_play as u8)
            .bool(self.confirming)
            .bool(self.affirmative)
            .u32(self.alert_scroll)
            .u32(self.values.len() as u32);
        for v in &self.values {
            c.str(v);
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str(&format!(
            "settings picker={:?} sel={} busy={} confirm={}",
            self.picker, self.selected, self.busy, self.confirming
        ));
    }
}

enum Pending {
    Local(Receiver<bool>),
}

/// The Settings root: every group of the mockup on one page, plus the pickers its rows open.
pub(crate) struct RootPage {
    entry: EntryId,
    /// The page's own list of rows — the parent level of every picker. `pub(super)` for the
    /// family's test modules, which assert the drawn row set and measure its text fit.
    pub(super) table: TableView,
    /// The open (or leaving) picker's options.
    picker_table: TableView,
    /// The submenu push: open while a picker is, run back on close.
    submenu: RoutePush,
    /// The picker sliding back out, still drawn in the child role until `submenu` rests closed.
    leaving: Option<Field>,
    actions: Vec<Action>,
    state: State,
    parent_row: i32,
    copy: String,
    pending: Option<Pending>,
    /// The "Show recently added" switch's optimistic value until its write lands (#297). Kept
    /// outside [`State`] on purpose: this page's logical state is a recorded shape, and the
    /// setting is an install-wide preference with a process copy of its own.
    pending_recent: Option<(bool, crate::storage_worker::TypedTicket<bool>)>,
    /// The "Screensaver" switch's optimistic value until its write lands, `pending_recent`'s twin
    /// and outside [`State`] for the same reason.
    pending_idle: Option<(bool, crate::storage_worker::TypedTicket<bool>)>,
    alert: DecisionPrompt,
}

impl RootPage {
    pub(crate) fn new(entry: EntryId) -> Self {
        let mut s = Self {
            entry,
            table: TableView::new(),
            picker_table: TableView::new(),
            submenu: RoutePush::new(),
            leaving: None,
            actions: Vec::new(),
            state: State {
                picker: None,
                selected: 0,
                busy: false,
                status: String::new(),
                quality: crate::route::quality(),
                direct_play: crate::route::direct_play_mode(),
                values: Vec::new(),
                confirming: false,
                affirmative: false,
                alert_scroll: 0,
            },
            parent_row: 0,
            copy: String::new(),
            pending: None,
            pending_recent: None,
            pending_idle: None,
            alert: DecisionPrompt::new(
                ALERT_GROUP,
                ALERT,
                ALERT + 1,
                crate::i18n::msg::settings_cancel_c(),
                crate::i18n::msg::settings_playback_enable_force_c(),
            ),
        };
        s.rebuild(0);
        s
    }

    /// The page's one layout: a centred column, title block and cards in the same measure. Focus,
    /// paint and the tick's frame height all read it, so they cannot drift apart.
    pub(super) fn layout() -> RouteLayout {
        RouteLayout::settings()
    }

    /// The table focus and activation act on: the open picker's, else the page's own.
    fn active(&self) -> &TableView {
        if self.state.picker.is_some() {
            &self.picker_table
        } else {
            &self.table
        }
    }

    fn active_mut(&mut self) -> &mut TableView {
        if self.state.picker.is_some() {
            &mut self.picker_table
        } else {
            &mut self.table
        }
    }

    /// The line under the title when it is saying something about a save: the in-flight or failed
    /// status, with the Force note appended while Force is on. Empty when there is nothing to say.
    fn note_text(&self) -> Cow<'_, str> {
        if !self.state.status.is_empty() {
            return if self.state.direct_play == DirectPlayMode::Forced {
                Cow::Owned(format!(
                    "{}\n\n{}",
                    self.state.status,
                    crate::i18n::msg::settings_playback_force_note()
                ))
            } else {
                Cow::Borrowed(&self.state.status)
            };
        }
        if self.state.direct_play == DirectPlayMode::Forced {
            Cow::Borrowed(crate::i18n::msg::settings_playback_force_note())
        } else {
            Cow::Borrowed("")
        }
    }

    /// The header's second line for one level: the note when there is one, else the page's own
    /// blurb on the page and nothing on a picker — the options ARE that level, and the page's
    /// sentence under the name of one knob reads as a caption of the wrong thing.
    fn header_copy(&self, level: Option<Field>) -> &str {
        if !self.copy.is_empty() {
            return &self.copy;
        }
        match level {
            Some(_) => "",
            None => crate::i18n::msg::settings_root_jellyfin_copy(),
        }
    }

    fn view(&self) -> TableScreen<'_> {
        self.view_at(self.state.picker)
    }

    /// One level of the page: the row list (`None`) or a picker, each with its own title. The
    /// crumb is a drawn Back disc on both, so which word it would carry never shows.
    fn view_at(&self, level: Option<Field>) -> TableScreen<'_> {
        let (title, table) = match level {
            Some(field) => (field.title(), &self.picker_table),
            None => (crate::i18n::msg::settings_title(), &self.table),
        };
        let layout = Self::layout();
        TableScreen::new(
            Header::new(layout, None, title, self.header_copy(level)).with_back_disc(),
            table,
            GroupId(0),
            self.entry,
        )
        .with_frame(layout.content)
    }

    /// Draw one level through `p` and hand back the focus stops it registered.
    fn draw_level(
        &self,
        f: &DrawFrame<'_, '_, InnerHost>,
        level: Option<Field>,
        p: Painter,
    ) -> Vec<crate::ui::screen::Stop<u32>> {
        let mut inner = DrawFrame::with_navigation(f.cx, p, f.navigation());
        let mut v = self.view_at(level);
        crate::ui::screen::Part::<InnerHost>::draw(&mut v, &mut inner, Rect::FULL);
        inner.into_stops()
    }

    fn focus(&self, fx: &mut Effects<'_, InnerHost>, group: GroupId) {
        fx.push(Fx::Deliver(
            MachineId::Instance(InstanceId(0)),
            Delivery::Screen(ScreenEvent::Enter(Enter::Fresh {
                focus: if group == GroupId(0) {
                    FocusTarget::Elem(FocusKey {
                        entry: self.entry,
                        elem: self.active().sel.max(0) as u32,
                    })
                } else {
                    FocusTarget::ContainerGroup(group)
                },
            })),
        ));
    }

    fn save_local(&mut self, value: Value, fx: &mut Effects<'_, InnerHost>) {
        let (reply, rx) = mpsc::channel();
        let command = match value {
            Value::Quality(quality) => PreferenceCmd::Quality { quality, reply },
            Value::DirectPlay(mode) => PreferenceCmd::DirectPlay { mode, reply },
            Value::LipSync(offset_ms) => PreferenceCmd::LipSync { offset_ms, reply },
            Value::UpNext(policy) => PreferenceCmd::UpNext { policy, reply },
            Value::Window(seconds) => PreferenceCmd::UpNextWindow { seconds, reply },
            Value::SubtitleSize(step) => PreferenceCmd::SubtitleScale { step, reply },
            Value::SubtitlePosition(step) => PreferenceCmd::SubtitlePosition { step, reply },
            Value::SubtitleBackground(step) => PreferenceCmd::SubtitleBackground { step, reply },
            Value::Markers(policy) => PreferenceCmd::MarkerPolicy { policy, reply },
        };
        self.pending = Some(Pending::Local(rx));
        self.state.busy = true;
        self.state.status = crate::i18n::msg::settings_playback_saving().into();
        fx.push(Fx::App(AppFx::Preferences(command)));
    }

    fn current_value(&self, field: Field) -> Value {
        match field {
            Field::Quality => Value::Quality(self.state.quality),
            Field::DirectPlay => Value::DirectPlay(self.state.direct_play),
            Field::LipSync => Value::LipSync(crate::route::lip_sync_offset_ms()),
            Field::UpNext => Value::UpNext(crate::route::up_next_policy()),
            Field::UpNextWindow => Value::Window(crate::route::up_next_window_s()),
            Field::SubtitleSize => Value::SubtitleSize(crate::route::subtitle_scale_step()),
            Field::SubtitlePosition => Value::SubtitlePosition(crate::route::subtitle_position_step()),
            Field::SubtitleBackground => {
                Value::SubtitleBackground(crate::route::subtitle_background_step())
            }
            Field::SkipMarkers => Value::Markers(crate::route::marker_policy()),
        }
    }

    fn options(&self, field: Field) -> Vec<(String, Value)> {
        match field {
            Field::Quality => crate::route::available_quality_ladder()
                .iter()
                .map(|q| (q.label().into(), Value::Quality(*q)))
                .collect(),
            Field::DirectPlay => [
                DirectPlayMode::Auto,
                DirectPlayMode::Forced,
                DirectPlayMode::Disabled,
            ]
            .into_iter()
            .map(|m| (mode_label(m).into(), Value::DirectPlay(m)))
            .collect(),
            Field::LipSync => {
                let (min, max) = crate::route::lip_sync_offset_range_ms();
                let mut result = Vec::new();
                let mut ms = min;
                while ms <= max {
                    result.push((lip_sync_label(ms), Value::LipSync(ms)));
                    ms += crate::route::LIP_SYNC_STEP_MS;
                }
                result
            }
            Field::UpNext => [
                crate::server::session::UpNextPolicy::Auto,
                crate::server::session::UpNextPolicy::Ask,
                crate::server::session::UpNextPolicy::Off,
            ]
            .into_iter()
            .map(|p| (up_next_label(p).into(), Value::UpNext(p)))
            .collect(),
            Field::UpNextWindow => {
                let (min, max) = crate::route::up_next_window_range_s();
                let mut result = Vec::new();
                let mut s = min;
                while s <= max {
                    result.push((format!("{s} s"), Value::Window(s)));
                    s += crate::server::session::UP_NEXT_WINDOW_STEP_S;
                }
                result
            }
            Field::SubtitleSize => [
                crate::i18n::msg::settings_playback_subtitle_size_smaller(),
                crate::i18n::msg::settings_playback_subtitle_size_small(),
                crate::i18n::msg::settings_playback_subtitle_size_normal(),
                crate::i18n::msg::settings_playback_subtitle_size_large(),
                crate::i18n::msg::settings_playback_subtitle_size_larger(),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, label)| (label.into(), Value::SubtitleSize(i as u8)))
            .collect(),
            Field::SubtitlePosition => [
                crate::i18n::msg::settings_playback_subtitle_position_bottom(),
                crate::i18n::msg::settings_playback_subtitle_position_low(),
                crate::i18n::msg::settings_playback_subtitle_position_middle(),
                crate::i18n::msg::settings_playback_subtitle_position_high(),
                crate::i18n::msg::settings_playback_subtitle_position_top(),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, label)| (label.into(), Value::SubtitlePosition(i as u8)))
            .collect(),
            Field::SubtitleBackground => [
                crate::i18n::msg::settings_playback_subtitle_background_off(),
                crate::i18n::msg::settings_playback_subtitle_background_half(),
                crate::i18n::msg::settings_playback_subtitle_background_solid(),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, label)| (label.into(), Value::SubtitleBackground(i as u8)))
            .collect(),
            Field::SkipMarkers => [
                crate::server::session::MarkerPolicy::Ask,
                crate::server::session::MarkerPolicy::Auto,
            ]
            .into_iter()
            .map(|p| (marker_label(p).into(), Value::Markers(p)))
            .collect(),
        }
    }

    fn value_label(&self, field: Field) -> String {
        let current = self.current_value(field);
        self.options(field)
            .into_iter()
            .find(|(_, v)| *v == current)
            .map_or_else(
                || crate::i18n::msg::settings_audio_not_set().into(),
                |(s, _)| s,
            )
    }

    /// One field's row, the action its press runs and the value it shows, as ONE triple: the three
    /// things that must never come apart (see the module doc).
    fn field_row(&self, field: Field) -> (Row, Action, String) {
        let value = self.value_label(field);
        let mut row = Row::new(field.title())
            .detail(field.detail())
            .value(&value)
            .chevron(true)
            .dim(self.state.busy);
        if field == Field::Quality && self.state.direct_play == DirectPlayMode::Forced {
            row = row.detail(crate::i18n::msg::settings_playback_overridden());
        }
        (row, Action::Open(field), value)
    }

    /// The active server as the mockup's first row draws it: the account in the value slot and the
    /// address underneath. Both are the install's own data, read only, and empty when no server is
    /// configured yet.
    fn server_line() -> (String, String) {
        let Some(list) = crate::jellyfin::boot::load_servers() else {
            return (String::new(), String::new());
        };
        match list.servers.get(list.active).or_else(|| list.servers.first()) {
            Some(entry) => (entry.user.clone(), entry.url.clone()),
            None => (String::new(), String::new()),
        }
    }

    fn rebuild(&mut self, selected: i32) {
        self.state.quality = crate::route::quality();
        self.state.direct_play = crate::route::direct_play_mode();
        self.copy = self.note_text().into_owned();
        self.state.confirming = self.alert.is_open();
        self.state.affirmative = self.alert.choice();
        self.state.alert_scroll = self.alert.scroll_target_bits();
        self.state.values.clear();
        self.actions.clear();

        // The rows, group by group, in the order they are painted: `Section` by `Section`, and
        // row-major inside each one, which is exactly the flattened index `activate` reads.
        let mut server = Section::new(crate::i18n::msg::settings_servers_section());
        let mut playback = Section::new(crate::i18n::msg::settings_playback_section());
        let mut audio = Section::new(crate::i18n::msg::settings_audio_title());
        let mut interface = Section::new(crate::i18n::msg::settings_interface_section());
        let mut triples: Vec<(Row, Action, String)> = Vec::new();

        // Servidor: the account this television uses, the door to another one, and the way out.
        let (user, url) = Self::server_line();
        let value = if user.is_empty() {
            crate::i18n::msg::settings_audio_not_set().to_string()
        } else {
            user
        };
        let detail = if url.is_empty() {
            crate::i18n::msg::settings_servers_select_detail().to_string()
        } else {
            url
        };
        triples.push((
            Row::new(crate::i18n::msg::settings_servers_current())
                .detail(detail)
                .value(&value)
                .chevron(true)
                .dim(self.state.busy),
            Action::Servers,
            value,
        ));
        triples.push((
            Row::new(crate::i18n::msg::settings_servers_add())
                .detail(crate::i18n::msg::settings_servers_add_detail())
                .chevron(true)
                .dim(self.state.busy),
            Action::AddServer,
            String::new(),
        ));
        triples.push((
            Row::new(crate::i18n::msg::settings_servers_disconnect())
                .detail(crate::i18n::msg::settings_servers_disconnect_detail())
                .destructive(true)
                .dim(self.state.busy),
            Action::SignOut,
            String::new(),
        ));

        // Reproducción: the transport knobs, then the two switches that are about the house rather
        // than about one playback. The window configures a card that "Do not offer" never draws,
        // so the row goes with it.
        triples.push(self.field_row(Field::Quality));
        triples.push(self.field_row(Field::DirectPlay));
        triples.push(self.field_row(Field::LipSync));
        triples.push(self.field_row(Field::UpNext));
        if crate::route::up_next_offered() {
            triples.push(self.field_row(Field::UpNextWindow));
        }
        triples.push(self.field_row(Field::SkipMarkers));
        let recent = self
            .pending_recent
            .as_ref()
            .map_or_else(crate::route::show_recently_added, |(on, _)| *on);
        triples.push((
            Row::new(crate::i18n::msg::settings_library_recently_added())
                .detail(crate::i18n::msg::settings_library_recently_added_detail())
                .toggle(recent)
                .dim(self.state.busy),
            Action::ToggleRecentlyAdded,
            if recent {
                crate::i18n::msg::widgets_toggle_on()
            } else {
                crate::i18n::msg::widgets_toggle_off()
            }
            .into(),
        ));
        let idle = self
            .pending_idle
            .as_ref()
            .map_or_else(crate::route::idle_screen, |(on, _)| *on);
        triples.push((
            Row::new(crate::i18n::msg::settings_playback_idle_screen())
                .detail(crate::i18n::msg::settings_playback_idle_screen_detail())
                .toggle(idle)
                .dim(self.state.busy),
            Action::ToggleIdleScreen,
            if idle {
                crate::i18n::msg::widgets_toggle_on()
            } else {
                crate::i18n::msg::widgets_toggle_off()
            }
            .into(),
        ));

        // Audio y subtítulos: the look of a track the app composes.
        triples.push(self.field_row(Field::SubtitleSize));
        triples.push(self.field_row(Field::SubtitlePosition));
        triples.push(self.field_row(Field::SubtitleBackground));

        // Interfaz: the three doors out of this page. They are the mockup's, and each one pushes a
        // page of the surface's own stack (About included: see `screens::legal`).
        let language = preference_name(crate::i18n::saved_preference()).to_string();
        triples.push((
            Row::new(crate::i18n::msg::settings_language_title())
                .detail(crate::i18n::msg::settings_language_detail())
                .value(&language)
                .chevron(true)
                .dim(self.state.busy),
            Action::Language,
            language,
        ));
        triples.push((
            Row::new(crate::i18n::msg::settings_legal_title())
                .detail(crate::i18n::msg::settings_legal_detail())
                .chevron(true)
                .dim(self.state.busy),
            Action::Legal,
            String::new(),
        ));
        triples.push((
            Row::new(crate::i18n::msg::settings_about_title())
                .detail(crate::i18n::msg::settings_about_detail())
                .chevron(true)
                .dim(self.state.busy),
            Action::About,
            String::new(),
        ));

        let mut list_values = Vec::new();
        let mut list_actions = Vec::new();
        for (row, action, value) in triples {
            match group_of(&action) {
                Group::Servers => server = server.row(row),
                Group::Playback => playback = playback.row(row),
                Group::Audio => audio = audio.row(row),
                Group::Interface => interface = interface.row(row),
            }
            list_values.push(value);
            list_actions.push(action);
        }
        let list_sel = if self.state.picker.is_some() {
            self.parent_row
        } else {
            selected
        };
        for table in [&mut self.table, &mut self.picker_table] {
            table.compact = false;
            table.header_ink = theme::TEXT_READING;
            table.list_focused = true;
        }
        // **The mockup's two-column grid of cards** (issue #415): every group pairs its rows, and
        // every row is its own plate. The PICKER stays a one-column list, because a popover of
        // options is one column on one ground.
        self.table.grid(2);
        self.table.cards(true);
        self.table.set_sections(vec![server, playback, audio, interface], list_sel, false);
        if let Some(field) = self.state.picker {
            let current = self.current_value(field);
            let mut section = Section::new("");
            for (label, value) in self.options(field) {
                section = section.row(Row::new(&label).checked(value == current).dim(self.state.busy));
                self.state.values.push(label);
                self.actions.push(Action::Pick(value));
            }
            self.picker_table.set_sections(vec![section], selected, false);
        } else {
            self.state.values = list_values;
            self.actions = list_actions;
        }
        self.state.selected = self.active().sel;
    }

    fn close_picker(&mut self, fx: &mut Effects<'_, InnerHost>) {
        self.leaving = self.state.picker.take().or(self.leaving);
        self.rebuild(self.parent_row);
        self.focus(fx, GroupId(0));
    }

    pub(super) fn activate(&mut self, row: usize, fx: &mut Effects<'_, InnerHost>) {
        if self.state.busy {
            return;
        }
        let Some(action) = self.actions.get(row).cloned() else {
            return;
        };
        match action {
            Action::Open(field) => {
                self.parent_row = row as i32;
                self.state.picker = Some(field);
                self.leaving = None;
                let current = self.current_value(field);
                let selected = self
                    .options(field)
                    .iter()
                    .position(|(_, v)| *v == current)
                    .unwrap_or(0);
                self.rebuild(selected as i32);
                self.focus(fx, GroupId(0));
            }
            Action::Pick(value) => {
                if value == Value::DirectPlay(DirectPlayMode::Forced)
                    && self.state.direct_play != DirectPlayMode::Forced
                {
                    self.alert.open(
                        crate::i18n::msg::settings_playback_force_question_c(),
                        crate::i18n::msg::settings_playback_force_body(),
                    );
                    self.state.confirming = true;
                    self.state.affirmative = false;
                    self.state.alert_scroll = self.alert.scroll_target_bits();
                    self.focus(fx, ALERT_GROUP);
                    return;
                }
                self.commit(value, fx);
                self.close_picker(fx);
            }
            Action::ToggleRecentlyAdded => {
                // Flip the process copy this page shows (the same value the Library reads) and
                // queue the durable write; the switch's face follows `pending_recent` until it
                // lands, and the whole page is `busy` so a second press cannot race it.
                let current = self
                    .pending_recent
                    .as_ref()
                    .map_or_else(crate::route::show_recently_added, |(on, _)| *on);
                let on = !current;
                if let Ok(ticket) = crate::server::session::queue_update_ticket(move |current| {
                    (current.show_recently_added() != on).then(|| current.with_show_recently_added(on))
                }) {
                    self.pending_recent = Some((on, ticket));
                    self.state.busy = true;
                }
                self.rebuild(self.active().sel);
            }
            Action::ToggleIdleScreen => {
                let current = self
                    .pending_idle
                    .as_ref()
                    .map_or_else(crate::route::idle_screen, |(on, _)| *on);
                let on = !current;
                if let Ok(ticket) = crate::server::session::queue_update_ticket(move |current| {
                    (current.idle_screen() != on).then(|| current.with_idle_screen(on))
                }) {
                    self.pending_idle = Some((on, ticket));
                    self.state.busy = true;
                }
                self.rebuild(self.active().sel);
            }
            Action::Servers => fx.push(Fx::Nav(NavOp::Push(SettingsPage::Servers))),
            Action::AddServer => fx.push(Fx::App(AppFx::Loop(super::super::registry::LoopReq::JfQuickConnect))),
            Action::SignOut => fx.push(Fx::App(AppFx::Loop(super::super::registry::LoopReq::JfDisconnectServer))),
            Action::Language => fx.push(Fx::Nav(NavOp::Push(SettingsPage::Language))),
            Action::Legal => fx.push(Fx::Nav(NavOp::Push(SettingsPage::Legal))),
            Action::About => fx.push(Fx::Nav(NavOp::Push(SettingsPage::About))),
        }
        fx.invalidate(crate::ui::present::Provenance::Input);
    }

    fn commit(&mut self, value: Value, fx: &mut Effects<'_, InnerHost>) {
        match value {
            Value::Quality(_)
            | Value::DirectPlay(_)
            | Value::LipSync(_)
            | Value::UpNext(_)
            | Value::Window(_)
            | Value::SubtitleSize(_)
            | Value::SubtitlePosition(_)
            | Value::SubtitleBackground(_)
            | Value::Markers(_) => self.save_local(value, fx),
        }
    }

    fn poll(&mut self) -> bool {
        enum Receipt {
            Local(bool),
            Failed,
        }
        let receipt = match &self.pending {
            Some(Pending::Local(ticket)) => match ticket.try_recv() {
                Ok(result) => Some(Receipt::Local(result)),
                Err(mpsc::TryRecvError::Disconnected) => Some(Receipt::Failed),
                _ => None,
            },
            None => None,
        };
        let Some(receipt) = receipt else {
            return false;
        };
        self.pending = None;
        self.state.busy = false;
        match receipt {
            Receipt::Local(true) => self.state.status.clear(),
            Receipt::Local(false) | Receipt::Failed => {
                self.state.status = crate::i18n::msg::settings_playback_save_failed().into()
            }
        }
        true
    }
}

impl Field {
    /// Whether this field's row belongs to the *Audio & subtitles* group. One half of the ONE
    /// predicate the section split and the row/action pairing share, so a field cannot land in a
    /// group that its own action does not name.
    fn is_subtitle(self) -> bool {
        matches!(
            self,
            Self::SubtitleSize | Self::SubtitlePosition | Self::SubtitleBackground
        )
    }
}

/// The mockup's four groups.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Group {
    Servers,
    Playback,
    Audio,
    Interface,
}

/// Which group a row belongs to. **The one predicate the paint order and the section split
/// share**: a row cannot be built into the list and then land in a group its action does not
/// name, which is the drift #322 introduced when the sections were split by hand.
fn group_of(action: &Action) -> Group {
    match action {
        Action::Servers | Action::AddServer | Action::SignOut => Group::Servers,
        Action::Language | Action::Legal | Action::About => Group::Interface,
        Action::Open(field) if field.is_subtitle() => Group::Audio,
        _ => Group::Playback,
    }
}

fn mode_label(mode: DirectPlayMode) -> &'static str {
    match mode {
        DirectPlayMode::Auto => crate::i18n::msg::settings_playback_auto(),
        DirectPlayMode::Forced => crate::i18n::msg::settings_playback_forced(),
        DirectPlayMode::Disabled => crate::i18n::msg::settings_playback_disabled(),
    }
}

/// The next-episode picker's row label. `Auto` is spelled out rather than left as "Automatic":
/// the card counts down and starts the episode on its own, and that is worth saying plainly.
fn up_next_label(policy: crate::server::session::UpNextPolicy) -> &'static str {
    match policy {
        crate::server::session::UpNextPolicy::Auto => crate::i18n::msg::settings_playback_up_next_auto(),
        crate::server::session::UpNextPolicy::Ask => crate::i18n::msg::settings_playback_up_next_ask(),
        crate::server::session::UpNextPolicy::Off => crate::i18n::msg::settings_playback_up_next_off(),
    }
}

/// The skip picker's row label (#370). `Ask` is spelled as the promise it makes: the control
/// appears and waits.
fn marker_label(policy: crate::server::session::MarkerPolicy) -> &'static str {
    match policy {
        crate::server::session::MarkerPolicy::Ask => crate::i18n::msg::settings_playback_skip_markers_ask(),
        crate::server::session::MarkerPolicy::Auto => crate::i18n::msg::settings_playback_skip_markers_auto(),
    }
}

/// The lip-sync picker's row label: `0 ms` at the centre, a signed value off it.
fn lip_sync_label(ms: i64) -> String {
    if ms == 0 {
        "0 ms".to_string()
    } else {
        format!("{ms:+} ms")
    }
}

impl Machine<InnerHost> for RootPage {
    type Ev = ScreenEvent<InnerHost>;
    fn step(
        &mut self,
        ev: &Self::Ev,
        cx: &Cx<'_, InnerHost>,
        fx: &mut Effects<'_, InnerHost>,
    ) -> Handled {
        match self.alert.step(ev, cx) {
            PromptStep::Pass => (),
            PromptStep::Done(handled) => {
                self.state.affirmative = self.alert.choice();
                self.state.alert_scroll = self.alert.scroll_target_bits();
                return handled;
            }
            PromptStep::Answer(yes) => {
                if yes {
                    self.save_local(Value::DirectPlay(DirectPlayMode::Forced), fx);
                }
                self.state.confirming = false;
                if yes {
                    self.close_picker(fx);
                } else {
                    self.focus(fx, GroupId(0));
                }
                fx.invalidate(crate::ui::present::Provenance::Input);
                return Handled::Yes;
            }
        }
        match ev {
            // The page re-reads the install's own facts (the active server, a translated label)
            // whenever the surface lands on it; the tables' cursors are carried through.
            ScreenEvent::Enter(_) => {
                self.rebuild(self.active().sel);
                Handled::Yes
            }
            ScreenEvent::Tick(t) => {
                self.alert.update(t.dt());
                let had_rows = self.active().n_rows() > 0;
                // #297: the recently-added switch's write landed. Publish the file's own answer
                // into the process copy the Library reads before the page rebuilds its face.
                let mut landed = false;
                let recent_receipt = self.pending_recent.as_ref().and_then(|(_, ticket)| {
                    match ticket.try_recv() {
                        Ok(saved) => Some(saved),
                        Err(mpsc::TryRecvError::Disconnected) => Some(false),
                        Err(mpsc::TryRecvError::Empty) => None,
                    }
                });
                if let Some(saved) = recent_receipt {
                    if let Some(snapshot) = crate::server::session::peek_settled() {
                        crate::route::restore_show_recently_added(snapshot.show_recently_added());
                    }
                    self.pending_recent = None;
                    self.state.busy = false;
                    if !saved {
                        self.state.status = crate::i18n::msg::settings_playback_save_failed().into();
                    }
                    landed = true;
                }
                let idle_receipt = self.pending_idle.as_ref().and_then(|(_, ticket)| {
                    match ticket.try_recv() {
                        Ok(saved) => Some(saved),
                        Err(mpsc::TryRecvError::Disconnected) => Some(false),
                        Err(mpsc::TryRecvError::Empty) => None,
                    }
                });
                if let Some(saved) = idle_receipt {
                    if let Some(snapshot) = crate::server::session::peek_settled() {
                        crate::route::restore_idle_screen(snapshot.idle_screen());
                    }
                    self.pending_idle = None;
                    self.state.busy = false;
                    if !saved {
                        self.state.status = crate::i18n::msg::settings_playback_save_failed().into();
                    }
                    landed = true;
                }
                if self.poll()
                    || landed
                    || self.state.quality != crate::route::quality()
                    || self.state.direct_play != crate::route::direct_play_mode()
                {
                    self.rebuild(self.active().sel);
                    // An empty loading table had no engine seat. Give its first landing (or
                    // Retry row) one so OK works immediately, without moving an existing cursor
                    // when an ordinary save completes or another page owns focus.
                    if !had_rows && self.active().n_rows() > 0 && cx.focus.current.is_none() {
                        self.focus(fx, GroupId(0));
                    }
                    fx.invalidate(crate::ui::present::Provenance::Landing(MachineId::Session));
                }
                let h = Self::layout().content.h;
                self.table.update(t.dt(), h);
                self.picker_table.update(t.dt(), h);
                let open = self.state.picker.is_some();
                self.submenu.tick(open, *t, &mut fx.present());
                if !open && self.submenu.resting(false) {
                    self.leaving = None;
                }
                Handled::Yes
            }
            ScreenEvent::FocusMoved { to, .. } => {
                table_focus(self.active_mut(), to.elem);
                self.state.selected = self.active().sel;
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                self.activate(*elem as usize, fx);
                Handled::Yes
            }
            ScreenEvent::Input(InputEvent {
                kind:
                    InputKind::Key {
                        key: Key::Back,
                        edge: Edge::Down,
                        ..
                    },
                ..
            }) if self.state.picker.is_some() => {
                self.close_picker(fx);
                Handled::Yes
            }
            ScreenEvent::Input(InputEvent {
                kind:
                    InputKind::Key {
                        key: Key::Right,
                        at_edge: true,
                        ..
                    },
                ..
            }) if self.state.picker.is_none() => {
                if let Some(key) = cx.focus.current {
                    // Only a chevron row ENTERS something; a switch changes in place, so RIGHT is
                    // inert on it.
                    if self.active().row_opens(key.elem as i32) {
                        self.activate(key.elem as usize, fx);
                    }
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

impl Focusable<InnerHost> for RootPage {
    fn groups(&self, cx: &Cx<'_, InnerHost>, out: &mut Vec<GroupSpec>) {
        if !self.alert.groups(out, cx.measure) {
            Focusable::<InnerHost>::groups(&self.view(), cx, out)
        }
    }
    fn group_of(&self, key: &u32, cx: &Cx<'_, InnerHost>) -> Option<GroupId> {
        match self.alert.group_of(*key) {
            Some(answer) => answer,
            None => Focusable::<InnerHost>::group_of(&self.view(), key, cx),
        }
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, cx: &Cx<'_, InnerHost>) -> Step<u32> {
        match self.alert.neighbour(key, dir) {
            Some(step) => step,
            None => Focusable::<InnerHost>::neighbour(&self.view(), key, dir, cx),
        }
    }
    fn place(&self, key: &u32, cx: &Cx<'_, InnerHost>, at: At) -> Option<Placed> {
        match self.alert.place(*key, cx.measure) {
            Some(placed) => placed,
            None => Focusable::<InnerHost>::place(&self.view(), key, cx, at),
        }
    }
    fn reconcile(&self, want: FocusKey<u32>, cx: &Cx<'_, InnerHost>) -> FocusKey<u32> {
        if let Some(key) = self.alert.reconcile(want) {
            return key;
        }
        if self.alert.owns(want.elem) {
            return FocusKey {
                entry: self.entry,
                elem: self.active().sel.max(0) as u32,
            };
        }
        Focusable::<InnerHost>::reconcile(&self.view(), want, cx)
    }
    fn seat(&self, g: GroupId, from: Placed, cx: &Cx<'_, InnerHost>) -> FocusKey<u32> {
        match self.alert.seat(g, self.entry) {
            Some(key) => key,
            None => Focusable::<InnerHost>::seat(&self.view(), g, from, cx),
        }
    }
}

impl Screen<InnerHost> for RootPage {
    fn name(&self) -> &'static str {
        word::SETTINGS
    }
    fn state(&self) -> &dyn LogicalState {
        &self.state
    }
    fn crumb(&self, _cx: &Cx<'_, InnerHost>) -> Option<Cow<'_, str>> {
        None
    }
    fn back_control(&self, cx: &Cx<'_, InnerHost>) -> Option<Rect> {
        self.view().crumb_rect(cx.measure)
    }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, InnerHost>) {}
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, InnerHost>) {
        let open = self.state.picker.is_some();
        if self.submenu.resting(open) {
            let mut v = self.view();
            crate::ui::screen::Part::<InnerHost>::draw(&mut v, f, Rect::FULL);
        } else {
            // Mid-push: the row list in the parent role, the picker (or the one leaving) in the
            // child role; only the level focus acts on contributes stops.
            let (parent_p, child_p) = (self.submenu.parent(f.painter), self.submenu.child(f.painter));
            let parent = self.draw_level(f, None, parent_p);
            let child = self
                .state
                .picker
                .or(self.leaving)
                .map(|field| self.draw_level(f, Some(field), child_p))
                .unwrap_or_default();
            for s in if open { child } else { parent } {
                f.stop(Painter::root(), s);
            }
        }
        self.alert.draw(f, self.entry);
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    fn focus_source(&self) -> FocusSource {
        FocusSource::Engine
    }
    fn hit_source(&self) -> HitSource {
        HitSource::Engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fixture::FixtureMeasure;
    use crate::ui::machine::{FocusRead, InputOwner, PressRead, Tick};
    use crate::ui::present::Present;

    fn with_fx(f: impl FnOnce(&mut Effects<'_, InnerHost>)) {
        let mut out = Vec::new();
        let mut present = Present::new();
        f(&mut Effects::new(
            &mut out,
            MachineId::Instance(InstanceId(0)),
            &mut present,
        ));
    }

    fn context(focus: u32) -> Cx<'static, InnerHost> {
        static MEASURE: FixtureMeasure = FixtureMeasure;
        Cx {
            views: crate::stores::browse::DirectoryView::empty_for_test(),
            tick: Tick::default(),
            measure: &MEASURE,
            press: PressRead::default(),
            focus: FocusRead {
                current: Some(FocusKey {
                    entry: EntryId(0),
                    elem: focus,
                }),
                ..Default::default()
            },
            owner: InputOwner::Entry(EntryId(0)),
        }
    }

    fn row_of(page: &RootPage, label: &str) -> i32 {
        page.table
            .sections
            .iter()
            .flat_map(|s| s.rows.iter())
            .position(|r| r.label == label)
            .unwrap_or_else(|| panic!("no row labelled {label}")) as i32
    }

    #[test]
    fn force_requires_acknowledgement_and_cancel_never_saves() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("force-confirm-cancel");
        crate::route::restore_direct_play_mode(DirectPlayMode::Auto);
        let mut page = RootPage::new(EntryId(0));
        let direct = row_of(&page, crate::i18n::msg::settings_playback_direct_play()) as usize;
        with_fx(|fx| page.activate(direct, fx));
        assert_eq!(page.state.picker, Some(Field::DirectPlay));
        with_fx(|fx| page.activate(1, fx));
        assert!(page.alert.is_open());
        assert!(!page.alert.choice(), "Cancel is the default answer");
        assert!(page.pending.is_none(), "opening the warning must not persist Force");
        assert_eq!(crate::route::direct_play_mode(), DirectPlayMode::Auto);
        let cx = context(ALERT);
        let cancel = ScreenEvent::Input(InputEvent {
            at: Tick::default(),
            source: crate::ui::machine::Source::RemoteFifo,
            kind: InputKind::Key {
                key: Key::Back,
                edge: Edge::Down,
                sym: 0,
                wcode: 0,
                at_edge: false,
            },
        });
        with_fx(|fx| {
            page.step(&cancel, &cx, fx);
        });
        assert!(!page.alert.is_open());
        assert!(page.pending.is_none());
        assert_eq!(crate::route::direct_play_mode(), DirectPlayMode::Auto);
    }

    #[test]
    fn confirming_force_emits_a_preference_effect_without_executing_it() {
        let _serial = crate::testlock::serial();
        let previous = crate::route::direct_play_mode();
        crate::route::restore_direct_play_mode(DirectPlayMode::Auto);
        let mut page = RootPage::new(EntryId(0));
        let direct = row_of(&page, crate::i18n::msg::settings_playback_direct_play()) as usize;
        with_fx(|fx| page.activate(direct, fx));
        with_fx(|fx| page.activate(1, fx));
        let mut emitted = Vec::new();
        let mut present = Present::new();
        let mut fx = Effects::new(
            &mut emitted,
            MachineId::Instance(InstanceId(0)),
            &mut present,
        );
        page.step(
            &ScreenEvent::PressCommit(crate::ui::machine::PressId(1)),
            &context(ALERT + 1),
            &mut fx,
        );
        assert!(page.state.busy);
        assert!(emitted.iter().any(|event| matches!(
            &event.fx,
            Fx::App(AppFx::Preferences(PreferenceCmd::DirectPlay {
                mode: DirectPlayMode::Forced,
                ..
            }))
        )));
        assert_eq!(
            crate::route::direct_play_mode(),
            DirectPlayMode::Auto,
            "only an admitted app executor can persist and activate Force"
        );
        crate::route::restore_direct_play_mode(previous);
    }

    /// Owner report: Direct Play / Default Quality (and every Audio & Subtitles picker) switched
    /// to the next screen instantly. A picker is this page's own submenu, so it must run the
    /// family's push spring — in on open, back out on close with the picker still drawn.
    #[test]
    fn a_picker_slides_in_and_out_on_the_family_push_spring() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("picker-push-spring");
        let mut page = RootPage::new(EntryId(0));
        let quality = row_of(&page, crate::i18n::msg::settings_playback_quality());
        assert!(page.submenu.resting(false), "no picker: the list is at rest");
        with_fx(|fx| page.activate(quality as usize, fx));
        assert_eq!(page.state.picker, Some(Field::Quality));
        assert!(
            page.submenu.amount() < 0.01 && !page.submenu.resting(true),
            "opening a picker starts the push rather than cutting to it"
        );
        let t = Tick {
            ms: 16,
            dt_us: 16_000,
        };
        for _ in 0..120 {
            with_fx(|fx| {
                page.step(&ScreenEvent::Tick(t), &context(0), fx);
            });
        }
        assert!(page.submenu.resting(true), "the push settles open");
        with_fx(|fx| page.close_picker(fx));
        assert_eq!(page.state.picker, None);
        assert!(
            page.submenu.amount() > 0.99 && !page.submenu.resting(false),
            "closing runs the same spring back"
        );
        assert_eq!(
            page.leaving,
            Some(Field::Quality),
            "the picker keeps drawing while it leaves"
        );
        for _ in 0..120 {
            with_fx(|fx| {
                page.step(&ScreenEvent::Tick(t), &context(0), fx);
            });
        }
        assert!(page.submenu.resting(false));
        assert_eq!(page.leaving, None, "…and is released once the spring settles");
    }

    #[test]
    fn picker_back_restores_parent_row_without_saving() {
        let _serial = crate::testlock::serial();
        let mut page = RootPage::new(EntryId(0));
        let quality = row_of(&page, crate::i18n::msg::settings_playback_quality());
        with_fx(|fx| page.activate(quality as usize, fx));
        with_fx(|fx| page.close_picker(fx));
        assert_eq!(page.state.picker, None);
        assert_eq!(page.table.sel, quality);
        assert!(page.pending.is_none());
    }

    /// #297: the Library's opt-in Recently Added shelf is a switch on this page. Its face reads
    /// the process copy and its press arms a durable write, then the page publishes the file's own
    /// answer.
    #[test]
    fn the_recently_added_switch_flips_in_place_and_publishes_its_write() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("prefs-recently-added");
        crate::route::restore_show_recently_added(false);
        let mut page = RootPage::new(EntryId(0));
        let last = page
            .actions
            .iter()
            .position(|action| matches!(action, Action::ToggleRecentlyAdded))
            .expect("the recently-added switch is an action") as i32;
        let row = page
            .table
            .sections
            .iter()
            .flat_map(|s| s.rows.iter())
            .find(|r| r.label == crate::i18n::msg::settings_library_recently_added())
            .expect("the switch is in the table");
        assert_eq!(row.label, crate::i18n::msg::settings_library_recently_added());
        assert_eq!(row.toggle, Some(false), "off by default");
        assert!(matches!(page.actions[last as usize], Action::ToggleRecentlyAdded));
        assert!(row.ticon.is_none(), "a switch opens nothing, so RIGHT is inert on it");

        with_fx(|fx| page.activate(last as usize, fx));
        assert!(page.pending_recent.is_some(), "the press arms the durable write");
        assert!(page.state.busy, "the page shows the save in flight");
        let row = page
            .table
            .sections
            .iter()
            .flat_map(|s| s.rows.iter())
            .find(|r| r.label == crate::i18n::msg::settings_library_recently_added())
            .expect("the switch is in the table");
        assert_eq!(row.toggle, Some(true), "optimistic face");

        // …and the landing publishes the file's answer into the process copy the Library reads.
        let tick = Tick {
            ms: 16,
            dt_us: 16_000,
        };
        for _ in 0..1000 {
            if page.pending_recent.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            with_fx(|fx| {
                page.step(&ScreenEvent::Tick(tick), &context(last as u32), fx);
            });
        }
        assert!(page.pending_recent.is_none(), "the write landed");
        assert!(!page.state.busy);
        assert!(
            crate::route::show_recently_added(),
            "the file says on and the process copy follows"
        );
        let row = page
            .table
            .sections
            .iter()
            .flat_map(|s| s.rows.iter())
            .find(|r| r.label == crate::i18n::msg::settings_library_recently_added())
            .expect("the switch is in the table");
        assert_eq!(row.toggle, Some(true));
        crate::route::restore_show_recently_added(false);
    }

    /// **Every row presses the action it names, and shows that row's own value.** A row's element
    /// is its flattened index across the sections, so the vector `activate` reads must be in paint
    /// order. It was two parallel vectors built in different orders, and from the first switch row
    /// down every press ran somebody else's field. The assertion is per row rather than per field,
    /// so a new row cannot be added without its action.
    #[test]
    fn every_row_presses_the_action_it_names() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("prefs-row-action-pairing");
        let page = RootPage::new(EntryId(0));
        let rows: Vec<&Row> = page.table.sections.iter().flat_map(|s| s.rows.iter()).collect();
        assert_eq!(rows.len(), page.actions.len(), "one action per drawn row");
        for (i, row) in rows.iter().enumerate() {
            match &page.actions[i] {
                Action::Open(field) => {
                    assert_eq!(row.label, field.title(), "row {i} presses another field");
                    let shown = page.value_label(*field);
                    assert_eq!(
                        row.value.as_deref(),
                        Some(shown.as_str()),
                        "row {i} shows a value that is not the field's own"
                    );
                    assert_eq!(row.detail, field.detail(), "row {i} shows another field's line");
                    assert!(row.ticon == Some(crate::ui::icons::Icon::Chevron));
                }
                Action::ToggleRecentlyAdded => assert_eq!(
                    row.label,
                    crate::i18n::msg::settings_library_recently_added(),
                    "row {i} is not the switch its action toggles"
                ),
                Action::ToggleIdleScreen => assert_eq!(
                    row.label,
                    crate::i18n::msg::settings_playback_idle_screen(),
                    "row {i} is not the switch its action toggles"
                ),
                Action::Servers => assert_eq!(row.label, crate::i18n::msg::settings_servers_current()),
                Action::AddServer => assert_eq!(row.label, crate::i18n::msg::settings_servers_add()),
                Action::SignOut => {
                    assert_eq!(row.label, crate::i18n::msg::settings_servers_disconnect())
                }
                Action::Language => assert_eq!(row.label, crate::i18n::msg::settings_language_title()),
                Action::Legal => assert_eq!(row.label, crate::i18n::msg::settings_legal_title()),
                Action::About => assert_eq!(row.label, crate::i18n::msg::settings_about_title()),
                Action::Pick(_) => panic!("row {i}: the row list holds no picker option"),
            }
        }
    }

    /// The two presses the owner would have felt: OK (or RIGHT) on *Subtitle size* must open the
    /// subtitle-size picker and touch no switch, and OK on *Show recently added* must arm that
    /// switch's own write instead of opening a picker.
    #[test]
    fn the_subtitle_rows_open_their_own_pickers_and_the_switches_flip_themselves() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("prefs-row-action-press");
        crate::route::restore_show_recently_added(false);
        let mut page = RootPage::new(EntryId(0));
        let size = row_of(&page, crate::i18n::msg::settings_playback_subtitle_size());
        with_fx(|fx| page.activate(size as usize, fx));
        assert_eq!(
            page.state.picker,
            Some(Field::SubtitleSize),
            "OK on the subtitle-size row opens the subtitle-size picker"
        );
        assert!(
            page.pending_recent.is_none() && page.pending_idle.is_none(),
            "…and moves no switch"
        );
        with_fx(|fx| page.close_picker(fx));

        let recent = row_of(&page, crate::i18n::msg::settings_library_recently_added());
        with_fx(|fx| page.activate(recent as usize, fx));
        assert!(page.pending_recent.is_some(), "OK on the switch arms the switch's write");
        assert_eq!(page.state.picker, None, "…and opens no picker");
        // Let the storage worker land its write before the next test takes the session, as the
        // switch's own test does.
        let tick = Tick {
            ms: 16,
            dt_us: 16_000,
        };
        for _ in 0..1000 {
            if page.pending_recent.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            with_fx(|fx| {
                page.step(&ScreenEvent::Tick(tick), &context(recent as u32), fx);
            });
        }
        crate::route::restore_show_recently_added(false);
    }

    /// **The mockup's four groups, in its order, and every row on the page.** The groups are the
    /// grid's pairing unit, so the section list is what decides where the two-column layout breaks
    /// — and the crash this guards against is a group label that a translated string leaves empty.
    #[test]
    fn the_page_is_the_mockups_four_groups_in_order() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("settings-groups");
        let page = RootPage::new(EntryId(0));
        let headers: Vec<&str> = page.table.sections.iter().map(|s| s.header.as_str()).collect();
        assert_eq!(
            headers,
            vec![
                crate::i18n::msg::settings_servers_section(),
                crate::i18n::msg::settings_playback_section(),
                crate::i18n::msg::settings_audio_title(),
                crate::i18n::msg::settings_interface_section(),
            ],
            "the mockup's four groups, each with its own word"
        );
        assert_eq!(page.table.cols(), 2, "the mockup pairs rows in two columns");
        assert_eq!(
            page.actions.len(),
            page.table.sections.iter().map(|s| s.rows.len()).sum::<usize>(),
            "one action per drawn row, so a press still lands on the row it names"
        );
        assert_eq!(page.table.sections[0].rows.len(), 3, "server, add, disconnect");
        assert_eq!(page.table.sections[2].rows.len(), 3, "the three subtitle rows");
        assert_eq!(page.table.sections[3].rows.len(), 3, "language, legal, about");
    }
}
