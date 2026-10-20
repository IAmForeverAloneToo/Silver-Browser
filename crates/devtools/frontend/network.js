// The network log (M36.3): the tab's requests as they are made, filtered
// by type, and one's headers, timing and body.

tool("network", "Network", (pane) => {
  pane.classList.add("network");
  const bar = el("div", "bar");
  const clear = el("button", "", "Clear");
  clear.id = "network-clear";
  bar.appendChild(clear);
  const types = ["all", "document", "style", "script", "image", "fetch", "other"];
  let shownType = "all";
  const typeButtons = {};
  for (const type of types) {
    const button = el("button", type === "all" ? "kind on" : "kind", type);
    button.id = "network-type-" + type;
    typeButtons[type] = button;
    button.addEventListener("click", () => {
      typeButtons[shownType].classList.remove("on");
      shownType = type;
      button.classList.add("on");
      draw();
    });
    bar.appendChild(button);
  }
  const body = el("div", "split");
  const list = el("div", "requests mono");
  list.id = "network-list";
  const detail = el("div", "detail mono");
  detail.id = "network-detail";
  body.appendChild(list);
  body.appendChild(detail);
  pane.appendChild(bar);
  pane.appendChild(body);

  let entries = [];
  let selected = null;

  // Whether a request is of the type shown: "other" is what no button
  // names.
  function shows(entry) {
    if (shownType === "all") {
      return true;
    }
    if (shownType === "other") {
      return !types.includes(entry.type) || entry.type === "other";
    }
    if (shownType === "document") {
      return entry.type === "document" || entry.type === "iframe";
    }
    return entry.type === shownType;
  }

  function name(url) {
    const path = url.split(/[?#]/)[0];
    return path.split("/").pop() || url;
  }

  function draw() {
    list.textContent = "";
    for (const entry of entries) {
      if (!shows(entry)) {
        continue;
      }
      const line = el("div", "request" + (entry.error || entry.status >= 400 ? " failed" : ""));
      line.dataset.id = String(entry.id);
      line.appendChild(el("span", "req-name", name(entry.url)));
      line.appendChild(el("span", "req-status", entry.error ? "(failed)" : entry.status === null ? "(pending)" : String(entry.status)));
      line.appendChild(el("span", "req-type", entry.type));
      line.appendChild(el("span", "req-size", entry.cached ? "(cache)" : entry.size + " B"));
      const time = entry.end !== null ? Math.round(entry.end - entry.start) + " ms" : "";
      line.appendChild(el("span", "req-time", time));
      if (entry.id === selected) {
        line.classList.add("selected");
      }
      line.addEventListener("click", () => open(entry.id));
      list.appendChild(line);
    }
  }

  function section(title, rows) {
    const box = el("div", "section");
    box.appendChild(el("div", "heading", title));
    for (const [name, value] of rows) {
      const line = el("div", "decl");
      line.appendChild(el("span", "name", name));
      line.appendChild(document.createTextNode(": "));
      line.appendChild(el("span", "value", value));
      box.appendChild(line);
    }
    return box;
  }

  function open(id) {
    selected = id;
    draw();
    call("network.entry", { id }).then((entry) => {
      detail.textContent = "";
      detail.appendChild(section("General", [
        ["URL", entry.url],
        ["Method", entry.method],
        ["Status", entry.error ? entry.error : String(entry.status)],
        ["Type", entry.type],
        ["Initiator", entry.initiator || ""],
      ].concat(entry.urls.length > 1 ? [["Redirected from", entry.urls.slice(0, -1).join(" -> ")]] : [])));
      const ms = (t) => (t === null ? "" : Math.round(t - entry.start) + " ms");
      detail.appendChild(section("Timing", [
        ["Response head", ms(entry.head)],
        ["Done", ms(entry.end)],
      ]));
      detail.appendChild(section("Response headers", entry.responseHeaders));
      detail.appendChild(section("Request headers", entry.requestHeaders));
      const content = el("div", "section");
      content.id = "network-body";
      content.appendChild(el("div", "heading", "Body"));
      if (entry.body === null) {
        content.appendChild(el("div", "quiet", "(not shown: the page could not read it)"));
      } else if (!entry.bodyIsText && entry.mime && entry.mime.startsWith("image/")) {
        const img = el("img");
        img.src = "data:" + entry.mime + ";base64," + entry.body;
        content.appendChild(img);
      } else if (entry.bodyIsText) {
        content.appendChild(el("pre", "body-text", entry.body));
      } else {
        content.appendChild(el("div", "quiet", "(binary, " + entry.size + " bytes)"));
      }
      if (entry.bodyCut) {
        content.appendChild(el("div", "quiet", "(the first part of it)"));
      }
      detail.appendChild(content);
    }, (error) => {
      detail.textContent = error.message;
    });
  }

  function load() {
    call("network.list").then((list) => {
      entries = list;
      draw();
    }, () => {});
  }

  clear.addEventListener("click", () => {
    entries = [];
    selected = null;
    detail.textContent = "";
    draw();
    call("network.clear").catch(() => {});
  });

  // The log changed: what it holds is read again, a little later, once.
  let pending = false;
  on("network.updated", () => {
    if (pending) {
      return;
    }
    pending = true;
    setTimeout(() => {
      pending = false;
      load();
    }, 100);
  });

  load();
  return { navigated: load };
});
