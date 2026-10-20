// The console (M36.1): the page's entries as they come, each argument a
// preview that opens; errors with their stacks; groups; filters by
// level and by text; and a line that evaluates in the page.

// A preview (docs/design/devtools.md, "The console") as a node: a
// primitive as itself, an object as its class and its first properties,
// which a click opens into a tree of them.
function previewNode(p, top) {
  switch (p.type) {
    case "undefined":
    case "null":
      return el("span", "v-null", p.type);
    case "boolean":
      return el("span", "v-bool", String(p.value));
    case "number":
      return el("span", "v-number", "value" in p ? String(p.value) : p.description);
    case "bigint":
    case "symbol":
    case "date":
    case "regexp":
      return el("span", "v-" + p.type, p.description);
    case "string":
      return el("span", top ? "v-text" : "v-string", top ? p.value : JSON.stringify(p.value));
    case "function":
      return el("span", "v-function", "f " + (p.name || "anonymous") + "()");
    case "accessor":
      return el("span", "quiet", "(...)");
    case "error":
      return errorNode(p);
    case "node":
      return nodeNode(p);
  }
  return objectNode(p);
}

// A node as the inspector names it: <tag#id.class>.
function nodeNode(p) {
  let text;
  if (p.nodeType === 1) {
    text = "<" + p.tag + (p.id ? "#" + p.id : "") +
      (p.classes ? "." + p.classes.trim().split(/\s+/).join(".") : "") + ">";
  } else {
    text = p.name;
  }
  const node = el("span", "v-node", text);
  node.dataset.node = String(p.node);
  return node;
}

function errorNode(p) {
  const box = el("span", "v-error");
  box.appendChild(el("span", "", p.stack ? p.class + ": " + p.message : p.class + ": " + p.message));
  if (p.stack) {
    box.appendChild(el("div", "stack mono", p.stack));
  }
  return box;
}

// A short summary: `Class {a: 1, b: "x"}` or `Array(3) [1, 2, 3]`.
function summary(p) {
  if (p.circular) {
    return p.class + " [circular]";
  }
  if (p.collapsed) {
    return p.class;
  }
  const parts = [];
  if (p.type === "map" || p.type === "set") {
    for (const entry of p.entries.slice(0, 5)) {
      parts.push(p.type === "map" ? brief(entry[0]) + " => " + brief(entry[1]) : brief(entry));
    }
    return p.class + "(" + p.size + ") {" + parts.join(", ") + (p.more || p.entries.length > 5 ? ", ..." : "") + "}";
  }
  const props = p.properties || [];
  for (const [name, value] of props.slice(0, 5)) {
    parts.push(p.type === "array" && /^\d+$/.test(name) ? brief(value) : name + ": " + brief(value));
  }
  const more = p.more || props.length > 5 ? ", ..." : "";
  if (p.type === "array") {
    return (p.class === "Array" ? "" : p.class) + "(" + p.length + ") [" + parts.join(", ") + more + "]";
  }
  return (p.class === "Object" ? "" : p.class + " ") + "{" + parts.join(", ") + more + "}";
}

// A preview in a word or two, inside a summary.
function brief(p) {
  switch (p.type) {
    case "string":
      return JSON.stringify(p.value.length > 30 ? p.value.slice(0, 30) + "..." : p.value);
    case "number":
      return "value" in p ? String(p.value) : p.description;
    case "boolean":
      return String(p.value);
    case "undefined":
    case "null":
      return p.type;
    case "function":
      return "f";
    case "node":
      return p.nodeType === 1 ? p.tag : p.name;
    case "array":
      return "Array(" + p.length + ")";
    case "error":
      return p.class;
    case "accessor":
      return "(...)";
  }
  return p.description || p.class || p.type;
}

function objectNode(p) {
  const box = el("span", "v-object");
  const head = el("span", "toggle", summary(p));
  box.appendChild(head);
  let body = null;
  head.addEventListener("click", () => {
    if (body !== null) {
      body.remove();
      body = null;
      head.classList.remove("open");
      return;
    }
    head.classList.add("open");
    body = el("div", "tree");
    const rows = p.type === "map"
      ? (p.entries || []).map((e) => [brief(e[0]), e[1]])
      : p.type === "set"
        ? (p.entries || []).map((e, i) => [String(i), e])
        : p.properties || [];
    for (const [name, value] of rows) {
      const row = el("div", "prop");
      row.appendChild(el("span", "name", name + ": "));
      row.appendChild(previewNode(value, false));
      body.appendChild(row);
    }
    if (p.more) {
      body.appendChild(el("div", "quiet", "..."));
    }
    if (p.collapsed) {
      body.appendChild(el("div", "quiet", "(deeper than the snapshot)"));
    }
    box.appendChild(body);
  });
  return box;
}

// console.table's preview as a table: its rows' properties as columns.
function tableNode(p) {
  if (!p || !p.properties) {
    return previewNode(p, true);
  }
  const columns = [];
  for (const [, row] of p.properties) {
    for (const [name] of row.properties || []) {
      if (!columns.includes(name)) {
        columns.push(name);
      }
    }
  }
  const table = el("table", "table");
  const head = el("tr");
  head.appendChild(el("th", "", "(index)"));
  for (const name of columns) {
    head.appendChild(el("th", "", name));
  }
  table.appendChild(head);
  for (const [index, row] of p.properties) {
    const tr = el("tr");
    tr.appendChild(el("td", "", index));
    for (const name of columns) {
      const cell = el("td");
      const found = (row.properties || []).find(([n]) => n === name);
      if (found) {
        cell.appendChild(previewNode(found[1], false));
      } else if (columns.length === 0 || !row.properties) {
        cell.appendChild(previewNode(row, false));
      }
      tr.appendChild(cell);
    }
    if (columns.length === 0) {
      const cell = el("td");
      cell.appendChild(previewNode(row, false));
      tr.appendChild(cell);
    }
    table.appendChild(tr);
  }
  return table;
}

