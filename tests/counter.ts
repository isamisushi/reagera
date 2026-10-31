import { codec, defineApp, type EventRecord, type Infer } from "../src/index.js";

export const Events = codec.variant({
  add: codec.i64,
  reset: codec.unit,
  rename: codec.struct({ label: codec.string, note: codec.option(codec.string) }),
  explode: codec.unit,
});
export type Event = Infer<typeof Events>;

export interface State {
  readonly count: number;
  readonly label: string;
  readonly applied: string[];
}

export const Counter = defineApp<State, Event>({
  name: "counter",
  events: Events,
  checkpointInterval: 4,
  init: () => ({ count: 0, label: "", applied: [] }),
  apply(state, ev: EventRecord<Event>) {
    const applied = [...state.applied, ev.id];
    switch (ev.body.type) {
      case "add":
        return { ...state, count: state.count + ev.body.payload, applied };
      case "reset":
        return { ...state, count: 0, applied };
      case "rename":
        return { ...state, label: ev.body.payload.label, applied };
      case "explode":
        throw new Error("boom");
    }
  },
});

/** A clock that returns t0, t0+step, t0+2*step, ... */
export function ticking(t0 = 1000, step = 10): () => number {
  let t = t0 - step;
  return () => (t += step);
}
