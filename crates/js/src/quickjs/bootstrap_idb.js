// IndexedDB (Indexed Database API 3.0; M36.7) and `navigator.storage`
// (the Storage Standard; M36.8): a binding chunk (D64), compiled apart
// from the bootstrap and read into a realm only when a page first uses
// them. A function of what of the bootstrap's it uses, as the bootstrap
// names it, giving what the bootstrap calls of it. The bootstrap's rules
// hold here: what is kept of the built-ins was kept before any page ran,
// nothing looked up on a built-in's prototype when a page calls in, no
// iterator.
(function (env) {
  "use strict";
  const {
    $ArrayIsArray, $DOMException, $Date, $DateGetTime, $Map, $Number, $Promise, $TypeError,
    ArrayIncludes, ArrayPush, ArraySlice, ArraySort, ArraySplice, EVENT_INIT, Event, EventTarget,
    FromBase64, GeneratorNext, JSONParse, JSONStringify, KEY, MapDelete, MapGet, MapSet,
    ObjectCreate, ObjectHasOwn, StringIndexOf, StringSlice, ToBase64, WeakMapDelete, WeakMapSet,
    branded, bufferBytes, defineHandlers, defineProperty, deserialize, dispatching, domError, drain,
    eventArgs, eventParents, eventState, expose, fieldsOf, illegalConstructor, made, members,
    native, nothing, prefix, serialize, toString, unsigned, unwire, wire,
  } = env;

  // ---- IndexedDB (Indexed Database API 3.0; M36.7) -------------------------

  // The databases are the browser's (the engine's store, docs/design/
  // devtools.md, "IndexedDB"): each request crosses as JSON through
  // `native.idb`, the storage key the host's own. Here are what a script
  // holds: the keys it gives as values, the key paths evaluated on the
  // clones of its values, the requests and their events, each a task of
  // its own, and the transactions' lifetimes and their order.

  // `op`, a request of the store's: its result, or the DOMException it
  // fails with, worded after `where`.
  function idbAsk(op, where) {
    const answer = JSONParse(native.idb(JSONStringify(op)));
    if (ObjectHasOwn(answer, "error")) {
      const error = answer.error;
      if (error.name === "TypeError") {
        throw new $TypeError(where + error.message);
      }
      throw new $DOMException(where + error.message, error.name);
    }
    return answer.result;
  }

  // A request's op: an object of no prototype, so that nothing a page
  // puts on Object.prototype is written with it.
  function idbOp(pairs) {
    const o = ObjectCreate(null);
    for (let i = 0; i < pairs.length; i += 2) {
      if (pairs[i + 1] !== undefined) {
        o[pairs[i]] = pairs[i + 1];
      }
    }
    return o;
  }

  // ---- Keys (section 2.7) ----

  // Section 7.1's "convert a value to a key": the key as the store
  // writes it, or undefined for a value that is no key. `seen` holds the
  // arrays on the way down.
  function toIdbKey(value, seen) {
    if (typeof value === "number") {
      return value !== value ? undefined : idbOp(["n", value === 0 ? 0 : value]);
    }
    if (typeof value === "string") {
      return idbOp(["s", value]);
    }
    if (typeof value !== "object" || value === null) {
      return undefined;
    }
    if (branded($DateGetTime, value)) {
      const time = $DateGetTime(value);
      return time !== time ? undefined : idbOp(["d", time === 0 ? 0 : time]);
    }
    const bytes = bufferBytes(value);
    if (bytes !== undefined) {
      return idbOp(["b", ToBase64(bytes)]);
    }
    if ($ArrayIsArray(value)) {
      for (let i = 0; i < seen.length; i++) {
        if (seen[i] === value) {
          return undefined;
        }
      }
      if (seen.length >= 64) {
        return undefined;
      }
      ArrayPush(seen, value);
      const items = [];
      const length = value.length;
      for (let i = 0; i < length; i++) {
        if (!ObjectHasOwn(value, i)) {
          seen.length--;
          return undefined;
        }
        const item = toIdbKey(value[i], seen);
        if (item === undefined) {
          seen.length--;
          return undefined;
        }
        ArrayPush(items, item);
      }
      seen.length--;
      return idbOp(["a", items]);
    }
    return undefined;
  }

  // A key the store wrote, as a script's value (section 7.2).
  function fromIdbKey(key) {
    if (ObjectHasOwn(key, "n")) {
      return key.n;
    }
    if (ObjectHasOwn(key, "s")) {
      return key.s;
    }
    if (ObjectHasOwn(key, "d")) {
      return new $Date(key.d);
    }
    if (ObjectHasOwn(key, "b")) {
      return FromBase64(key.b).buffer;
    }
    const items = key.a;
    const out = [];
    for (let i = 0; i < items.length; i++) {
      ArrayPush(out, fromIdbKey(items[i]));
    }
    return out;
  }

  // A key of an argument, or the DataError of one that is none.
  function idbKeyArg(value, method, iface) {
    const key = toIdbKey(value, []);
    if (key === undefined) {
      throw domError("DataError", "The parameter is not a valid key.", method, iface);
    }
    return key;
  }

  // Section 2.7's "compare two keys", of keys as the store writes them.
  function compareIdbKeys(a, b) {
    const rank = (k) => ObjectHasOwn(k, "n") ? 0 : ObjectHasOwn(k, "d") ? 1 :
      ObjectHasOwn(k, "s") ? 2 : ObjectHasOwn(k, "b") ? 3 : 4;
    const ra = rank(a);
    const rb = rank(b);
    if (ra !== rb) {
      return ra < rb ? -1 : 1;
    }
    if (ra === 0 || ra === 1) {
      const x = ra === 0 ? a.n : a.d;
      const y = ra === 0 ? b.n : b.d;
      return x < y ? -1 : x > y ? 1 : 0;
    }
    if (ra === 2) {
      return a.s < b.s ? -1 : a.s > b.s ? 1 : 0;
    }
    if (ra === 3) {
      const x = FromBase64(a.b);
      const y = FromBase64(b.b);
      const n = x.length < y.length ? x.length : y.length;
      for (let i = 0; i < n; i++) {
        if (x[i] !== y[i]) {
          return x[i] < y[i] ? -1 : 1;
        }
      }
      return x.length === y.length ? 0 : x.length < y.length ? -1 : 1;
    }
    const n = a.a.length < b.a.length ? a.a.length : b.a.length;
    for (let i = 0; i < n; i++) {
      const c = compareIdbKeys(a.a[i], b.a[i]);
      if (c !== 0) {
        return c;
      }
    }
    return a.a.length === b.a.length ? 0 : a.a.length < b.a.length ? -1 : 1;
  }

  // ---- Key paths (section 2.5) ----

  // Whether `path` is a valid key path: an identifier, or identifiers
  // joined by dots, or the empty string; a list of such.
  function validKeyPath(path) {
    if (typeof path === "string") {
      if (path === "") {
        return true;
      }
      let start = 0;
      for (;;) {
        const dot = StringIndexOf(path, ".", start);
        const part = StringSlice(path, start, dot === -1 ? path.length : dot);
        if (!KEY_PATH_PART.test(part)) {
          return false;
        }
        if (dot === -1) {
          return true;
        }
        start = dot + 1;
      }
    }
    if ($ArrayIsArray(path)) {
      if (path.length === 0) {
        return false;
      }
      for (let i = 0; i < path.length; i++) {
        if (typeof path[i] !== "string" || !validKeyPath(path[i])) {
          return false;
        }
      }
      return true;
    }
    return false;
  }
  const KEY_PATH_PART = /^[$_\p{ID_Start}][$_\u200C\u200D\p{ID_Continue}]*$/u;

  // A key path as a script gives it: a string, or a sequence of them.
  function keyPathArg(path) {
    if (path === null || path === undefined) {
      return null;
    }
    if ($ArrayIsArray(path)) {
      const out = [];
      for (let i = 0; i < path.length; i++) {
        ArrayPush(out, toString(path[i]));
      }
      return out;
    }
    return toString(path);
  }

  // Section 7.4's "evaluate a key path on a value": the value, or
  // undefined where the path leads nowhere.
  function evaluateKeyPath(value, path) {
    if ($ArrayIsArray(path)) {
      const out = [];
      for (let i = 0; i < path.length; i++) {
        const v = evaluateKeyPath(value, path[i]);
        if (v === undefined) {
          return undefined;
        }
        ArrayPush(out, v);
      }
      return out;
    }
    if (path === "") {
      return value;
    }
    let at = value;
    let start = 0;
    for (;;) {
      const dot = StringIndexOf(path, ".", start);
      const part = StringSlice(path, start, dot === -1 ? path.length : dot);
      if (typeof at === "string" && part === "length") {
        at = at.length;
      } else if ($ArrayIsArray(at) && part === "length") {
        at = at.length;
      } else if (typeof at !== "object" || at === null || !ObjectHasOwn(at, part)) {
        return undefined;
      } else {
        at = at[part];
      }
      if (dot === -1) {
        return at;
      }
      start = dot + 1;
    }
  }

  // Section 7.6's "inject a key into a value", the generator's key at
  // `path`, a string; whether it could be.
  function injectKey(value, path, key) {
    let at = value;
    let start = 0;
    for (;;) {
      const dot = StringIndexOf(path, ".", start);
      const part = StringSlice(path, start, dot === -1 ? path.length : dot);
      if (typeof at !== "object" || at === null) {
        return false;
      }
      if (dot === -1) {
        at[part] = key;
        return true;
      }
      if (!ObjectHasOwn(at, part)) {
        at[part] = {};
      }
      at = at[part];
      start = dot + 1;
    }
  }

  // The keys of index `index` of a value's clone: its key, or for a
  // multiEntry index the keys of its array, each once.
  function indexKeys(clone, index) {
    const v = evaluateKeyPath(clone, index.keyPath);
    if (v === undefined) {
      return [];
    }
    if (index.multiEntry && $ArrayIsArray(v)) {
      const keys = [];
      for (let i = 0; i < v.length; i++) {
        const k = toIdbKey(v[i], []);
        if (k === undefined) {
          continue;
        }
        let twice = false;
        for (let j = 0; j < keys.length; j++) {
          if (compareIdbKeys(keys[j], k) === 0) {
            twice = true;
            break;
          }
        }
        if (!twice) {
          ArrayPush(keys, k);
        }
      }
      return keys;
    }
    const k = toIdbKey(v, []);
    return k === undefined ? [] : [k];
  }

  // ---- Key ranges (section 2.9) ----

  let idbRangeOf;
  class IDBKeyRange {
    #lower;
    #upper;
    #lowerOpen;
    #upperOpen;

    constructor(key, lower, upper, lowerOpen, upperOpen) {
      if (key !== KEY) {
        throw illegalConstructor("IDBKeyRange");
      }
      this.#lower = lower;
      this.#upper = upper;
      this.#lowerOpen = lowerOpen;
      this.#upperOpen = upperOpen;
    }

    static {
      idbRangeOf = (value) => {
        if (typeof value !== "object" || value === null || !(#lower in value)) {
          return undefined;
        }
        return idbOp([
          "lower", value.#lower, "upper", value.#upper,
          "lowerOpen", value.#lowerOpen, "upperOpen", value.#upperOpen,
        ]);
      };
    }

    get lower() {
      return this.#lower === undefined ? undefined : fromIdbKey(this.#lower);
    }

    get upper() {
      return this.#upper === undefined ? undefined : fromIdbKey(this.#upper);
    }

    get lowerOpen() {
      return this.#lowerOpen;
    }

    get upperOpen() {
      return this.#upperOpen;
    }

    static only(value) {
      const key = idbKeyArg(value, "only", "IDBKeyRange");
      return new IDBKeyRange(KEY, key, key, false, false);
    }

    static lowerBound(lower, open = false) {
      return new IDBKeyRange(KEY, idbKeyArg(lower, "lowerBound", "IDBKeyRange"), undefined, !!open, true);
    }

    static upperBound(upper, open = false) {
      return new IDBKeyRange(KEY, undefined, idbKeyArg(upper, "upperBound", "IDBKeyRange"), true, !!open);
    }

    static bound(lower, upper, lowerOpen = false, upperOpen = false) {
      const l = idbKeyArg(lower, "bound", "IDBKeyRange");
      const u = idbKeyArg(upper, "bound", "IDBKeyRange");
      const c = compareIdbKeys(l, u);
      if (c > 0 || (c === 0 && (lowerOpen || upperOpen))) {
        throw domError("DataError", "The lower key is greater than the upper key.", "bound", "IDBKeyRange");
      }
      return new IDBKeyRange(KEY, l, u, !!lowerOpen, !!upperOpen);
    }

    includes(value) {
      const range = idbRangeOf(this);
      const key = idbKeyArg(value, "includes", "IDBKeyRange");
      return rangeHolds(range, key);
    }
  }
  expose(IDBKeyRange, "IDBKeyRange");

  function rangeHolds(range, key) {
    if (range.lower !== undefined) {
      const c = compareIdbKeys(key, range.lower);
      if (c < 0 || (c === 0 && range.lowerOpen)) {
        return false;
      }
    }
    if (range.upper !== undefined) {
      const c = compareIdbKeys(key, range.upper);
      if (c > 0 || (c === 0 && range.upperOpen)) {
        return false;
      }
    }
    return true;
  }

  // Section 7.3's "convert a value to a key range": a range, a key's
  // range of one, or none for null or undefined when `nullable`.
  function idbRangeArg(value, nullable, method, iface) {
    const range = idbRangeOf(value);
    if (range !== undefined) {
      return range;
    }
    if (value === undefined || value === null) {
      if (!nullable) {
        throw domError("DataError", "No key or key range specified.", method, iface);
      }
      return null;
    }
    const key = idbKeyArg(value, method, iface);
    return idbOp(["lower", key, "upper", key, "lowerOpen", false, "upperOpen", false]);
  }

  // ---- Events ----

  // Each request's, transaction's and database's parent in an event's
  // path (section 2.2's "get the parent"): its transaction, its
  // database.
  // (eventParents is consulted by `dispatching`.)

  const VERSION_CHANGE = 262144;
  const VERSION_CHANGE_INIT = members(EVENT_INIT, [
    ["oldVersion", "unsigned long long", 0],
    ["newVersion", "unsigned long long?", null],
  ], "IDBVersionChangeEventInit");

  class IDBVersionChangeEvent extends Event {
    constructor(type, eventInitDict = undefined) {
      const args = eventArgs(arguments.length, type, eventInitDict, VERSION_CHANGE_INIT,
        "IDBVersionChangeEvent");
      super(args[0], args[1]);
      eventState(this).kinds |= VERSION_CHANGE;
    }

    get oldVersion() {
      return fieldsOf(this, VERSION_CHANGE).oldVersion;
    }

    get newVersion() {
      return fieldsOf(this, VERSION_CHANGE).newVersion;
    }
  }
  expose(IDBVersionChangeEvent, "IDBVersionChangeEvent");

  function versionEvent(type, oldVersion, newVersion) {
    return new IDBVersionChangeEvent(type, made(VERSION_CHANGE_INIT, [
      "oldVersion", oldVersion, "newVersion", newVersion,
    ]));
  }

  // The completions waiting to be delivered, each in a task of its own,
  // which `native.idbWake` asks the host for.
  const idbTasks = [];
  let idbTaskAt = 0;

  function idbQueue(task) {
    ArrayPush(idbTasks, task);
    native.idbWake();
  }

  // The next completion's task: the realm's `idb` entry.
  function idbTask() {
    if (idbTaskAt >= idbTasks.length) {
      return nothing();
    }
    const task = idbTasks[idbTaskAt];
    idbTasks[idbTaskAt] = undefined;
    idbTaskAt++;
    if (idbTaskAt === idbTasks.length) {
      idbTasks.length = 0;
      idbTaskAt = 0;
    } else {
      native.idbWake();
    }
    return task();
  }

  // A generator that runs `run` to its end, then `after`: what a task
  // dispatches, with the steps that follow it.
  function* idbThen(run, after) {
    for (;;) {
      const step = GeneratorNext(run);
      if (step.done) {
        break;
      }
      yield;
    }
    after();
    return false;
  }

  // ---- Requests (section 4.4) ----

  let idbRequestState;
  class IDBRequest extends EventTarget {
    #result = undefined;
    #error = null;
    #source = null;
    #transaction = null;
    #done = false;

    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("IDBRequest");
      }
      super();
    }

    static {
      idbRequestState = (request) => ({
        __proto__: null,
        set(done, result, error) {
          request.#done = done;
          request.#result = result;
          request.#error = error;
        },
        source(source, transaction) {
          request.#source = source;
          request.#transaction = transaction;
        },
        transaction(transaction) {
          request.#transaction = transaction;
        },
      });
    }

    get result() {
      if (!this.#done) {
        throw new $DOMException(
          "Failed to read the 'result' property from 'IDBRequest': The request has not finished.",
          "InvalidStateError",
        );
      }
      return this.#result;
    }

    get error() {
      if (!this.#done) {
        throw new $DOMException(
          "Failed to read the 'error' property from 'IDBRequest': The request has not finished.",
          "InvalidStateError",
        );
      }
      return this.#error;
    }

    get source() {
      return this.#source;
    }

    get transaction() {
      return this.#transaction;
    }

    get readyState() {
      return this.#done ? "done" : "pending";
    }
  }
  defineHandlers(IDBRequest.prototype, ["success", "error"], false);
  expose(IDBRequest, "IDBRequest");

  class IDBOpenDBRequest extends IDBRequest {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("IDBOpenDBRequest");
      }
      super(KEY);
    }
  }
  defineHandlers(IDBOpenDBRequest.prototype, ["blocked", "upgradeneeded"], false);
  expose(IDBOpenDBRequest, "IDBOpenDBRequest");

  // ---- Transactions (section 4.9) ----

  // The transactions of each database name, in the order they were
  // made: what waits for one whose scope overlaps it to finish.
  const idbOrder = new $Map();

  let txnState;
  class IDBTransaction extends EventTarget {
    #db;
    #mode;
    #scope;
    // The store's number for it, once it has begun there.
    #id = null;
    // "active", "inactive", "committing" or "finished".
    #state = "active";
    #pending = 0;
    #error = null;
    #requests = [];
    #stores = new $Map();
    #committed = false;

    constructor(key, db, mode, scope, id) {
      if (key !== KEY) {
        throw illegalConstructor("IDBTransaction");
      }
      super();
      this.#db = db;
      this.#mode = mode;
      this.#scope = scope;
      this.#id = id;
      WeakMapSet(eventParents, this, db);
    }

    static {
      txnState = (t) => ({
        __proto__: null,
        get id() {
          return t.#id;
        },
        set id(id) {
          t.#id = id;
        },
        get state() {
          return t.#state;
        },
        set state(s) {
          t.#state = s;
        },
        get mode() {
          return t.#mode;
        },
        get scope() {
          return t.#scope;
        },
        get db() {
          return t.#db;
        },
        get pending() {
          return t.#pending;
        },
        set pending(n) {
          t.#pending = n;
        },
        get requests() {
          return t.#requests;
        },
        get stores() {
          return t.#stores;
        },
        set error(e) {
          t.#error = e;
        },
        get committed() {
          return t.#committed;
        },
        set committed(c) {
          t.#committed = c;
        },
      });
    }

    get objectStoreNames() {
      return domStringList(this.#mode === "versionchange" ? dbState(this.#db).storeNames() : this.#scope);
    }

    get mode() {
      return this.#mode;
    }

    get durability() {
      return "default";
    }

    get db() {
      return this.#db;
    }

    get error() {
      return this.#error;
    }

    objectStore(name) {
      name = toString(name);
      if (this.#state === "finished") {
        throw domError("InvalidStateError", "The transaction has finished.", "objectStore", "IDBTransaction");
      }
      const known = this.#mode === "versionchange"
        ? dbState(this.#db).info(name) !== undefined
        : ArrayIncludes(this.#scope, name);
      if (!known) {
        throw domError("NotFoundError", "The specified object store was not found.", "objectStore",
          "IDBTransaction");
      }
      let store = MapGet(this.#stores, name);
      if (store === undefined) {
        store = new IDBObjectStore(KEY, this, name);
        MapSet(this.#stores, name, store);
      }
      return store;
    }

    abort() {
      if (this.#state === "committing" || this.#state === "finished") {
        throw domError("InvalidStateError", "The transaction has finished.", "abort", "IDBTransaction");
      }
      abortTransaction(this, null);
    }

    commit() {
      if (this.#state !== "active") {
        throw domError("InvalidStateError", "The transaction is not active.", "commit", "IDBTransaction");
      }
      this.#state = "committing";
      if (this.#pending === 0) {
        finishTransaction(this);
      }
    }
  }
  defineHandlers(IDBTransaction.prototype, ["abort", "complete", "error"], false);
  expose(IDBTransaction, "IDBTransaction");


  // A DOMStringList of `names`, sorted (the IDL's "sorted name list").
  function domStringList(names) {
    const sorted = ArraySlice(names, 0);
    ArraySort(sorted, (a, b) => (a < b ? -1 : a > b ? 1 : 0));
    return new DOMStringList(KEY, sorted);
  }

  class DOMStringList {
    #names;
    constructor(key, names) {
      if (key !== KEY) {
        throw illegalConstructor("DOMStringList");
      }
      this.#names = names;
      for (let i = 0; i < names.length; i++) {
        defineProperty(this, i, { value: names[i], enumerable: true, configurable: true });
      }
    }

    get length() {
      return this.#names.length;
    }

    item(index) {
      const i = unsigned(index);
      return i < this.#names.length ? this.#names[i] : null;
    }

    contains(name) {
      return ArrayIncludes(this.#names, toString(name));
    }
  }
  expose(DOMStringList, "DOMStringList");

  // Whether transaction `t` may begin: none made before it on its
  // database and not finished has a scope it shares, a writer among the
  // two; a version change waits for every one before it, and every one
  // after waits for it.
  function mayBegin(t) {
    const s = txnState(t);
    const list = MapGet(idbOrder, dbState(s.db).name) || [];
    for (let i = 0; i < list.length; i++) {
      const other = list[i];
      if (other === t) {
        return true;
      }
      const o = txnState(other);
      if (o.state === "finished") {
        continue;
      }
      if (s.mode === "versionchange" || o.mode === "versionchange") {
        return false;
      }
      if (s.mode === "readonly" && o.mode === "readonly") {
        continue;
      }
      for (let j = 0; j < s.scope.length; j++) {
        if (ArrayIncludes(o.scope, s.scope[j])) {
          return false;
        }
      }
    }
    return true;
  }

  // Begins the transactions that may, each running what waits in it.
  function beginWaiting(name) {
    const list = MapGet(idbOrder, name) || [];
    for (let i = 0; i < list.length; i++) {
      const t = list[i];
      const s = txnState(t);
      if (s.id !== null || s.state === "finished" || !mayBegin(t)) {
        continue;
      }
      try {
        s.id = idbAsk(idbOp([
          "op", "begin", "name", name, "mode", s.mode, "scope", s.scope,
        ]), "");
      } catch (e) {
        abortTransaction(t, e);
        continue;
      }
      const waiting = s.requests;
      for (let j = 0; j < waiting.length; j++) {
        runRequest(t, waiting[j]);
      }
      waiting.length = 0;
      checkFinish(t);
    }
  }

  // A request of transaction `t`: `work` run against the store when it
  // has begun, its result delivered at `request`'s in a task.
  function placeRequest(t, request, work) {
    const s = txnState(t);
    s.pending = s.pending + 1;
    const entry = { __proto__: null, request, work };
    if (s.id === null) {
      ArrayPush(s.requests, entry);
    } else {
      runRequest(t, entry);
    }
    return request;
  }

  function runRequest(t, entry) {
    let result;
    let error = null;
    try {
      result = entry.work(txnState(t).id);
    } catch (e) {
      error = e;
    }
    idbQueue(() => idbDeliver(t, entry.request, result, error));
  }

  // Section 5.7's "fire a success event" or "fire an error event" at
  // `request`, then what follows: an error not prevented aborts the
  // transaction, and one with nothing left to do commits.
  function idbDeliver(t, request, result, error) {
    const s = txnState(t);
    s.pending = s.pending - 1;
    if (s.state === "finished") {
      return nothing();
    }
    const event = error === null
      ? new Event("success", made(EVENT_INIT, ["bubbles", false, "cancelable", false]))
      : new Event("error", made(EVENT_INIT, ["bubbles", true, "cancelable", true]));
    const rs = idbRequestState(request);
    if (error === null) {
      rs.set(true, typeof result === "function" ? result() : result, null);
    } else {
      rs.set(true, undefined, error);
    }
    if (s.state === "inactive") {
      s.state = "active";
    }
    eventState(event).trusted = true;
    return idbThen(dispatching(request, event), () => {
      if (s.state === "active") {
        s.state = "inactive";
      }
      if (error !== null && !eventState(event).canceled && s.state !== "finished") {
        abortTransaction(t, error);
        return;
      }
      checkFinish(t);
    });
  }

  // A transaction with nothing pending and none to come commits: once
  // its creating task has ended (no request was made in it), or after
  // its last request's event.
  function checkFinish(t) {
    const s = txnState(t);
    if (s.pending === 0 && s.id !== null && (s.state === "inactive" || s.state === "committing")) {
      finishTransaction(t);
    }
  }

  function finishTransaction(t) {
    const s = txnState(t);
    if (s.state === "finished") {
      return;
    }
    s.state = "finished";
    let failure = null;
    try {
      idbAsk(idbOp(["op", "commit", "txn", s.id]), "");
      s.committed = true;
    } catch (e) {
      failure = e;
    }
    idbQueue(() => {
      const done = failure === null
        ? new Event("complete", made(EVENT_INIT, ["bubbles", false, "cancelable", false]))
        : new Event("abort", made(EVENT_INIT, ["bubbles", true, "cancelable", false]));
      if (failure !== null) {
        s.error = failure;
      }
      eventState(done).trusted = true;
      return idbThen(dispatching(t, done), () => {
        transactionDone(t);
      });
    });
  }

  // Section 5.8's "abort a transaction" with `error`.
  function abortTransaction(t, error) {
    const s = txnState(t);
    if (s.state === "finished") {
      return;
    }
    s.state = "finished";
    s.error = error;
    if (s.id !== null) {
      try {
        idbAsk(idbOp(["op", "abort", "txn", s.id]), "");
      } catch {
        // Gone with its database.
      }
    }
    // A version change aborted: the connection's view of its stores goes
    // back to what it was.
    if (s.mode === "versionchange") {
      dbState(s.db).revert();
    }
    // Its requests not yet delivered fail with AbortError.
    const waiting = s.requests;
    for (let i = 0; i < waiting.length; i++) {
      const request = waiting[i].request;
      idbQueue(() => {
        idbRequestState(request).set(true, undefined,
          new $DOMException("The transaction was aborted, so the request cannot be fulfilled.",
            "AbortError"));
        const event = new Event("error", made(EVENT_INIT, ["bubbles", true, "cancelable", true]));
        eventState(event).trusted = true;
        return dispatching(request, event);
      });
    }
    waiting.length = 0;
    idbQueue(() => {
      const event = new Event("abort", made(EVENT_INIT, ["bubbles", true, "cancelable", false]));
      eventState(event).trusted = true;
      return idbThen(dispatching(t, event), () => {
        transactionDone(t);
      });
    });
  }

  function transactionDone(t) {
    const s = txnState(t);
    const name = dbState(s.db).name;
    const list = MapGet(idbOrder, name);
    if (list !== undefined) {
      for (let i = 0; i < list.length; i++) {
        if (list[i] === t) {
          ArraySplice(list, i, 1);
          break;
        }
      }
    }
    const after = dbState(s.db).afterVersionChange;
    if (s.mode === "versionchange" && after !== null) {
      dbState(s.db).afterVersionChange = null;
      after(s.committed);
    }
    beginWaiting(name);
  }

  // A request's work needs its transaction active.
  function activeCheck(t, method, iface) {
    const state = txnState(t).state;
    if (state !== "active") {
      throw domError("TransactionInactiveError", "The transaction is not active.", method, iface);
    }
  }

  function writableCheck(t, method, iface) {
    if (txnState(t).mode === "readonly") {
      throw domError("ReadOnlyError", "The transaction is read-only.", method, iface);
    }
  }

  // ---- Databases (section 4.8) ----

  let dbState;
  class IDBDatabase extends EventTarget {
    #name;
    #version;
    // The stores as the connection knows them: [{name, keyPath,
    // autoIncrement, indices: [{name, keyPath, unique, multiEntry}]}].
    #stores;
    #before = null;
    #closed = false;
    #afterVersionChange = null;
    #upgrade = null;

    constructor(key, name, version, stores) {
      if (key !== KEY) {
        throw illegalConstructor("IDBDatabase");
      }
      super();
      this.#name = name;
      this.#version = version;
      this.#stores = stores;
    }

    static {
      dbState = (db) => ({
        __proto__: null,
        get name() {
          return db.#name;
        },
        set version(v) {
          db.#version = v;
        },
        get closed() {
          return db.#closed;
        },
        set closed(c) {
          db.#closed = c;
        },
        get upgrade() {
          return db.#upgrade;
        },
        set upgrade(t) {
          db.#upgrade = t;
        },
        get afterVersionChange() {
          return db.#afterVersionChange;
        },
        set afterVersionChange(f) {
          db.#afterVersionChange = f;
        },
        storeNames() {
          const names = [];
          for (let i = 0; i < db.#stores.length; i++) {
            ArrayPush(names, db.#stores[i].name);
          }
          return names;
        },
        info(name) {
          for (let i = 0; i < db.#stores.length; i++) {
            if (db.#stores[i].name === name) {
              return db.#stores[i];
            }
          }
          return undefined;
        },
        keep() {
          db.#before = JSONParse(JSONStringify(db.#stores));
        },
        revert() {
          if (db.#before !== null) {
            db.#stores = db.#before;
          }
        },
        setStores(stores) {
          db.#stores = stores;
        },
        get stores() {
          return db.#stores;
        },
      });
    }

    get name() {
      return this.#name;
    }

    get version() {
      return this.#version;
    }

    get objectStoreNames() {
      return domStringList(dbState(this).storeNames());
    }

    createObjectStore(name, options = undefined) {
      name = toString(name);
      const t = this.#upgrade;
      if (t === null || txnState(t).state === "finished") {
        throw domError("InvalidStateError", "The database is not running a version change transaction.",
          "createObjectStore", "IDBDatabase");
      }
      activeCheck(t, "createObjectStore", "IDBDatabase");
      const keyPath = options === undefined || options === null ? null : keyPathArg(options.keyPath);
      const autoIncrement = options !== undefined && options !== null && !!options.autoIncrement;
      if (keyPath !== null && !validKeyPath(keyPath)) {
        throw domError("SyntaxError", "The keyPath option is not a valid key path.", "createObjectStore",
          "IDBDatabase");
      }
      if (dbState(this).info(name) !== undefined) {
        throw domError("ConstraintError", "An object store with the specified name already exists.",
          "createObjectStore", "IDBDatabase");
      }
      if (autoIncrement && (keyPath === "" || $ArrayIsArray(keyPath))) {
        throw domError("InvalidAccessError",
          "The autoIncrement option was set but the keyPath option was empty or an array.",
          "createObjectStore", "IDBDatabase");
      }
      idbAsk(idbOp(["op", "createStore", "txn", txnState(t).id, "name", name, "keyPath", keyPath,
        "autoIncrement", autoIncrement]), prefix("createObjectStore", "IDBDatabase"));
      ArrayPush(this.#stores, { __proto__: null, name, keyPath, autoIncrement, indices: [] });
      return t.objectStore(name);
    }

    deleteObjectStore(name) {
      name = toString(name);
      const t = this.#upgrade;
      if (t === null || txnState(t).state === "finished") {
        throw domError("InvalidStateError", "The database is not running a version change transaction.",
          "deleteObjectStore", "IDBDatabase");
      }
      activeCheck(t, "deleteObjectStore", "IDBDatabase");
      idbAsk(idbOp(["op", "deleteStore", "txn", txnState(t).id, "name", name]),
        prefix("deleteObjectStore", "IDBDatabase"));
      for (let i = 0; i < this.#stores.length; i++) {
        if (this.#stores[i].name === name) {
          ArraySplice(this.#stores, i, 1);
          break;
        }
      }
      MapDelete(txnState(t).stores, name);
    }

    transaction(storeNames, mode = "readonly", options = undefined) {
      if (this.#upgrade !== null && txnState(this.#upgrade).state !== "finished") {
        throw domError("InvalidStateError", "A version change transaction is running.", "transaction",
          "IDBDatabase");
      }
      if (this.#closed) {
        throw domError("InvalidStateError", "The database connection is closing.", "transaction",
          "IDBDatabase");
      }
      const names = [];
      if (typeof storeNames === "string" || !$ArrayIsArray(storeNames)) {
        ArrayPush(names, toString(storeNames));
      } else {
        for (let i = 0; i < storeNames.length; i++) {
          const n = toString(storeNames[i]);
          if (!ArrayIncludes(names, n)) {
            ArrayPush(names, n);
          }
        }
      }
      for (let i = 0; i < names.length; i++) {
        if (dbState(this).info(names[i]) === undefined) {
          throw domError("NotFoundError", "One of the specified object stores was not found.",
            "transaction", "IDBDatabase");
        }
      }
      if (names.length === 0) {
        throw domError("InvalidAccessError", "The storeNames parameter was empty.", "transaction",
          "IDBDatabase");
      }
      mode = toString(mode);
      if (mode !== "readonly" && mode !== "readwrite") {
        throw new $TypeError(prefix("transaction", "IDBDatabase") + "The provided value '" + mode +
          "' is not a valid enum value of type IDBTransactionMode.");
      }
      const t = new IDBTransaction(KEY, this, mode, names, null);
      let list = MapGet(idbOrder, this.#name);
      if (list === undefined) {
        list = [];
        MapSet(idbOrder, this.#name, list);
      }
      ArrayPush(list, t);
      beginWaiting(this.#name);
      // Its creating task ends: inactive, and done if nothing was asked.
      idbQueue(() => {
        const s = txnState(t);
        if (s.state === "active") {
          s.state = "inactive";
        }
        checkFinish(t);
        return nothing();
      });
      return t;
    }

    close() {
      this.#closed = true;
    }
  }
  defineHandlers(IDBDatabase.prototype, ["abort", "close", "error", "versionchange"], false);
  expose(IDBDatabase, "IDBDatabase");

  // The connections open in this realm, by database name: those a
  // version change of their database is told of.
  const idbConnections = new $Map();

  // ---- Object stores and indices (sections 4.5 and 4.6) ----

  class IDBObjectStore {
    #t;
    #name;

    constructor(key, t, name) {
      if (key !== KEY) {
        throw illegalConstructor("IDBObjectStore");
      }
      this.#t = t;
      this.#name = name;
    }

    #info() {
      const info = dbState(txnState(this.#t).db).info(this.#name);
      if (info === undefined) {
        throw new $DOMException("The object store has been deleted.", "InvalidStateError");
      }
      return info;
    }

    get name() {
      return this.#name;
    }

    get keyPath() {
      const path = this.#info().keyPath;
      return $ArrayIsArray(path) ? ArraySlice(path, 0) : path;
    }

    get indexNames() {
      const names = [];
      const indices = this.#info().indices;
      for (let i = 0; i < indices.length; i++) {
        ArrayPush(names, indices[i].name);
      }
      return domStringList(names);
    }

    get transaction() {
      return this.#t;
    }

    get autoIncrement() {
      return this.#info().autoIncrement;
    }

    put(value, key = undefined) {
      return this.#store(value, key, false, "put");
    }

    add(value, key = undefined) {
      return this.#store(value, key, true, "add");
    }

    // Section 5.4's "add or put".
    #store(value, key, noOverwrite, method) {
      const t = this.#t;
      const info = this.#info();
      activeCheck(t, method, "IDBObjectStore");
      writableCheck(t, method, "IDBObjectStore");
      const where = prefix(method, "IDBObjectStore");
      if (info.keyPath !== null && key !== undefined) {
        throw domError("DataError",
          "The object store uses in-line keys and the key parameter was provided.", method, "IDBObjectStore");
      }
      if (info.keyPath === null && !info.autoIncrement && key === undefined) {
        throw domError("DataError",
          "The object store uses out-of-line keys and has no key generator and the key parameter was not provided.",
          method, "IDBObjectStore");
      }
      let given = key === undefined ? undefined : idbKeyArg(key, method, "IDBObjectStore");
      const record = serialize(value, [], where, null);
      let clone = deserialize(record);
      let inject = false;
      if (info.keyPath !== null) {
        const v = evaluateKeyPath(clone, info.keyPath);
        if (v !== undefined) {
          given = toIdbKey(v, []);
          if (given === undefined) {
            throw domError("DataError", "Evaluating the object store's key path yielded a value that is not a valid key.",
              method, "IDBObjectStore");
          }
        } else if (!info.autoIncrement) {
          throw domError("DataError", "Evaluating the object store's key path did not yield a value.", method,
            "IDBObjectStore");
        } else {
          // Section 7.5's "check that a key could be injected".
          inject = true;
          if (!canInject(clone, info.keyPath)) {
            throw domError("DataError", "A generated key could not be inserted into the value.", method,
              "IDBObjectStore");
          }
        }
      }
      const request = new IDBRequest(KEY);
      idbRequestState(request).source(this, t);
      WeakMapSet(eventParents, request, t);
      const name = this.#name;
      return placeRequest(t, request, (id) => {
        let k = given;
        let text;
        if (inject) {
          k = idbAsk(idbOp(["op", "generateKey", "txn", id, "store", name]), where);
          injectKey(clone, info.keyPath, fromIdbKey(k));
          text = wire(serialize(clone, [], where, null));
        } else {
          text = wire(record);
        }
        const keys = [];
        for (let i = 0; i < info.indices.length; i++) {
          const index = info.indices[i];
          ArrayPush(keys, [index.name, indexKeys(clone, index)]);
        }
        const stored = idbAsk(idbOp(["op", "put", "txn", id, "store", name, "key", k, "value", text,
          "indexKeys", keys, "add", noOverwrite]), where);
        return fromIdbKey(stored);
      });
    }

    delete(query) {
      return this.#write("delete", query, false);
    }

    clear() {
      return this.#write("clear", null, true);
    }

    #write(method, query, all) {
      const t = this.#t;
      this.#info();
      activeCheck(t, method, "IDBObjectStore");
      writableCheck(t, method, "IDBObjectStore");
      const range = all ? null : idbRangeArg(query, false, method, "IDBObjectStore");
      const request = new IDBRequest(KEY);
      idbRequestState(request).source(this, t);
      WeakMapSet(eventParents, request, t);
      const name = this.#name;
      return placeRequest(t, request, (id) => {
        idbAsk(idbOp(["op", "delete", "txn", id, "store", name, "range", range]), prefix(method, "IDBObjectStore"));
        return undefined;
      });
    }

    get(query) {
      return readRequest(this, this.#t, this.#name, null, "get", query, false, "IDBObjectStore");
    }

    getKey(query) {
      return readRequest(this, this.#t, this.#name, null, "getKey", query, false, "IDBObjectStore");
    }

    getAll(query = undefined, count = undefined) {
      return readRequest(this, this.#t, this.#name, null, "getAll", query, count, "IDBObjectStore");
    }

    getAllKeys(query = undefined, count = undefined) {
      return readRequest(this, this.#t, this.#name, null, "getAllKeys", query, count, "IDBObjectStore");
    }

    count(query = undefined) {
      return readRequest(this, this.#t, this.#name, null, "count", query, false, "IDBObjectStore");
    }

    openCursor(query = undefined, direction = "next") {
      return openIdbCursor(this, this.#t, this.#name, null, query, direction, false, "IDBObjectStore");
    }

    openKeyCursor(query = undefined, direction = "next") {
      return openIdbCursor(this, this.#t, this.#name, null, query, direction, true, "IDBObjectStore");
    }

    index(name) {
      name = toString(name);
      const indices = this.#info().indices;
      for (let i = 0; i < indices.length; i++) {
        if (indices[i].name === name) {
          return new IDBIndex(KEY, this, this.#t, name);
        }
      }
      throw domError("NotFoundError", "The specified index was not found.", "index", "IDBObjectStore");
    }

    createIndex(name, keyPath, options = undefined) {
      name = toString(name);
      const t = this.#t;
      const info = this.#info();
      if (txnState(t).mode !== "versionchange") {
        throw domError("InvalidStateError", "The database is not running a version change transaction.",
          "createIndex", "IDBObjectStore");
      }
      activeCheck(t, "createIndex", "IDBObjectStore");
      keyPath = keyPathArg(keyPath);
      if (!validKeyPath(keyPath)) {
        throw domError("SyntaxError", "The keyPath argument contains an invalid key path.", "createIndex",
          "IDBObjectStore");
      }
      const unique = options !== undefined && options !== null && !!options.unique;
      const multiEntry = options !== undefined && options !== null && !!options.multiEntry;
      if (multiEntry && $ArrayIsArray(keyPath)) {
        throw domError("InvalidAccessError", "The keyPath argument was an array and the multiEntry option is true.",
          "createIndex", "IDBObjectStore");
      }
      for (let i = 0; i < info.indices.length; i++) {
        if (info.indices[i].name === name) {
          throw domError("ConstraintError", "An index with the specified name already exists.", "createIndex",
            "IDBObjectStore");
        }
      }
      const index = { __proto__: null, name, keyPath, unique, multiEntry };
      // The records there now, and their keys in it.
      const id = txnState(t).id;
      const records = idbAsk(idbOp(["op", "getAll", "txn", id, "store", this.#name]), "");
      const keys = [];
      for (let i = 0; i < records.length; i++) {
        const clone = deserialize(unwire(records[i].value));
        ArrayPush(keys, [records[i].primaryKey, indexKeys(clone, index)]);
      }
      try {
        idbAsk(idbOp(["op", "createIndex", "txn", id, "store", this.#name, "name", name, "keyPath", keyPath,
          "unique", unique, "multiEntry", multiEntry, "keys", keys]), prefix("createIndex", "IDBObjectStore"));
      } catch (e) {
        // Section 4.6: records sharing a key of a unique index abort
        // the version change, in a task.
        if (e.name === "ConstraintError") {
          idbQueue(() => {
            abortTransaction(t, new $DOMException(e.message, "ConstraintError"));
            return nothing();
          });
        } else {
          throw e;
        }
      }
      ArrayPush(info.indices, index);
      return new IDBIndex(KEY, this, t, name);
    }

    deleteIndex(name) {
      name = toString(name);
      const t = this.#t;
      const info = this.#info();
      if (txnState(t).mode !== "versionchange") {
        throw domError("InvalidStateError", "The database is not running a version change transaction.",
          "deleteIndex", "IDBObjectStore");
      }
      activeCheck(t, "deleteIndex", "IDBObjectStore");
      idbAsk(idbOp(["op", "deleteIndex", "txn", txnState(t).id, "store", this.#name, "name", name]),
        prefix("deleteIndex", "IDBObjectStore"));
      for (let i = 0; i < info.indices.length; i++) {
        if (info.indices[i].name === name) {
          ArraySplice(info.indices, i, 1);
          break;
        }
      }
    }
  }
  expose(IDBObjectStore, "IDBObjectStore");

  // Whether a key could be injected at `path` (section 7.5).
  function canInject(value, path) {
    let at = value;
    let start = 0;
    for (;;) {
      if (typeof at !== "object" || at === null) {
        return false;
      }
      const dot = StringIndexOf(path, ".", start);
      if (dot === -1) {
        return true;
      }
      const part = StringSlice(path, start, dot);
      if (!ObjectHasOwn(at, part)) {
        return true;
      }
      at = at[part];
      start = dot + 1;
    }
  }

  class IDBIndex {
    #store;
    #t;
    #name;

    constructor(key, store, t, name) {
      if (key !== KEY) {
        throw illegalConstructor("IDBIndex");
      }
      this.#store = store;
      this.#t = t;
      this.#name = name;
    }

    #info() {
      const store = dbState(txnState(this.#t).db).info(this.#store.name);
      const indices = store === undefined ? [] : store.indices;
      for (let i = 0; i < indices.length; i++) {
        if (indices[i].name === this.#name) {
          return indices[i];
        }
      }
      throw new $DOMException("The index has been deleted.", "InvalidStateError");
    }

    get name() {
      return this.#name;
    }

    get objectStore() {
      return this.#store;
    }

    get keyPath() {
      const path = this.#info().keyPath;
      return $ArrayIsArray(path) ? ArraySlice(path, 0) : path;
    }

    get multiEntry() {
      return this.#info().multiEntry;
    }

    get unique() {
      return this.#info().unique;
    }

    get(query) {
      this.#info();
      return readRequest(this, this.#t, this.#store.name, this.#name, "get", query, false, "IDBIndex");
    }

    getKey(query) {
      this.#info();
      return readRequest(this, this.#t, this.#store.name, this.#name, "getKey", query, false, "IDBIndex");
    }

    getAll(query = undefined, count = undefined) {
      this.#info();
      return readRequest(this, this.#t, this.#store.name, this.#name, "getAll", query, count, "IDBIndex");
    }

    getAllKeys(query = undefined, count = undefined) {
      this.#info();
      return readRequest(this, this.#t, this.#store.name, this.#name, "getAllKeys", query, count, "IDBIndex");
    }

    count(query = undefined) {
      this.#info();
      return readRequest(this, this.#t, this.#store.name, this.#name, "count", query, false, "IDBIndex");
    }

    openCursor(query = undefined, direction = "next") {
      this.#info();
      return openIdbCursor(this, this.#t, this.#store.name, this.#name, query, direction, false, "IDBIndex");
    }

    openKeyCursor(query = undefined, direction = "next") {
      this.#info();
      return openIdbCursor(this, this.#t, this.#store.name, this.#name, query, direction, true, "IDBIndex");
    }
  }
  expose(IDBIndex, "IDBIndex");

  // A read of a store or an index: `get`, `getKey`, `getAll`,
  // `getAllKeys` or `count`.
  function readRequest(source, t, store, index, method, query, count, iface) {
    activeCheck(t, method, iface);
    const all = method === "getAll" || method === "getAllKeys" || method === "count";
    const range = idbRangeArg(query, all, method, iface);
    const limit = count === undefined || count === false ? 0 : unsigned(count);
    const request = new IDBRequest(KEY);
    idbRequestState(request).source(source, t);
    WeakMapSet(eventParents, request, t);
    const where = prefix(method, iface);
    return placeRequest(t, request, (id) => {
      if (method === "count") {
        return idbAsk(idbOp(["op", "count", "txn", id, "store", store, "index", index ?? undefined, "range", range]),
          where);
      }
      const keysOnly = method === "getKey" || method === "getAllKeys";
      const found = idbAsk(idbOp(["op", "getAll", "txn", id, "store", store, "index", index ?? undefined,
        "range", range, "count", all ? limit : 1, "keysOnly", keysOnly]), where);
      if (!all) {
        if (found.length === 0) {
          return undefined;
        }
        const f = found[0];
        if (method === "getKey") {
          return fromIdbKey(f.primaryKey);
        }
        // A value is made again in this realm when it is delivered.
        const text = f.value;
        return () => deserialize(unwire(text));
      }
      if (keysOnly) {
        const keys = [];
        for (let i = 0; i < found.length; i++) {
          ArrayPush(keys, fromIdbKey(found[i].primaryKey));
        }
        return keys;
      }
      const texts = [];
      for (let i = 0; i < found.length; i++) {
        ArrayPush(texts, found[i].value);
      }
      return () => {
        const values = [];
        for (let i = 0; i < texts.length; i++) {
          ArrayPush(values, deserialize(unwire(texts[i])));
        }
        return values;
      };
    });
  }

  // ---- Cursors (section 4.11) ----

  let cursorState;
  class IDBCursor {
    #source;
    #t;
    #store;
    #index;
    #range;
    #direction;
    #keysOnly;
    #request;
    #key = undefined;
    #primary = undefined;
    #value = undefined;
    #gotValue = false;

    constructor(key, source, t, store, index, range, direction, keysOnly, request) {
      if (key !== KEY) {
        throw illegalConstructor("IDBCursor");
      }
      this.#source = source;
      this.#t = t;
      this.#store = store;
      this.#index = index;
      this.#range = range;
      this.#direction = direction;
      this.#keysOnly = keysOnly;
      this.#request = request;
    }

    static {
      cursorState = (c) => ({
        __proto__: null,
        // Iterates once: the cursor at its next record, or null.
        step(id, count, targetKey, targetPrimary, where) {
          const found = idbAsk(idbOp([
            "op", "iterate", "txn", id, "store", c.#store, "index", c.#index ?? undefined,
            "range", c.#range, "direction", c.#direction,
            "positionKey", c.#key, "positionPrimary", c.#primary,
            "targetKey", targetKey, "targetPrimary", targetPrimary,
            "count", count, "keysOnly", c.#keysOnly,
          ]), where);
          if (found === null) {
            c.#key = undefined;
            c.#primary = undefined;
            c.#value = undefined;
            return null;
          }
          c.#key = found.key;
          c.#primary = found.primaryKey;
          c.#value = c.#keysOnly ? undefined : found.value;
          c.#gotValue = false;
          return c;
        },
        get value() {
          return c.#value;
        },
        get got() {
          return c.#gotValue;
        },
        set got(g) {
          c.#gotValue = g;
        },
      });
    }

    get source() {
      return this.#source;
    }

    get direction() {
      return this.#direction;
    }

    get key() {
      return this.#key === undefined ? undefined : fromIdbKey(this.#key);
    }

    get primaryKey() {
      return this.#primary === undefined ? undefined : fromIdbKey(this.#primary);
    }

    get request() {
      return this.#request;
    }

    #iterate(method, count, targetKey, targetPrimary) {
      const t = this.#t;
      activeCheck(t, method, "IDBCursor");
      if (!this.#gotValue) {
        throw domError("InvalidStateError", "The cursor is being iterated or has iterated past its end.",
          method, "IDBCursor");
      }
      this.#gotValue = false;
      const request = this.#request;
      idbRequestState(request).set(false, undefined, null);
      const cursor = this;
      const where = prefix(method, "IDBCursor");
      placeRequest(t, request, (id) => {
        const at = cursorState(cursor).step(id, count, targetKey, targetPrimary, where);
        return at === null ? null : () => {
          cursorState(cursor).got = true;
          return cursor;
        };
      });
    }

    advance(count) {
      const n = unsigned(count);
      if (n === 0) {
        throw new $TypeError(prefix("advance", "IDBCursor") + "A count argument with value 0 (zero) was supplied, must be greater than 0.");
      }
      this.#iterate("advance", n, undefined, undefined);
    }

    continue(key = undefined) {
      let target;
      if (key !== undefined) {
        target = idbKeyArg(key, "continue", "IDBCursor");
        const c = compareIdbKeys(target, this.#key);
        const forward = this.#direction === "next" || this.#direction === "nextunique";
        if ((forward && c <= 0) || (!forward && c >= 0)) {
          throw domError("DataError", "The parameter is less than or equal to this cursor's position.",
            "continue", "IDBCursor");
        }
      }
      this.#iterate("continue", 1, target, undefined);
    }

    continuePrimaryKey(key, primaryKey) {
      if (this.#index === null) {
        throw domError("InvalidAccessError", "The cursor's source is not an index.", "continuePrimaryKey",
          "IDBCursor");
      }
      if (this.#direction !== "next" && this.#direction !== "prev") {
        throw domError("InvalidAccessError", "The cursor's direction is unique.", "continuePrimaryKey",
          "IDBCursor");
      }
      const k = idbKeyArg(key, "continuePrimaryKey", "IDBCursor");
      const p = idbKeyArg(primaryKey, "continuePrimaryKey", "IDBCursor");
      this.#iterate("continuePrimaryKey", 1, k, p);
    }

    update(value) {
      const t = this.#t;
      activeCheck(t, "update", "IDBCursor");
      writableCheck(t, "update", "IDBCursor");
      if (!this.#gotValue || this.#keysOnly) {
        throw domError("InvalidStateError", "The cursor is being iterated or has iterated past its end.",
          "update", "IDBCursor");
      }
      const store = this.#index === null ? this.#source : this.#source.objectStore;
      const info = dbState(txnState(t).db).info(this.#store);
      if (info.keyPath !== null) {
        const clone = deserialize(serialize(value, [], prefix("update", "IDBCursor"), null));
        const k = toIdbKey(evaluateKeyPath(clone, info.keyPath), []);
        if (k === undefined || compareIdbKeys(k, this.#primary) !== 0) {
          throw domError("DataError", "The effective object store of this cursor uses in-line keys and evaluating the key path of the value parameter results in a different value than the cursor's effective key.",
            "update", "IDBCursor");
        }
        return store.put(value);
      }
      return store.put(value, fromIdbKey(this.#primary));
    }

    delete() {
      const t = this.#t;
      activeCheck(t, "delete", "IDBCursor");
      writableCheck(t, "delete", "IDBCursor");
      if (!this.#gotValue || this.#keysOnly) {
        throw domError("InvalidStateError", "The cursor is being iterated or has iterated past its end.",
          "delete", "IDBCursor");
      }
      const store = this.#index === null ? this.#source : this.#source.objectStore;
      return store.delete(fromIdbKey(this.#primary));
    }
  }
  expose(IDBCursor, "IDBCursor");

  class IDBCursorWithValue extends IDBCursor {
    constructor(key, ...rest) {
      if (key !== KEY) {
        throw illegalConstructor("IDBCursorWithValue");
      }
      super(KEY, rest[0], rest[1], rest[2], rest[3], rest[4], rest[5], rest[6], rest[7]);
    }

    get value() {
      const text = cursorState(this).value;
      return text === undefined ? undefined : deserialize(unwire(text));
    }
  }
  expose(IDBCursorWithValue, "IDBCursorWithValue");

  const CURSOR_DIRECTIONS = ["next", "nextunique", "prev", "prevunique"];

  function openIdbCursor(source, t, store, index, query, direction, keysOnly, iface) {
    const method = keysOnly ? "openKeyCursor" : "openCursor";
    activeCheck(t, method, iface);
    direction = toString(direction);
    if (!ArrayIncludes(CURSOR_DIRECTIONS, direction)) {
      throw new $TypeError(prefix(method, iface) + "The provided value '" + direction +
        "' is not a valid enum value of type IDBCursorDirection.");
    }
    const range = idbRangeArg(query, true, method, iface);
    const request = new IDBRequest(KEY);
    idbRequestState(request).source(source, t);
    WeakMapSet(eventParents, request, t);
    const cursor = keysOnly
      ? new IDBCursor(KEY, source, t, store, index, range, direction, true, request)
      : new IDBCursorWithValue(KEY, source, t, store, index, range, direction, false, request);
    const where = prefix(method, iface);
    return placeRequest(t, request, (id) => {
      const at = cursorState(cursor).step(id, 1, undefined, undefined, where);
      return at === null ? null : () => {
        cursorState(cursor).got = true;
        return cursor;
      };
    });
  }

  // ---- The factory (section 4.2) ----

  class IDBFactory {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("IDBFactory");
      }
    }

    open(name, version = undefined) {
      name = toString(name);
      let wanted = null;
      if (version !== undefined) {
        const n = $Number(version);
        if (n !== n || n < 1 || n > 9007199254740991 || n === Infinity) {
          throw new $TypeError(prefix("open", "IDBFactory") + "The version provided must not be 0.");
        }
        wanted = n - (n % 1);
      }
      idbAllowed("open");
      const request = new IDBOpenDBRequest(KEY);
      idbQueue(() => openSteps(request, name, wanted));
      return request;
    }

    deleteDatabase(name) {
      name = toString(name);
      idbAllowed("deleteDatabase");
      const request = new IDBOpenDBRequest(KEY);
      idbQueue(() => {
        const old = idbAsk(idbOp(["op", "deleteDatabase", "name", name]), "");
        tellVersionChange(name, null, old, null);
        idbRequestState(request).set(true, undefined, null);
        const event = versionEvent("success", old, null);
        eventState(event).trusted = true;
        return dispatching(request, event);
      });
      return request;
    }

    databases() {
      idbAllowed("databases");
      return new $Promise((resolve) => {
        const list = idbAsk(idbOp(["op", "databases"]), "");
        const out = [];
        for (let i = 0; i < list.length; i++) {
          ArrayPush(out, { name: list[i].name, version: list[i].version });
        }
        resolve(out);
      });
    }

    cmp(first, second) {
      return compareIdbKeys(idbKeyArg(first, "cmp", "IDBFactory"), idbKeyArg(second, "cmp", "IDBFactory"));
    }
  }
  expose(IDBFactory, "IDBFactory");

  // Whether the document may reach IndexedDB: as its Web Storage, an
  // opaque origin may not (the reference browser's SecurityError).
  function idbAllowed(method) {
    const why = native.storageOpen(true);
    if (why !== "") {
      throw domError("SecurityError", "access to the Indexed Database API is denied in this context.",
        method, "IDBFactory");
    }
  }

  // The connections of this realm to database `name` but `except`
  // hear `versionchange`.
  function tellVersionChange(name, except, oldVersion, newVersion) {
    const list = MapGet(idbConnections, name) || [];
    const dispatches = [];
    for (let i = 0; i < list.length; i++) {
      const db = list[i];
      if (db === except || dbState(db).closed) {
        continue;
      }
      const event = versionEvent("versionchange", oldVersion, newVersion);
      eventState(event).trusted = true;
      drain(dispatching(db, event));
      ArrayPush(dispatches, db);
    }
    return dispatches;
  }

  // Section 4.1's "open a database connection", and its upgrade.
  function openSteps(request, name, version) {
    let opened;
    try {
      opened = idbAsk(idbOp(["op", "open", "name", name, "version", version ?? undefined]), "");
    } catch (e) {
      idbRequestState(request).set(true, undefined, e);
      const event = new Event("error", made(EVENT_INIT, ["bubbles", true, "cancelable", true]));
      eventState(event).trusted = true;
      return dispatching(request, event);
    }
    const db = new IDBDatabase(KEY, name, opened.txn === null ? opened.version : opened.oldVersion,
      opened.stores);
    let list = MapGet(idbConnections, name);
    if (list === undefined) {
      list = [];
      MapSet(idbConnections, name, list);
    }
    ArrayPush(list, db);
    if (opened.txn === null) {
      idbRequestState(request).set(true, db, null);
      const event = new Event("success", made(EVENT_INIT, ["bubbles", false, "cancelable", false]));
      eventState(event).trusted = true;
      return dispatching(request, event);
    }
    // The other connections are told; those left open block it, which is
    // said, and it goes on (M36: a connection of another realm is not
    // told, nor waited for).
    const others = tellVersionChange(name, db, opened.oldVersion, opened.version);
    let open = false;
    for (let i = 0; i < others.length; i++) {
      if (!dbState(others[i]).closed) {
        open = true;
      }
    }
    if (open) {
      const blocked = versionEvent("blocked", opened.oldVersion, opened.version);
      eventState(blocked).trusted = true;
      drain(dispatching(request, blocked));
    }
    // The version change: its transaction, active while `upgradeneeded`
    // is dispatched.
    const t = new IDBTransaction(KEY, db, "versionchange", [], opened.txn);
    dbState(db).upgrade = t;
    dbState(db).keep();
    let order = MapGet(idbOrder, name);
    if (order === undefined) {
      order = [];
      MapSet(idbOrder, name, order);
    }
    ArrayPush(order, t);
    WeakMapSet(eventParents, request, t);
    idbRequestState(request).set(true, db, null);
    idbRequestState(request).transaction(t);
    dbState(db).afterVersionChange = (committed) => {
      idbRequestState(request).transaction(null);
      WeakMapDelete(eventParents, request);
      if (!committed) {
        dbState(db).closed = true;
        dbState(db).version = opened.oldVersion;
        idbQueue(() => {
          idbRequestState(request).set(true, undefined,
            new $DOMException("Version change transaction was aborted in upgradeneeded event handler.",
              "AbortError"));
          const event = new Event("error", made(EVENT_INIT, ["bubbles", true, "cancelable", true]));
          eventState(event).trusted = true;
          return dispatching(request, event);
        });
        return;
      }
      dbState(db).version = opened.version;
      idbQueue(() => {
        const event = new Event("success", made(EVENT_INIT, ["bubbles", false, "cancelable", false]));
        eventState(event).trusted = true;
        return dispatching(request, event);
      });
    };
    const event = versionEvent("upgradeneeded", opened.oldVersion, opened.version);
    eventState(event).trusted = true;
    dbState(db).version = opened.version;
    return idbThen(dispatching(request, event), () => {
      const s = txnState(t);
      if (s.state === "active") {
        s.state = "inactive";
      }
      checkFinish(t);
    });
  }

  const indexedDBObject = new IDBFactory(KEY);

  // ---- StorageManager (the Storage Standard, section 6; M36.8) -------------

  // What the document's storage key uses and may use, and whether its
  // storage is persistent: it never is, as the browser does not evict.
  class StorageManager {
    constructor(key) {
      if (key !== KEY) {
        throw illegalConstructor("StorageManager");
      }
    }

    estimate() {
      return new $Promise((resolve, reject) => {
        try {
          const used = native.storageUsage();
          // What uses it (M39): the caches, as the reference names them.
          const details = {};
          if ($Number(used[2]) > 0) details.caches = $Number(used[2]);
          resolve({ usage: $Number(used[0]), quota: $Number(used[1]), usageDetails: details });
        } catch (e) {
          reject(new $TypeError("Failed to execute 'estimate' on 'StorageManager': Access is denied."));
        }
      });
    }

    persisted() {
      return new $Promise((resolve) => resolve(false));
    }

    persist() {
      return new $Promise((resolve) => resolve(false));
    }
  }
  expose(StorageManager, "StorageManager");
  const storageManager = new StorageManager(KEY);

  return { idbTask, indexedDBObject, storageManager };
})
