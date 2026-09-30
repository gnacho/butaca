//! The Jellyfin flavor's **server picker** — a table page of the Settings surface, one row per
//! configured server (name + URL), the active one checked. OK on a row signs in against that
//! server and makes it active; the store reset and navigation are the loop's, requested through
//! [`LoopReq::JfSwitchServer`] exactly as the root's other door rows reach the loop (§2.1: a
//! screen names no `Route`).
//!
//! Modeled on `screens::legal::LegalIndex` (a list of rows, OK activates, BACK pops the surface's
//! own stack) and `screens::settings::LanguagePage` (the `.checked` active marker).
#![cfg(feature = "jellyfin")]

use std::borrow::Cow;

use crate::jellyfin::boot::ServerEntry;
use crate::ui::frame::Budget;
use crate::ui::machine::{
    Canon, Cx, Effects, EntryId, Fx, GroupId, Handled, Key, LogicalState, Machine,
};
use crate::ui::route_screen::RouteLayout;
use crate::ui::screen::{
    DrawFrame, FocusSource, HitSource, Part, RenderStrategy, Screen, ScreenEvent,
};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::table_screen::{Header, TableScreen};
use crate::ui::{theme, Rect};

use super::family::{table_focus, InnerHost};
use super::registry::{AppFx, LoopReq};

pub(crate) struct ServersPage {
    entry: EntryId,
    table: TableView,
    state: ServersState,
    /// How many server rows the table has — the picker's index → server mapping. A switch request
    /// names a LIST index, so the row position is the index the loop resolves.
    count: usize,
}

struct ServersState {
    sel: i32,
}

impl LogicalState for ServersState {
    fn write(&self, w: &mut Canon) {
        w.u32(self.sel as u32);
    }
    fn probe(&self, out: &mut String) {
        out.push_str(&format!("servers sel={}", self.sel));
    }
}

impl ServersPage {
    pub(crate) fn new(entry: EntryId) -> Self {
        let (servers, active) = match crate::jellyfin::boot::load_servers() {
            Some(list) => (list.servers, list.active),
            None => (Vec::new(), 0),
        };
        Self::with_servers(entry, servers, active)
    }

    /// Build the page from an explicit list — the filesystem read is `new`'s, this is the pure
    /// half the tests drive.
    fn with_servers(entry: EntryId, servers: Vec<ServerEntry>, active: usize) -> Self {
        let count = servers.len();
        let section = Self::sections(&servers, active);
        let mut table = TableView::new();
        table.compact = false;
        table.header_ink = theme::TEXT_READING;
        table.set_sections(vec![section], 0, false);
        table.list_focused = true;
        Self {
            entry,
            table,
            state: ServersState { sel: 0 },
            count,
        }
    }

    /// One section, one row per server: the account name labels it, the URL is the detail, and the
    /// active server carries the leading check.
    fn sections(servers: &[ServerEntry], active: usize) -> Section {
        let mut section = Section::new("");
        for (i, server) in servers.iter().enumerate() {
            section = section.row(
                Row::new(server.user.clone())
                    .detail(server.url.clone())
                    .checked(i == active),
            );
        }
        section
    }

    fn view(&self) -> TableScreen<'_> {
        TableScreen::new(
            Header::new(
                RouteLayout::screen(),
                Some(crate::i18n::msg::settings_title()),
                crate::i18n::msg::settings_servers_select(),
                crate::i18n::msg::settings_servers_copy(),
            ),
            &self.table,
            GroupId(0),
            self.entry,
        )
    }

    fn activate(&mut self, row: i32, fx: &mut Effects<'_, InnerHost>) {
        if row >= 0 && (row as usize) < self.count {
            fx.push(Fx::App(AppFx::Loop(LoopReq::JfSwitchServer(row as usize))));
        }
    }
}

impl Machine<InnerHost> for ServersPage {
    type Ev = ScreenEvent<InnerHost>;
    fn step(
        &mut self,
        ev: &Self::Ev,
        cx: &Cx<'_, InnerHost>,
        fx: &mut Effects<'_, InnerHost>,
    ) -> Handled {
        match ev {
            ScreenEvent::Tick(t) => {
                self.table
                    .update(t.dt(), RouteLayout::screen().sectioned_table().h);
                Handled::Yes
            }
            ScreenEvent::FocusMoved { to, .. } => {
                table_focus(&mut self.table, to.elem);
                self.state.sel = self.table.sel;
                Handled::Yes
            }
            ScreenEvent::Activate(e) => {
                self.activate(*e as i32, fx);
                Handled::Yes
            }
            ScreenEvent::Input(crate::ui::machine::InputEvent {
                kind:
                    crate::ui::machine::InputKind::Key {
                        key: Key::Right,
                        at_edge: true,
                        ..
                    },
                ..
            }) => {
                // rule 8: RIGHT on a row that opens nested content enters it, exactly as OK does
                if let Some(k) = cx.focus.current {
                    if self.table.row_opens(k.elem as i32) {
                        self.activate(k.elem as i32, fx);
                    }
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

crate::focusable_via_view!(ServersPage, InnerHost, view);

impl Screen<InnerHost> for ServersPage {
    fn name(&self) -> &'static str {
        "servers"
    }
    fn state(&self) -> &dyn LogicalState {
        &self.state
    }
    fn crumb(&self, _cx: &Cx<'_, InnerHost>) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(crate::i18n::msg::settings_title()))
    }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, InnerHost>) {}
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, InnerHost>) {
        let mut v = self.view();
        Part::<InnerHost>::draw(&mut v, f, Rect::FULL);
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
    use crate::ui::machine::{InstanceId, MachineId};
    use crate::ui::present::Present;

    fn server(url: &str, user: &str) -> ServerEntry {
        ServerEntry {
            url: url.to_string(),
            user: user.to_string(),
            password: "pw".to_string(),
            device_id: Some("dev".to_string()),
        }
    }

    /// The picker builds one row per configured server, the active one checked.
    #[test]
    fn the_picker_builds_one_row_per_configured_server() {
        let page = ServersPage::with_servers(
            EntryId(0),
            vec![server("http://10.0.0.2:8096", "gleb"), server("http://10.0.0.3:8096", "kid")],
            1,
        );
        assert_eq!(page.count, 2);
        let rows: Vec<_> = page.table.sections.iter().flat_map(|s| &s.rows).collect();
        assert_eq!(rows.len(), 2, "one row per configured server");
        assert_eq!(rows[0].label, "gleb");
        assert_eq!(rows[0].detail, "http://10.0.0.2:8096");
        assert!(!rows[0].checked);
        assert_eq!(rows[1].label, "kid");
        assert!(rows[1].checked, "the active server carries the check");
    }

    /// A switch request names the list index of the pressed row.
    #[test]
    fn activating_a_row_requests_a_switch_to_that_index() {
        let mut page = ServersPage::with_servers(
            EntryId(0),
            vec![server("http://10.0.0.2:8096", "a"), server("http://10.0.0.3:8096", "b")],
            0,
        );
        let mut out = Vec::new();
        let mut present = Present::new();
        let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
        page.activate(1, &mut fx);
        let reqs: Vec<_> = out
            .iter()
            .filter_map(|st| match &st.fx {
                Fx::App(AppFx::Loop(req)) => Some(*req),
                _ => None,
            })
            .collect();
        assert_eq!(reqs, vec![LoopReq::JfSwitchServer(1)]);
    }
}
