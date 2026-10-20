// The element inspector (M36.2): the page's tree, opened as the user
// opens it and kept up with the page; the node selected highlighted in
// the page; its rules that matched, overridden declarations struck out;
// its computed values; its attributes and its style attribute edited;
// and a pick of the node under the pointer.

tool("elements", "Elements", (pane) => {
  pane.classList.add("elements");
  const bar = el("div", "bar");
  const pick = el("button", "", "Pick");
  pick.id = "elements-pick";
  bar.appendChild(pick);
  const tree = el("div", "dom mono");
  tree.id = "elements-tree";
  const side = el("div", "side");
  const styles = el("div", "styles mono");
  styles.id = "elements-styles";
  const computed = el("div", "computed mono");
  computed.id = "elements-computed";
  side.appendChild(el("div", "heading", "Styles"));
  side.appendChild(styles);
  side.appendChild(el("div", "heading", "Computed"));
  side.appendChild(computed);
  const body = el("div", "split");
  body.appendChild(tree);
  body.appendChild(side);
  pane.appendChild(bar);
  pane.appendChild(body);

  // The rows shown, by node, and the nodes opened.
  const rows = new Map();
  const opened = new Set();
  let selected = null;

  // A row of `node` (the protocol's node, docs/design/devtools.md),
  // `depth` deep, with its children's rows under it if it is open.
  function row(node, depth) {
    // A node given with its children is open, as the page opened it.
    if (node.children) {
      opened.add(node.id);
    }
    const box = el("div", "node");
    const line = el("div", "line");
    line.style.paddingLeft = depth * 12 + "px";
    line.dataset.node = String(node.id);
    const open = node.childCount > 0;
    const toggle = el("span", open ? (opened.has(node.id) ? "toggle open" : "toggle") : "leaf");
    line.appendChild(toggle);
    if (node.type === 1) {
      line.appendChild(el("span", "tag", "<" + node.name));
      for (const [name, value] of node.attributes) {
        line.appendChild(document.createTextNode(" "));
        const attr = el("span", "attr");
        attr.appendChild(el("span", "attr-name", name));
        attr.appendChild(document.createTextNode("=\""));
        attr.appendChild(el("span", "attr-value", value));
        attr.appendChild(document.createTextNode("\""));
        attr.addEventListener("dblclick", (event) => {
          event.stopPropagation();
          editAttribute(node, name, value, attr);
        });
        line.appendChild(attr);
      }
      line.appendChild(el("span", "tag", ">"));
    } else if (node.type === 3) {
      line.appendChild(el("span", "text", JSON.stringify(node.text)));
    } else if (node.type === 8) {
      line.appendChild(el("span", "comment", "<!--" + node.text + "-->"));
    } else {
      line.appendChild(el("span", "quiet", node.name));
    }
    line.addEventListener("click", () => select(node.id));
    line.addEventListener("mouseenter", () => call("dom.highlight", { node: node.id }).catch(() => {}));
    toggle.addEventListener("click", (event) => {
      event.stopPropagation();
      if (opened.has(node.id)) {
        opened.delete(node.id);
      } else {
        opened.add(node.id);
      }
      refresh(node.id);
    });
    box.appendChild(line);
    const kids = el("div", "kids");
    box.appendChild(kids);
    rows.set(node.id, { box, line, kids, depth });
    if (selected === node.id) {
      line.classList.add("selected");
    }
    if (opened.has(node.id) && node.children) {
      for (const child of node.children) {
        kids.appendChild(row(child, depth + 1));
      }
    }
    return box;
  }

  // Asks for `id`'s subtree again, as deep as it is open, and shows it.
  function refresh(id) {
    const at = rows.get(id);
    if (!at) {
      return;
    }
    call("dom.tree", { node: id, open: [...opened] }).then((node) => {
      const now = rows.get(id);
      if (now) {
        now.box.replaceWith(row(node, now.depth));
      }
    }, () => {});
  }

  function load() {
    tree.textContent = "";
    rows.clear();
    call("dom.tree", { open: [...opened] }).then((node) => {
      opened.add(node.id);
      tree.textContent = "";
      tree.appendChild(row(node, 0));
    }, () => {});
  }

  function editAttribute(node, name, value, span) {
    const input = el("input", "edit");
    input.value = value;
    span.replaceWith(input);
    input.focus();
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        const params = { node: node.id, name, value: input.value };
        call("dom.setAttribute", params).then(() => refresh(node.id), () => refresh(node.id));
      } else if (event.key === "Escape") {
        refresh(node.id);
      }
    });
  }

  // Selects node `id`: its row marked, the page's node highlighted and
  // made `$0`, and its styles shown.
  function select(id) {
    if (selected !== null && rows.has(selected)) {
      rows.get(selected).line.classList.remove("selected");
    }
    selected = id;
    if (rows.has(id)) {
      rows.get(id).line.classList.add("selected");
    }
    call("dom.select", { node: id }).catch(() => {});
    showStyles(id);
  }

  function showStyles(id) {
    call("css.matched", { node: id }).then((matched) => {
      styles.textContent = "";
      styles.appendChild(inlineRule(id, matched.inline));
      for (const rule of matched.rules) {
        styles.appendChild(ruleNode(rule));
      }
    }, (error) => {
      styles.textContent = error.message;
    });
    call("css.computed", { node: id }).then((values) => {
      computed.textContent = "";
      for (const [name, value] of values) {
        const line = el("div", "decl");
        line.appendChild(el("span", "name", name));
        line.appendChild(document.createTextNode(": "));
        line.appendChild(el("span", "value", value));
        computed.appendChild(line);
      }
    }, () => {
      computed.textContent = "";
    });
  }

  function declaration(decl) {
    const line = el("div", decl.overridden ? "decl overridden" : "decl");
    line.appendChild(el("span", "name", decl.name));
    line.appendChild(document.createTextNode(": "));
    line.appendChild(el("span", "value", decl.value + (decl.important ? " !important" : "")));
    line.appendChild(document.createTextNode(";"));
    return line;
  }

  function ruleNode(rule) {
    const box = el("div", "rule");
    const head = el("div", "rule-head");
    head.appendChild(el("span", "source quiet", rule.source));
    for (const condition of rule.conditions) {
      box.appendChild(el("div", "condition quiet", condition));
    }
    head.appendChild(el("span", "selector", rule.selector + " {"));
    box.appendChild(head);
    for (const decl of rule.declarations) {
      box.appendChild(declaration(decl));
    }
    box.appendChild(el("div", "", "}"));
    return box;
  }

  // The element's style attribute: its declarations, each toggled by
  // its box, and a line that adds one.
  function inlineRule(id, inline) {
    const box = el("div", "rule inline");
    box.appendChild(el("div", "rule-head", "element.style {"));
    const decls = inline ? inline.declarations : [];
    const write = (list) => {
      const text = list
        .filter((d) => d.on)
        .map((d) => d.name + ": " + d.value + (d.important ? " !important" : ""))
        .join("; ");
      call("css.setStyle", { node: id, text }).then(() => showStyles(id), () => showStyles(id));
    };
    const state = decls.map((d) => ({ name: d.name, value: d.value, important: d.important, on: true }));
    state.forEach((d, i) => {
      const line = declaration(decls[i]);
      const check = el("input");
      check.type = "checkbox";
      check.checked = true;
      check.className = "check";
      check.addEventListener("change", () => {
        d.on = check.checked;
        write(state);
      });
      line.insertBefore(check, line.firstChild);
      box.appendChild(line);
    });
    const add = el("input", "add");
    add.id = "elements-add-style";
    add.placeholder = "name: value";
    add.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") {
        return;
      }
      const at = add.value.indexOf(":");
      if (at > 0) {
        state.push({
          name: add.value.slice(0, at).trim(),
          value: add.value.slice(at + 1).trim(),
          important: false,
          on: true,
        });
        write(state);
      }
    });
    box.appendChild(add);
    box.appendChild(el("div", "", "}"));
    return box;
  }

  pick.addEventListener("click", () => {
    const on = !pick.classList.contains("on");
    pick.classList.toggle("on", on);
    call("dom.pick", { on }).catch(() => {});
  });

  // The node picked in the page: the tree opened down to it, selected.
  on("dom.picked", (params) => {
    pick.classList.remove("on");
    for (const id of params.path) {
      opened.add(id);
    }
    call("dom.tree", { open: [...opened] }).then((node) => {
      tree.textContent = "";
      rows.clear();
      tree.appendChild(row(node, 0));
      select(params.node);
      const at = rows.get(params.node);
      if (at) {
        at.line.scrollIntoView();
      }
    }, () => {});
  });

  // The page's tree changed: what is shown of it is asked for again.
  let pending = false;
  on("dom.updated", () => {
    if (pending) {
      return;
    }
    pending = true;
    setTimeout(() => {
      pending = false;
      call("dom.tree", { open: [...opened] }).then((node) => {
        tree.textContent = "";
        rows.clear();
        tree.appendChild(row(node, 0));
        if (selected !== null && rows.has(selected)) {
          showStyles(selected);
        }
      }, () => {});
    }, 100);
  });

  load();
  return {
    navigated() {
      opened.clear();
      selected = null;
      styles.textContent = "";
      computed.textContent = "";
      load();
    },
  };
});
