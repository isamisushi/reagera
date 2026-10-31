//! The devtools panel: a timeline to scrub, pending requests, the event list,
//! export and import.
//! Everything it shows comes from the store's public API, so anything you can do
//! here you can also do in a test.

use crate::{
    effect::short,
    html::{button, div, input, li, ol, span, strong, text, textarea, ul, Html},
    App, Options, Store,
};

/// Messages of the panel. They are not part of the app log.
#[derive(Clone, Debug)]
pub enum DevMsg {
    /// Show the state as of this millisecond.
    Backtrack(i64),
    /// Follow the newest event again.
    Live,
    /// One event earlier.
    StepBack,
    /// One event later.
    StepForward,
    /// Put the session text into the export box.
    Export,
    /// The import box changed.
    ImportText(String),
    /// Replace the history with the import box.
    Import,
}

/// Local state of the panel.
#[derive(Default)]
pub struct DevState {
    exported: String,
    import_text: String,
    error: String,
}

/// Apply a panel message to the store and the panel.
pub fn handle<A: App>(store: &mut Store<A>, local: &mut DevState, msg: DevMsg) {
    match msg {
        DevMsg::Backtrack(ms) => store.backtrack(ms),
        DevMsg::Live => store.go_live(),
        DevMsg::StepBack => {
            let head = store.head_ms();
            let before = store.events_between(i64::MIN, head - 1).last().map(|r| r.ms());
            store.backtrack(before.unwrap_or_else(|| store.earliest_ms() - 1));
        }
        DevMsg::StepForward => {
            let head = store.head_ms();
            match store.events_between(head, i64::MAX).first().map(|r| r.ms()) {
                Some(ms) => store.backtrack(ms),
                None => store.go_live(),
            }
        }
        DevMsg::Export => local.exported = store.export_text(),
        DevMsg::ImportText(s) => local.import_text = s,
        DevMsg::Import => match Store::<A>::import_text(&local.import_text, Options::default()) {
            Ok(mut other) => {
                for r in other.events() {
                    store.receive(r);
                }
                if other.live() {
                    store.go_live();
                } else {
                    store.backtrack(other.head_ms());
                }
                local.error.clear();
            }
            Err(e) => local.error = e.to_string(),
        },
    }
}

/// The panel for a store.
pub fn view<A: App>(store: &mut Store<A>, local: &DevState) -> Html<DevMsg> {
    let events = store.events();
    let status = store.engine_status();
    let min = store.earliest_ms();
    let max = store.latest_ms().max(min + 1);
    let head = store.head_ms();
    let live = store.live();
    let pending = store.pending().to_vec();
    let mine = store.origin().to_string();
    div()
        .style(PANEL)
        .child(
            div()
                .style("display:flex;gap:8px;align-items:center")
                .child(strong().text("reagera"))
                .child(span().text(format!("{} events", events.len())))
                .child_if(!pending.is_empty(), || span().style("color:#fbbf24").text(format!("{} pending", pending.len())).build())
                .child(span().attr("title", "ConTime worker: histories, pending, checkpoint spacing").text(format!("contime {}h {}p cp{}", status.histories, status.pending, status.checkpoint_interval)))
                .child(span().style(if live { "margin-left:auto;color:#34d399" } else { "margin-left:auto;color:#fbbf24" }).text(if live { "LIVE".to_string() } else { format!("@ +{}ms", head - min) })),
        )
        .child_if(!live, || span().style("color:#fbbf24").text("read-only: the head is in the past, press live to edit").build())
        .child(
            input()
                .attr("type", "range")
                .attr("min", min)
                .attr("max", max)
                .attr("step", 1)
                .style("width:100%")
                .value(head)
                .on("input", |e| e.value.parse().ok().map(DevMsg::Backtrack)),
        )
        .child(
            div()
                .style("display:flex;gap:6px;flex-wrap:wrap")
                .child(btn("live", DevMsg::Live))
                .child(btn("step back", DevMsg::StepBack))
                .child(btn("step forward", DevMsg::StepForward))
                .child(btn("export", DevMsg::Export))
                .child(btn("import", DevMsg::Import)),
        )
        .child_if(!local.exported.is_empty(), || textarea().attr("readonly", "").attr("rows", 3).style("width:100%").value(local.exported.clone()).build())
        .child(
            textarea()
                .attr("rows", 2)
                .attr("placeholder", "paste an exported session here, then press import")
                .style("width:100%")
                .value(local.import_text.clone())
                .on_input(DevMsg::ImportText),
        )
        .child_if(!local.error.is_empty(), || span().style("color:#f87171").text(local.error.clone()).build())
        .child_if(!pending.is_empty(), || {
            ul().style("margin:6px 0 0;padding-left:20px;color:#fbbf24")
                .children(pending.iter().map(|p| {
                    let who = if p.origin == mine { "waiting for the answer" } else { "another origin's, never performed here" };
                    li().key(short(p.id)).attr("title", who).text(format!("+{}ms #{} {} {:?}", p.ms - min, short(p.id), p.origin, p.request)).build()
                }))
                .build()
        })
        .child(ol().style("margin:6px 0 0;padding-left:20px").children(events.iter().map(|r| {
            let ms = r.ms();
            li().key(format!("{:x}", r.id()))
                .style(if ms <= head { "cursor:pointer" } else { "cursor:pointer;opacity:.45" })
                .on("click", move |_| Some(DevMsg::Backtrack(ms)))
                .text(format!("+{}ms {}#{} {:?}", ms - min, r.origin(), r.seq(), r.body()))
                .build()
        })))
        .build()
}

fn btn(label: &str, msg: DevMsg) -> Html<DevMsg> {
    button().style("font:inherit;padding:2px 8px").on_click(msg).child(text(label)).build()
}

const PANEL: &str = "position:fixed;right:0;bottom:0;width:360px;max-height:50vh;overflow:auto;background:#111827;color:#e5e7eb;\
font:12px/1.4 ui-monospace,SFMono-Regular,Menlo,monospace;padding:8px;box-sizing:border-box;border-top-left-radius:8px;\
box-shadow:0 0 12px rgba(0,0,0,.4);z-index:99999";
