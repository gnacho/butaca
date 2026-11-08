//! Playback defaults on the shared Settings table.
//! A picker edits one field, and a failed save leaves the confirmed value intact.
//! A picker is this page's own submenu over the confirmed value it edits, so it is a second
//! table rather than a page of the surface's stack - but it moves on the family's one push spring
//! (`route_screen::RoutePush`, the surface's own), sliding in and out exactly like a pushed page.
use std::borrow::Cow;
use std::sync::mpsc::{self, Receiver};
use crate::route::{DirectPlayMode, Quality};
use crate::ui::decision_prompt::{DecisionPrompt, PromptStep};
use crate::ui::frame::Budget;
use crate::ui::machine::{Canon, Cx, Delivery, Edge, Effects, EntryId, FocusKey, Fx, GroupId,
    Handled, InputEvent, InputKind, InstanceId, Key, LogicalState, Machine, MachineId};
use crate::ui::screen::{At, Dir, DrawFrame, Enter, FocusSource, FocusTarget, Focusable, GroupSpec,
    HitSource, Placed, RenderStrategy, Screen, ScreenEvent, Step};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::table_screen::{Header, TableScreen};
use crate::ui::route_screen::{RouteLayout, RoutePush};
use crate::ui::{theme, Painter, Rect};
use super::family::{table_focus, InnerHost, ALERT_GROUP};
use super::registry::{AppFx, PreferenceCmd, ALERT};

pub(crate) const SHAPE: &str = "PreferencesV2{kind:u8,picker:u8,selection:u32,busy:bool,status:str,quality:u8,direct_play:u8,confirm:bool,affirm:bool,alert_scroll:u32,values:[str]}";


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind { Playback }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field { Quality, DirectPlay, LipSync, UpNext, UpNextWindow, SubtitleSize, SubtitlePosition, SubtitleBackground }
impl Field {
    fn title(self) -> &'static str { match self {
        Self::Quality => crate::i18n::msg::settings_playback_quality(), Self::DirectPlay => crate::i18n::msg::settings_playback_direct_play(),
        Self::LipSync => crate::i18n::msg::settings_playback_lip_sync(),
        Self::UpNext => crate::i18n::msg::settings_playback_up_next(),
        Self::UpNextWindow => crate::i18n::msg::settings_playback_up_next_window(),
        Self::SubtitleSize => crate::i18n::msg::settings_playback_subtitle_size(),
        Self::SubtitlePosition => crate::i18n::msg::settings_playback_subtitle_position(),
        Self::SubtitleBackground => crate::i18n::msg::settings_playback_subtitle_background(),
    }}
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value { Quality(Quality), DirectPlay(DirectPlayMode), LipSync(i64), UpNext(crate::server::session::UpNextPolicy), Window(i64),
    SubtitleSize(u8), SubtitlePosition(u8), SubtitleBackground(u8) }
#[derive(Clone)]
enum Action { Open(Field), Pick(Value), ToggleRecentlyAdded }
struct State {
    kind: Kind, picker: Option<Field>, selected: i32, busy: bool, status: String,
    quality: Quality, direct_play: DirectPlayMode, values: Vec<String>,
    confirming: bool, affirmative: bool, alert_scroll: u32,
}
impl LogicalState for State {
    fn write(&self, c: &mut Canon) {
        c.u8(self.kind as u8).u8(self.picker.map_or(0, |p| p as u8 + 1))
            .u32(self.selected.max(0) as u32).bool(self.busy).str(&self.status)
            .u8(self.quality as u8).u8(self.direct_play as u8)
            .bool(self.confirming).bool(self.affirmative).u32(self.alert_scroll).u32(self.values.len() as u32);
        for v in &self.values { c.str(v); }
    }
    fn probe(&self, out: &mut String) {
        out.push_str(&format!("preferences {:?} picker={:?} sel={} busy={} confirm={}",
            self.kind, self.picker, self.selected, self.busy, self.confirming));
    }
}
enum Pending {
    Local(Receiver<bool>),
}

