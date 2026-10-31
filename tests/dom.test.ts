import { test } from "node:test";
import assert from "node:assert/strict";
import { createRoot, h } from "../src/index.js";

// A very small DOM stand-in so the diff algorithm can be exercised in Node.
class FakeNode {
  childNodes: FakeNode[] = [];
  parent: FakeNode | undefined;
  attrs = new Map<string, string>();
  listeners = new Map<string, Function[]>();
  className = "";
  style: Record<string, string> = {};
  value: unknown;
  constructor(public tag: string, public text = "") {}
  get textContent(): string {
    return this.tag === "#text" ? this.text : this.childNodes.map((c) => c.textContent).join("");
  }
  set textContent(v: string) {
    if (this.tag === "#text") this.text = v;
    else this.childNodes = v === "" ? [] : [new FakeNode("#text", v)];
  }
  appendChild(n: FakeNode): void { n.parent = this; this.childNodes.push(n); }
  removeChild(n: FakeNode): void { this.childNodes = this.childNodes.filter((c) => c !== n); }
  replaceChild(n: FakeNode, old: FakeNode): void { this.childNodes[this.childNodes.indexOf(old)] = n; n.parent = this; }
  setAttribute(k: string, v: string): void { this.attrs.set(k, v); }
  removeAttribute(k: string): void { this.attrs.delete(k); }
  addEventListener(k: string, f: Function): void { this.listeners.set(k, [...(this.listeners.get(k) ?? []), f]); }
  removeEventListener(k: string, f: Function): void { this.listeners.set(k, (this.listeners.get(k) ?? []).filter((x) => x !== f)); }
  html(): string {
    if (this.tag === "#text") return this.text;
    const attrs = [...this.attrs].map(([k, v]) => ` ${k}="${v}"`).join("") + (this.className ? ` class="${this.className}"` : "");
    return `<${this.tag}${attrs}>${this.childNodes.map((c) => c.html()).join("")}</${this.tag}>`;
  }
}
(globalThis as Record<string, unknown>).document = {
  createElement: (tag: string) => new FakeNode(tag),
  createTextNode: (text: string) => new FakeNode("#text", text),
};

test("createRoot diffs attributes, children and listeners", () => {
  const container = new FakeNode("div");
  const root = createRoot(container as unknown as Element);
  const clicks: string[] = [];
  root.render(h("ul", { id: "list" }, h("li", { class: "a", onclick: () => clicks.push("first") }, "one"), h("li", null, "two")));
  assert.equal(container.html(), '<div><ul id="list"><li class="a">one</li><li>two</li></ul></div>');
  const ul = container.childNodes[0];
  const li = ul.childNodes[0];
  li.listeners.get("click")![0]();
  root.render(h("ul", { id: "list", title: "t" }, h("li", { class: "b" }, "uno"), h("li", null, "two"), h("li", null, 3)));
  assert.equal(container.html(), '<div><ul id="list" title="t"><li class="b">uno</li><li>two</li><li>3</li></ul></div>');
  assert.equal(ul.childNodes[0], li, "same node is reused");
  assert.equal(li.listeners.get("click")!.length, 0, "stale listener removed");
  root.render(h("p", null, "bye"));
  assert.equal(container.html(), "<div><p>bye</p></div>");
  assert.deepEqual(clicks, ["first"]);
  root.unmount();
  assert.equal(container.html(), "<div></div>");
});

test("h flattens arrays and drops null, undefined and booleans", () => {
  const v = h("div", null, [1, null, ["x", false]], undefined, true, "y");
  assert.deepEqual(v.children, ["1", "x", "y"]);
});
