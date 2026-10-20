// What a site keeps (M36.8): the page's localStorage, sessionStorage,
// cookies and IndexedDB databases, each item's deletion and the site's
// clearing; and every site the browser keeps anything for, with what it
// uses.

tool("storage", "Storage", (pane) => {
  pane.classList.add("storage");
  const bar = el("div", "bar");
  const refresh = el("button", "", "Refresh");
  refresh.id = "storage-refresh";
  const clear = el("button", "", "Clear site data");
  clear.id = "storage-clear";
  const usage = el("span", "quiet usage");
  usage.id = "storage-usage";
  bar.appendChild(refresh);
  bar.appendChild(clear);
  bar.appendChild(usage);
  const body = el("div", "store-body mono");
  body.id = "storage-body";
  pane.appendChild(bar);
  pane.appendChild(body);

  function table(columns, rows, remove) {
    const t = el("table", "table");
    const head = el("tr");
    for (const name of columns) {
      head.appendChild(el("th", "", name));
    }
    if (remove) {
      head.appendChild(el("th", "", ""));
    }
    t.appendChild(head);
    for (const row of rows) {
      const tr = el("tr");
      for (const cell of row.cells) {
        tr.appendChild(el("td", "", cell));
      }
      if (remove) {
        const td = el("td");
        const button = el("button", "remove", "Delete");
        button.addEventListener("click", () => remove(row));
        td.appendChild(button);
        tr.appendChild(td);
      }
      t.appendChild(tr);
    }
    return t;
  }

  function section(title, id) {
    const box = el("div", "section");
    box.id = id;
    box.appendChild(el("div", "heading", title));
    body.appendChild(box);
    return box;
  }

  function keyText(key) {
    if ("n" in key) {
      return String(key.n);
    }
    if ("s" in key) {
      return JSON.stringify(key.s);
    }
    if ("d" in key) {
      return "Date(" + key.d + ")";
    }
    if ("b" in key) {
      return "bytes " + key.b;
    }
    return "[" + key.a.map(keyText).join(", ") + "]";
  }

  function area(name, title) {
    const box = section(title, "storage-" + name);
    call("storage." + name).then((items) => {
      box.appendChild(table(["Key", "Value"], items.map(([k, v]) => ({ cells: [k, v], key: k })),
        (row) => call("storage.removeItem", { area: name, key: row.key }).then(load, load)));
    }, () => {});
  }

  function load() {
    body.textContent = "";
    call("storage.usage").then((u) => {
      usage.textContent = u.usage + " of " + u.quota + " bytes";
    }, () => {});
    area("local", "Local storage");
    area("session", "Session storage");
    const cookies = section("Cookies", "storage-cookies");
    call("storage.cookies").then((list) => {
      cookies.appendChild(table(
        ["Name", "Value", "Domain", "Path", "Expires", "HttpOnly", "Secure", "SameSite"],
        list.map((c) => ({
          cells: [c.name, c.value, c.domain, c.path,
            c.expires === null ? "Session" : new Date(c.expires).toISOString(),
            c.httpOnly ? "yes" : "", c.secure ? "yes" : "", c.sameSite],
          cookie: c,
        })),
        (row) => call("storage.removeCookie", {
          name: row.cookie.name, domain: row.cookie.domain, path: row.cookie.path,
        }).then(load, load),
      ));
    }, () => {});
    const idb = section("IndexedDB", "storage-idb");
    call("storage.idb").then((databases) => {
      for (const db of databases) {
        const box = el("div", "database");
        const head = el("div", "rule-head");
        head.appendChild(el("span", "", db.name + " (version " + db.version + ")"));
        const drop = el("button", "remove", "Delete database");
        drop.addEventListener("click", () => call("storage.deleteDatabase", { name: db.name }).then(load, load));
        head.appendChild(drop);
        box.appendChild(head);
        for (const store of db.stores) {
          box.appendChild(el("div", "quiet", store.name + ": " + store.count + " records" +
            (store.indices.length ? ", indices " + store.indices.join(", ") : "")));
          box.appendChild(table(["Key", "Value"],
            store.records.map(([k, v]) => ({ cells: [keyText(k), v] })), null));
        }
        idb.appendChild(box);
      }
    }, () => {});
    const sites = section("Every site's storage", "storage-sites");
    call("storage.sites").then((list) => {
      sites.appendChild(table(["Site", "Bytes"],
        list.map((s) => ({ cells: [s.origin, String(s.usage)], origin: s.origin })),
        (row) => call("storage.clearSite", { origin: row.origin }).then(load, load)));
    }, () => {});
  }

  refresh.addEventListener("click", load);
  clear.addEventListener("click", () => call("storage.clearSite").then(load, load));
  load();
  return { navigated: load, shown: load };
});