pub(crate) struct PreferencesPage {
    entry: EntryId,
    /// The page's own list of fields — the parent level of every picker.
    table: TableView,
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
    alert: DecisionPrompt,
}
impl PreferencesPage {
    pub(crate) fn new(entry: EntryId, kind: Kind) -> Self {
        let mut s = Self { entry, table: TableView::new(), picker_table: TableView::new(),
            submenu: RoutePush::new(), leaving: None, actions: Vec::new(),
            state: State { kind, picker: None, selected: 0, busy: false, status: String::new(),
                quality: crate::route::quality(), direct_play: crate::route::direct_play_mode(),
                values: Vec::new(), confirming: false, affirmative: false, alert_scroll: 0 },
            parent_row: 0, copy: String::new(), pending: None,
            pending_recent: None,
            alert: DecisionPrompt::new(ALERT_GROUP, ALERT, ALERT + 1, crate::i18n::msg::settings_cancel_c(), crate::i18n::msg::settings_playback_enable_force_c()) };
        s.rebuild(0);
        s
    }
    fn kind_title(&self) -> &'static str {
        match self.state.kind {
            Kind::Playback => crate::i18n::msg::settings_playback_title(),
        }
    }
    /// The table focus and activation act on: the open picker's, else the field list.
    fn active(&self) -> &TableView {
        if self.state.picker.is_some() { &self.picker_table } else { &self.table }
    }
    fn active_mut(&mut self) -> &mut TableView {
        if self.state.picker.is_some() { &mut self.picker_table } else { &mut self.table }
    }
    fn copy_text(&self) -> Cow<'_, str> {
        if !self.state.status.is_empty() {
            return if self.state.kind == Kind::Playback && self.state.direct_play == DirectPlayMode::Forced {
                Cow::Owned(format!("{}\n\n{}", self.state.status, crate::i18n::msg::settings_playback_force_note()))
            } else { Cow::Borrowed(&self.state.status) };
        }
        match self.state.kind {
            Kind::Playback if self.state.direct_play == DirectPlayMode::Forced => Cow::Borrowed(crate::i18n::msg::settings_playback_force_note()),
            Kind::Playback => Cow::Borrowed(crate::i18n::msg::settings_playback_copy()),
        }
    }
    fn view(&self) -> TableScreen<'_> {
        self.view_at(self.state.picker)
    }
    /// One level of the page: the field list (`None`) or a picker, each with its own crumb/title.
    fn view_at(&self, level: Option<Field>) -> TableScreen<'_> {
        let (crumb, title, table) = match level {
            Some(field) => (self.kind_title(), field.title(), &self.picker_table),
            None => (crate::i18n::msg::settings_title(), self.kind_title(), &self.table),
        };
        TableScreen::new(Header::new(RouteLayout::screen(), Some(crumb), title, &self.copy),
            table, GroupId(0), self.entry).with_home()
    }
    /// Draw one level through `p` and hand back the focus stops it registered.
    fn draw_level(&self, f: &DrawFrame<'_, '_, InnerHost>, level: Option<Field>, p: Painter) -> Vec<crate::ui::screen::Stop<u32>> {
        let mut inner = DrawFrame::with_navigation(f.cx, p, f.navigation());
        let mut v = self.view_at(level);
        crate::ui::screen::Part::<InnerHost>::draw(&mut v, &mut inner, Rect::FULL);
        inner.into_stops()
    }
    fn focus(&self, fx: &mut Effects<'_, InnerHost>, group: GroupId) {
        fx.push(Fx::Deliver(MachineId::Instance(InstanceId(0)),
            Delivery::Screen(ScreenEvent::Enter(Enter::Fresh { focus: if group == GroupId(0) {
                FocusTarget::Elem(FocusKey { entry: self.entry, elem: self.active().sel.max(0) as u32 })
            } else { FocusTarget::ContainerGroup(group) } }))));
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
        };
        self.pending = Some(Pending::Local(rx)); self.state.busy = true;
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
            Field::SubtitleBackground => Value::SubtitleBackground(crate::route::subtitle_background_step()),
        }
    }
    fn options(&self, field: Field) -> Vec<(String, Value)> {
        match field {
            Field::Quality => crate::route::available_quality_ladder().iter()
                .map(|q| (q.label().into(), Value::Quality(*q))).collect(),
            Field::DirectPlay => [DirectPlayMode::Auto, DirectPlayMode::Forced, DirectPlayMode::Disabled]
                .into_iter().map(|m| (mode_label(m).into(), Value::DirectPlay(m))).collect(),
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
            ].into_iter().map(|p| (up_next_label(p).into(), Value::UpNext(p))).collect(),
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
            ].into_iter().enumerate().map(|(i, label)| (label.into(), Value::SubtitleSize(i as u8))).collect(),
            Field::SubtitlePosition => [
                crate::i18n::msg::settings_playback_subtitle_position_bottom(),
                crate::i18n::msg::settings_playback_subtitle_position_low(),
                crate::i18n::msg::settings_playback_subtitle_position_middle(),
                crate::i18n::msg::settings_playback_subtitle_position_high(),
                crate::i18n::msg::settings_playback_subtitle_position_top(),
            ].into_iter().enumerate().map(|(i, label)| (label.into(), Value::SubtitlePosition(i as u8))).collect(),
            Field::SubtitleBackground => [
                crate::i18n::msg::settings_playback_subtitle_background_off(),
                crate::i18n::msg::settings_playback_subtitle_background_half(),
                crate::i18n::msg::settings_playback_subtitle_background_solid(),
            ].into_iter().enumerate().map(|(i, label)| (label.into(), Value::SubtitleBackground(i as u8))).collect(),
        }
    }
    fn value_label(&self, field: Field) -> String {
        let current = self.current_value(field);
        self.options(field).into_iter().find(|(_, v)| *v == current).map_or_else(|| crate::i18n::msg::settings_audio_not_set().into(), |(s, _)| s)
    }
    fn rebuild(&mut self, selected: i32) {
        self.state.quality = crate::route::quality();
        self.state.direct_play = crate::route::direct_play_mode();
        self.copy = self.copy_text().into_owned();
        self.state.confirming = self.alert.is_open(); self.state.affirmative = self.alert.choice();
        self.state.alert_scroll = self.alert.scroll_target_bits();
        self.state.values.clear(); self.actions.clear();
        // The field list is rebuilt at every level: it is the parent a picker slides over.
        // #322: the subtitle look is a group of its own, not a tail of the playback rows. The
        // SECTIONS change and the ROW ORDER does not: the recorded replay presses this page by
        // position, so regrouping must not move a single stop. The first group keeps its empty
        // header (the page title already names it); only the subtitle group needs a word.
        let mut section = Section::new("");
        let mut subtitles = Section::new(crate::i18n::msg::settings_audio_subtitles());
        let fields: Vec<Field> = match self.state.kind {
            // The window configures a card that "Do not offer" never draws, so the row goes with it.
            Kind::Playback => {
                let mut f = vec![Field::Quality, Field::DirectPlay, Field::LipSync, Field::UpNext];
                if crate::route::up_next_offered() { f.push(Field::UpNextWindow); }
                // The look of a subtitle track the app composes. Tail position: these never
                // displace the transport rows a person reaches for while watching.
                f.push(Field::SubtitleSize);
                f.push(Field::SubtitlePosition);
                f.push(Field::SubtitleBackground);
                f
            }
        };
        let mut list_values = Vec::new(); let mut list_actions = Vec::new();
        for &field in &fields {
            let value = self.value_label(field);
            let mut row = Row::new(field.title()).value(&value).chevron(true).dim(self.state.busy);
            if field == Field::Quality && self.state.direct_play == DirectPlayMode::Forced {
                row = row.detail(crate::i18n::msg::settings_playback_overridden());
            }
            section = if matches!(
                field,
                Field::SubtitleSize | Field::SubtitlePosition | Field::SubtitleBackground
            ) {
                subtitles = subtitles.row(row);
                section
            } else {
                section.row(row)
            };
            list_values.push(value);
            list_actions.push(Action::Open(field));
        }
        // #297: the Library's opt-in Recently Added shelf. A SWITCH, not a picker: it flips in
        // place, wears no chevron (so RIGHT does not enter it), and its row lives here rather
        // than on the Settings root because the controlled replay opens this page but never
        // records it.
        let recent = self
            .pending_recent
            .as_ref()
            .map_or_else(crate::route::show_recently_added, |(on, _)| *on);
        section = section.row(
            Row::new(crate::i18n::msg::settings_library_recently_added())
                .detail(crate::i18n::msg::settings_library_recently_added_detail())
                .toggle(recent)
                .dim(self.state.busy),
        );
        list_values.push(
            if recent {
                crate::i18n::msg::widgets_toggle_on()
            } else {
                crate::i18n::msg::widgets_toggle_off()
            }
            .into(),
        );
        list_actions.push(Action::ToggleRecentlyAdded);
        let list_sel = if self.state.picker.is_some() { self.parent_row } else { selected };
        for table in [&mut self.table, &mut self.picker_table] {
            table.compact = false; table.header_ink = theme::TEXT_READING; table.list_focused = true;
        }
        self.table.set_sections(vec![section, subtitles], list_sel, false);
        if let Some(field) = self.state.picker {
            let current = self.current_value(field);
            let mut section = Section::new("");
            for (label, value) in self.options(field) {
                section = section.row(Row::new(&label).checked(value == current).dim(self.state.busy));
                self.state.values.push(label); self.actions.push(Action::Pick(value));
            }
            self.picker_table.set_sections(vec![section], selected, false);
        } else {
            self.state.values = list_values; self.actions = list_actions;
        }
        self.state.selected = self.active().sel;
    }
    fn close_picker(&mut self, fx: &mut Effects<'_, InnerHost>) {
        self.leaving = self.state.picker.take().or(self.leaving);
        self.rebuild(self.parent_row); self.focus(fx, GroupId(0));
    }
    fn activate(&mut self, row: usize, fx: &mut Effects<'_, InnerHost>) {
        if self.state.busy { return; }
        let Some(action) = self.actions.get(row).cloned() else { return; };
        match action {
            Action::Open(field) => {
                self.parent_row = row as i32; self.state.picker = Some(field); self.leaving = None;
                let current = self.current_value(field);
                let selected = self.options(field).iter().position(|(_, v)| *v == current).unwrap_or(0);
                self.rebuild(selected as i32); self.focus(fx, GroupId(0));
            }
            Action::Pick(value) => {
                if value == Value::DirectPlay(DirectPlayMode::Forced) && self.state.direct_play != DirectPlayMode::Forced {
                    self.alert.open(crate::i18n::msg::settings_playback_force_question_c(), crate::i18n::msg::settings_playback_force_body());
                    self.state.confirming = true; self.state.affirmative = false;
                    self.state.alert_scroll = self.alert.scroll_target_bits(); self.focus(fx, ALERT_GROUP);
                    return;
                }
                self.commit(value, fx); self.close_picker(fx);
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
        }
        fx.invalidate(crate::ui::present::Provenance::Input);
    }
    fn commit(&mut self, value: Value, fx: &mut Effects<'_, InnerHost>) {
        match value {
            Value::Quality(_) | Value::DirectPlay(_) | Value::LipSync(_)
            | Value::UpNext(_) | Value::Window(_) | Value::SubtitleSize(_)
            | Value::SubtitlePosition(_) | Value::SubtitleBackground(_) => self.save_local(value, fx),
        }
    }
    fn poll(&mut self) -> bool {
        enum Receipt { Local(bool), Failed }
        let receipt = match &self.pending {
            Some(Pending::Local(ticket)) => match ticket.try_recv() {
                Ok(result) => Some(Receipt::Local(result)),
                Err(mpsc::TryRecvError::Disconnected) => Some(Receipt::Failed), _ => None,
            },
            None => None,
        };
        let Some(receipt) = receipt else { return false; };
        self.pending = None; self.state.busy = false;
        match receipt {
            Receipt::Local(true) => self.state.status.clear(),
            Receipt::Local(false) | Receipt::Failed => self.state.status = crate::i18n::msg::settings_playback_save_failed().into(),
        }
        true
    }
}
fn mode_label(mode: DirectPlayMode) -> &'static str {
    match mode { DirectPlayMode::Auto => crate::i18n::msg::settings_playback_auto(), DirectPlayMode::Forced => crate::i18n::msg::settings_playback_forced(), DirectPlayMode::Disabled => crate::i18n::msg::settings_playback_disabled() }
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

