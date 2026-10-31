/**
 * Latency lab: two clients share an arena. Each click is an event. Events reach
 * the other client after a random delay, so they often arrive out of order.
 * Both clients keep a continuous-time history, so a late event is inserted at
 * its own time and the picture is corrected. When nothing is in flight, both
 * clients and the reference store agree byte for byte.
 */
import { codec, createStore, createRoot, defineApp, h, Store, type EventRecord, type Infer, type VNode } from "../../src/index.js";

// ----- the app ---------------------------------------------------------------

const Events = codec.variant({
  place: codec.struct({ x: codec.u16, y: codec.u16 }),
  clear: codec.unit,
});
type ArenaEvent = Infer<typeof Events>;

interface Dot { readonly id: string; readonly x: number; readonly y: number; readonly origin: string; readonly time: number; readonly n: number }
interface State { readonly dots: readonly Dot[]; readonly placed: number; readonly cleared: number }

const Arena = defineApp<State, ArenaEvent>({
  name: "latency-lab",
  events: Events,
  checkpointInterval: 16,
  init: () => ({ dots: [], placed: 0, cleared: 0 }),
  apply(state, ev: EventRecord<ArenaEvent>) {
    switch (ev.body.type) {
      case "place": {
        const n = state.placed + 1;
        return { ...state, placed: n, dots: [...state.dots, { id: ev.id, ...ev.body.payload, origin: ev.origin, time: ev.time, n }] };
      }
      case "clear":
        return { ...state, dots: [], cleared: state.cleared + 1 };
    }
  },
});

// ----- the simulation (not part of the app log) -------------------------------

const W = 320;
const H = 220;
const colors: Record<string, string> = { alice: "#2563eb", bob: "#dc2626" };
const t0 = Date.now();
const now = () => Date.now() - t0;

const hash = location.hash.slice(1);
const reference = hash ? Store.importText(Arena, decodeURIComponent(hash), { origin: "ref", clock: now }) : createStore(Arena, { origin: "ref", clock: now });
const clients = {
  alice: createStore(Arena, { origin: "alice", clock: now }),
  bob: createStore(Arena, { origin: "bob", clock: now }),
};
type ClientName = keyof typeof clients;
for (const ev of reference.events()) for (const c of Object.values(clients)) c.receive(ev);

const settings = { latency: 800, jitter: 800, bot: true, paused: false };
const inFlight = new Map<ClientName, { record: EventRecord<ArenaEvent>; dueAt: number }[]>([["alice", []], ["bob", []]]);
const lateIds = new Map<ClientName, Set<string>>([["alice", new Set()], ["bob", new Set()]]);
let scrub: number | undefined; // undefined = live

function emit(from: ClientName, body: ArenaEvent): void {
  const record = clients[from].dispatch(body);
  reference.receive(record);
  const to: ClientName = from === "alice" ? "bob" : "alice";
  const delay = Math.max(0, settings.latency + (Math.random() * 2 - 1) * settings.jitter);
  inFlight.get(to)!.push({ record, dueAt: now() + delay });
  render();
}

function deliverDue(): void {
  for (const [to, queue] of inFlight) {
    const ready = queue.filter((m) => m.dueAt <= now());
    if (!ready.length) continue;
    inFlight.set(to, queue.filter((m) => m.dueAt > now()));
    for (const m of ready) {
      const store = clients[to];
      if (m.record.time < store.latestTime) lateIds.get(to)!.add(m.record.id);
      store.receive(m.record);
    }
    render();
  }
}

function deliverAll(): void {
  for (const q of inFlight.values()) for (const m of q) m.dueAt = 0;
  deliverDue();
}

let botTimer: ReturnType<typeof setInterval> | undefined;
function bot(): void {
  if (!settings.bot || settings.paused) return;
  emit("bob", { type: "place", payload: { x: Math.floor(Math.random() * W), y: Math.floor(Math.random() * H) } });
}
function restartBot(): void {
  if (botTimer) clearInterval(botTimer);
  botTimer = setInterval(bot, 1500);
}

// ----- the view ------------------------------------------------------------