// The last part of a URL's path, with the line: where an entry is from.
function place(entry) {
  if (!entry.url) {
    return "";
  }
  const name = entry.url.split(/[?#]/)[0].split("/").pop() || entry.url;
  return name + ":" + entry.line;
}

tool("console", "Console", (pane) => {
  const bar = el("div", "bar");
  const clear = el("button", "", "Clear");
  clear.id = "console-clear";
  const filter = el("input");
  filter.id = "console-filter";
  filter.placeholder = "Filter";
  bar.appendChild(clear);
  bar.appendChild(filter);
  const levels = {};
  for (const level of ["error", "warn", "info", "log", "debug"]) {
    const button = el("button", "level on", level);
    button.id = "console-level-" + level;
    levels[level] = true;
    button.addEventListener("click", () => {
      levels[level] = !levels[level];
      button.classList.toggle("on", levels[level]);
      refilter();
    });
    bar.appendChild(button);
  }
  const list = el("div", "lines mono");
  list.id = "console-lines";
  const prompt = el("input", "prompt mono");
  prompt.id = "console-prompt";
  pane.appendChild(bar);
  pane.appendChild(list);
  pane.appendChild(prompt);
  pane.classList.add("console");

  // The groups open, innermost last: where the next entry goes.
  let groups = [list];

  function shows(line) {
    const text = filter.value.toLowerCase();
    return levels[line.dataset.level] && (!text || line.textContent.toLowerCase().includes(text));
  }

  function refilter() {
    for (const line of list.querySelectorAll(".entry")) {
      line.classList.toggle("hidden", !shows(line));
    }
  }

  function add(entry) {
    if (entry.kind === "clear") {
      list.textContent = "";
      groups = [list];
      list.appendChild(el("div", "entry quiet", "Console was cleared"));
      return;
    }
    if (entry.kind === "groupEnd") {
      if (groups.length > 1) {
        groups.pop();
      }
      return;
    }
    const line = el("div", "entry " + entry.level + " source-" + entry.source);
    line.dataset.level = entry.level;
    const where = place(entry);
    if (where) {
      line.appendChild(el("span", "place quiet", where));
    }
    const args = entry.args || [];
    if (entry.kind === "table" && args.length > 0) {
      line.appendChild(tableNode(args[0]));
    } else if (args.length > 0 && entry.kind !== "dir") {
      args.forEach((p, i) => {
        if (i > 0) {
          line.appendChild(document.createTextNode(" "));
        }
        line.appendChild(previewNode(p, true));
      });
    } else if (args.length > 0) {
      line.appendChild(previewNode(args[0], false));
    } else {
      line.appendChild(el("span", "v-text", entry.text));
    }
    if (entry.stack && entry.stack.length > 0 && (entry.level === "error" || entry.kind === "trace")) {
      const stack = el("div", "stack");
      for (const frame of entry.stack) {
        stack.appendChild(el("div", "", "at " + (frame.function || "(anonymous)") + " (" +
          frame.url + ":" + frame.line + ":" + frame.column + ")"));
      }
      line.appendChild(stack);
    }
    line.classList.toggle("hidden", !shows(line));
    const into = groups[groups.length - 1];
    into.appendChild(line);
    if (entry.kind === "group" || entry.kind === "groupCollapsed") {
      line.classList.add("group-head");
      const inner = el("div", "group");
      if (entry.kind === "groupCollapsed") {
        inner.classList.add("closed");
      }
      line.addEventListener("click", () => inner.classList.toggle("closed"));
      into.appendChild(inner);
      groups.push(inner);
    }
    list.scrollTop = list.scrollHeight;
  }

  function reset() {
    list.textContent = "";
    groups = [list];
  }

  clear.addEventListener("click", () => {
    reset();
    call("console.clear").catch(() => {});
  });
  filter.addEventListener("input", refilter);
  prompt.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || prompt.value === "") {
      return;
    }
    const expression = prompt.value;
    prompt.value = "";
    const asked = el("div", "entry input", "> " + expression);
    asked.dataset.level = "log";
    list.appendChild(asked);
    call("console.evaluate", { expression }).then((answer) => {
      const line = el("div", answer.exception ? "entry error result" : "entry result");
      line.dataset.level = answer.exception ? "error" : "log";
      if (answer.exception) {
        line.appendChild(el("span", "", "Uncaught "));
      }
      line.appendChild(previewNode(answer.exception || answer.result, false));
      list.appendChild(line);
      list.scrollTop = list.scrollHeight;
    }, (error) => {
      const line = el("div", "entry error result", String(error.message));
      line.dataset.level = "error";
      list.appendChild(line);
    });
  });

  // The entries told before the history came are in it: they are left
  // to it, and those told after it are added as they come.
  let loaded = false;
  on("console.entry", (entry) => {
    if (loaded) {
      add(entry);
    }
  });
  function load() {
    reset();
    loaded = false;
    call("console.history").then((entries) => {
      entries.forEach(add);
      loaded = true;
    }, () => {});
  }
  load();
  return { navigated: load };
});