/// The lip-sync picker's row label: `0 ms` at the centre, a signed value off it.
fn lip_sync_label(ms: i64) -> String {
    if ms == 0 { "0 ms".to_string() } else { format!("{ms:+} ms") }
}
impl Machine<InnerHost> for PreferencesPage {
    type Ev = ScreenEvent<InnerHost>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, InnerHost>, fx: &mut Effects<'_, InnerHost>) -> Handled {
        match self.alert.step(ev, cx) {
            PromptStep::Pass => (),
            PromptStep::Done(handled) => {
                self.state.affirmative = self.alert.choice();
                self.state.alert_scroll = self.alert.scroll_target_bits();
                return handled;
            }
            PromptStep::Answer(yes) => {
                if yes { self.save_local(Value::DirectPlay(DirectPlayMode::Forced), fx); }
                self.state.confirming = false;
                if yes { self.close_picker(fx); } else { self.focus(fx, GroupId(0)); }
                fx.invalidate(crate::ui::present::Provenance::Input); return Handled::Yes;
            }
        }
        match ev {
            ScreenEvent::Enter(_) => {
                Handled::No
            }
            ScreenEvent::Tick(t) => {
                self.alert.update(t.dt());
                let had_rows = self.active().n_rows() > 0;
                // #297: the recently-added switch's write landed. Publish the file's own answer
                // into the process copy the Library reads before the page rebuilds its face.
                let mut recent_landed = false;
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
                    recent_landed = true;
                }
                if self.poll() || recent_landed
                    || self.state.quality != crate::route::quality()
                    || self.state.direct_play != crate::route::direct_play_mode() {
                    self.rebuild(self.active().sel);
                    // An empty loading table had no engine seat. Give its first landing (or
                    // Retry row) one so OK works immediately, without moving an existing
                    // cursor when an ordinary save completes or another page owns focus.
                    if !had_rows && self.active().n_rows() > 0 && cx.focus.current.is_none() {
                        self.focus(fx, GroupId(0));
                    }
                    fx.invalidate(crate::ui::present::Provenance::Landing(MachineId::Session));
                }
                let h = RouteLayout::screen().sectioned_table().h;
                self.table.update(t.dt(), h); self.picker_table.update(t.dt(), h);
                let open = self.state.picker.is_some();
                self.submenu.tick(open, *t, &mut fx.present());
                if !open && self.submenu.resting(false) { self.leaving = None; }
                Handled::Yes
            }
            ScreenEvent::FocusMoved { to, .. } => {
                table_focus(self.active_mut(), to.elem); self.state.selected = self.active().sel; Handled::Yes
            }
            ScreenEvent::Activate(elem) => { self.activate(*elem as usize, fx); Handled::Yes }
            ScreenEvent::Input(InputEvent { kind: InputKind::Key { key: Key::Back, edge: Edge::Down, .. }, .. }) if self.state.picker.is_some() => {
                self.close_picker(fx); Handled::Yes
            }
            ScreenEvent::Input(InputEvent { kind: InputKind::Key { key: Key::Right, at_edge: true, .. }, .. }) if self.state.picker.is_none() => {
                if let Some(key) = cx.focus.current {
                    // Only a chevron row ENTERS something; a switch changes in place, so RIGHT is
                    // inert on it exactly as it is on the Settings root.
                    if self.active().row_opens(key.elem as i32) { self.activate(key.elem as usize, fx); }
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

impl Focusable<InnerHost> for PreferencesPage {
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
            return FocusKey { entry: self.entry, elem: self.active().sel.max(0) as u32 };
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

impl Screen<InnerHost> for PreferencesPage {
    fn name(&self) -> &'static str {
        match self.state.kind { Kind::Playback => "settings-playback" }
    }
    fn state(&self) -> &dyn LogicalState {
        &self.state
    }
    fn crumb(&self, _cx: &Cx<'_, InnerHost>) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(crate::i18n::msg::settings_title()))
    }
    fn back_control(&self, cx: &Cx<'_, InnerHost>) -> Option<crate::ui::Rect> {
        self.view().crumb_rect(cx.measure)
    }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, InnerHost>) {}
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, InnerHost>) {
        let open = self.state.picker.is_some();
        if self.submenu.resting(open) {
            let mut v = self.view();
            crate::ui::screen::Part::<InnerHost>::draw(&mut v, f, Rect::FULL);
        } else {
            // Mid-push: the field list in the parent role, the picker (or the one leaving) in
            // the child role; only the level focus acts on contributes stops.
            let (parent_p, child_p) = (self.submenu.parent(f.painter), self.submenu.child(f.painter));
            let parent = self.draw_level(f, None, parent_p);
            let child = self.state.picker.or(self.leaving)
                .map(|field| self.draw_level(f, Some(field), child_p)).unwrap_or_default();
            for s in if open { child } else { parent } { f.stop(Painter::root(), s); }
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
    use crate::ui::machine::{FocusRead, InputOwner, PressRead, Tick};
    use crate::ui::present::Present;
    use crate::ui::fixture::FixtureMeasure;

    fn with_fx(f: impl FnOnce(&mut Effects<'_, InnerHost>)) {
        let mut out = Vec::new(); let mut present = Present::new();
        f(&mut Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present));
    }
    fn context(focus: u32) -> Cx<'static, InnerHost> {
        static MEASURE: FixtureMeasure = FixtureMeasure;
        Cx { views: crate::stores::browse::DirectoryView::empty_for_test(), tick: Tick::default(),
            measure: &MEASURE, press: PressRead::default(),
            focus: FocusRead { current: Some(FocusKey { entry: EntryId(0), elem: focus }), ..Default::default() },
            owner: InputOwner::Entry(EntryId(0)) }
    }
    #[test]
    fn force_requires_acknowledgement_and_cancel_never_saves() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("force-confirm-cancel");
        crate::route::restore_direct_play_mode(DirectPlayMode::Auto);
        let mut page = PreferencesPage::new(EntryId(0), Kind::Playback);
        with_fx(|fx| page.activate(1, fx));
        assert_eq!(page.state.picker, Some(Field::DirectPlay));
        with_fx(|fx| page.activate(1, fx));
        assert!(page.alert.is_open());
        assert!(!page.alert.choice(), "Cancel is the default answer");
        assert!(page.pending.is_none(), "opening the warning must not persist Force");
        assert_eq!(crate::route::direct_play_mode(), DirectPlayMode::Auto);
        let cx = context(ALERT);
        let cancel = ScreenEvent::Input(InputEvent { at: Tick::default(), source: crate::ui::machine::Source::RemoteFifo,
            kind: InputKind::Key { key: Key::Back, edge: Edge::Down, sym: 0, wcode: 0, at_edge: false } });
        with_fx(|fx| { page.step(&cancel, &cx, fx); });
        assert!(!page.alert.is_open());
        assert!(page.pending.is_none());
        assert_eq!(crate::route::direct_play_mode(), DirectPlayMode::Auto);
    }
    #[test]
    fn confirming_force_emits_a_preference_effect_without_executing_it() {
        let _serial = crate::testlock::serial();
        let previous = crate::route::direct_play_mode();
        crate::route::restore_direct_play_mode(DirectPlayMode::Auto);
        let mut page = PreferencesPage::new(EntryId(0), Kind::Playback);
        with_fx(|fx| page.activate(1, fx));
        with_fx(|fx| page.activate(1, fx));
        let mut emitted = Vec::new(); let mut present = Present::new();
        let mut fx = Effects::new(&mut emitted, MachineId::Instance(InstanceId(0)), &mut present);
        page.step(&ScreenEvent::PressCommit(crate::ui::machine::PressId(1)), &context(ALERT + 1), &mut fx);
        assert!(page.state.busy);
        assert!(emitted.iter().any(|event| matches!(&event.fx,
            Fx::App(AppFx::Preferences(PreferenceCmd::DirectPlay { mode: DirectPlayMode::Forced, .. })))));
        assert_eq!(crate::route::direct_play_mode(), DirectPlayMode::Auto,
            "only an admitted app executor can persist and activate Force");
        crate::route::restore_direct_play_mode(previous);
    }

    /// Owner report: Direct Play / Default Quality (and every Audio & Subtitles picker) switched
    /// to the next screen instantly. A picker is this page's own submenu, so it must run the
    /// family's push spring — in on open, back out on close with the picker still drawn.
    #[test]
    fn a_picker_slides_in_and_out_on_the_family_push_spring() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("picker-push-spring");
        let mut page = PreferencesPage::new(EntryId(0), Kind::Playback);
        assert!(page.submenu.resting(false), "no picker: the list is at rest");
        with_fx(|fx| page.activate(0, fx));
        assert_eq!(page.state.picker, Some(Field::Quality));
        assert!(page.submenu.amount() < 0.01 && !page.submenu.resting(true),
            "opening a picker starts the push rather than cutting to it");
        let t = Tick { ms: 16, dt_us: 16_000 };
        for _ in 0..120 { with_fx(|fx| { page.step(&ScreenEvent::Tick(t), &context(0), fx); }); }
        assert!(page.submenu.resting(true), "the push settles open");
        with_fx(|fx| page.close_picker(fx));
        assert_eq!(page.state.picker, None);
        assert!(page.submenu.amount() > 0.99 && !page.submenu.resting(false),
            "closing runs the same spring back");
        assert_eq!(page.leaving, Some(Field::Quality), "the picker keeps drawing while it leaves");
        for _ in 0..120 { with_fx(|fx| { page.step(&ScreenEvent::Tick(t), &context(0), fx); }); }
        assert!(page.submenu.resting(false));
        assert_eq!(page.leaving, None, "…and is released once the spring settles");
    }
    #[test]
    fn picker_back_restores_parent_row_without_saving() {
        let _serial = crate::testlock::serial();
        let mut page = PreferencesPage::new(EntryId(0), Kind::Playback);
        with_fx(|fx| page.activate(1, fx));
        with_fx(|fx| page.close_picker(fx));
        assert_eq!(page.state.picker, None);
        assert_eq!(page.table.sel, 1);
        assert!(page.pending.is_none());
    }

