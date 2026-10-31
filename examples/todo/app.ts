import { codec, createStore, defineApp, h, mount, Store, type EventRecord, type Infer } from "../../src/index.js";
import { devtools } from "../../src/devtools.js";

// 1. The wire format of every event. This is the whole contract of the app.
const Events = codec.variant({
  add: codec.struct({ id: codec.u32, title: codec.string }),
  toggle: codec.u32,
  remove: codec.u32,
  edit: codec.struct({ id: codec.u32, title: codec.string }),
  clearDone: codec.unit,
  filter: codec.u8, // 0 all, 1 active, 2 done
});
type Event = Infer<typeof Events>;

// 2. The state and how each event changes it. Pure, immutable, replayable.
interface Todo {
  readonly id: number;
  readonly title: string;
  readonly done: boolean;
  readonly createdAt: number;
}
interface State {
  readonly todos: readonly Todo[];
  readonly filter: 0 | 1 | 2;
  readonly nextId: number;
}

const Todos = defineApp<State, Event>({
  name: "todo",
  events: Events,
  init: () => ({ todos: [], filter: 0, nextId: 1 }),
  apply(state, ev: EventRecord<Event>) {
    const { type, payload } = ev.body;
    switch (type) {
      case "add":
        return {
          ...state,
          nextId: Math.max(state.nextId, payload.id + 1),
          todos: [...state.todos, { id: payload.id, title: payload.title, done: false, createdAt: ev.time }],
        };
      case "toggle":
        return { ...state, todos: state.todos.map((t) => (t.id === payload ? { ...t, done: !t.done } : t)) };
      case "remove":
        return { ...state, todos: state.todos.filter((t) => t.id !== payload) };
      case "edit":
        return { ...state, todos: state.todos.map((t) => (t.id === payload.id ? { ...t, title: payload.title } : t)) };
      case "clearDone":
        return { ...state, todos: state.todos.filter((t) => !t.done) };
      case "filter":
        return { ...state, filter: payload as 0 | 1 | 2 };
    }
  },
});

// 3. The view: a pure function of state. Local input text lives in the DOM only.
const view = (state: State, dispatch: (e: Event) => void, store: Store<State, Event>) => {
  const visible = state.todos.filter((t) => (state.filter === 0 ? true : state.filter === 1 ? !t.done : t.done));
  const left = state.todos.filter((t) => !t.done).length;
  return h("main", { class: "app" },
    h("h1", null, "todos"),
    h("form", {
      onsubmit: (e: Event & { preventDefault(): void; target: HTMLFormElement }) => {
        e.preventDefault();
        const input = e.target.elements.namedItem("title") as HTMLInputElement;
        const title = input.value.trim();
        if (!title) return;
        dispatch({ type: "add", payload: { id: state.nextId, title } });
        input.value = "";
      },
    },
      h("input", { name: "title", placeholder: "What needs to be done?", autofocus: true, autocomplete: "off" }),
    ),
    h("ul", { class: "list" },
      visible.map((t) =>
        h("li", { key: t.id, class: t.done ? "done" : "" },
          h("input", { type: "checkbox", checked: t.done, onchange: () => dispatch({ type: "toggle", payload: t.id }) }),
          h("span", {
            ondblclick: () => {
              const title = window.prompt("Edit", t.title);
              if (title && title !== t.title) dispatch({ type: "edit", payload: { id: t.id, title } });
            },
          }, t.title),
          h("button", { class: "x", onclick: () => dispatch({ type: "remove", payload: t.id }) }, "×"),
        ),
      ),
    ),
    h("footer", null,
      h("span", null, `${left} left`),
      [["all", 0], ["active", 1], ["done", 2]].map(([label, f]) =>
        h("button", { class: state.filter === f ? "on" : "", onclick: () => dispatch({ type: "filter", payload: f as number }) }, label as string),
      ),
      h("button", { onclick: () => dispatch({ type: "clearDone", payload: undefined }) }, "clear done"),
      store.live ? null : h("em", null, " viewing the past "),
    ),
  );
};

// 4. Wire it up. A session in the URL hash is replayed instead of starting fresh.
const hash = location.hash.slice(1);
const store = hash ? Store.importText(Todos, decodeURIComponent(hash)) : createStore(Todos);
mount(store, view, document.getElementById("root")!);
devtools(store, { describe: (ev) => `${ev.body.type} ${JSON.stringify(ev.body.payload) ?? ""}` });

// Keep the hash in sync so a reload, or a copied URL, reproduces this exact session.
store.subscribe(() => {
  history.replaceState(null, "", "#" + encodeURIComponent(store.exportText()));
});
(window as unknown as Record<string, unknown>).store = store;
