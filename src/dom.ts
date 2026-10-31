/**
 * A small virtual DOM. The view is a pure function of state; every re-render
 * is a diff against the previous tree. No components with local state: state
 * that is not in the log cannot be replayed, so there is none.
 */
import type { Store } from "./store.js";

export type Props = Record<string, unknown>;
export type Child = VNode | string | number | boolean | null | undefined | Child[];

export interface VNode {
  readonly tag: string;
  readonly props: Props;
  readonly children: (VNode | string)[];
  readonly key?: string | number;
}

export function h(tag: string, props: Props | null = null, ...children: Child[]): VNode {
  const flat: (VNode | string)[] = [];
  const push = (c: Child): void => {
    if (Array.isArray(c)) c.forEach(push);
    else if (c === null || c === undefined || c === false || c === true) return;
    else if (typeof c === "object") flat.push(c);
    else flat.push(String(c));
  };
  children.forEach(push);
  const p = props ?? {};
  const key = p.key as string | number | undefined;
  return { tag, props: p, children: flat, key };
}

type Rendered = { node: Node; vnode: VNode | string };

function create(vnode: VNode | string): Node {
  if (typeof vnode === "string") return document.createTextNode(vnode);
  const el = document.createElement(vnode.tag);
  for (const [k, v] of Object.entries(vnode.props)) setProp(el, k, v, undefined);
  for (const child of vnode.children) el.appendChild(create(child));
  return el;
}

function setProp(el: HTMLElement, name: string, value: unknown, previous: unknown): void {
  if (name === "key" || value === previous) return;
  if (name.startsWith("on") && (typeof value === "function" || typeof previous === "function")) {
    const eventName = name.slice(2).toLowerCase();
    if (typeof previous === "function") el.removeEventListener(eventName, previous as EventListener);
    if (typeof value === "function") el.addEventListener(eventName, value as EventListener);
    return;
  }
  if (name === "value" || name === "checked" || name === "selected" || name === "disabled") {
    (el as unknown as Record<string, unknown>)[name] = value;
    return;
  }
  if (name === "class" || name === "className") {
    el.className = value == null ? "" : String(value);
    return;
  }
  if (name === "style" && typeof value === "object" && value !== null) {
    el.removeAttribute("style");
    Object.assign(el.style, value);
    return;
  }
  if (value === null || value === undefined || value === false) el.removeAttribute(name);
  else el.setAttribute(name, value === true ? "" : String(value));
}

function patch(parent: Node, current: Rendered | undefined, next: VNode | string): Rendered {
  if (!current) {
    const node = create(next);
    parent.appendChild(node);
    return { node, vnode: next };
  }
  const prev = current.vnode;
  const sameKind =
    typeof prev === typeof next &&
    (typeof prev === "string" || (typeof next !== "string" && prev.tag === next.tag && prev.key === next.key));
  if (!sameKind) {
    const node = create(next);
    parent.replaceChild(node, current.node);
    return { node, vnode: next };
  }
  if (typeof prev === "string" || typeof next === "string") {
    if (prev !== next) current.node.textContent = next as string;
    return { node: current.node, vnode: next };
  }
  const el = current.node as HTMLElement;
  for (const k of Object.keys(prev.props)) {
    if (!(k in next.props)) setProp(el, k, undefined, prev.props[k]);
  }
  for (const [k, v] of Object.entries(next.props)) setProp(el, k, v, prev.props[k]);
  patchChildren(el, prev.children, next.children);
  return { node: el, vnode: next };
}

function patchChildren(el: Node, prev: (VNode | string)[], next: (VNode | string)[]): void {
  const nodes = Array.from(el.childNodes);
  const n = Math.min(prev.length, next.length);
  for (let i = 0; i < n; i++) patch(el, { node: nodes[i], vnode: prev[i] }, next[i]);
  for (let i = n; i < next.length; i++) el.appendChild(create(next[i]));
  for (let i = prev.length - 1; i >= n; i--) el.removeChild(nodes[i]);
}

export type View<S, E> = (state: S, dispatch: (body: E) => void, store: Store<S, E>) => VNode;

export interface Root {
  /** Diff the new tree against the previous one and apply the changes. */
  render(tree: VNode): void;
  /** Empty the container. */
  unmount(): void;
}

/** A render target that keeps the previous tree for diffing. */
export function createRoot(container: Element): Root {
  let current: Rendered | undefined;
  container.textContent = "";
  return {
    render(tree) {
      current = patch(container, current, tree);
    },
    unmount() {
      container.textContent = "";
      current = undefined;
    },
  };
}

/**
 * Render the view whenever the store changes. Returns an unmount function.
 * Renders are batched to one per animation frame when available.
 */
export function mount<S, E>(store: Store<S, E>, view: View<S, E>, container: Element): () => void {
  const root = createRoot(container);
  let scheduled = false;
  const dispatch = (body: E): void => {
    store.dispatch(body);
  };
  const render = (): void => {
    scheduled = false;
    root.render(view(store.state, dispatch, store));
  };
  const schedule = (): void => {
    if (scheduled) return;
    scheduled = true;
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(render);
    else queueMicrotask(render);
  };
  render();
  const unsubscribe = store.subscribe(schedule);
  return () => {
    unsubscribe();
    root.unmount();
  };
}