    /// #297: the Library's opt-in Recently Added shelf is a switch at the tail of *Video &
    /// playback* — the Settings root stays four rows. Its face reads the process copy and its
    /// press arms a durable write, then the page publishes the file's own answer.
    #[test]
    fn the_recently_added_switch_flips_in_place_and_publishes_its_write() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("prefs-recently-added");
        crate::route::restore_show_recently_added(false);
        let mut page = PreferencesPage::new(EntryId(0), Kind::Playback);
        // #322 split the rows into groups, so the switch is the last ACTION, not the last row of
        // the first section.
        let last = page.actions.len() as i32 - 1;
        let row = page.table.sections.iter().flat_map(|s| s.rows.iter()).find(|r| r.label == crate::i18n::msg::settings_library_recently_added()).expect("the switch is in the table");
        assert_eq!(row.label, crate::i18n::msg::settings_library_recently_added());
        assert_eq!(row.toggle, Some(false), "off by default");
        assert!(matches!(page.actions[last as usize], Action::ToggleRecentlyAdded));
        assert!(row.ticon.is_none(), "a switch opens nothing, so RIGHT is inert on it");

        with_fx(|fx| page.activate(last as usize, fx));
        assert!(page.pending_recent.is_some(), "the press arms the durable write");
        assert!(page.state.busy, "the page shows the save in flight");
        let row = page.table.sections.iter().flat_map(|s| s.rows.iter()).find(|r| r.label == crate::i18n::msg::settings_library_recently_added()).expect("the switch is in the table");
        assert_eq!(row.toggle, Some(true), "optimistic face");

