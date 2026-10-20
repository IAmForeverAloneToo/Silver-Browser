"use strict";
// The panel's frame (M36): the bridge to the browser, the tabs, and the
// line naming the page inspected. Each tool registers itself below it
// with `tool(name, title, build)`; `build(pane)` fills its pane as the
// panel starts, and what it returns may have `shown()`, called each time
// it is shown, and `navigated()`, called when the page inspected
// changes.
//
// Everything shown of the page is the page's to choose: it is put in as
// text (`textContent`), never as markup.

const bridge = {
  next: 1,
  pending: new Map(),
  events: new Map(),
};

// Sends request `method` with `params`: a promise of its result, which
// rejects with the error the browser gave.
function call(method, params) {
  const id = bridge.next++;
  const message = JSON.stringify({ id, method, params: params || {} });
  return new Promise((resolve, reject) => {
    bridge.pending.set(id, { resolve, reject });
    devtools.send(message);
  });
}

// Calls `handler(params)` for each event `name` the browser sends.
function on(name, handler) {
  if (!bridge.events.has(name)) {
    bridge.events.set(name, []);
  }
  bridge.events.get(name).push(handler);
}

devtools.onmessage = (text) => {
  let message;
  try {
    message = JSON.parse(text);
  } catch (e) {
    return;
  }
  if (typeof message.id === "number") {
    const waiting = bridge.pending.get(message.id);
    if (waiting === undefined) {
      return;
    }
    bridge.pending.delete(message.id);
    if ("error" in message) {
      waiting.reject(new Error(String(message.error)));
    } else {
      waiting.resolve(message.result);
    }
  } else if (typeof message.event === "string") {
    for (const handler of bridge.events.get(message.event) || []) {
      handler(message.params || {});
    }
  }
};

// Makes element `tag` with class `cls` and text `text`, either optional.
function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) {
    node.className = cls;
  }
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

const tools = [];
let shown = null;

function tool(name, title, build) {
  const button = el("button", "", title);
  button.id = "tab-" + name;
  const pane = el("section", "pane off");
  pane.id = "pane-" + name;
  document.getElementById("tabs").appendChild(button);
  document.getElementById("panes").appendChild(pane);
  const entry = { name, button, pane, made: build(pane) || {} };
  button.addEventListener("click", () => show(entry));
  tools.push(entry);
  if (shown === null) {
    show(entry);
  }
}

function show(entry) {
  if (shown !== null) {
    shown.button.classList.remove("on");
    shown.pane.classList.add("off");
  }
  shown = entry;
  entry.button.classList.add("on");
  entry.pane.classList.remove("off");
  if (entry.made.shown) {
    entry.made.shown();
  }
}

// Shows the node of the page numbered `id` in the element inspector,
// selected: what the inspector registers as `revealer`.
let revealer = null;
function reveal(id) {
  for (const entry of tools) {
    if (entry.name === "elements") {
      show(entry);
    }
  }
  if (revealer !== null) {
    revealer(id);
  }
}

// The line naming the page inspected, its URL and title, again each
// time it navigates.
function inspected() {
  call("page.info").then((info) => {
    const line = document.getElementById("inspected");
    line.textContent = (info.title ? info.title + " - " : "") + (info.url || "");
  }, () => {});
}

on("page.navigated", () => {
  inspected();
  for (const entry of tools) {
    if (entry.made.navigated) {
      entry.made.navigated();
    }
  }
});
