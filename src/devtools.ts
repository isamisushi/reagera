/**
 * In-page devtools. A timeline you can scrub, the event list, and
 * export/import of the session. Everything it shows comes from the store's
 * public API, so anything you can do here you can also do in a test.
 */
import { createRoot, h, type VNode } from "./dom.js";
import type { EventRecord } from "./log.js";
import type { Store } from "./store.js";

export interface DevtoolsOptions<E> {
  /** Render event bodies. Default: JSON. */
  describe?: (event: EventRecord<E>) => string;
  /** Where to mount. Default: a fixed panel appended to document.body. */
  container?: Element;
}

const panelStyle = {
  position: "fixed",
  right: "0",
  bottom: "0",
  width: "360px",
  maxHeight: "50vh",
  overflow: "auto",
  background: "#111827",
  color: "#e5e7eb",
  font: "12px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace",
  padding: "8px",
  boxSizing: "border-box",
  borderTopLeftRadius: "8px",
  boxShadow: "0 0 12px rgba(0,0,0,.4)",
  zIndex: "99999",
};

export function devtools<S, E>(store: Store<S, E>, options: DevtoolsOptions<E> = {}): () => void {
  const describe = options.describe ?? ((ev: EventRecord<E>) => JSON.stringify(ev.body));
  const container = options.container ?? document.body.appendChild(document.createElement("div"));
  let importText = "";
  let exported = "";

  // Devtools have their own tiny local UI state (the textarea). It is not part of
  // the app log on purpose: it is not app behaviour and should not be replayed.
  const render = (): VNode => {
    const events = store.events();
    const min = events.length ? events[0].time : 0;
    const max = store.latestTime === -Infinity ? 0 : store.latestTime;
    const head = store.head === -Infinity ? 0 : store.head;
    const faults = store.faults.length;
    return h("div", { style: panelStyle },
      h("div", { style: { display: "flex", gap: "8px", alignItems: "center" } },
        h("strong", null, "reagera"),
        h("span", null, `${events.length} events`),
        faults ? h("span", { style: { color: "#f87171" } }, `${faults} faults`) : null,
        h("span", { style: { marginLeft: "auto", color: store.live ? "#34d399" : "#fbbf24" } },
          store.live ? "LIVE" : `@ ${head - min}ms`),
      ),
      h("input", {
        type: "range", min, max, step: 1, value: head,
        style: { width: "100%" },
        oninput: (e: Event) => store.backtrack(Number((e.target as HTMLInputElement).value)),
      }),
      h("div", { style: { display: "flex", gap: "6px", flexWrap: "wrap" } },
        button("live", () => store.goLive()),
        button("step back", () => stepBack(store)),
        button("step forward", () => stepForward(store)),
        button("export", () => { exported = store.exportText(); rerender(); }),
        button("import", () => { if (importText.trim()) importInto(store, importText); }),
      ),
      exported ? h("textarea", { readonly: true, rows: 3, style: { width: "100%" }, value: exported }) : null,
      h("textarea", {
        rows: 2, placeholder: "paste an exported session here, then press import",
        style: { width: "100%" }, value: importText,
        oninput: (e: Event) => { importText = (e.target as HTMLTextAreaElement).value; },
      }),
      h("ol", { style: { margin: "6px 0 0", paddingLeft: "20px" } },
        events.map((ev) =>
          h("li", {
            key: ev.id,
            style: {
              cursor: "pointer",
              opacity: ev.time <= store.head ? "1" : ".45",
              color: store.faults.some((f) => f.id === ev.id) ? "#f87171" : undefined,
            },
            onclick: () => store.backtrack(ev.time),
          }, `+${ev.time - min}ms ${ev.origin}#${ev.seq} ${describe(ev)}`),
        ),
      ),
    );
  };

  const root = createRoot(container);
  const rerender = (): void => root.render(render());
  rerender();
  const unsubscribe = store.subscribe(rerender);
  return () => {
    unsubscribe();
    root.unmount();
    if (!options.container) container.remove();
  };
}

function button(label: string, onclick: () => void): VNode {
  return h("button", { onclick, style: { font: "inherit", padding: "2px 8px" } }, label);
}

function stepBack<S, E>(store: Store<S, E>): void {
  const events = store.events();
  const before = events.filter((e) => e.time < store.head);
  store.backtrack(before.length ? before[before.length - 1].time : store.earliestRetainedTime);
}

function stepForward<S, E>(store: Store<S, E>): void {
  const after = store.events().find((e) => e.time > store.head);
  if (after) store.backtrack(after.time);
  else store.goLive();
}

function importInto<S, E>(store: Store<S, E>, text: string): void {
  // Replace this store's history in place by receiving every event of the pasted session.
  const Ctor = store.constructor as typeof Store;
  const other = Ctor.importText(store.definition, text);
  for (const ev of other.events()) store.receive(ev);
  if (other.live) store.goLive();
  else store.backtrack(other.head);
}