        // …and the landing publishes the file's answer into the process copy the Library reads.
        // The write is the storage worker's, so the tick loop waits for it rather than assuming
        // it finished inside a fixed number of frames.
        let tick = Tick { ms: 16, dt_us: 16_000 };
        for _ in 0..1000 {
            if page.pending_recent.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            with_fx(|fx| { page.step(&ScreenEvent::Tick(tick), &context(last as u32), fx); });
        }
        assert!(page.pending_recent.is_none(), "the write landed");
        assert!(!page.state.busy);
        assert!(crate::route::show_recently_added(), "the file says on and the process copy follows");
        let row = page.table.sections.iter().flat_map(|s| s.rows.iter()).find(|r| r.label == crate::i18n::msg::settings_library_recently_added()).expect("the switch is in the table");
        assert_eq!(row.toggle, Some(true));
        crate::route::restore_show_recently_added(false);
    }

    /// #322: the subtitle look stops being the tail of a long list of playback knobs and gets a
    /// group of its own, with a word on it. The ROW ORDER is what the recorded replay presses by
    /// position, so the split is not allowed to move a single stop: the flattened rows must still
    /// read playback first, subtitles second, and the Library switch last.
    #[test]
    fn the_subtitle_look_gets_its_own_group_and_no_row_moves() {
        let _serial = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("prefs-subtitle-group");
        let page = PreferencesPage::new(EntryId(0), Kind::Playback);
        let headers: Vec<&str> = page
            .table
            .sections
            .iter()
            .map(|s| s.header.as_str())
            .collect();
        assert_eq!(
            headers,
            vec!["", crate::i18n::msg::settings_audio_subtitles()],
            "the playback group needs no word (the page title has one), the subtitle group does"
        );
        let labels: Vec<&str> = page
            .table
            .sections
            .iter()
            .flat_map(|s| s.rows.iter())
            .map(|r| r.label.as_str())
            .collect();
        let subtitle = crate::i18n::msg::settings_playback_subtitle_size();
        let at = labels
            .iter()
            .position(|l| *l == subtitle)
            .expect("the subtitle group is on the page");
        assert!(
            at > 0,
            "the subtitle rows come after the playback rows, never before"
        );
        for label in labels[..at].iter() {
            assert!(
                !label.starts_with("Subtitle "),
                "a subtitle row stayed behind in the playback group: {label}"
            );
        }
        for label in labels[at..].iter() {
            assert!(
                label.starts_with("Subtitle ")
                    || *label == crate::i18n::msg::settings_library_recently_added(),
                "a playback row fell into the subtitle group: {label}"
            );
        }
        assert_eq!(
            page.actions.len(),
            labels.len(),
            "one action per row, so a press still lands on the row it names"
        );
    }
}