function arena(name: ClientName): VNode {
  const store = clients[name];
  const state = scrub === undefined ? store.state : store.stateAt(scrub);
  const pending = inFlight.get(name)!.length;
  const late = lateIds.get(name)!;
  return h("section", { class: "client" },
    h("header", null,
      h("strong", { style: { color: colors[name] } }, name),
      h("span", null, `${store.events().length} events`),
      pending ? h("span", { class: "pending" }, `${pending} in flight`) : h("span", { class: "ok" }, "nothing in flight"),
      h("span", null, `${late.size} arrived late`),
    ),
    h("svg", {
      viewBox: `0 0 ${W} ${H}`, width: W, height: H, class: "arena",
      onclick: (e: MouseEvent) => {
        const rect = (e.currentTarget as SVGSVGElement).getBoundingClientRect();
        const x = Math.round(((e.clientX - rect.left) / rect.width) * W);
        const y = Math.round(((e.clientY - rect.top) / rect.height) * H);
        emit(name, { type: "place", payload: { x, y } });
      },
    },
      state.dots.map((d) => h("g", { key: d.id },
        h("circle", { cx: d.x, cy: d.y, r: 9, fill: colors[d.origin] ?? "#6b7280", stroke: late.has(d.id) ? "#f59e0b" : "none", "stroke-width": 3 }),
        h("text", { x: d.x, y: d.y + 4, "text-anchor": "middle", "font-size": 10, fill: "#fff" }, d.n),
      )),
    ),
    h("ol", { class: "log" },
      store.events().slice(-8).map((ev) => h("li", {
        key: ev.id,
        class: (late.has(ev.id) ? "late " : "") + (scrub !== undefined && ev.time > scrub ? "future" : ""),
      }, `+${(ev.time / 1000).toFixed(2)}s ${ev.origin} ${ev.body.type} ${ev.body.type === "place" ? `${ev.body.payload.x},${ev.body.payload.y}` : ""}`)),
    ),
  );
}

function converged(): boolean {
  const a = JSON.stringify(clients.alice.state);
  return a === JSON.stringify(clients.bob.state) && a === JSON.stringify(reference.state);
}

function slider(label: string, key: "latency" | "jitter", max: number): VNode {
  return h("label", null, `${label} ${settings[key]}ms`,
    h("input", { type: "range", min: 0, max, step: 50, value: settings[key], oninput: (e: Event) => { settings[key] = Number((e.target as HTMLInputElement).value); render(); } }));
}

function view(): VNode {
  const latest = reference.latestTime === -Infinity ? 0 : reference.latestTime;
  const inflight = [...inFlight.values()].reduce((n, q) => n + q.length, 0);
  return h("main", null,
    h("h1", null, "Latency lab"),
    h("p", { class: "hint" }, "Click either arena to place a dot. Dots travel to the other client with the delay below. ",
      "An orange ring marks a dot that arrived late and was inserted behind newer ones. ",
      "Number labels show the order each client's history settles on. Both agree when nothing is in flight."),
    h("div", { class: "controls" },
      slider("latency", "latency", 3000),
      slider("jitter", "jitter", 3000),
      h("label", null, h("input", { type: "checkbox", checked: settings.bot, onchange: () => { settings.bot = !settings.bot; render(); } }), " bob clicks on his own"),
      h("button", { onclick: deliverAll }, "deliver everything now"),
      h("button", { onclick: () => emit("alice", { type: "clear", payload: undefined }) }, "clear (as alice)"),
      h("button", { onclick: () => { location.hash = encodeURIComponent(reference.exportText()); render(); } }, "put session in URL"),
    ),
    h("div", { class: "status " + (converged() ? "ok" : "pending") },
      converged() ? "alice, bob and the reference store agree" : `histories differ: ${inflight} event(s) still in flight`),
    h("label", { class: "scrub" }, scrub === undefined ? "live" : `showing +${(scrub / 1000).toFixed(2)}s`,
      h("input", { type: "range", min: 0, max: Math.max(latest, 1), step: 10, value: scrub ?? latest,
        oninput: (e: Event) => { scrub = Number((e.target as HTMLInputElement).value); render(); } }),
      h("button", { onclick: () => { scrub = undefined; render(); } }, "live")),
    h("div", { class: "clients" }, arena("alice"), arena("bob")),
    h("p", { class: "hint" }, `reference store: ${reference.events().length} events, ${reference.faults.length} faults. `,
      "The scrubber queries stateAt(t) on each client without moving its head."),
  );
}

const root = createRoot(document.getElementById("root")!);
let scheduled = false;
function render(): void {
  if (scheduled) return;
  scheduled = true;
  requestAnimationFrame(() => { scheduled = false; root.render(view()); });
}

setInterval(deliverDue, 50);
restartBot();
render();
(window as unknown as Record<string, unknown>).lab = { clients, reference, settings };
