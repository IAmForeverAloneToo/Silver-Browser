(function () {
'use strict';
const __defs = new Map();
const __cache = new Map();
function __define(path, f) { __defs.set(path, f); }
function __get(o, name, get) { Object.defineProperty(o, name, { get, enumerable: true, configurable: true }); }
function __star(o, from) { for (const k of Object.keys(from)) if (k !== 'default') __get(o, k, () => from[k]); }
function __norm(path) {
  const parts = [];
  for (const p of path.split('/')) { if (p === '..') parts.pop(); else if (p && p !== '.') parts.push(p); }
  return parts.join('/');
}
function __require(path) {
  if (__cache.has(path)) return __cache.get(path);
  const f = __defs.get(path);
  if (!f) throw new Error('not in the bundle: ' + path);
  const exports = {};
  __cache.set(path, exports);
  f(exports);
  __done.add(path);
  for (let i = 0; i < __waiting.length;) {
    if (__waiting[i][0] === path) __waiting.splice(i, 1)[0][1](exports);
    else i++;
  }
  return exports;
}
const __done = new Set();
const __waiting = [];
function __bind(path, f) {
  const m = __require(path);
  try { f(m); } catch (e) {
    if (!(e instanceof ReferenceError) || __done.has(path)) throw e;
    __waiting.push([path, f]);
  }
}
// The engine has no URL class (G68): where there is none, the bundle's
// own, enough for the CTS's runtime (absolute and relative URLs of a
// path, their query's searchParams), for the bundle alone.
const URL = globalThis.URL || class URL {
  constructor(input, base) {
    input = String(input);
    let m = /^([a-z][a-z0-9+.-]*:)\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/i.exec(input);
    if (!m) {
      if (base === undefined) throw new TypeError("Failed to construct 'URL': Invalid URL");
      const b = new URL(base);
      let path = input, search = '', hash = '';
      const h = path.indexOf('#');
      if (h >= 0) { hash = path.slice(h); path = path.slice(0, h); }
      const q = path.indexOf('?');
      if (q >= 0) { search = path.slice(q); path = path.slice(0, q); }
      if (path === '') { path = b.pathname; if (q < 0) search = b.search; }
      else if (path[0] !== '/') path = b.pathname.replace(/[^/]*$/, '') + path;
      const out = [];
      for (const s of path.split('/').slice(1)) { if (s === '..') out.pop(); else if (s !== '.') out.push(s); }
      m = [null, b.protocol, b.host, '/' + out.join('/'), search, hash];
    }
    this.protocol = m[1].toLowerCase();
    this.host = m[2];
    this.pathname = m[3] || '/';
    this.search = m[4] && m[4] !== '?' ? m[4] : '';
    this.hash = m[5] && m[5] !== '#' ? m[5] : '';
    this.searchParams = new URLSearchParams(this.search);
  }
  get origin() { return this.protocol + '//' + this.host; }
  get href() { return this.origin + this.pathname + this.search + this.hash; }
  get hostname() { return this.host.replace(/:\d+$/, ''); }
  toString() { return this.href; }
  toJSON() { return this.href; }
};
function __url(path) { return new URL('/' + path, location.href).href; }
function __import(from, spec) {
  try {
    const base = from.split('/').slice(0, -1).join('/');
    const url = new URL(String(spec), new URL('/' + base + '/', location.href));
    return Promise.resolve(__require(__norm(url.pathname)));
  } catch (e) { return Promise.reject(e); }
}
__define('common/runtime/wpt.js', function (__exports) {
'use strict';
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ // Implements the wpt-embedded test runner (see also: wpt/cts.https.html).
let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });let DefaultTestFileLoader; __bind('common/internal/file_loader.js', (__m) => { ({ DefaultTestFileLoader } = __m); });
let prettyPrintLog; __bind('common/internal/logging/log_message.js', (__m) => { ({ prettyPrintLog } = __m); });
let Logger; __bind('common/internal/logging/logger.js', (__m) => { ({ Logger } = __m); });
let parseQuery; __bind('common/internal/query/parseQuery.js', (__m) => { ({ parseQuery } = __m); });
let parseExpectationsForTestQuery, relativeQueryString; __bind('common/internal/query/query.js', (__m) => { ({ parseExpectationsForTestQuery, relativeQueryString } = __m); });
let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });

let optionEnabled, optionWorkerMode; __bind('common/runtime/helper/options.js', (__m) => { ({ optionEnabled, optionWorkerMode } = __m); });
let TestDedicatedWorker, TestServiceWorker, TestSharedWorker; __bind('common/runtime/helper/test_worker.js', (__m) => { ({ TestDedicatedWorker, TestServiceWorker, TestSharedWorker } = __m); });

// testharness.js API (https://web-platform-tests.org/writing-tests/testharness-api.html)












setup({
  // It's convenient for us to asynchronously add tests to the page. Prevent done() from being
  // called implicitly when the page is finished loading.
  explicit_done: true
});

void (async () => {
  const workerString = optionWorkerMode('worker');
  const dedicatedWorker = workerString === 'dedicated' ? new TestDedicatedWorker() : undefined;
  const sharedWorker = workerString === 'shared' ? new TestSharedWorker() : undefined;
  const serviceWorker = workerString === 'service' ? new TestServiceWorker() : undefined;

  globalTestConfig.unrollConstEvalLoops = optionEnabled('unroll_const_eval_loops');

  const failOnWarnings =
  typeof shouldWebGPUCTSFailOnWarnings !== 'undefined' && (await shouldWebGPUCTSFailOnWarnings);

  const loader = new DefaultTestFileLoader();
  const qs = new URLSearchParams(window.location.search).getAll('q');
  assert(qs.length === 1, 'currently, there must be exactly one ?q=');
  const filterQuery = parseQuery(qs[0]);
  const testcases = await loader.loadCases(filterQuery);

  const expectations =
  typeof loadWebGPUExpectations !== 'undefined' ?
  parseExpectationsForTestQuery(
    await loadWebGPUExpectations,
    filterQuery,
    new URL(window.location.href)
  ) :
  [];

  const log = new Logger();

  for (const testcase of testcases) {
    const name = testcase.query.toString();
    // For brevity, display the case name "relative" to the ?q= path.
    const shortName = relativeQueryString(filterQuery, testcase.query) || '(case)';

    const wpt_fn = async () => {
      const [rec, res] = log.record(name);
      if (dedicatedWorker) {
        await dedicatedWorker.run(rec, name, expectations);
      } else if (sharedWorker) {
        await sharedWorker.run(rec, name, expectations);
      } else if (serviceWorker) {
        await serviceWorker.run(rec, name, expectations);
      } else {
        await testcase.run(rec, expectations);
      }

      // Unfortunately, it seems not possible to surface any logs for warn/skip.
      if (res.status === 'fail' || res.status === 'warn' && failOnWarnings) {
        const logs = (res.logs ?? []).map(prettyPrintLog);
        assert_unreached('\n' + logs.join('\n') + '\n');
      }
    };

    promise_test(wpt_fn, shortName);
  }

  done();
})();
});
__define('common/framework/test_config.js', function (__exports) {
'use strict';
__get(__exports, 'globalTestConfig', () => globalTestConfig);
__get(__exports, 'isCompatibilityMode', () => isCompatibilityMode);
__get(__exports, 'isCompatibilityDevice', () => isCompatibilityDevice);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert, hasFeature; __bind('common/util/util.js', (__m) => { ({ assert, hasFeature } = __m); });



































































/** Test configuration options. Globally modifiable global state. */
const globalTestConfig = {
  enableDebugLogs: false,
  maxSubcasesInFlight: 100,
  subcasesBetweenAttemptingGC: 5000,
  casesBetweenReplacingDevice: Infinity,
  testHeartbeatCallback: () => {},
  noRaceWithRejectOnTimeout: false,
  unrollConstEvalLoops: false,
  compatibility: false,
  forceFallbackAdapter: false,
  enforceDefaultLimits: false,
  blockAllFeatures: false,
  logToWebSocket: false
};

// Check if features has 'core-features-and-limits'.
// Note: The CTS generally, requires that if globalTestConfig.compatibility
// is true then the device MUST be a compatibility device since the CTS
// is trying to test that compatibility devices have the correct validation.
function isCompatibilityMode(features) {
  if (globalTestConfig.compatibility) {
    assert(!hasFeature(features, 'core-features-and-limits'));
  }
  return globalTestConfig.compatibility;
}

// Check if a device is a compatibility device.
// Note: The CTS generally, requires that if globalTestConfig.compatibility
// is true then the device MUST be a compatibility device since the CTS
// is trying to test that compatibility devices have the correct validation.
function isCompatibilityDevice(device) {
  return isCompatibilityMode(device.features);
}
});
__define('common/util/util.js', function (__exports) {
'use strict';
__get(__exports, 'ErrorWithExtra', () => ErrorWithExtra);
__get(__exports, 'assert', () => assert);
__get(__exports, 'assertOK', () => assertOK);
__get(__exports, 'assertReject', () => assertReject);
__get(__exports, 'unreachable', () => unreachable);
__get(__exports, 'skipTestCase', () => skipTestCase);
__get(__exports, 'now', () => now);
__get(__exports, 'resolveOnTimeout', () => resolveOnTimeout);
__get(__exports, 'PromiseTimeoutError', () => PromiseTimeoutError);
__get(__exports, 'rejectOnTimeout', () => rejectOnTimeout);
__get(__exports, 'raceWithRejectOnTimeout', () => raceWithRejectOnTimeout);
__get(__exports, 'assertNotSettledWithinTime', () => assertNotSettledWithinTime);
__get(__exports, 'rejectWithoutUncaught', () => rejectWithoutUncaught);
__get(__exports, 'isPlainObject', () => isPlainObject);
__get(__exports, 'sortObjectByKey', () => sortObjectByKey);
__get(__exports, 'objectEquals', () => objectEquals);
__get(__exports, 'range', () => range);
__get(__exports, 'iterRange', () => iterRange);
__get(__exports, 'mapLazy', () => mapLazy);
__get(__exports, 'count', () => count);
__get(__exports, 'kReorderOrderKeys', () => kReorderOrderKeys);
__get(__exports, 'shiftByHalf', () => shiftByHalf);
__get(__exports, 'reorder', () => reorder);
__get(__exports, 'typedEntries', () => typedEntries);
__get(__exports, 'kTypedArrayBufferViews', () => kTypedArrayBufferViews);
__get(__exports, 'kTypedArrayBufferViewKeys', () => kTypedArrayBufferViewKeys);
__get(__exports, 'kTypedArrayBufferViewConstructors', () => kTypedArrayBufferViewConstructors);
__get(__exports, 'typedArrayParam', () => typedArrayParam);
__get(__exports, 'createTypedArray', () => createTypedArray);
__get(__exports, 'typedArrayFromParam', () => typedArrayFromParam);
__get(__exports, 'memcpy', () => memcpy);
__get(__exports, 'filterUniqueValueTestVariants', () => filterUniqueValueTestVariants);
__get(__exports, 'makeValueTestVariant', () => makeValueTestVariant);
__get(__exports, 'hasFeature', () => hasFeature);
__get(__exports, 'combinationsOfOneOrTwoUsages', () => combinationsOfOneOrTwoUsages);
__get(__exports, 'supportsImmediateData', () => supportsImmediateData);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let Float16Array; __bind('external/petamoriken/float16/float16.js', (__m) => { ({ Float16Array } = __m); });let SkipTestCase; __bind('common/framework/fixture.js', (__m) => { ({ SkipTestCase } = __m); });let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });

let keysOf; __bind('common/util/data_tables.js', (__m) => { ({ keysOf } = __m); });
let timeout; __bind('common/util/timeout.js', (__m) => { ({ timeout } = __m); });

/**
 * Error with arbitrary `extra` data attached, for debugging.
 * The extra data is omitted if not running the test in debug mode (`?debug=1`).
 */
class ErrorWithExtra extends Error {


  /**
   * `extra` function is only called if in debug mode.
   * If an `ErrorWithExtra` is passed, its message is used and its extras are passed through.
   */


  constructor(baseOrMessage, newExtra) {
    const message = typeof baseOrMessage === 'string' ? baseOrMessage : baseOrMessage.message;
    super(message);

    const oldExtras = baseOrMessage instanceof ErrorWithExtra ? baseOrMessage.extra : {};
    this.extra = globalTestConfig.enableDebugLogs ?
    { ...oldExtras, ...newExtra() } :
    { omitted: 'pass ?debug=1' };
  }
}

/**
 * Asserts `condition` is true. Otherwise, throws an `Error` with the provided message.
 */
function assert(condition, msg) {
  if (!condition) {
    throw new Error(msg && (typeof msg === 'string' ? msg : msg()));
  }
}

/** If the argument is an Error, throw it. Otherwise, pass it back. */
function assertOK(value) {
  if (value instanceof Error) {
    throw value;
  }
  return value;
}

/** Options for assertReject, shouldReject, and friends. */


/**
 * Resolves if the provided promise rejects; rejects if it does not.
 */
async function assertReject(
expectedName,
p,
{ allowMissingStack = false, message } = {})
{
  await p.then(
    () => {
      unreachable(message);
    },
    (ex) => {
      assert(ex instanceof Error, 'rejected with a non-Error object');
      assert(ex.name === expectedName, `rejected with name ${ex.name} instead of ${expectedName}`);
      // Asserted as expected
      if (!allowMissingStack) {
        const m = message ? ` (${message})` : '';
        assert(typeof ex.stack === 'string', 'threw as expected, but missing stack' + m);
      }
    }
  );
}

/**
 * Assert this code is unreachable. Unconditionally throws an `Error`.
 */
function unreachable(msg) {
  throw new Error(msg);
}

/**
 * Throw a `SkipTestCase` exception, which skips the test case.
 */
function skipTestCase(msg) {
  throw new SkipTestCase(msg);
}

/**
 * The `performance` interface.
 * It is available in all browsers, but it is not in scope by default in Node.
 */

const perf = typeof performance !== 'undefined' ? performance : require('perf_hooks').performance;

/**
 * Calls the appropriate `performance.now()` depending on whether running in a browser or Node.
 */
function now() {
  return perf.now();
}

/**
 * Returns a promise which resolves after the specified time.
 */
function resolveOnTimeout(ms) {
  return new Promise((resolve) => {
    timeout(() => {
      resolve();
    }, ms);
  });
}

class PromiseTimeoutError extends Error {}

/**
 * Returns a promise which rejects after the specified time.
 */
function rejectOnTimeout(ms, msg) {
  return new Promise((_resolve, reject) => {
    timeout(() => {
      reject(new PromiseTimeoutError(msg));
    }, ms);
  });
}

/**
 * Takes a promise `p`, and returns a new one which rejects if `p` takes too long,
 * and otherwise passes the result through.
 */
function raceWithRejectOnTimeout(p, ms, msg) {
  if (globalTestConfig.noRaceWithRejectOnTimeout) {
    return p;
  }
  // Setup a promise that will reject after `ms` milliseconds. We cancel this timeout when
  // `p` is finalized, so the JavaScript VM doesn't hang around waiting for the timer to
  // complete, once the test runner has finished executing the tests.
  const timeoutPromise = new Promise((_resolve, reject) => {
    const handle = timeout(() => {
      reject(new PromiseTimeoutError(msg));
    }, ms);
    p = p.finally(() => clearTimeout(handle));
  });
  return Promise.race([p, timeoutPromise]);
}

/**
 * Takes a promise `p` and returns a new one which rejects if `p` resolves or rejects,
 * and otherwise resolves after the specified time.
 */
function assertNotSettledWithinTime(
p,
ms,
msg)
{
  // Rejects regardless of whether p resolves or rejects.
  const rejectWhenSettled = p.then(() => Promise.reject(new Error(msg)));
  // Resolves after `ms` milliseconds.
  const timeoutPromise = new Promise((resolve) => {
    const handle = timeout(() => {
      resolve(undefined);
    }, ms);
    void p.finally(() => clearTimeout(handle));
  });
  return Promise.race([rejectWhenSettled, timeoutPromise]);
}

/**
 * Returns a `Promise.reject()`, but also registers a dummy `.catch()` handler so it doesn't count
 * as an uncaught promise rejection in the runtime.
 */
function rejectWithoutUncaught(err) {
  const p = Promise.reject(err);
  // Suppress uncaught promise rejection.
  p.catch(() => {});
  return p;
}

/**
 * Returns true if v is a plain JavaScript object.
 */
function isPlainObject(v) {
  return !!v && Object.getPrototypeOf(v).constructor === Object.prototype.constructor;
}

/**
 * Makes a copy of a JS `object`, with the keys reordered into sorted order.
 */
function sortObjectByKey(v) {
  const sortedObject = {};
  for (const k of Object.keys(v).sort()) {
    sortedObject[k] = v[k];
  }
  return sortedObject;
}

/**
 * Determines whether two JS values are equal, recursing into objects and arrays.
 * NaN is treated specially, such that `objectEquals(NaN, NaN)`. +/-0.0 are treated as equal
 * by default, but can be opted to be distinguished.
 * @param x the first JS values that get compared
 * @param y the second JS values that get compared
 * @param distinguishSignedZero if set to true, treat 0.0 and -0.0 as unequal. Default to false.
 */
function objectEquals(
x,
y,
distinguishSignedZero = false)
{
  if (typeof x !== 'object' || typeof y !== 'object') {
    if (typeof x === 'number' && typeof y === 'number' && Number.isNaN(x) && Number.isNaN(y)) {
      return true;
    }
    // Object.is(0.0, -0.0) is false while (0.0 === -0.0) is true. Other than +/-0.0 and NaN cases,
    // Object.is works in the same way as ===.
    return distinguishSignedZero ? Object.is(x, y) : x === y;
  }
  if (x === null || y === null) return x === y;
  if (x.constructor !== y.constructor) return false;
  if (x instanceof Function) return x === y;
  if (x instanceof RegExp) return x === y;
  if (x === y || x.valueOf() === y.valueOf()) return true;
  if (Array.isArray(x) && Array.isArray(y) && x.length !== y.length) return false;
  if (x instanceof Date) return false;
  if (!(x instanceof Object)) return false;
  if (!(y instanceof Object)) return false;

  const x1 = x;
  const y1 = y;
  const p = Object.keys(x);
  return Object.keys(y).every((i) => p.indexOf(i) !== -1) && p.every((i) => objectEquals(x1[i], y1[i]));
}

/**
 * Generates a range of values `fn(0)..fn(n-1)`.
 */
function range(n, fn) {
  return [...new Array(n)].map((_, i) => fn(i));
}

/**
 * Generates a range of values `fn(0)..fn(n-1)`.
 */
function* iterRange(n, fn) {
  for (let i = 0; i < n; ++i) {
    yield fn(i);
  }
}

/** Creates a (reusable) iterable object that maps `f` over `xs`, lazily. */
function mapLazy(xs, f) {
  return {
    *[Symbol.iterator]() {
      for (const x of xs) {
        yield f(x);
      }
    }
  };
}

/** Count the number of elements `x` for which `predicate(x)` is true. */
function count(xs, predicate) {
  let count = 0;
  for (const x of xs) {
    if (predicate(x)) count++;
  }
  return count;
}

const ReorderOrders = {
  forward: true,
  backward: true,
  shiftByHalf: true
};

const kReorderOrderKeys = keysOf(ReorderOrders);

/**
 * Creates a new array from the given array with the first half
 * swapped with the last half.
 */
function shiftByHalf(arr) {
  const len = arr.length;
  const half = len / 2 | 0;
  const firstHalf = arr.splice(0, half);
  return [...arr, ...firstHalf];
}

/**
 * Creates a reordered array from the input array based on the Order
 */
function reorder(order, arr) {
  switch (order) {
    case 'forward':
      return arr.slice();
    case 'backward':
      return arr.slice().reverse();
    case 'shiftByHalf':{
        // should this be pseudo random?
        return shiftByHalf(arr);
      }
  }
}

/**
 * A typed version of Object.entries
 */

function typedEntries(obj) {
  // The cast is done once, inside the helper function,
  // keeping the call site clean and type-safe.
  return Object.entries(obj);
}

const TypedArrayBufferViewInstances = [
new Uint8Array(),
new Uint8ClampedArray(),
new Uint16Array(),
new Uint32Array(),
new Int8Array(),
new Int16Array(),
new Int32Array(),
new Float16Array(),
new Float32Array(),
new Float64Array(),
new BigInt64Array(),
new BigUint64Array()];
























const kTypedArrayBufferViews =

{
  ...(() => {

    const result = {};
    for (const v of TypedArrayBufferViewInstances) {
      result[v.constructor.name] = v.constructor;
    }
    return result;
  })()
};
const kTypedArrayBufferViewKeys = keysOf(kTypedArrayBufferViews);
const kTypedArrayBufferViewConstructors = Object.values(kTypedArrayBufferViews);





















/**
 * Creates a case parameter for a typedarray.
 *
 * You can't put typedarrays in case parameters directly so instead of
 *
 * ```
 * u.combine('data', [
 *   new Uint8Array([1, 2, 3]),
 *   new Float32Array([4, 5, 6]),
 * ])
 * ```
 *
 * You can use
 *
 * ```
 * u.combine('data', [
 *   typedArrayParam('Uint8Array' [1, 2, 3]),
 *   typedArrayParam('Float32Array' [4, 5, 6]),
 * ])
 * ```
 *
 * and then convert the params to typedarrays eg.
 *
 * ```
 *  .fn(t => {
 *    const data = t.params.data.map(v => typedArrayFromParam(v));
 *  })
 * ```
 */
function typedArrayParam(
type,
data)
{
  return { type, data };
}

function createTypedArray(
type,
data)
{
  return new kTypedArrayBufferViews[type](data);
}

/**
 * Converts a TypedArrayParam to a typedarray. See typedArrayParam
 */
function typedArrayFromParam(
param)
{
  const { type, data } = param;
  return createTypedArray(type, data);
}

function subarrayAsU8(
buf,
{ start = 0, length })
{
  if (buf instanceof ArrayBuffer) {
    return new Uint8Array(buf, start, length);
  } else if (buf instanceof Uint8Array || buf instanceof Uint8ClampedArray) {
    // Don't wrap in new views if we don't need to.
    if (start === 0 && (length === undefined || length === buf.byteLength)) {
      return buf;
    }
  }
  const byteOffset = buf.byteOffset + start * buf.BYTES_PER_ELEMENT;
  const byteLength =
  length !== undefined ?
  length * buf.BYTES_PER_ELEMENT :
  buf.byteLength - (byteOffset - buf.byteOffset);
  return new Uint8Array(buf.buffer, byteOffset, byteLength);
}

/**
 * Copy a range of bytes from one ArrayBuffer or TypedArray to another.
 *
 * `start`/`length` are in elements (or in bytes, if ArrayBuffer).
 */
function memcpy(
src,
dst)
{
  subarrayAsU8(dst.dst, dst).set(subarrayAsU8(src.src, src));
}

/**
 * Used to create a value that is specified by multiplying some runtime value
 * by a constant and then adding a constant to it.
 */





/**
 * Filters out SpecValues that are the same.
 */
function filterUniqueValueTestVariants(valueTestVariants) {
  return new Map(
    valueTestVariants.map((v) => [`m:${v.mult},a:${v.add}`, v])
  ).values();
}

/**
 * Used to create a value that is specified by multiplied some runtime value
 * by a constant and then adding a constant to it. This happens often in test
 * with limits that can only be known at runtime and yet we need a way to
 * add parameters to a test and those parameters must be constants.
 */
function makeValueTestVariant(base, variant) {
  return base * variant.mult + variant.add;
}

/**
 * Use instead of features.has because feature's has takes any string
 * and we want to prevent typos.
 */
function hasFeature(features, feature) {

  return features.has(feature);
}

/** Convenience helper for combinations of 1-2 usage bits from a list of usage bits. */
function combinationsOfOneOrTwoUsages(usages) {
  const combinations = [];
  for (const usage0 of usages) {
    for (const usage1 of usages) {
      if (usage0 <= usage1) {
        combinations.push(usage0 | usage1);
      }
    }
  }
  return combinations;
}

/**
 * Checks if the browser supports immediate data (experimental).
 *
 * Checks for:
 * - `setImmediates` method on `GPURenderPassEncoder`, `GPUComputePassEncoder`, or `GPURenderBundleEncoder` prototypes.
 * - `maxImmediateSize` property on `GPUSupportedLimits` prototype.
 * - `immediate_address_space` feature in `gpu.wgslLanguageFeatures`.
 *
 * This helper is used to skip tests when the environment does not support immediate data functionality.
 */
function supportsImmediateData(gpu) {
  return (
    'setImmediates' in GPURenderPassEncoder.prototype ||
    'setImmediates' in GPUComputePassEncoder.prototype ||
    'setImmediates' in GPURenderBundleEncoder.prototype ||
    'maxImmediateSize' in GPUSupportedLimits.prototype ||
    gpu.wgslLanguageFeatures.has('immediate_address_space'));

}
});
__define('external/petamoriken/float16/float16.js', function (__exports) {
'use strict';
__get(__exports, 'Float16Array', () => Float16Array);
__get(__exports, 'f16round', () => f16round);
__get(__exports, 'getFloat16', () => getFloat16);
__get(__exports, 'hfround', () => f16round);
__get(__exports, 'isFloat16Array', () => isFloat16Array);
__get(__exports, 'isTypedArray', () => isTypedArray);
__get(__exports, 'setFloat16', () => setFloat16);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /*! @petamoriken/float16 v3.8.6 | MIT License - https://github.com/petamoriken/float16 */const THIS_IS_NOT_AN_OBJECT = "This is not an object";
const THIS_IS_NOT_A_FLOAT16ARRAY_OBJECT = "This is not a Float16Array object";
const THIS_CONSTRUCTOR_IS_NOT_A_SUBCLASS_OF_FLOAT16ARRAY =
"This constructor is not a subclass of Float16Array";
const THE_CONSTRUCTOR_PROPERTY_VALUE_IS_NOT_AN_OBJECT =
"The constructor property value is not an object";
const SPECIES_CONSTRUCTOR_DIDNT_RETURN_TYPEDARRAY_OBJECT =
"Species constructor didn't return TypedArray object";
const DERIVED_CONSTRUCTOR_CREATED_TYPEDARRAY_OBJECT_WHICH_WAS_TOO_SMALL_LENGTH =
"Derived constructor created TypedArray object which was too small length";
const ATTEMPTING_TO_ACCESS_DETACHED_ARRAYBUFFER =
"Attempting to access detached ArrayBuffer";
const CANNOT_CONVERT_UNDEFINED_OR_NULL_TO_OBJECT =
"Cannot convert undefined or null to object";
const CANNOT_MIX_BIGINT_AND_OTHER_TYPES =
"Cannot mix BigInt and other types, use explicit conversions";
const ITERATOR_PROPERTY_IS_NOT_CALLABLE = "@@iterator property is not callable";
const REDUCE_OF_EMPTY_ARRAY_WITH_NO_INITIAL_VALUE =
"Reduce of empty array with no initial value";
const THE_COMPARISON_FUNCTION_MUST_BE_EITHER_A_FUNCTION_OR_UNDEFINED =
"The comparison function must be either a function or undefined";
const OFFSET_IS_OUT_OF_BOUNDS = "Offset is out of bounds";

function uncurryThis(target) {
  return (thisArg, ...args) => {
    return ReflectApply(target, thisArg, args);
  };
}
function uncurryThisGetter(target, key) {
  return uncurryThis(
    ReflectGetOwnPropertyDescriptor(
      target,
      key
    ).get
  );
}
const {
  apply: ReflectApply,
  construct: ReflectConstruct,
  defineProperty: ReflectDefineProperty,
  get: ReflectGet,
  getOwnPropertyDescriptor: ReflectGetOwnPropertyDescriptor,
  getPrototypeOf: ReflectGetPrototypeOf,
  has: ReflectHas,
  ownKeys: ReflectOwnKeys,
  set: ReflectSet,
  setPrototypeOf: ReflectSetPrototypeOf
} = Reflect;
const NativeProxy = Proxy;
const {
  EPSILON,
  MAX_SAFE_INTEGER,
  isFinite: NumberIsFinite,
  isNaN: NumberIsNaN
} = Number;
const {
  iterator: SymbolIterator,
  species: SymbolSpecies,
  toStringTag: SymbolToStringTag,
  for: SymbolFor
} = Symbol;
const NativeObject = Object;
const {
  create: ObjectCreate,
  defineProperty: ObjectDefineProperty,
  freeze: ObjectFreeze,
  is: ObjectIs
} = NativeObject;
const ObjectPrototype = NativeObject.prototype;
const ObjectPrototype__lookupGetter__ = ObjectPrototype.__lookupGetter__ ?
uncurryThis(ObjectPrototype.__lookupGetter__) :
(object, key) => {
  if (object == null) {
    throw NativeTypeError(
      CANNOT_CONVERT_UNDEFINED_OR_NULL_TO_OBJECT
    );
  }
  let target = NativeObject(object);
  do {
    const descriptor = ReflectGetOwnPropertyDescriptor(target, key);
    if (descriptor !== undefined) {
      if (ObjectHasOwn(descriptor, "get")) {
        return descriptor.get;
      }
      return;
    }
  } while ((target = ReflectGetPrototypeOf(target)) !== null);
};
const ObjectHasOwn = NativeObject.hasOwn ||
uncurryThis(ObjectPrototype.hasOwnProperty);
const NativeArray = Array;
const ArrayIsArray = NativeArray.isArray;
const ArrayPrototype = NativeArray.prototype;
const ArrayPrototypeJoin = uncurryThis(ArrayPrototype.join);
const ArrayPrototypePush = uncurryThis(ArrayPrototype.push);
const ArrayPrototypeToLocaleString = uncurryThis(
  ArrayPrototype.toLocaleString
);
const NativeArrayPrototypeSymbolIterator = ArrayPrototype[SymbolIterator];
const ArrayPrototypeSymbolIterator = uncurryThis(NativeArrayPrototypeSymbolIterator);
const {
  abs: MathAbs,
  trunc: MathTrunc
} = Math;
const NativeArrayBuffer = ArrayBuffer;
const ArrayBufferIsView = NativeArrayBuffer.isView;
const ArrayBufferPrototype = NativeArrayBuffer.prototype;
const ArrayBufferPrototypeSlice = uncurryThis(ArrayBufferPrototype.slice);
const ArrayBufferPrototypeGetByteLength = uncurryThisGetter(ArrayBufferPrototype, "byteLength");
const NativeSharedArrayBuffer = typeof SharedArrayBuffer !== "undefined" ? SharedArrayBuffer : null;
const SharedArrayBufferPrototypeGetByteLength = NativeSharedArrayBuffer &&
uncurryThisGetter(NativeSharedArrayBuffer.prototype, "byteLength");
const TypedArray = ReflectGetPrototypeOf(Uint8Array);
const TypedArrayFrom = TypedArray.from;
const TypedArrayPrototype = TypedArray.prototype;
const NativeTypedArrayPrototypeSymbolIterator = TypedArrayPrototype[SymbolIterator];
const TypedArrayPrototypeKeys = uncurryThis(TypedArrayPrototype.keys);
const TypedArrayPrototypeValues = uncurryThis(
  TypedArrayPrototype.values
);
const TypedArrayPrototypeEntries = uncurryThis(
  TypedArrayPrototype.entries
);
const TypedArrayPrototypeSet = uncurryThis(TypedArrayPrototype.set);
const TypedArrayPrototypeReverse = uncurryThis(
  TypedArrayPrototype.reverse
);
const TypedArrayPrototypeFill = uncurryThis(TypedArrayPrototype.fill);
const TypedArrayPrototypeCopyWithin = uncurryThis(
  TypedArrayPrototype.copyWithin
);
const TypedArrayPrototypeSort = uncurryThis(TypedArrayPrototype.sort);
const TypedArrayPrototypeSlice = uncurryThis(TypedArrayPrototype.slice);
const TypedArrayPrototypeSubarray = uncurryThis(
  TypedArrayPrototype.subarray
);
const TypedArrayPrototypeGetBuffer = uncurryThisGetter(
  TypedArrayPrototype,
  "buffer"
);
const TypedArrayPrototypeGetByteOffset = uncurryThisGetter(
  TypedArrayPrototype,
  "byteOffset"
);
const TypedArrayPrototypeGetLength = uncurryThisGetter(
  TypedArrayPrototype,
  "length"
);
const TypedArrayPrototypeGetSymbolToStringTag = uncurryThisGetter(
  TypedArrayPrototype,
  SymbolToStringTag
);
const NativeUint8Array = Uint8Array;
const NativeUint16Array = Uint16Array;
const Uint16ArrayFrom = (...args) => {
  return ReflectApply(TypedArrayFrom, NativeUint16Array, args);
};
const NativeUint32Array = Uint32Array;
const NativeFloat32Array = Float32Array;
const ArrayIteratorPrototype = ReflectGetPrototypeOf([][SymbolIterator]());
const ArrayIteratorPrototypeNext = uncurryThis(ArrayIteratorPrototype.next);
const GeneratorPrototypeNext = uncurryThis(function* () {}().next);
const IteratorPrototype = ReflectGetPrototypeOf(ArrayIteratorPrototype);
const DataViewPrototype = DataView.prototype;
const DataViewPrototypeGetUint16 = uncurryThis(
  DataViewPrototype.getUint16
);
const DataViewPrototypeSetUint16 = uncurryThis(
  DataViewPrototype.setUint16
);
const NativeTypeError = TypeError;
const NativeRangeError = RangeError;
const NativeWeakSet = WeakSet;
const WeakSetPrototype = NativeWeakSet.prototype;
const WeakSetPrototypeAdd = uncurryThis(WeakSetPrototype.add);
const WeakSetPrototypeHas = uncurryThis(WeakSetPrototype.has);
const NativeWeakMap = WeakMap;
const WeakMapPrototype = NativeWeakMap.prototype;
const WeakMapPrototypeGet = uncurryThis(WeakMapPrototype.get);
const WeakMapPrototypeHas = uncurryThis(WeakMapPrototype.has);
const WeakMapPrototypeSet = uncurryThis(WeakMapPrototype.set);

const arrayIterators = new NativeWeakMap();
const SafeIteratorPrototype = ObjectCreate(null, {
  next: {
    value: function next() {
      const arrayIterator = WeakMapPrototypeGet(arrayIterators, this);
      return ArrayIteratorPrototypeNext(arrayIterator);
    }
  },
  [SymbolIterator]: {
    value: function values() {
      return this;
    }
  }
});
function safeIfNeeded(array) {
  if (
  array[SymbolIterator] === NativeArrayPrototypeSymbolIterator &&
  ArrayIteratorPrototype.next === ArrayIteratorPrototypeNext)
  {
    return array;
  }
  const safe = ObjectCreate(SafeIteratorPrototype);
  WeakMapPrototypeSet(arrayIterators, safe, ArrayPrototypeSymbolIterator(array));
  return safe;
}
const generators = new NativeWeakMap();
const DummyArrayIteratorPrototype = ObjectCreate(IteratorPrototype, {
  next: {
    value: function next() {
      const generator = WeakMapPrototypeGet(generators, this);
      return GeneratorPrototypeNext(generator);
    },
    writable: true,
    configurable: true
  }
});
for (const key of ReflectOwnKeys(ArrayIteratorPrototype)) {
  if (key === "next") {
    continue;
  }
  ObjectDefineProperty(DummyArrayIteratorPrototype, key, ReflectGetOwnPropertyDescriptor(ArrayIteratorPrototype, key));
}
function wrap(generator) {
  const dummy = ObjectCreate(DummyArrayIteratorPrototype);
  WeakMapPrototypeSet(generators, dummy, generator);
  return dummy;
}

function isObject(value) {
  return (
    value !== null && typeof value === "object" ||
    typeof value === "function");

}
function isObjectLike(value) {
  return value !== null && typeof value === "object";
}
function isNativeTypedArray(value) {
  return TypedArrayPrototypeGetSymbolToStringTag(value) !== undefined;
}
function isNativeBigIntTypedArray(value) {
  const typedArrayName = TypedArrayPrototypeGetSymbolToStringTag(value);
  return (
    typedArrayName === "BigInt64Array" ||
    typedArrayName === "BigUint64Array");

}
function isArrayBuffer(value) {
  try {
    if (ArrayIsArray(value)) {
      return false;
    }
    ArrayBufferPrototypeGetByteLength(value);
    return true;
  } catch (e) {
    return false;
  }
}
function isSharedArrayBuffer(value) {
  if (NativeSharedArrayBuffer === null) {
    return false;
  }
  try {
    SharedArrayBufferPrototypeGetByteLength(value);
    return true;
  } catch (e) {
    return false;
  }
}
function isAnyArrayBuffer(value) {
  return isArrayBuffer(value) || isSharedArrayBuffer(value);
}
function isOrdinaryArray(value) {
  if (!ArrayIsArray(value)) {
    return false;
  }
  return (
    value[SymbolIterator] === NativeArrayPrototypeSymbolIterator &&
    ArrayIteratorPrototype.next === ArrayIteratorPrototypeNext);

}
function isOrdinaryNativeTypedArray(value) {
  if (!isNativeTypedArray(value)) {
    return false;
  }
  return (
    value[SymbolIterator] === NativeTypedArrayPrototypeSymbolIterator &&
    ArrayIteratorPrototype.next === ArrayIteratorPrototypeNext);

}
function isCanonicalIntegerIndexString(value) {
  if (typeof value !== "string") {
    return false;
  }
  const number = +value;
  if (value !== number + "") {
    return false;
  }
  if (!NumberIsFinite(number)) {
    return false;
  }
  return number === MathTrunc(number);
}

const brand = SymbolFor("__Float16Array__");
function hasFloat16ArrayBrand(target) {
  if (!isObjectLike(target)) {
    return false;
  }
  const prototype = ReflectGetPrototypeOf(target);
  if (!isObjectLike(prototype)) {
    return false;
  }
  const constructor = prototype.constructor;
  if (constructor === undefined) {
    return false;
  }
  if (!isObject(constructor)) {
    throw NativeTypeError(THE_CONSTRUCTOR_PROPERTY_VALUE_IS_NOT_AN_OBJECT);
  }
  return ReflectHas(constructor, brand);
}

const INVERSE_OF_EPSILON = 1 / EPSILON;
function roundTiesToEven(num) {
  return num + INVERSE_OF_EPSILON - INVERSE_OF_EPSILON;
}
const FLOAT16_MIN_VALUE = 6.103515625e-05;
const FLOAT16_MAX_VALUE = 65504;
const FLOAT16_EPSILON = 0.0009765625;
const FLOAT16_EPSILON_MULTIPLIED_BY_FLOAT16_MIN_VALUE = FLOAT16_EPSILON * FLOAT16_MIN_VALUE;
const FLOAT16_EPSILON_DEVIDED_BY_EPSILON = FLOAT16_EPSILON * INVERSE_OF_EPSILON;
function roundToFloat16(num) {
  const number = +num;
  if (!NumberIsFinite(number) || number === 0) {
    return number;
  }
  const sign = number > 0 ? 1 : -1;
  const absolute = MathAbs(number);
  if (absolute < FLOAT16_MIN_VALUE) {
    return sign * roundTiesToEven(absolute / FLOAT16_EPSILON_MULTIPLIED_BY_FLOAT16_MIN_VALUE) * FLOAT16_EPSILON_MULTIPLIED_BY_FLOAT16_MIN_VALUE;
  }
  const temp = (1 + FLOAT16_EPSILON_DEVIDED_BY_EPSILON) * absolute;
  const result = temp - (temp - absolute);
  if (result > FLOAT16_MAX_VALUE || NumberIsNaN(result)) {
    return sign * Infinity;
  }
  return sign * result;
}
const buffer = new NativeArrayBuffer(4);
const floatView = new NativeFloat32Array(buffer);
const uint32View = new NativeUint32Array(buffer);
const baseTable = new NativeUint16Array(512);
const shiftTable = new NativeUint8Array(512);
for (let i = 0; i < 256; ++i) {
  const e = i - 127;
  if (e < -27) {
    baseTable[i] = 0x0000;
    baseTable[i | 0x100] = 0x8000;
    shiftTable[i] = 24;
    shiftTable[i | 0x100] = 24;
  } else if (e < -14) {
    baseTable[i] = 0x0400 >> -e - 14;
    baseTable[i | 0x100] = 0x0400 >> -e - 14 | 0x8000;
    shiftTable[i] = -e - 1;
    shiftTable[i | 0x100] = -e - 1;
  } else if (e <= 15) {
    baseTable[i] = e + 15 << 10;
    baseTable[i | 0x100] = e + 15 << 10 | 0x8000;
    shiftTable[i] = 13;
    shiftTable[i | 0x100] = 13;
  } else if (e < 128) {
    baseTable[i] = 0x7c00;
    baseTable[i | 0x100] = 0xfc00;
    shiftTable[i] = 24;
    shiftTable[i | 0x100] = 24;
  } else {
    baseTable[i] = 0x7c00;
    baseTable[i | 0x100] = 0xfc00;
    shiftTable[i] = 13;
    shiftTable[i | 0x100] = 13;
  }
}
function roundToFloat16Bits(num) {
  floatView[0] = roundToFloat16(num);
  const f = uint32View[0];
  const e = f >> 23 & 0x1ff;
  return baseTable[e] + ((f & 0x007fffff) >> shiftTable[e]);
}
const mantissaTable = new NativeUint32Array(2048);
for (let i = 1; i < 1024; ++i) {
  let m = i << 13;
  let e = 0;
  while ((m & 0x00800000) === 0) {
    m <<= 1;
    e -= 0x00800000;
  }
  m &= ~0x00800000;
  e += 0x38800000;
  mantissaTable[i] = m | e;
}
for (let i = 1024; i < 2048; ++i) {
  mantissaTable[i] = 0x38000000 + (i - 1024 << 13);
}
const exponentTable = new NativeUint32Array(64);
for (let i = 1; i < 31; ++i) {
  exponentTable[i] = i << 23;
}
exponentTable[31] = 0x47800000;
exponentTable[32] = 0x80000000;
for (let i = 33; i < 63; ++i) {
  exponentTable[i] = 0x80000000 + (i - 32 << 23);
}
exponentTable[63] = 0xc7800000;
const offsetTable = new NativeUint16Array(64);
for (let i = 1; i < 64; ++i) {
  if (i !== 32) {
    offsetTable[i] = 1024;
  }
}
function convertToNumber(float16bits) {
  const i = float16bits >> 10;
  uint32View[0] = mantissaTable[offsetTable[i] + (float16bits & 0x3ff)] + exponentTable[i];
  return floatView[0];
}

function ToIntegerOrInfinity(target) {
  const number = +target;
  if (NumberIsNaN(number) || number === 0) {
    return 0;
  }
  return MathTrunc(number);
}
function ToLength(target) {
  const length = ToIntegerOrInfinity(target);
  if (length < 0) {
    return 0;
  }
  return length < MAX_SAFE_INTEGER ?
  length :
  MAX_SAFE_INTEGER;
}
function SpeciesConstructor(target, defaultConstructor) {
  if (!isObject(target)) {
    throw NativeTypeError(THIS_IS_NOT_AN_OBJECT);
  }
  const constructor = target.constructor;
  if (constructor === undefined) {
    return defaultConstructor;
  }
  if (!isObject(constructor)) {
    throw NativeTypeError(THE_CONSTRUCTOR_PROPERTY_VALUE_IS_NOT_AN_OBJECT);
  }
  const species = constructor[SymbolSpecies];
  if (species == null) {
    return defaultConstructor;
  }
  return species;
}
function IsDetachedBuffer(buffer) {
  if (isSharedArrayBuffer(buffer)) {
    return false;
  }
  try {
    ArrayBufferPrototypeSlice(buffer, 0, 0);
    return false;
  } catch (e) {}
  return true;
}
function defaultCompare(x, y) {
  const isXNaN = NumberIsNaN(x);
  const isYNaN = NumberIsNaN(y);
  if (isXNaN && isYNaN) {
    return 0;
  }
  if (isXNaN) {
    return 1;
  }
  if (isYNaN) {
    return -1;
  }
  if (x < y) {
    return -1;
  }
  if (x > y) {
    return 1;
  }
  if (x === 0 && y === 0) {
    const isXPlusZero = ObjectIs(x, 0);
    const isYPlusZero = ObjectIs(y, 0);
    if (!isXPlusZero && isYPlusZero) {
      return -1;
    }
    if (isXPlusZero && !isYPlusZero) {
      return 1;
    }
  }
  return 0;
}

const BYTES_PER_ELEMENT = 2;
const float16bitsArrays = new NativeWeakMap();
function isFloat16Array(target) {
  return WeakMapPrototypeHas(float16bitsArrays, target) ||
  !ArrayBufferIsView(target) && hasFloat16ArrayBrand(target);
}
function assertFloat16Array(target) {
  if (!isFloat16Array(target)) {
    throw NativeTypeError(THIS_IS_NOT_A_FLOAT16ARRAY_OBJECT);
  }
}
function assertSpeciesTypedArray(target, count) {
  const isTargetFloat16Array = isFloat16Array(target);
  const isTargetTypedArray = isNativeTypedArray(target);
  if (!isTargetFloat16Array && !isTargetTypedArray) {
    throw NativeTypeError(SPECIES_CONSTRUCTOR_DIDNT_RETURN_TYPEDARRAY_OBJECT);
  }
  if (typeof count === "number") {
    let length;
    if (isTargetFloat16Array) {
      const float16bitsArray = getFloat16BitsArray(target);
      length = TypedArrayPrototypeGetLength(float16bitsArray);
    } else {
      length = TypedArrayPrototypeGetLength(target);
    }
    if (length < count) {
      throw NativeTypeError(
        DERIVED_CONSTRUCTOR_CREATED_TYPEDARRAY_OBJECT_WHICH_WAS_TOO_SMALL_LENGTH
      );
    }
  }
  if (isNativeBigIntTypedArray(target)) {
    throw NativeTypeError(CANNOT_MIX_BIGINT_AND_OTHER_TYPES);
  }
}
function getFloat16BitsArray(float16) {
  const float16bitsArray = WeakMapPrototypeGet(float16bitsArrays, float16);
  if (float16bitsArray !== undefined) {
    const buffer = TypedArrayPrototypeGetBuffer(float16bitsArray);
    if (IsDetachedBuffer(buffer)) {
      throw NativeTypeError(ATTEMPTING_TO_ACCESS_DETACHED_ARRAYBUFFER);
    }
    return float16bitsArray;
  }
  const buffer = float16.buffer;
  if (IsDetachedBuffer(buffer)) {
    throw NativeTypeError(ATTEMPTING_TO_ACCESS_DETACHED_ARRAYBUFFER);
  }
  const cloned = ReflectConstruct(Float16Array, [
  buffer,
  float16.byteOffset,
  float16.length],
  float16.constructor);
  return WeakMapPrototypeGet(float16bitsArrays, cloned);
}
function copyToArray(float16bitsArray) {
  const length = TypedArrayPrototypeGetLength(float16bitsArray);
  const array = [];
  for (let i = 0; i < length; ++i) {
    array[i] = convertToNumber(float16bitsArray[i]);
  }
  return array;
}
const TypedArrayPrototypeGetters = new NativeWeakSet();
for (const key of ReflectOwnKeys(TypedArrayPrototype)) {
  if (key === SymbolToStringTag) {
    continue;
  }
  const descriptor = ReflectGetOwnPropertyDescriptor(TypedArrayPrototype, key);
  if (ObjectHasOwn(descriptor, "get") && typeof descriptor.get === "function") {
    WeakSetPrototypeAdd(TypedArrayPrototypeGetters, descriptor.get);
  }
}
const handler = ObjectFreeze({
  get(target, key, receiver) {
    if (isCanonicalIntegerIndexString(key) && ObjectHasOwn(target, key)) {
      return convertToNumber(ReflectGet(target, key));
    }
    if (WeakSetPrototypeHas(TypedArrayPrototypeGetters, ObjectPrototype__lookupGetter__(target, key))) {
      return ReflectGet(target, key);
    }
    return ReflectGet(target, key, receiver);
  },
  set(target, key, value, receiver) {
    if (isCanonicalIntegerIndexString(key) && ObjectHasOwn(target, key)) {
      return ReflectSet(target, key, roundToFloat16Bits(value));
    }
    return ReflectSet(target, key, value, receiver);
  },
  getOwnPropertyDescriptor(target, key) {
    if (isCanonicalIntegerIndexString(key) && ObjectHasOwn(target, key)) {
      const descriptor = ReflectGetOwnPropertyDescriptor(target, key);
      descriptor.value = convertToNumber(descriptor.value);
      return descriptor;
    }
    return ReflectGetOwnPropertyDescriptor(target, key);
  },
  defineProperty(target, key, descriptor) {
    if (
    isCanonicalIntegerIndexString(key) &&
    ObjectHasOwn(target, key) &&
    ObjectHasOwn(descriptor, "value"))
    {
      descriptor.value = roundToFloat16Bits(descriptor.value);
      return ReflectDefineProperty(target, key, descriptor);
    }
    return ReflectDefineProperty(target, key, descriptor);
  }
});
class Float16Array {
  constructor(input, _byteOffset, _length) {
    let float16bitsArray;
    if (isFloat16Array(input)) {
      float16bitsArray = ReflectConstruct(NativeUint16Array, [getFloat16BitsArray(input)], new.target);
    } else if (isObject(input) && !isAnyArrayBuffer(input)) {
      let list;
      let length;
      if (isNativeTypedArray(input)) {
        list = input;
        length = TypedArrayPrototypeGetLength(input);
        const buffer = TypedArrayPrototypeGetBuffer(input);
        if (IsDetachedBuffer(buffer)) {
          throw NativeTypeError(ATTEMPTING_TO_ACCESS_DETACHED_ARRAYBUFFER);
        }
        if (isNativeBigIntTypedArray(input)) {
          throw NativeTypeError(CANNOT_MIX_BIGINT_AND_OTHER_TYPES);
        }
        const data = new NativeArrayBuffer(
          length * BYTES_PER_ELEMENT
        );
        float16bitsArray = ReflectConstruct(NativeUint16Array, [data], new.target);
      } else {
        const iterator = input[SymbolIterator];
        if (iterator != null && typeof iterator !== "function") {
          throw NativeTypeError(ITERATOR_PROPERTY_IS_NOT_CALLABLE);
        }
        if (iterator != null) {
          if (isOrdinaryArray(input)) {
            list = input;
            length = input.length;
          } else {
            list = [...input];
            length = list.length;
          }
        } else {
          list = input;
          length = ToLength(list.length);
        }
        float16bitsArray = ReflectConstruct(NativeUint16Array, [length], new.target);
      }
      for (let i = 0; i < length; ++i) {
        float16bitsArray[i] = roundToFloat16Bits(list[i]);
      }
    } else {
      float16bitsArray = ReflectConstruct(NativeUint16Array, arguments, new.target);
    }
    const proxy = new NativeProxy(float16bitsArray, handler);
    WeakMapPrototypeSet(float16bitsArrays, proxy, float16bitsArray);
    return proxy;
  }
  static from(src, ...opts) {
    const Constructor = this;
    if (!ReflectHas(Constructor, brand)) {
      throw NativeTypeError(
        THIS_CONSTRUCTOR_IS_NOT_A_SUBCLASS_OF_FLOAT16ARRAY
      );
    }
    if (Constructor === Float16Array) {
      if (isFloat16Array(src) && opts.length === 0) {
        const float16bitsArray = getFloat16BitsArray(src);
        const uint16 = new NativeUint16Array(
          TypedArrayPrototypeGetBuffer(float16bitsArray),
          TypedArrayPrototypeGetByteOffset(float16bitsArray),
          TypedArrayPrototypeGetLength(float16bitsArray)
        );
        return new Float16Array(
          TypedArrayPrototypeGetBuffer(TypedArrayPrototypeSlice(uint16))
        );
      }
      if (opts.length === 0) {
        return new Float16Array(
          TypedArrayPrototypeGetBuffer(
            Uint16ArrayFrom(src, roundToFloat16Bits)
          )
        );
      }
      const mapFunc = opts[0];
      const thisArg = opts[1];
      return new Float16Array(
        TypedArrayPrototypeGetBuffer(
          Uint16ArrayFrom(src, function (val, ...args) {
            return roundToFloat16Bits(
              ReflectApply(mapFunc, this, [val, ...safeIfNeeded(args)])
            );
          }, thisArg)
        )
      );
    }
    let list;
    let length;
    const iterator = src[SymbolIterator];
    if (iterator != null && typeof iterator !== "function") {
      throw NativeTypeError(ITERATOR_PROPERTY_IS_NOT_CALLABLE);
    }
    if (iterator != null) {
      if (isOrdinaryArray(src)) {
        list = src;
        length = src.length;
      } else if (isOrdinaryNativeTypedArray(src)) {
        list = src;
        length = TypedArrayPrototypeGetLength(src);
      } else {
        list = [...src];
        length = list.length;
      }
    } else {
      if (src == null) {
        throw NativeTypeError(
          CANNOT_CONVERT_UNDEFINED_OR_NULL_TO_OBJECT
        );
      }
      list = NativeObject(src);
      length = ToLength(list.length);
    }
    const array = new Constructor(length);
    if (opts.length === 0) {
      for (let i = 0; i < length; ++i) {
        array[i] = list[i];
      }
    } else {
      const mapFunc = opts[0];
      const thisArg = opts[1];
      for (let i = 0; i < length; ++i) {
        array[i] = ReflectApply(mapFunc, thisArg, [list[i], i]);
      }
    }
    return array;
  }
  static of(...items) {
    const Constructor = this;
    if (!ReflectHas(Constructor, brand)) {
      throw NativeTypeError(
        THIS_CONSTRUCTOR_IS_NOT_A_SUBCLASS_OF_FLOAT16ARRAY
      );
    }
    const length = items.length;
    if (Constructor === Float16Array) {
      const proxy = new Float16Array(length);
      const float16bitsArray = getFloat16BitsArray(proxy);
      for (let i = 0; i < length; ++i) {
        float16bitsArray[i] = roundToFloat16Bits(items[i]);
      }
      return proxy;
    }
    const array = new Constructor(length);
    for (let i = 0; i < length; ++i) {
      array[i] = items[i];
    }
    return array;
  }
  keys() {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    return TypedArrayPrototypeKeys(float16bitsArray);
  }
  values() {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    return wrap(function* () {
      for (const val of TypedArrayPrototypeValues(float16bitsArray)) {
        yield convertToNumber(val);
      }
    }());
  }
  entries() {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    return wrap(function* () {
      for (const [i, val] of TypedArrayPrototypeEntries(float16bitsArray)) {
        yield [i, convertToNumber(val)];
      }
    }());
  }
  at(index) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const relativeIndex = ToIntegerOrInfinity(index);
    const k = relativeIndex >= 0 ? relativeIndex : length + relativeIndex;
    if (k < 0 || k >= length) {
      return;
    }
    return convertToNumber(float16bitsArray[k]);
  }
  with(index, value) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const relativeIndex = ToIntegerOrInfinity(index);
    const k = relativeIndex >= 0 ? relativeIndex : length + relativeIndex;
    const number = +value;
    if (k < 0 || k >= length) {
      throw NativeRangeError(OFFSET_IS_OUT_OF_BOUNDS);
    }
    const uint16 = new NativeUint16Array(
      TypedArrayPrototypeGetBuffer(float16bitsArray),
      TypedArrayPrototypeGetByteOffset(float16bitsArray),
      TypedArrayPrototypeGetLength(float16bitsArray)
    );
    const cloned = new Float16Array(
      TypedArrayPrototypeGetBuffer(
        TypedArrayPrototypeSlice(uint16)
      )
    );
    const array = getFloat16BitsArray(cloned);
    array[k] = roundToFloat16Bits(number);
    return cloned;
  }
  map(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    const Constructor = SpeciesConstructor(float16bitsArray, Float16Array);
    if (Constructor === Float16Array) {
      const proxy = new Float16Array(length);
      const array = getFloat16BitsArray(proxy);
      for (let i = 0; i < length; ++i) {
        const val = convertToNumber(float16bitsArray[i]);
        array[i] = roundToFloat16Bits(
          ReflectApply(callback, thisArg, [val, i, this])
        );
      }
      return proxy;
    }
    const array = new Constructor(length);
    assertSpeciesTypedArray(array, length);
    for (let i = 0; i < length; ++i) {
      const val = convertToNumber(float16bitsArray[i]);
      array[i] = ReflectApply(callback, thisArg, [val, i, this]);
    }
    return array;
  }
  filter(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    const kept = [];
    for (let i = 0; i < length; ++i) {
      const val = convertToNumber(float16bitsArray[i]);
      if (ReflectApply(callback, thisArg, [val, i, this])) {
        ArrayPrototypePush(kept, val);
      }
    }
    const Constructor = SpeciesConstructor(float16bitsArray, Float16Array);
    const array = new Constructor(kept);
    assertSpeciesTypedArray(array);
    return array;
  }
  reduce(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    if (length === 0 && opts.length === 0) {
      throw NativeTypeError(REDUCE_OF_EMPTY_ARRAY_WITH_NO_INITIAL_VALUE);
    }
    let accumulator, start;
    if (opts.length === 0) {
      accumulator = convertToNumber(float16bitsArray[0]);
      start = 1;
    } else {
      accumulator = opts[0];
      start = 0;
    }
    for (let i = start; i < length; ++i) {
      accumulator = callback(
        accumulator,
        convertToNumber(float16bitsArray[i]),
        i,
        this
      );
    }
    return accumulator;
  }
  reduceRight(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    if (length === 0 && opts.length === 0) {
      throw NativeTypeError(REDUCE_OF_EMPTY_ARRAY_WITH_NO_INITIAL_VALUE);
    }
    let accumulator, start;
    if (opts.length === 0) {
      accumulator = convertToNumber(float16bitsArray[length - 1]);
      start = length - 2;
    } else {
      accumulator = opts[0];
      start = length - 1;
    }
    for (let i = start; i >= 0; --i) {
      accumulator = callback(
        accumulator,
        convertToNumber(float16bitsArray[i]),
        i,
        this
      );
    }
    return accumulator;
  }
  forEach(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    for (let i = 0; i < length; ++i) {
      ReflectApply(callback, thisArg, [
      convertToNumber(float16bitsArray[i]),
      i,
      this]
      );
    }
  }
  find(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    for (let i = 0; i < length; ++i) {
      const value = convertToNumber(float16bitsArray[i]);
      if (ReflectApply(callback, thisArg, [value, i, this])) {
        return value;
      }
    }
  }
  findIndex(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    for (let i = 0; i < length; ++i) {
      const value = convertToNumber(float16bitsArray[i]);
      if (ReflectApply(callback, thisArg, [value, i, this])) {
        return i;
      }
    }
    return -1;
  }
  findLast(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    for (let i = length - 1; i >= 0; --i) {
      const value = convertToNumber(float16bitsArray[i]);
      if (ReflectApply(callback, thisArg, [value, i, this])) {
        return value;
      }
    }
  }
  findLastIndex(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    for (let i = length - 1; i >= 0; --i) {
      const value = convertToNumber(float16bitsArray[i]);
      if (ReflectApply(callback, thisArg, [value, i, this])) {
        return i;
      }
    }
    return -1;
  }
  every(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    for (let i = 0; i < length; ++i) {
      if (
      !ReflectApply(callback, thisArg, [
      convertToNumber(float16bitsArray[i]),
      i,
      this]
      ))
      {
        return false;
      }
    }
    return true;
  }
  some(callback, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const thisArg = opts[0];
    for (let i = 0; i < length; ++i) {
      if (
      ReflectApply(callback, thisArg, [
      convertToNumber(float16bitsArray[i]),
      i,
      this]
      ))
      {
        return true;
      }
    }
    return false;
  }
  set(input, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const targetOffset = ToIntegerOrInfinity(opts[0]);
    if (targetOffset < 0) {
      throw NativeRangeError(OFFSET_IS_OUT_OF_BOUNDS);
    }
    if (input == null) {
      throw NativeTypeError(
        CANNOT_CONVERT_UNDEFINED_OR_NULL_TO_OBJECT
      );
    }
    if (isNativeBigIntTypedArray(input)) {
      throw NativeTypeError(
        CANNOT_MIX_BIGINT_AND_OTHER_TYPES
      );
    }
    if (isFloat16Array(input)) {
      return TypedArrayPrototypeSet(
        getFloat16BitsArray(this),
        getFloat16BitsArray(input),
        targetOffset
      );
    }
    if (isNativeTypedArray(input)) {
      const buffer = TypedArrayPrototypeGetBuffer(input);
      if (IsDetachedBuffer(buffer)) {
        throw NativeTypeError(ATTEMPTING_TO_ACCESS_DETACHED_ARRAYBUFFER);
      }
    }
    const targetLength = TypedArrayPrototypeGetLength(float16bitsArray);
    const src = NativeObject(input);
    const srcLength = ToLength(src.length);
    if (targetOffset === Infinity || srcLength + targetOffset > targetLength) {
      throw NativeRangeError(OFFSET_IS_OUT_OF_BOUNDS);
    }
    for (let i = 0; i < srcLength; ++i) {
      float16bitsArray[i + targetOffset] = roundToFloat16Bits(src[i]);
    }
  }
  reverse() {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    TypedArrayPrototypeReverse(float16bitsArray);
    return this;
  }
  toReversed() {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const uint16 = new NativeUint16Array(
      TypedArrayPrototypeGetBuffer(float16bitsArray),
      TypedArrayPrototypeGetByteOffset(float16bitsArray),
      TypedArrayPrototypeGetLength(float16bitsArray)
    );
    const cloned = new Float16Array(
      TypedArrayPrototypeGetBuffer(
        TypedArrayPrototypeSlice(uint16)
      )
    );
    const clonedFloat16bitsArray = getFloat16BitsArray(cloned);
    TypedArrayPrototypeReverse(clonedFloat16bitsArray);
    return cloned;
  }
  fill(value, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    TypedArrayPrototypeFill(
      float16bitsArray,
      roundToFloat16Bits(value),
      ...safeIfNeeded(opts)
    );
    return this;
  }
  copyWithin(target, start, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    TypedArrayPrototypeCopyWithin(float16bitsArray, target, start, ...safeIfNeeded(opts));
    return this;
  }
  sort(compareFn) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const sortCompare = compareFn !== undefined ? compareFn : defaultCompare;
    TypedArrayPrototypeSort(float16bitsArray, (x, y) => {
      return sortCompare(convertToNumber(x), convertToNumber(y));
    });
    return this;
  }
  toSorted(compareFn) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    if (compareFn !== undefined && typeof compareFn !== "function") {
      throw new NativeTypeError(THE_COMPARISON_FUNCTION_MUST_BE_EITHER_A_FUNCTION_OR_UNDEFINED);
    }
    const sortCompare = compareFn !== undefined ? compareFn : defaultCompare;
    const uint16 = new NativeUint16Array(
      TypedArrayPrototypeGetBuffer(float16bitsArray),
      TypedArrayPrototypeGetByteOffset(float16bitsArray),
      TypedArrayPrototypeGetLength(float16bitsArray)
    );
    const cloned = new Float16Array(
      TypedArrayPrototypeGetBuffer(
        TypedArrayPrototypeSlice(uint16)
      )
    );
    const clonedFloat16bitsArray = getFloat16BitsArray(cloned);
    TypedArrayPrototypeSort(clonedFloat16bitsArray, (x, y) => {
      return sortCompare(convertToNumber(x), convertToNumber(y));
    });
    return cloned;
  }
  slice(start, end) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const Constructor = SpeciesConstructor(float16bitsArray, Float16Array);
    if (Constructor === Float16Array) {
      const uint16 = new NativeUint16Array(
        TypedArrayPrototypeGetBuffer(float16bitsArray),
        TypedArrayPrototypeGetByteOffset(float16bitsArray),
        TypedArrayPrototypeGetLength(float16bitsArray)
      );
      return new Float16Array(
        TypedArrayPrototypeGetBuffer(
          TypedArrayPrototypeSlice(uint16, start, end)
        )
      );
    }
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    const relativeStart = ToIntegerOrInfinity(start);
    const relativeEnd = end === undefined ? length : ToIntegerOrInfinity(end);
    let k;
    if (relativeStart === -Infinity) {
      k = 0;
    } else if (relativeStart < 0) {
      k = length + relativeStart > 0 ? length + relativeStart : 0;
    } else {
      k = length < relativeStart ? length : relativeStart;
    }
    let final;
    if (relativeEnd === -Infinity) {
      final = 0;
    } else if (relativeEnd < 0) {
      final = length + relativeEnd > 0 ? length + relativeEnd : 0;
    } else {
      final = length < relativeEnd ? length : relativeEnd;
    }
    const count = final - k > 0 ? final - k : 0;
    const array = new Constructor(count);
    assertSpeciesTypedArray(array, count);
    if (count === 0) {
      return array;
    }
    const buffer = TypedArrayPrototypeGetBuffer(float16bitsArray);
    if (IsDetachedBuffer(buffer)) {
      throw NativeTypeError(ATTEMPTING_TO_ACCESS_DETACHED_ARRAYBUFFER);
    }
    let n = 0;
    while (k < final) {
      array[n] = convertToNumber(float16bitsArray[k]);
      ++k;
      ++n;
    }
    return array;
  }
  subarray(begin, end) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const Constructor = SpeciesConstructor(float16bitsArray, Float16Array);
    const uint16 = new NativeUint16Array(
      TypedArrayPrototypeGetBuffer(float16bitsArray),
      TypedArrayPrototypeGetByteOffset(float16bitsArray),
      TypedArrayPrototypeGetLength(float16bitsArray)
    );
    const uint16Subarray = TypedArrayPrototypeSubarray(uint16, begin, end);
    const array = new Constructor(
      TypedArrayPrototypeGetBuffer(uint16Subarray),
      TypedArrayPrototypeGetByteOffset(uint16Subarray),
      TypedArrayPrototypeGetLength(uint16Subarray)
    );
    assertSpeciesTypedArray(array);
    return array;
  }
  indexOf(element, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    let from = ToIntegerOrInfinity(opts[0]);
    if (from === Infinity) {
      return -1;
    }
    if (from < 0) {
      from += length;
      if (from < 0) {
        from = 0;
      }
    }
    for (let i = from; i < length; ++i) {
      if (
      ObjectHasOwn(float16bitsArray, i) &&
      convertToNumber(float16bitsArray[i]) === element)
      {
        return i;
      }
    }
    return -1;
  }
  lastIndexOf(element, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    let from = opts.length >= 1 ? ToIntegerOrInfinity(opts[0]) : length - 1;
    if (from === -Infinity) {
      return -1;
    }
    if (from >= 0) {
      from = from < length - 1 ? from : length - 1;
    } else {
      from += length;
    }
    for (let i = from; i >= 0; --i) {
      if (
      ObjectHasOwn(float16bitsArray, i) &&
      convertToNumber(float16bitsArray[i]) === element)
      {
        return i;
      }
    }
    return -1;
  }
  includes(element, ...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const length = TypedArrayPrototypeGetLength(float16bitsArray);
    let from = ToIntegerOrInfinity(opts[0]);
    if (from === Infinity) {
      return false;
    }
    if (from < 0) {
      from += length;
      if (from < 0) {
        from = 0;
      }
    }
    const isNaN = NumberIsNaN(element);
    for (let i = from; i < length; ++i) {
      const value = convertToNumber(float16bitsArray[i]);
      if (isNaN && NumberIsNaN(value)) {
        return true;
      }
      if (value === element) {
        return true;
      }
    }
    return false;
  }
  join(separator) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const array = copyToArray(float16bitsArray);
    return ArrayPrototypeJoin(array, separator);
  }
  toLocaleString(...opts) {
    assertFloat16Array(this);
    const float16bitsArray = getFloat16BitsArray(this);
    const array = copyToArray(float16bitsArray);
    return ArrayPrototypeToLocaleString(array, ...safeIfNeeded(opts));
  }
  get [SymbolToStringTag]() {
    if (isFloat16Array(this)) {
      return "Float16Array";
    }
  }
}
ObjectDefineProperty(Float16Array, "BYTES_PER_ELEMENT", {
  value: BYTES_PER_ELEMENT
});
ObjectDefineProperty(Float16Array, brand, {});
ReflectSetPrototypeOf(Float16Array, TypedArray);
const Float16ArrayPrototype = Float16Array.prototype;
ObjectDefineProperty(Float16ArrayPrototype, "BYTES_PER_ELEMENT", {
  value: BYTES_PER_ELEMENT
});
ObjectDefineProperty(Float16ArrayPrototype, SymbolIterator, {
  value: Float16ArrayPrototype.values,
  writable: true,
  configurable: true
});
ReflectSetPrototypeOf(Float16ArrayPrototype, TypedArrayPrototype);

function isTypedArray(target) {
  return isNativeTypedArray(target) || isFloat16Array(target);
}

function getFloat16(dataView, byteOffset, ...opts) {
  return convertToNumber(
    DataViewPrototypeGetUint16(dataView, byteOffset, ...safeIfNeeded(opts))
  );
}
function setFloat16(dataView, byteOffset, value, ...opts) {
  return DataViewPrototypeSetUint16(
    dataView,
    byteOffset,
    roundToFloat16Bits(value),
    ...safeIfNeeded(opts)
  );
}

function f16round(x) {
  return roundToFloat16(x);
}


});
__define('common/framework/fixture.js', function (__exports) {
'use strict';
__get(__exports, 'SkipTestCase', () => SkipTestCase);
__get(__exports, 'UnexpectedPassError', () => UnexpectedPassError);
__get(__exports, 'SubcaseBatchState', () => SubcaseBatchState);
__get(__exports, 'Fixture', () => Fixture);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert, unreachable; __bind('common/util/util.js', (__m) => { ({ assert, unreachable } = __m); });

class SkipTestCase extends Error {}
class UnexpectedPassError extends Error {}

__get(__exports, 'TestCaseRecorder', () => __require('common/internal/logging/test_case_recorder.js').TestCaseRecorder);

/** The fully-general type for params passed to a test function invocation. */











class SubcaseBatchState {
  constructor(
  recorder,
  /** The case parameters for this test fixture shared state. Subcase params are not included. */
  params)
  {this.recorder = recorder;this.params = params;}

  /**
   * Runs before the `.before()` function.
   * @internal MAINTENANCE_TODO: Make this not visible to test code?
   */
  async init() {}
  /**
   * Runs between the `.before()` function and the subcases.
   * @internal MAINTENANCE_TODO: Make this not visible to test code?
   */
  async postInit() {}
  /**
   * Runs after all subcases finish.
   * @internal MAINTENANCE_TODO: Make this not visible to test code?
   */
  async finalize() {}

  /** Throws an exception marking the subcase as skipped. */
  skip(msg) {
    throw new SkipTestCase(msg);
  }

  /** Throws an exception making the subcase as skipped if condition is true */
  skipIf(cond, msg = '') {
    if (cond) {
      this.skip(typeof msg === 'function' ? msg() : msg);
    }
  }
}

/**
 * A Fixture is a class used to instantiate each test sub/case at run time.
 * A new instance of the Fixture is created for every single test subcase
 * (i.e. every time the test function is run).
 */
class Fixture {


  /**
   * Interface for recording logs and test status.
   *
   * @internal
   */

  eventualExpectations = [];
  numOutstandingAsyncExpectations = 0;
  objectsToCleanUp = [];

  static MakeSharedState(recorder, params) {
    return new SubcaseBatchState(recorder, params);
  }

  /** @internal */
  constructor(sharedState, rec, params) {
    this._sharedState = sharedState;
    this.rec = rec;
    this._params = params;
  }

  /**
   * Returns the (case+subcase) parameters for this test function invocation.
   */
  get params() {
    return this._params;
  }

  /**
   * Gets the test fixture's shared state. This object is shared between subcases
   * within the same testcase.
   */
  get sharedState() {
    return this._sharedState;
  }

  /**
   * Override this to do additional pre-test-function work in a derived fixture.
   * This has to be a member function instead of an async `createFixture` function, because
   * we need to be able to ergonomically override it in subclasses.
   *
   * @internal MAINTENANCE_TODO: Make this not visible to test code?
   */
  async init() {}

  /**
   * Override this to do additional post-test-function work in a derived fixture.
   *
   * Called even if init was unsuccessful.
   *
   * @internal MAINTENANCE_TODO: Make this not visible to test code?
   */
  async finalize() {
    assert(
      this.numOutstandingAsyncExpectations === 0,
      'there were outstanding immediateAsyncExpectations (e.g. expectUncapturedError) at the end of the test'
    );

    // Loop to exhaust the eventualExpectations in case they chain off each other.
    while (this.eventualExpectations.length) {
      const p = this.eventualExpectations.shift();
      try {
        await p;
      } catch (ex) {
        this.rec.threw(ex);
      }
    }

    // And clean up any objects now that they're done being used.
    for (const o of this.objectsToCleanUp) {
      if ('getExtension' in o) {
        const WEBGL_lose_context = o.getExtension('WEBGL_lose_context');
        if (WEBGL_lose_context) WEBGL_lose_context.loseContext();
      } else if ('destroy' in o) {
        o.destroy();
      } else if ('destroyAsync' in o) {
        await o.destroyAsync();
      } else if ('close' in o) {
        o.close();
      } else {
        // HTMLVideoElement
        o.src = '';
        o.srcObject = null;
      }
    }
  }

  /**
   * Tracks an object to be cleaned up after the test finishes.
   *
   * Usually when creating buffers/textures/query sets, you can use the helpers in GPUTest instead.
   */
  trackForCleanup(o) {
    if (o instanceof Promise) {
      this.eventualAsyncExpectation(() =>
      o.then(
        (o) => this.trackForCleanup(o),
        () => {}
      )
      );
      return o;
    }

    if (o instanceof GPUDevice) {
      this.objectsToCleanUp.push({
        async destroyAsync() {
          o.destroy();
          await o.lost;
        }
      });
    } else {
      this.objectsToCleanUp.push(o);
    }
    return o;
  }

  /** Tracks an object, if it's destroyable, to be cleaned up after the test finishes. */
  tryTrackForCleanup(o) {
    if (typeof o === 'object' && o !== null) {
      if (
      'destroy' in o ||
      'close' in o ||
      o instanceof WebGLRenderingContext ||
      o instanceof WebGL2RenderingContext)
      {
        this.objectsToCleanUp.push(o);
      }
    }
    return o;
  }

  /** Call requestDevice() and track the device for cleanup. */
  requestDeviceTracked(adapter, desc = undefined) {

    return this.trackForCleanup(adapter.requestDevice(desc));
  }

  /** Log a debug message. */
  debug(msg) {
    if (!this.rec.debugging) return;
    if (typeof msg === 'function') {
      msg = msg();
    }
    this.rec.debug(new Error(msg));
  }

  /**
   * Log an info message.
   * **Use sparingly. Use `debug()` instead if logs are only needed with debug logging enabled.**
   */
  info(msg) {
    this.rec.info(new Error(msg));
  }

  /** Throws an exception marking the subcase as skipped. */
  skip(msg) {
    throw new SkipTestCase(msg);
  }

  /** Throws an exception marking the subcase as skipped if condition is true */
  skipIf(cond, msg = '') {
    if (cond) {
      this.skip(typeof msg === 'function' ? msg() : msg);
    }
  }

  /** Log a warning and increase the result status to "Warn". */
  warn(msg) {
    this.rec.warn(new Error(msg));
  }

  /** Log an error and increase the result status to "ExpectFailed". */
  fail(msg) {
    this.rec.expectationFailed(new Error(msg));
  }

  /**
   * Wraps an async function. Tracks its status to fail if the test tries to report a test status
   * before the async work has finished.
   */
  async immediateAsyncExpectation(fn) {
    this.numOutstandingAsyncExpectations++;
    const ret = await fn();
    this.numOutstandingAsyncExpectations--;
    return ret;
  }

  /**
   * Wraps an async function, passing it an `Error` object recording the original stack trace.
   * The async work will be implicitly waited upon before reporting a test status.
   */
  eventualAsyncExpectation(fn) {
    const promise = fn(new Error());
    this.eventualExpectations.push(promise);
  }

  expectErrorValue(expectedError, ex, niceStack) {
    if (!(ex instanceof Error)) {
      niceStack.message = `THREW non-error value, of type ${typeof ex}: ${ex}`;
      this.rec.expectationFailed(niceStack);
      return;
    }
    const actualName = ex.name;
    if (expectedError !== true && actualName !== expectedError) {
      niceStack.message = `THREW ${actualName}, instead of ${expectedError}: ${ex}`;
      this.rec.expectationFailed(niceStack);
    } else {
      niceStack.message = `OK: threw ${actualName}: ${ex.message}`;
      this.rec.debug(niceStack);
    }
  }

  /** Expect that the provided promise resolves (fulfills). */
  shouldResolve(p, msg) {
    this.eventualAsyncExpectation(async (niceStack) => {
      const m = msg ? ': ' + msg : '';
      try {
        await p;
        niceStack.message = 'resolved as expected' + m;
      } catch (ex) {
        niceStack.message = `REJECTED${m}`;
        if (ex instanceof Error) {
          niceStack.message += '\n' + ex.message;
        }
        this.rec.expectationFailed(niceStack);
      }
    });
  }

  /** Expect that the provided promise rejects, with the provided exception name. */
  shouldReject(
  expectedName,
  p,
  { allowMissingStack = false, message } = {})
  {
    this.eventualAsyncExpectation(async (niceStack) => {
      const m = message ? ': ' + message : '';
      try {
        await p;
        niceStack.message = 'DID NOT REJECT' + m;
        this.rec.expectationFailed(niceStack);
      } catch (ex) {
        this.expectErrorValue(expectedName, ex, niceStack);
        if (!allowMissingStack) {
          if (!(ex instanceof Error && typeof ex.stack === 'string')) {
            const exMessage = ex instanceof Error ? ex.message : '?';
            niceStack.message = `rejected as expected, but missing stack (${exMessage})${m}`;
            this.rec.expectationFailed(niceStack);
          }
        }
      }
    });
  }

  /**
   * Expect that the provided function throws (if `true` or `string`) or not (if `false`).
   * If a string is provided, expect that the throw exception has that name.
   *
   * MAINTENANCE_TODO: Change to `string | false` so the exception name is always checked.
   */
  shouldThrow(
  expectedError,
  fn,
  { allowMissingStack = false, message } = {})
  {
    const m = message ? ': ' + message : '';
    try {
      fn();
      if (expectedError === false) {
        this.rec.debug(new Error('did not throw, as expected' + m));
      } else {
        this.rec.expectationFailed(new Error('unexpectedly did not throw' + m));
      }
    } catch (ex) {
      if (expectedError === false) {
        this.rec.expectationFailed(new Error('threw unexpectedly' + m));
      } else {
        this.expectErrorValue(expectedError, ex, new Error(m));
        if (!allowMissingStack) {
          if (!(ex instanceof Error && typeof ex.stack === 'string')) {
            this.rec.expectationFailed(new Error('threw as expected, but missing stack' + m));
          }
        }
      }
    }
  }

  /**
   * Expect that a condition is true.
   *
   * Note: You can pass a boolean condition, or a function that returns a boolean.
   * The advantage to passing a function is that if it's short it is self documenting.
   *
   * t.expect(size >= maxSize);      // prints Expect OK:
   * t.expect(() => size >= maxSize) // prints Expect OK: () => size >= maxSize
   */
  expect(cond, msg) {
    if (typeof cond === 'function') {
      if (msg === undefined) {
        msg = cond.toString();
      }
      cond = cond();
    }
    if (cond) {
      if (this.rec.debugging) {
        const m = msg ? ': ' + (typeof msg === 'function' ? msg() : msg) : '';
        this.rec.debug(new Error('expect OK' + m));
      }
    } else {
      this.rec.expectationFailed(new Error(typeof msg === 'function' ? msg() : msg));
    }
    return cond;
  }

  /**
   * If the argument is an `Error`, fail (or warn). If it's `undefined`, no-op.
   * If the argument is an array, apply the above behavior on each of elements.
   */
  expectOK(
  error,
  { mode = 'fail', niceStack } = {})
  {
    const handleError = (error) => {
      if (error instanceof Error) {
        if (niceStack) {
          error.stack = niceStack.stack;
        }
        if (mode === 'fail') {
          this.rec.expectationFailed(error);
        } else if (mode === 'warn') {
          this.rec.warn(error);
        } else {
          unreachable();
        }
      }
    };

    if (Array.isArray(error)) {
      for (const e of error) {
        handleError(e);
      }
    } else {
      handleError(error);
    }
  }

  eventualExpectOK(
  error,
  { mode = 'fail' } = {})
  {
    this.eventualAsyncExpectation(async (niceStack) => {
      this.expectOK(await error, { mode, niceStack });
    });
  }
}



/**
 * FixtureClass encapsulates a constructor for fixture and a corresponding
 * shared state factory function. An interface version of the type is also
 * defined for mixin declaration use ONLY. The interface version is necessary
 * because mixin classes need a constructor with a single any[] rest
 * parameter.
 */
});
__define('common/internal/logging/test_case_recorder.js', function (__exports) {
'use strict';
__get(__exports, 'TestCaseRecorder', () => TestCaseRecorder);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let SkipTestCase, UnexpectedPassError; __bind('common/framework/fixture.js', (__m) => { ({ SkipTestCase, UnexpectedPassError } = __m); });let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });let now, assert; __bind('common/util/util.js', (__m) => { ({ now, assert } = __m); });

let LogMessageWithStack; __bind('common/internal/logging/log_message.js', (__m) => { ({ LogMessageWithStack } = __m); });var


LogSeverity = /*#__PURE__*/function (LogSeverity) {LogSeverity[LogSeverity["NotRun"] = 0] = "NotRun";LogSeverity[LogSeverity["Skip"] = 1] = "Skip";LogSeverity[LogSeverity["Pass"] = 2] = "Pass";LogSeverity[LogSeverity["Warn"] = 3] = "Warn";LogSeverity[LogSeverity["ExpectFailed"] = 4] = "ExpectFailed";LogSeverity[LogSeverity["ValidationFailed"] = 5] = "ValidationFailed";LogSeverity[LogSeverity["ThrewException"] = 6] = "ThrewException";return LogSeverity;}(LogSeverity || {});









const kMaxLogStacks = 2;
const kMinSeverityForStack = LogSeverity.Warn;

function logSeverityToString(status) {
  switch (status) {
    case LogSeverity.NotRun:
      return 'notrun';
    case LogSeverity.Pass:
      return 'pass';
    case LogSeverity.Skip:
      return 'skip';
    case LogSeverity.Warn:
      return 'warn';
    default:
      return 'fail'; // Everything else is an error
  }
}

/** Holds onto a LiveTestCaseResult owned by the Logger, and writes the results into it. */
class TestCaseRecorder {

  nonskippedSubcaseCount = 0;
  inSubCase = false;
  subCaseStatus = LogSeverity.NotRun;
  finalCaseStatus = LogSeverity.NotRun;
  hideStacksBelowSeverity = kMinSeverityForStack;
  startTime = -1;
  logs = [];
  logLinesAtCurrentSeverity = 0;
  debugging = false;

  constructor(result, debugging) {
    this.result = result;
    this.debugging = debugging;
  }

  start() {
    assert(this.startTime < 0, 'TestCaseRecorder cannot be reused');
    this.startTime = now();
  }

  finish() {
    // This is a framework error. If this assert is hit, it won't be localized
    // to a test. The whole test run will fail out.
    assert(this.startTime >= 0, 'internal error: finish() before start()');

    const timeMilliseconds = now() - this.startTime;
    // Round to next microsecond to avoid storing useless .xxxx00000000000002 in results.
    this.result.timems = Math.ceil(timeMilliseconds * 1000) / 1000;

    if (this.finalCaseStatus === LogSeverity.Skip && this.nonskippedSubcaseCount !== 0) {
      this.threw(new Error('internal error: case is "skip" but has nonskipped subcases'));
    }

    // Convert numeric enum back to string (but expose 'exception' as 'fail')
    this.result.status = logSeverityToString(this.finalCaseStatus);

    this.result.logs = this.logs;
  }

  beginSubCase() {
    this.subCaseStatus = LogSeverity.NotRun;
    this.inSubCase = true;
  }

  endSubCase(expectedStatus) {
    if (this.subCaseStatus !== LogSeverity.Skip) {
      this.nonskippedSubcaseCount++;
    }
    try {
      if (expectedStatus === 'fail') {
        if (this.subCaseStatus <= LogSeverity.Warn) {
          throw new UnexpectedPassError();
        } else {
          this.subCaseStatus = LogSeverity.Pass;
        }
      }
    } finally {
      this.inSubCase = false;
      this.finalCaseStatus = Math.max(this.finalCaseStatus, this.subCaseStatus);
    }
  }

  injectResult(injectedResult) {
    Object.assign(this.result, injectedResult);
  }

  debug(ex) {
    if (!this.debugging) return;
    this.logImpl(LogSeverity.Pass, 'DEBUG', ex);
  }

  info(ex) {
    // We need this to use the lowest LogSeverity so it doesn't override the current severity for this test case.
    this.logImpl(LogSeverity.NotRun, 'INFO', ex);
  }

  skipped(ex) {
    this.logImpl(LogSeverity.Skip, 'SKIP', ex);
  }

  warn(ex) {
    this.logImpl(LogSeverity.Warn, 'WARN', ex);
  }

  expectationFailed(ex) {
    this.logImpl(LogSeverity.ExpectFailed, 'EXPECTATION FAILED', ex);
  }

  validationFailed(ex) {
    this.logImpl(LogSeverity.ValidationFailed, 'VALIDATION FAILED', ex);
  }

  passed() {
    if (this.inSubCase) {
      this.subCaseStatus = Math.max(this.subCaseStatus, LogSeverity.Pass);
    } else {
      this.finalCaseStatus = Math.max(this.finalCaseStatus, LogSeverity.Pass);
    }
  }

  threw(ex) {
    if (ex instanceof SkipTestCase) {
      this.skipped(ex);
      return;
    }
    // logImpl will discard the original error's ex.name. Preserve it here.
    const name = ex instanceof Error ? `EXCEPTION: ${ex.name}` : 'EXCEPTION';
    this.logImpl(LogSeverity.ThrewException, name, ex);
  }

  logImpl(level, name, baseException) {
    assert(baseException instanceof Error, 'test threw a non-Error object');
    globalTestConfig.testHeartbeatCallback();
    const logMessage = LogMessageWithStack.wrapError(name, baseException);

    // Final case status should be the "worst" of all log entries.
    if (this.inSubCase) {
      this.subCaseStatus = Math.max(this.subCaseStatus, level);
    } else {
      this.finalCaseStatus = Math.max(this.finalCaseStatus, level);
    }

    // setFirstLineOnly for all logs except `kMaxLogStacks` stacks at the highest severity
    if (level > this.hideStacksBelowSeverity) {
      this.logLinesAtCurrentSeverity = 0;
      this.hideStacksBelowSeverity = level;

      // Go back and setFirstLineOnly for everything of a lower log level
      for (const log of this.logs) {
        log.setStackHidden('below max severity');
      }
    }
    if (level === this.hideStacksBelowSeverity) {
      this.logLinesAtCurrentSeverity++;
    } else if (level < kMinSeverityForStack) {
      logMessage.setStackHidden('');
    } else if (level < this.hideStacksBelowSeverity) {
      logMessage.setStackHidden('below max severity');
    }
    if (this.logLinesAtCurrentSeverity > kMaxLogStacks) {
      logMessage.setStackHidden(`only ${kMaxLogStacks} shown`);
    }

    this.logs.push(logMessage);
  }

  /**
   * Make a recorder that will defer all calls until `deferUntilPromise` resolves.
   * This is used for running subcases, which run concurrently, to ensure that
   * logs from all the previous subcases have been flushed before flushing new logs,
   * so all logs are in order as if the subcases had not been concurrent.
   */
  makeDeferredSubRecorder(prefix, deferUntilPromise) {
    return new Proxy(this, {
      get: (target, k) => {
        switch (k) {
          case 'logImpl':
            return function (level, name, baseException) {
              globalTestConfig.testHeartbeatCallback();
              void deferUntilPromise.then(() => {
                target.logImpl(level, prefix + name, baseException);
              });
            };
          case 'beginSubCase':
            return function () {
              globalTestConfig.testHeartbeatCallback();
              void deferUntilPromise.then(() => {
                target.beginSubCase();
              });
            };
          case 'endSubCase':
            return function (expectedStatus) {
              globalTestConfig.testHeartbeatCallback();
              void deferUntilPromise.then(() => {
                target.endSubCase(expectedStatus);
              });
            };
          default:

            return target[k];
        }
      }
    });
  }
}
});
__define('common/internal/logging/log_message.js', function (__exports) {
'use strict';
__get(__exports, 'LogMessageWithStack', () => LogMessageWithStack);
__get(__exports, 'prettyPrintLog', () => prettyPrintLog);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let extractImportantStackTrace; __bind('common/internal/stack.js', (__m) => { ({ extractImportantStackTrace } = __m); });


class LogMessageWithStack extends Error {


  stackHiddenMessage = undefined;

  /**
   * Wrap an Error (which was created to capture the stack at that point) into a
   * LogMessageWithStack (which has extra stuff for good log messages).
   *
   * The original `ex.name` is ignored. Inclued it in the `name` parameter if it
   * needs to be preserved.
   */
  static wrapError(name, ex) {
    return new LogMessageWithStack({
      name,
      message: ex.message,
      stackHiddenMessage: undefined,
      stack: ex.stack,
      extra: 'extra' in ex ? ex.extra : undefined
    });
  }

  constructor(o) {
    super(o.message);
    this.name = o.name;
    this.stackHiddenMessage = o.stackHiddenMessage;
    this.stack = o.stack;
    this.extra = o.extra;
  }

  /** Set a flag so the stack is not printed in toJSON(). */
  setStackHidden(stackHiddenMessage) {
    this.stackHiddenMessage ??= stackHiddenMessage;
  }

  /**
   * Print the message for display.
   *
   * Note: This is toJSON instead of toString to make it easy to save logs using JSON.stringify.
   */
  toJSON() {
    let m = this.name;
    if (this.message) m += ': ' + this.message;
    if (this.stack) {
      if (this.stackHiddenMessage === undefined) {
        m += '\n' + extractImportantStackTrace(this);
      } else if (this.stackHiddenMessage) {
        m += `\n  at (elided: ${this.stackHiddenMessage})`;
      }
    }
    return m;
  }

  /**
   * Flatten the message for sending over a message channel.
   *
   * Note `extra` may get mangled by postMessage.
   */
  toRawData() {
    let extra = this.extra;
    if (extra !== undefined) {
      extra = JSON.parse(JSON.stringify(extra));
    }

    return {
      name: this.name,
      message: this.message,
      stackHiddenMessage: this.stackHiddenMessage,
      stack: this.stack,
      extra
    };
  }
}

/**
 * Returns a string, nicely indented, for debug logs.
 * This is used in the cmdline and wpt runtimes. In WPT, it shows up in the `*-actual.txt` file.
 */
function prettyPrintLog(log) {
  return '  - ' + log.toJSON().replace(/\n/g, '\n    ');
}
});
__define('common/internal/stack.js', function (__exports) {
'use strict';
__get(__exports, 'extractImportantStackTrace', () => extractImportantStackTrace);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ // Returns the stack trace of an Error, but without the extra boilerplate at the bottom
// (e.g. RunCaseSpecific, processTicksAndRejections, etc.), for logging.
function extractImportantStackTrace(e) {let stack = e.stack;if (!stack) {
    return '';
  }
  const redundantMessage = 'Error: ' + e.message + '\n';
  if (stack.startsWith(redundantMessage)) {
    stack = stack.substring(redundantMessage.length);
  }

  const lines = stack.split('\n');
  for (let i = lines.length - 1; i >= 0; --i) {
    const line = lines[i];
    if (line.indexOf('.spec.') !== -1) {
      return lines.slice(0, i + 1).join('\n');
    }
  }
  return stack;
}

// *** Examples ***
//
// Node fail()
// > Error:
// >    at CaseRecorder.fail (/Users/kainino/src/cts/src/common/framework/logger.ts:99:30)
// >    at RunCaseSpecific.exports.g.test.t [as fn] (/Users/kainino/src/cts/src/unittests/logger.spec.ts:80:7)
// x    at RunCaseSpecific.run (/Users/kainino/src/cts/src/common/framework/test_group.ts:121:18)
// x    at processTicksAndRejections (internal/process/task_queues.js:86:5)
//
// Node throw
// > Error: hello
// >     at RunCaseSpecific.g.test.t [as fn] (/Users/kainino/src/cts/src/unittests/test_group.spec.ts:51:11)
// x     at RunCaseSpecific.run (/Users/kainino/src/cts/src/common/framework/test_group.ts:121:18)
// x     at processTicksAndRejections (internal/process/task_queues.js:86:5)
//
// Firefox fail()
// > fail@http://localhost:8080/out/framework/logger.js:104:30
// > expect@http://localhost:8080/out/framework/default_fixture.js:59:16
// > @http://localhost:8080/out/unittests/util.spec.js:35:5
// x run@http://localhost:8080/out/framework/test_group.js:119:18
//
// Firefox throw
// > @http://localhost:8080/out/unittests/test_group.spec.js:48:11
// x run@http://localhost:8080/out/framework/test_group.js:119:18
//
// Safari fail()
// > fail@http://localhost:8080/out/framework/logger.js:104:39
// > expect@http://localhost:8080/out/framework/default_fixture.js:59:20
// > http://localhost:8080/out/unittests/util.spec.js:35:11
// x http://localhost:8080/out/framework/test_group.js:119:20
// x asyncFunctionResume@[native code]
// x [native code]
// x promiseReactionJob@[native code]
//
// Safari throw
// > http://localhost:8080/out/unittests/test_group.spec.js:48:20
// x http://localhost:8080/out/framework/test_group.js:119:20
// x asyncFunctionResume@[native code]
// x [native code]
// x promiseReactionJob@[native code]
//
// Chrome fail()
// x Error
// x     at CaseRecorder.fail (http://localhost:8080/out/framework/logger.js:104:30)
// x     at DefaultFixture.expect (http://localhost:8080/out/framework/default_fixture.js:59:16)
// >     at RunCaseSpecific.fn (http://localhost:8080/out/unittests/util.spec.js:35:5)
// x     at RunCaseSpecific.run (http://localhost:8080/out/framework/test_group.js:119:18)
// x     at async runCase (http://localhost:8080/out/runtime/standalone.js:37:17)
// x     at async http://localhost:8080/out/runtime/standalone.js:102:7
//
// Chrome throw
// x Error: hello
// >     at RunCaseSpecific.fn (http://localhost:8080/out/unittests/test_group.spec.js:48:11)
// x     at RunCaseSpecific.run (http://localhost:8080/out/framework/test_group.js:119:18)"
// x     at async Promise.all (index 0)
// x     at async TestGroupTest.run (http://localhost:8080/out/unittests/test_group_test.js:6:5)
// x     at async RunCaseSpecific.fn (http://localhost:8080/out/unittests/test_group.spec.js:53:15)
// x     at async RunCaseSpecific.run (http://localhost:8080/out/framework/test_group.js:119:7)
// x     at async runCase (http://localhost:8080/out/runtime/standalone.js:37:17)
// x     at async http://localhost:8080/out/runtime/standalone.js:102:7
});
__define('common/util/data_tables.js', function (__exports) {
'use strict';
__get(__exports, 'keysOf', () => keysOf);
__get(__exports, 'numericKeysOf', () => numericKeysOf);
__get(__exports, 'objectsToRecord', () => objectsToRecord);
__get(__exports, 'makeTable', () => makeTable);
__get(__exports, 'makeTableRenameAndFilter', () => makeTableRenameAndFilter);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/

function keysOf(obj) {
  return Object.keys(obj);
}

function numericKeysOf(obj) {
  return Object.keys(obj).map((n) => Number(n));
}

/**
 * @returns a new Record from `objects`, using the string returned by Object.toString() as the keys
 * and the objects as the values.
 */
function objectsToRecord(objects) {
  const record = {};
  return objects.reduce((obj, type) => {
    return {
      ...obj,
      [type.toString()]: type
    };
  }, record);
}

/**
 * Creates an info lookup object from a more nicely-formatted table. See below for examples.
 *
 * Note: Using `as const` on the arguments to this function is necessary to infer the correct type.
 */
function makeTable(




members,
defaults,
table)


{
  const result = {};
  for (const [k, v] of Object.entries(table)) {
    const item = {};
    for (let i = 0; i < members.length; ++i) {
      item[members[i]] = v[i] ?? defaults[i];
    }
    result[k] = item;
  }

  return result;
}

/**
 * Creates an info lookup object from a more nicely-formatted table.
 *
 * Note: Using `as const` on the arguments to this function is necessary to infer the correct type.
 *
 * Example:
 *
 * ```
 * const t = makeTableWithDefaults(
 *   { c: 'default' },       // columnRenames
 *   ['a', 'default', 'd'],  // columnsKept
 *   ['a', 'b', 'c', 'd'],   // columns
 *   [123, 456, 789, 1011],  // defaults
 *   {                       // table
 *     foo: [1, 2, 3, 4],
 *     bar: [5,  ,  , 8],
 *     moo: [ , 9,10,  ],
 *   }
 * );
 *
 * // t = {
 * //   foo: { a:   1, default:   3, d:    4 },
 * //   bar: { a:   5, default: 789, d:    8 },
 * //   moo: { a: 123, default:  10, d: 1011 },
 * // };
 * ```
 *
 * MAINTENANCE_TODO: `ZipKeysWithValues<Members, Table[k], Defaults>` is incorrect
 * because Members no longer maps to Table[k]. It's not clear if this is even possible to fix
 * because it requires mapping, not zipping. Maybe passing in a index mapping
 * would fix it (which is gross) but if you have columnsKept as [0, 2, 3] then maybe it would
 * be possible to generate the correct type? I don't think we can generate the map at compile time
 * so we'd have to hand code it. Other ideas, don't generate kLimitsInfoCore and kLimitsInfoCompat
 * where they are keys of infos. Instead, generate kLimitsInfoCoreDefaults, kLimitsInfoCoreMaximums,
 * kLimitsInfoCoreClasses where each is just a `{[k: string]: type}`. Could zip those after or,
 * maybe that suggests passing in the hard coded indices would work.
 *
 * @param columnRenames the name of the column in the table that will be assigned to the 'default' property of each entry.
 * @param columnsKept the names of properties you want in the generated lookup table. This must be a subset of the columns of the tables except for the name 'default' which is looked from the previous argument.
 * @param columns the names of the columns of the name
 * @param defaults the default value by column for any element in a row of the table that is undefined
 * @param table named table rows.
 */
function makeTableRenameAndFilter(





columnRenames,
columnsKept,
columns,
defaults,
table)


{
  const result = {};
  const keyToIndex = new Map(
    columnsKept.map((name) => {
      const remappedName = columnRenames[name] === undefined ? name : columnRenames[name];
      return [name, columns.indexOf(remappedName)];
    })
  );
  for (const [k, v] of Object.entries(table)) {
    const item = {};
    for (const member of columnsKept) {
      const ndx = keyToIndex.get(member);
      item[member] = v[ndx] ?? defaults[ndx];
    }
    result[k] = item;
  }

  return result;
}
});
__define('common/util/timeout.js', function (__exports) {
'use strict';
__get(__exports, 'timeout', () => timeout);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /** Defined by WPT. Like `setTimeout`, but applies a timeout multiplier for slow test systems. */
/**
 * Equivalent of `setTimeout`, but redirects to WPT's `step_timeout` when it is defined.
 */
const timeout = typeof step_timeout !== 'undefined' ? step_timeout : setTimeout;
});
__define('common/internal/file_loader.js', function (__exports) {
'use strict';
__get(__exports, 'TestFileLoader', () => TestFileLoader);
__get(__exports, 'DefaultTestFileLoader', () => DefaultTestFileLoader);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });
let parseQuery; __bind('common/internal/query/parseQuery.js', (__m) => { ({ parseQuery } = __m); });


let loadTreeForQuery; __bind('common/internal/tree.js', (__m) => { ({ loadTreeForQuery } = __m); });

// A listing file, e.g. either of:
// - `src/webgpu/listing.ts` (which is dynamically computed, has a Promise<TestSuiteListing>)
// - `out/webgpu/listing.js` (which is pre-baked, has a TestSuiteListing)




// A .spec.ts file, as imported.















// Override the types for addEventListener/removeEventListener so the callbacks can be used as
// strongly-typed.
























// Base class for DefaultTestFileLoader and FakeTestFileLoader.

class TestFileLoader extends EventTarget {



  async importSpecFile(suite, path) {
    const url = `${suite}/${path.join('/')}.spec.js`;
    this.dispatchEvent(new MessageEvent('import', { data: { url } }));
    const ret = await this.import(url);
    this.dispatchEvent(new MessageEvent('imported', { data: { url } }));
    return ret;
  }

  async loadTree(
  query,
  {
    subqueriesToExpand = [],
    fullyExpandSubtrees = []
  } = {})
  {
    const tree = await loadTreeForQuery(this, query, {
      subqueriesToExpand: subqueriesToExpand.map((s) => {
        const q = parseQuery(s);
        assert(q.level >= 2, () => `subqueriesToExpand entries should not be multi-file:\n  ${q}`);
        return q;
      }),
      fullyExpandSubtrees: fullyExpandSubtrees.map((s) => parseQuery(s))
    });
    this.dispatchEvent(new MessageEvent('finish'));
    return tree;
  }

  async loadCases(query) {
    const tree = await this.loadTree(query);
    return tree.iterateLeaves();
  }
}

class DefaultTestFileLoader extends TestFileLoader {
  async listing(suite) {
    return (await __import('common/internal/file_loader.js', `../../${suite}/listing.js`)).listing;
  }

  import(path) {
    return __import('common/internal/file_loader.js', `../../${path}`);
  }
}
});
__define('common/internal/query/parseQuery.js', function (__exports) {
'use strict';
__get(__exports, 'parseQuery', () => parseQuery);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });let badParamValueChars, paramKeyIsPublic; __bind('common/internal/params_utils.js', (__m) => { ({ badParamValueChars, paramKeyIsPublic } = __m); });

let parseParamValue; __bind('common/internal/query/json_param_value.js', (__m) => { ({ parseParamValue } = __m); });
let TestQueryMultiFile, TestQueryMultiTest, TestQueryMultiCase, TestQuerySingleCase; __bind('common/internal/query/query.js', (__m) => { ({ TestQueryMultiFile, TestQueryMultiTest, TestQueryMultiCase, TestQuerySingleCase } = __m); });
let kBigSeparator, kWildcard, kPathSeparator, kParamSeparator; __bind('common/internal/query/separators.js', (__m) => { ({ kBigSeparator, kWildcard, kPathSeparator, kParamSeparator } = __m); });
let validQueryPart; __bind('common/internal/query/validQueryPart.js', (__m) => { ({ validQueryPart } = __m); });

/**
 * converts foo/bar/src/webgpu/this/that/file.spec.ts to webgpu:this,that,file,*
 */
function convertPathToQuery(path) {
  // removes .spec.ts and splits by directory separators.
  const parts = path.substring(0, path.length - 8).split(/\/|\\/g);
  // Gets parts only after the last `src`. Example: returns ['webgpu', 'foo', 'bar', 'test']
  // for ['Users', 'me', 'src', 'cts', 'src', 'webgpu', 'foo', 'bar', 'test']
  const partsAfterSrc = parts.slice(parts.lastIndexOf('src') + 1);
  const suite = partsAfterSrc.shift();
  return `${suite}:${partsAfterSrc.join(',')},*`;
}

/**
 * If a query looks like a path (ends in .spec.ts and has directory separators)
 * then convert try to convert it to a query.
 */
function convertPathLikeToQuery(queryOrPath) {
  return queryOrPath.endsWith('.spec.ts') && (
  queryOrPath.includes('/') || queryOrPath.includes('\\')) ?
  convertPathToQuery(queryOrPath) :
  queryOrPath;
}

/**
 * Convert long suite names (the part before the first colon) to the
 * shortest last word
 *    foo.bar.moo:test,subtest,foo -> moo:test,subtest,foo
 */
function shortenSuiteName(query) {
  const parts = query.split(':');
  // converts foo.bar.moo to moo
  const suite = parts.shift()?.replace(/.*\.(\w+)$/, '$1');
  return [suite, ...parts].join(':');
}

function parseQuery(queryLike) {
  try {
    const query = shortenSuiteName(convertPathLikeToQuery(queryLike));
    return parseQueryImpl(query);
  } catch (ex) {
    if (ex instanceof Error) {
      ex.message += `\n  on: ${queryLike}`;
    }
    throw ex;
  }
}

function parseQueryImpl(s) {
  // Undo encodeURIComponentSelectively
  s = decodeURIComponent(s);

  // bigParts are: suite, file, test, params (note kBigSeparator could appear in params)
  let suite;
  let fileString;
  let testString;
  let paramsString;
  {
    const i1 = s.indexOf(kBigSeparator);
    assert(i1 !== -1, `query string must have at least one ${kBigSeparator}`);
    suite = s.substring(0, i1);
    const i2 = s.indexOf(kBigSeparator, i1 + 1);
    if (i2 === -1) {
      fileString = s.substring(i1 + 1);
    } else {
      fileString = s.substring(i1 + 1, i2);
      const i3 = s.indexOf(kBigSeparator, i2 + 1);
      if (i3 === -1) {
        testString = s.substring(i2 + 1);
      } else {
        testString = s.substring(i2 + 1, i3);
        paramsString = s.substring(i3 + 1);
      }
    }
  }

  const { parts: file, wildcard: filePathHasWildcard } = parseBigPart(fileString, kPathSeparator);

  if (testString === undefined) {
    // Query is file-level
    assert(
      filePathHasWildcard,
      `File-level query without wildcard ${kWildcard}. Did you want a file-level query \
(append ${kPathSeparator}${kWildcard}) or test-level query (append ${kBigSeparator}${kWildcard})?`
    );
    return new TestQueryMultiFile(suite, file);
  }
  assert(!filePathHasWildcard, `Wildcard ${kWildcard} must be at the end of the query string`);

  const { parts: test, wildcard: testPathHasWildcard } = parseBigPart(testString, kPathSeparator);

  if (paramsString === undefined) {
    // Query is test-level
    assert(
      testPathHasWildcard,
      `Test-level query without wildcard ${kWildcard}; did you want a test-level query \
(append ${kPathSeparator}${kWildcard}) or case-level query (append ${kBigSeparator}${kWildcard})?`
    );
    assert(file.length > 0, 'File part of test-level query was empty (::)');
    return new TestQueryMultiTest(suite, file, test);
  }

  // Query is case-level
  assert(!testPathHasWildcard, `Wildcard ${kWildcard} must be at the end of the query string`);

  const { parts: paramsParts, wildcard: paramsHasWildcard } = parseBigPart(
    paramsString,
    kParamSeparator
  );

  assert(test.length > 0, 'Test part of case-level query was empty (::)');

  const params = {};
  for (const paramPart of paramsParts) {
    const [k, v] = parseSingleParam(paramPart);
    assert(validQueryPart.test(k), `param key names must match ${validQueryPart}`);
    params[k] = v;
  }
  if (paramsHasWildcard) {
    return new TestQueryMultiCase(suite, file, test, params);
  } else {
    return new TestQuerySingleCase(suite, file, test, params);
  }
}

// webgpu:a,b,* or webgpu:a,b,c:*
const kExampleQueries = `\
webgpu${kBigSeparator}a${kPathSeparator}b${kPathSeparator}${kWildcard} or \
webgpu${kBigSeparator}a${kPathSeparator}b${kPathSeparator}c${kBigSeparator}${kWildcard}`;

function parseBigPart(
s,
separator)
{
  if (s === '') {
    return { parts: [], wildcard: false };
  }
  const parts = s.split(separator);

  let endsWithWildcard = false;
  for (const [i, part] of parts.entries()) {
    if (i === parts.length - 1) {
      endsWithWildcard = part === kWildcard;
    }
    assert(
      part.indexOf(kWildcard) === -1 || endsWithWildcard,
      `Wildcard ${kWildcard} must be complete last part of a path (e.g. ${kExampleQueries})`
    );
  }
  if (endsWithWildcard) {
    // Remove the last element of the array (which is just the wildcard).
    parts.length = parts.length - 1;
  }
  return { parts, wildcard: endsWithWildcard };
}

function parseSingleParam(paramSubstring) {
  assert(paramSubstring !== '', 'Param in a query must not be blank (is there a trailing comma?)');
  const i = paramSubstring.indexOf('=');
  assert(i !== -1, 'Param in a query must be of form key=value');
  const k = paramSubstring.substring(0, i);
  assert(paramKeyIsPublic(k), 'Param in a query must not be private (start with _)');
  const v = paramSubstring.substring(i + 1);
  return [k, parseSingleParamValue(v)];
}

function parseSingleParamValue(s) {
  assert(
    !badParamValueChars.test(s),
    `param value must not match ${badParamValueChars} - was ${s}`
  );
  return parseParamValue(s);
}
});
__define('common/internal/params_utils.js', function (__exports) {
'use strict';
__get(__exports, 'paramKeyIsPublic', () => paramKeyIsPublic);
__get(__exports, 'extractPublicParams', () => extractPublicParams);
__get(__exports, 'badParamValueChars', () => badParamValueChars);
__get(__exports, 'publicParamsEquals', () => publicParamsEquals);
__get(__exports, 'mergeParams', () => mergeParams);
__get(__exports, 'mergeParamsChecked', () => mergeParamsChecked);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });

let comparePublicParamsPaths, Ordering; __bind('common/internal/query/compare.js', (__m) => { ({ comparePublicParamsPaths, Ordering } = __m); });
let kWildcard, kParamSeparator, kParamKVSeparator; __bind('common/internal/query/separators.js', (__m) => { ({ kWildcard, kParamSeparator, kParamKVSeparator } = __m); });















function paramKeyIsPublic(key) {
  return !key.startsWith('_');
}

function extractPublicParams(params) {
  const publicParams = {};
  for (const k of Object.keys(params)) {
    if (paramKeyIsPublic(k)) {
      publicParams[k] = params[k];
    }
  }
  return publicParams;
}

/** Used to escape reserved characters in URIs */
const kPercent = '%';

const badParamValueChars = new RegExp(
  '[' + kParamKVSeparator + kParamSeparator + kWildcard + kPercent + ']'
);

function publicParamsEquals(x, y) {
  return comparePublicParamsPaths(x, y) === Ordering.Equal;
}





/**
 * Flatten a union of interfaces into a single interface encoding the same type.
 *
 * Flattens a union in such a way that:
 * `{ a: number, b?: undefined } | { b: string, a?: undefined }`
 * (which is the value type of `[{ a: 1 }, { b: 1 }]`)
 * becomes `{ a: number | undefined, b: string | undefined }`.
 *
 * And also works for `{ a: number } | { b: string }` which maps to the same.
 */











function typeAssert() {}
{






















  {
    typeAssert();
    typeAssert();
    typeAssert();
    typeAssert();
    typeAssert();

    typeAssert();

    typeAssert();
    typeAssert();
    typeAssert();
    typeAssert();
    typeAssert();

    // Unexpected test results - hopefully okay to ignore these
    typeAssert();
    typeAssert();
  }
}






/** Merges two objects into one `{ ...a, ...b }` and return it with a flattened type. */
function mergeParams(a, b) {
  return { ...a, ...b };
}

/**
 * Merges two objects into one `{ ...a, ...b }` and asserts they had no overlapping keys.
 * This is slower than {@link mergeParams}.
 */
function mergeParamsChecked(a, b) {
  const merged = mergeParams(a, b);
  assert(
    Object.keys(merged).length === Object.keys(a).length + Object.keys(b).length,
    () => `Duplicate key between ${JSON.stringify(a)} and ${JSON.stringify(b)}`
  );
  return merged;
}
});
__define('common/internal/query/compare.js', function (__exports) {
'use strict';
__get(__exports, 'Ordering', () => Ordering);
__get(__exports, 'compareQueries', () => compareQueries);
__get(__exports, 'comparePaths', () => comparePaths);
__get(__exports, 'comparePublicParamsPaths', () => comparePublicParamsPaths);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert, objectEquals; __bind('common/util/util.js', (__m) => { ({ assert, objectEquals } = __m); });let paramKeyIsPublic; __bind('common/internal/params_utils.js', (__m) => { ({ paramKeyIsPublic } = __m); });



let Ordering = /*#__PURE__*/function (Ordering) {Ordering[Ordering["Unordered"] = 0] = "Unordered";Ordering[Ordering["StrictSuperset"] = 1] = "StrictSuperset";Ordering[Ordering["Equal"] = 2] = "Equal";Ordering[Ordering["StrictSubset"] = 3] = "StrictSubset";return Ordering;}({});






/**
 * Compares two queries for their ordering (which is used to build the tree).
 *
 * See src/unittests/query_compare.spec.ts for examples.
 */
function compareQueries(a, b) {
  if (a.suite !== b.suite) {
    return Ordering.Unordered;
  }

  const filePathOrdering = comparePaths(a.filePathParts, b.filePathParts);
  if (filePathOrdering !== Ordering.Equal || a.isMultiFile || b.isMultiFile) {
    return compareOneLevel(filePathOrdering, a.isMultiFile, b.isMultiFile);
  }
  assert('testPathParts' in a && 'testPathParts' in b);

  const testPathOrdering = comparePaths(a.testPathParts, b.testPathParts);
  if (testPathOrdering !== Ordering.Equal || a.isMultiTest || b.isMultiTest) {
    return compareOneLevel(testPathOrdering, a.isMultiTest, b.isMultiTest);
  }
  assert('params' in a && 'params' in b);

  const paramsPathOrdering = comparePublicParamsPaths(a.params, b.params);
  if (paramsPathOrdering !== Ordering.Equal || a.isMultiCase || b.isMultiCase) {
    return compareOneLevel(paramsPathOrdering, a.isMultiCase, b.isMultiCase);
  }
  return Ordering.Equal;
}

/**
 * Compares a single level of a query.
 *
 * "IsBig" means the query is big relative to the level, e.g. for test-level:
 *   - Anything >= `suite:a,*` is big
 *   - Anything <= `suite:a:*` is small
 */
function compareOneLevel(ordering, aIsBig, bIsBig) {
  assert(ordering !== Ordering.Equal || aIsBig || bIsBig);
  if (ordering === Ordering.Unordered) return Ordering.Unordered;
  if (aIsBig && bIsBig) return ordering;
  if (!aIsBig && !bIsBig) return Ordering.Unordered; // Equal case is already handled
  // Exactly one of (a, b) is big.
  if (aIsBig && ordering !== Ordering.StrictSubset) return Ordering.StrictSuperset;
  if (bIsBig && ordering !== Ordering.StrictSuperset) return Ordering.StrictSubset;
  return Ordering.Unordered;
}

/**
 * Compare two file paths, or file-local test paths, returning an Ordering between the two.
 */
function comparePaths(a, b) {
  const shorter = Math.min(a.length, b.length);

  for (let i = 0; i < shorter; ++i) {
    if (a[i] !== b[i]) {
      return Ordering.Unordered;
    }
  }
  if (a.length === b.length) {
    return Ordering.Equal;
  } else if (a.length < b.length) {
    return Ordering.StrictSuperset;
  } else {
    return Ordering.StrictSubset;
  }
}

function comparePublicParamsPaths(a, b) {
  const aKeys = Object.keys(a).filter((k) => paramKeyIsPublic(k));
  const commonKeys = new Set(aKeys.filter((k) => k in b));

  for (const k of commonKeys) {
    // Treat +/-0.0 as different query by distinguishing them in objectEquals
    if (!objectEquals(a[k], b[k], true)) {
      return Ordering.Unordered;
    }
  }
  const bKeys = Object.keys(b).filter((k) => paramKeyIsPublic(k));
  const aRemainingKeys = aKeys.length - commonKeys.size;
  const bRemainingKeys = bKeys.length - commonKeys.size;
  if (aRemainingKeys === 0 && bRemainingKeys === 0) return Ordering.Equal;
  if (aRemainingKeys === 0) return Ordering.StrictSuperset;
  if (bRemainingKeys === 0) return Ordering.StrictSubset;
  return Ordering.Unordered;
}
});
__define('common/internal/query/separators.js', function (__exports) {
'use strict';
__get(__exports, 'kBigSeparator', () => kBigSeparator);
__get(__exports, 'kPathSeparator', () => kPathSeparator);
__get(__exports, 'kParamSeparator', () => kParamSeparator);
__get(__exports, 'kParamKVSeparator', () => kParamKVSeparator);
__get(__exports, 'kWildcard', () => kWildcard);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /** Separator between big parts: suite:file:test:case */const kBigSeparator = ':';
/** Separator between path,to,file or path,to,test */
const kPathSeparator = ',';

/** Separator between k=v;k=v */
const kParamSeparator = ';';

/** Separator between key and value in k=v */
const kParamKVSeparator = '=';

/** Final wildcard, if query is not single-case */
const kWildcard = '*';
});
__define('common/internal/query/json_param_value.js', function (__exports) {
'use strict';
__get(__exports, 'stringifyParamValue', () => stringifyParamValue);
__get(__exports, 'stringifyParamValueUniquely', () => stringifyParamValueUniquely);
__get(__exports, 'parseParamValue', () => parseParamValue);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert, sortObjectByKey, isPlainObject; __bind('common/util/util.js', (__m) => { ({ assert, sortObjectByKey, isPlainObject } = __m); });
// JSON can't represent various values and by default stores them as `null`.
// Instead, storing them as a magic string values in JSON.
const jsUndefinedMagicValue = '_undef_';
const jsNaNMagicValue = '_nan_';
const jsPositiveInfinityMagicValue = '_posinfinity_';
const jsNegativeInfinityMagicValue = '_neginfinity_';

// -0 needs to be handled separately, because -0 === +0 returns true. Not
// special casing +0/0, since it behaves intuitively. Assuming that if -0 is
// being used, the differentiation from +0 is desired.
const jsNegativeZeroMagicValue = '_negzero_';

// bigint values are not defined in JSON, so need to wrap them up as strings
const jsBigIntMagicPattern = /^(\d+)n$/;

const toStringMagicValue = new Map([
[undefined, jsUndefinedMagicValue],
[NaN, jsNaNMagicValue],
[Number.POSITIVE_INFINITY, jsPositiveInfinityMagicValue],
[Number.NEGATIVE_INFINITY, jsNegativeInfinityMagicValue]
// No -0 handling because it is special cased.
]);

const fromStringMagicValue = new Map([
[jsUndefinedMagicValue, undefined],
[jsNaNMagicValue, NaN],
[jsPositiveInfinityMagicValue, Number.POSITIVE_INFINITY],
[jsNegativeInfinityMagicValue, Number.NEGATIVE_INFINITY],
// -0 is handled in this direction because there is no comparison issue.
[jsNegativeZeroMagicValue, -0]]
);

function stringifyFilter(_k, v) {
  // Make sure no one actually uses a magic value as a parameter.
  if (typeof v === 'string') {
    assert(
      !fromStringMagicValue.has(v),
      `${v} is a magic value for stringification, so cannot be used`
    );

    assert(
      v !== jsNegativeZeroMagicValue,
      `${v} is a magic value for stringification, so cannot be used`
    );

    assert(
      v.match(jsBigIntMagicPattern) === null,
      `${v} matches bigint magic pattern for stringification, so cannot be used`
    );
  }

  const isObject = v !== null && typeof v === 'object' && !Array.isArray(v);
  if (isObject) {
    assert(
      isPlainObject(v),
      `value must be a plain object but it appears to be a '${
      Object.getPrototypeOf(v).constructor.name
      }`
    );
  }
  assert(typeof v !== 'function', `${v} can not be a function`);

  if (Object.is(v, -0)) {
    return jsNegativeZeroMagicValue;
  }

  if (typeof v === 'bigint') {
    return `${v}n`;
  }

  return toStringMagicValue.has(v) ? toStringMagicValue.get(v) : v;
}

function stringifyParamValue(value) {
  return JSON.stringify(value, stringifyFilter);
}

/**
 * Like stringifyParamValue but sorts dictionaries by key, for hashing.
 */
function stringifyParamValueUniquely(value) {
  return JSON.stringify(value, (k, v) => {
    if (typeof v === 'object' && v !== null) {
      return sortObjectByKey(v);
    }

    return stringifyFilter(k, v);
  });
}

// 'any' is part of the JSON.parse reviver interface, so cannot be avoided.

function parseParamValueReviver(_k, v) {
  if (fromStringMagicValue.has(v)) {
    return fromStringMagicValue.get(v);
  }

  if (typeof v === 'string') {
    const match = v.match(jsBigIntMagicPattern);
    if (match !== null) {
      // [0] is the entire match, and following entries are the capture groups
      return BigInt(match[1]);
    }
  }

  return v;
}

function parseParamValue(s) {
  return JSON.parse(s, parseParamValueReviver);
}
});
__define('common/internal/query/query.js', function (__exports) {
'use strict';
__get(__exports, 'TestQueryMultiFile', () => TestQueryMultiFile);
__get(__exports, 'TestQueryMultiTest', () => TestQueryMultiTest);
__get(__exports, 'TestQueryMultiCase', () => TestQueryMultiCase);
__get(__exports, 'TestQuerySingleCase', () => TestQuerySingleCase);
__get(__exports, 'parseExpectationsForTestQuery', () => parseExpectationsForTestQuery);
__get(__exports, 'relativeQueryString', () => relativeQueryString);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let optionWorkerMode; __bind('common/runtime/helper/options.js', (__m) => { ({ optionWorkerMode } = __m); });let assert, unreachable; __bind('common/util/util.js', (__m) => { ({ assert, unreachable } = __m); });


let compareQueries, Ordering; __bind('common/internal/query/compare.js', (__m) => { ({ compareQueries, Ordering } = __m); });
let encodeURIComponentSelectively; __bind('common/internal/query/encode_selectively.js', (__m) => { ({ encodeURIComponentSelectively } = __m); });
let parseQuery; __bind('common/internal/query/parseQuery.js', (__m) => { ({ parseQuery } = __m); });
let kBigSeparator, kPathSeparator, kWildcard; __bind('common/internal/query/separators.js', (__m) => { ({ kBigSeparator, kPathSeparator, kWildcard } = __m); });
let stringifyPublicParams; __bind('common/internal/query/stringify_params.js', (__m) => { ({ stringifyPublicParams } = __m); });

/**
 * Represents a test query of some level.
 *
 * TestQuery types are immutable.
 */






/**
 * - 1 = MultiFile.
 * - 2 = MultiTest.
 * - 3 = MultiCase.
 * - 4 = SingleCase.
 */







/**
 * A multi-file test query, like `s:*` or `s:a,b,*`.
 *
 * Immutable (makes copies of constructor args).
 */
class TestQueryMultiFile {
  level = 1;
  isMultiFile = true;



  constructor(suite, file) {
    this.suite = suite;
    this.filePathParts = [...file];
  }

  get depthInLevel() {
    return this.filePathParts.length;
  }

  toString() {
    return encodeURIComponentSelectively(this.toStringHelper().join(kBigSeparator));
  }

  toStringHelper() {
    return [this.suite, [...this.filePathParts, kWildcard].join(kPathSeparator)];
  }
}

/**
 * A multi-test test query, like `s:f:*` or `s:f:a,b,*`.
 *
 * Immutable (makes copies of constructor args).
 */
class TestQueryMultiTest extends TestQueryMultiFile {
  level = 2;
  isMultiFile = false;
  isMultiTest = true;


  constructor(suite, file, test) {
    super(suite, file);
    assert(file.length > 0, 'multi-test (or finer) query must have file-path');
    this.testPathParts = [...test];
  }

  get depthInLevel() {
    return this.testPathParts.length;
  }

  toStringHelper() {
    return [
    this.suite,
    this.filePathParts.join(kPathSeparator),
    [...this.testPathParts, kWildcard].join(kPathSeparator)];

  }
}

/**
 * A multi-case test query, like `s:f:t:*` or `s:f:t:a,b,*`.
 *
 * Immutable (makes copies of constructor args), except for param values
 * (which aren't normally supposed to change; they're marked readonly in TestParams).
 */
class TestQueryMultiCase extends TestQueryMultiTest {
  level = 3;
  isMultiTest = false;
  isMultiCase = true;


  constructor(suite, file, test, params) {
    super(suite, file, test);
    assert(test.length > 0, 'multi-case (or finer) query must have test-path');
    this.params = { ...params };
  }

  get depthInLevel() {
    return Object.keys(this.params).length;
  }

  toStringHelper() {
    return [
    this.suite,
    this.filePathParts.join(kPathSeparator),
    this.testPathParts.join(kPathSeparator),
    stringifyPublicParams(this.params, true)];

  }
}

/**
 * A multi-case test query, like `s:f:t:` or `s:f:t:a=1,b=1`.
 *
 * Immutable (makes copies of constructor args).
 */
class TestQuerySingleCase extends TestQueryMultiCase {
  level = 4;
  isMultiCase = false;

  get depthInLevel() {
    return 0;
  }

  toStringHelper() {
    return [
    this.suite,
    this.filePathParts.join(kPathSeparator),
    this.testPathParts.join(kPathSeparator),
    stringifyPublicParams(this.params)];

  }
}

/**
 * Parse raw expectations input into TestQueryWithExpectation[], filtering so that only
 * expectations that are relevant for the provided query and wptURL.
 *
 * `rawExpectations` should be @type {{ query: string, expectation: Expectation }[]}
 *
 * The `rawExpectations` are parsed and validated that they are in the correct format.
 * If `wptURL` is passed, the query string should be of the full path format such
 * as `path/to/cts.https.html?worker=0&q=suite:test_path:test_name:foo=1;bar=2;*`.
 * If `wptURL` is `undefined`, the query string should be only the query
 * `suite:test_path:test_name:foo=1;bar=2;*`.
 */
function parseExpectationsForTestQuery(
rawExpectations,





query,
wptURL)
{
  if (!Array.isArray(rawExpectations)) {
    unreachable('Expectations should be an array');
  }
  const expectations = [];
  for (const entry of rawExpectations) {
    assert(typeof entry === 'object');
    const rawExpectation = entry;
    assert(rawExpectation.query !== undefined, 'Expectation missing query string');
    assert(rawExpectation.expectation !== undefined, 'Expectation missing expectation string');

    let expectationQuery;
    if (wptURL !== undefined) {
      const expectationURL = new URL(`${wptURL.origin}/${entry.query}`);
      if (expectationURL.pathname !== wptURL.pathname) {
        continue;
      }
      assert(
        expectationURL.pathname === wptURL.pathname,
        `Invalid expectation path ${expectationURL.pathname}
Expectation should be of the form path/to/cts.https.html?debug=0&q=suite:test_path:test_name:foo=1;bar=2;...
        `
      );

      const params = expectationURL.searchParams;
      if (optionWorkerMode('worker', params) !== optionWorkerMode('worker', wptURL.searchParams)) {
        continue;
      }

      const qs = params.getAll('q');
      assert(qs.length === 1, 'currently, there must be exactly one ?q= in the expectation string');
      expectationQuery = parseQuery(qs[0]);
    } else {
      expectationQuery = parseQuery(entry.query);
    }

    // Strip params from multicase expectations so that an expectation of foo=2;*
    // is stored if the test query is bar=3;*
    const queryForFilter =
    expectationQuery instanceof TestQueryMultiCase ?
    new TestQueryMultiCase(
      expectationQuery.suite,
      expectationQuery.filePathParts,
      expectationQuery.testPathParts,
      {}
    ) :
    expectationQuery;

    if (compareQueries(query, queryForFilter) === Ordering.Unordered) {
      continue;
    }

    switch (entry.expectation) {
      case 'pass':
      case 'skip':
      case 'fail':
        break;
      default:
        unreachable(`Invalid expectation ${entry.expectation}`);
    }

    expectations.push({
      query: expectationQuery,
      expectation: entry.expectation
    });
  }
  return expectations;
}

/**
 * For display purposes only, produces a "relative" query string from parent to child.
 * Used in the wpt runtime to reduce the verbosity of logs.
 */
function relativeQueryString(parent, child) {
  const ordering = compareQueries(parent, child);
  if (ordering === Ordering.Equal) {
    return '';
  } else if (ordering === Ordering.StrictSuperset) {
    const parentString = parent.toString();
    assert(parentString.endsWith(kWildcard));
    const childString = child.toString();
    assert(
      childString.startsWith(parentString.substring(0, parentString.length - 2)),
      'impossible?: childString does not start with parentString[:-2]'
    );
    return childString.substring(parentString.length - 2);
  } else {
    unreachable(
      `relativeQueryString arguments have invalid ordering ${ordering}:\n${parent}\n${child}`
    );
  }
}
});
__define('common/runtime/helper/options.js', function (__exports) {
'use strict';
__get(__exports, 'optionEnabled', () => optionEnabled);
__get(__exports, 'optionString', () => optionString);
__get(__exports, 'optionWorkerMode', () => optionWorkerMode);
__get(__exports, 'kDefaultCTSOptions', () => kDefaultCTSOptions);
__get(__exports, 'kCTSOptionsInfo', () => kCTSOptionsInfo);
__get(__exports, 'camelCaseToSnakeCase', () => camelCaseToSnakeCase);
__get(__exports, 'parseSearchParamLikeWithOptions', () => parseSearchParamLikeWithOptions);
__get(__exports, 'parseSearchParamLikeWithCTSOptions', () => parseSearchParamLikeWithCTSOptions);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let unreachable; __bind('common/util/util.js', (__m) => { ({ unreachable } = __m); });let windowURL = undefined;
function getWindowURL() {
  if (windowURL === undefined) {
    windowURL = new URL(window.location.toString());
  }
  return windowURL;
}

/** Parse a runner option that is always boolean-typed. False if missing or '0'. */
function optionEnabled(
opt,
searchParams = getWindowURL().searchParams)
{
  const val = searchParams.get(opt);
  return val !== null && val !== '0';
}

/** Parse a runner option that is string-typed. If the option is missing, returns `null`. */
function optionString(
opt,
searchParams = getWindowURL().searchParams)
{
  return searchParams.get(opt);
}

/** Runtime modes for running tests in different types of workers. */

/** Parse a runner option for different worker modes (as in `?worker=shared`). Null if no worker. */
function optionWorkerMode(
opt,
searchParams = getWindowURL().searchParams)
{
  const value = searchParams.get(opt);
  if (value === null || value === '0') {
    return null;
  } else if (value === 'service') {
    return 'service';
  } else if (value === 'shared') {
    return 'shared';
  } else if (value === '' || value === '1' || value === 'dedicated') {
    return 'dedicated';
  }
  unreachable('invalid worker= option value');
}

/**
 * The possible options for the tests.
 */














const kDefaultCTSOptions = {
  worker: null,
  debug: false,
  compatibility: false,
  forceFallbackAdapter: false,
  enforceDefaultLimits: false,
  blockAllFeatures: false,
  unrollConstEvalLoops: false,
  powerPreference: null,
  subcasesBetweenAttemptingGC: '5000',
  casesBetweenReplacingDevice: 'Infinity',
  logToWebSocket: false
};

/**
 * Extra per option info.
 */






/**
 * Type for info for every option. This definition means adding an option
 * will generate a compile time error if no extra info is provided.
 */


/**
 * Options to the CTS.
 */
const kCTSOptionsInfo = {
  worker: {
    description: 'run in a worker',
    parser: optionWorkerMode,
    selectValueDescriptions: [
    { value: null, description: 'no worker' },
    { value: 'dedicated', description: 'dedicated worker' },
    { value: 'shared', description: 'shared worker' },
    { value: 'service', description: 'service worker' }]

  },
  debug: { description: 'show more info' },
  compatibility: { description: 'request adapters with featureLevel: "compatibility"' },
  forceFallbackAdapter: { description: 'pass forceFallbackAdapter: true to requestAdapter' },
  enforceDefaultLimits: {
    description: `force the adapter limits to the default limits.
Note: May fail on tests for low-power/high-performance`
  },
  blockAllFeatures: {
    description: `block all features on adapter - except 'core-features-and-limits'.
Note: The spec requires bc or etc2+astc which means tests checking that one or other must exist will fail.
`
  },
  unrollConstEvalLoops: { description: 'unroll const eval loops in WGSL' },
  powerPreference: {
    description: 'set default powerPreference for some tests',
    parser: optionString,
    selectValueDescriptions: [
    { value: null, description: 'default' },
    { value: 'low-power', description: 'low-power' },
    { value: 'high-performance', description: 'high-performance' }]

  },
  subcasesBetweenAttemptingGC: {
    description:
    'After this many subcases, run attemptGarbageCollection(). (For custom values, edit the URL.)',
    parser: optionString,
    selectValueDescriptions: [
    { value: null, description: 'default' },
    { value: 'Infinity', description: 'Infinity' },
    { value: '5000', description: '5000' },
    { value: '50', description: '50' },
    { value: '1', description: '1' }]

  },
  casesBetweenReplacingDevice: {
    description:
    'After this many cases use a device, destroy and replace it to free GPU resources. (For custom values, edit the URL.)',
    parser: optionString,
    selectValueDescriptions: [
    { value: null, description: 'default' },
    { value: 'Infinity', description: 'Infinity' },
    { value: '5000', description: '5000' },
    { value: '50', description: '50' },
    { value: '1', description: '1' }]

  },
  logToWebSocket: { description: 'send some logs to ws://localhost:59497/' }
};

/**
 * Converts camel case to snake case.
 * Examples:
 *    fooBar -> foo_bar
 *    parseHTMLFile -> parse_html_file
 */
function camelCaseToSnakeCase(id) {
  return id.
  replace(/(.)([A-Z][a-z]+)/g, '$1_$2').
  replace(/([a-z0-9])([A-Z])/g, '$1_$2').
  toLowerCase();
}

/**
 * Creates a Options from search parameters.
 */
function getOptionsInfoFromSearchString(
optionsInfos,
searchString)
{
  const searchParams = new URLSearchParams(searchString);
  const optionValues = {};
  for (const [optionName, info] of Object.entries(optionsInfos)) {
    const parser = info.parser || optionEnabled;
    optionValues[optionName] = parser(camelCaseToSnakeCase(optionName), searchParams);
  }
  return optionValues;
}

/**
 * Given a test query string in the form of `suite:foo,bar,moo&opt1=val1&opt2=val2
 * returns the query and the options.
 */
function parseSearchParamLikeWithOptions(
optionsInfos,
query)



{
  const searchString = query.includes('q=') || query.startsWith('?') ? query : `q=${query}`;
  const queries = new URLSearchParams(searchString).getAll('q');
  const options = getOptionsInfoFromSearchString(optionsInfos, searchString);
  return { queries, options };
}

/**
 * Given a test query string in the form of `suite:foo,bar,moo&opt1=val1&opt2=val2
 * returns the query and the common options.
 */
function parseSearchParamLikeWithCTSOptions(query) {
  return parseSearchParamLikeWithOptions(kCTSOptionsInfo, query);
}
});
__define('common/internal/query/encode_selectively.js', function (__exports) {
'use strict';
__get(__exports, 'encodeURIComponentSelectively', () => encodeURIComponentSelectively);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /**
 * Encodes a stringified TestQuery so that it can be placed in a `?q=` parameter in a URL.
 *
 * `encodeURIComponent` encodes in accordance with `application/x-www-form-urlencoded`,
 * but URLs don't actually have to be as strict as HTML form encoding
 * (we interpret this purely from JavaScript).
 * So we encode the component, then selectively convert some %-encoded escape codes
 * back to their original form for readability/copyability.
 */function encodeURIComponentSelectively(s) {let ret = encodeURIComponent(s);
  ret = ret.replace(/%22/g, '"'); // for JSON strings
  ret = ret.replace(/%2C/g, ','); // for path separator, and JSON arrays
  ret = ret.replace(/%3A/g, ':'); // for big separator
  ret = ret.replace(/%3B/g, ';'); // for param separator
  ret = ret.replace(/%3D/g, '='); // for params (k=v)
  ret = ret.replace(/%5B/g, '['); // for JSON arrays
  ret = ret.replace(/%5D/g, ']'); // for JSON arrays
  ret = ret.replace(/%7B/g, '{'); // for JSON objects
  ret = ret.replace(/%7D/g, '}'); // for JSON objects
  ret = ret.replace(/%E2%9C%97/g, '\u2717'); // for jsUndefinedMagicValue
  return ret;
}
});
__define('common/internal/query/stringify_params.js', function (__exports) {
'use strict';
__get(__exports, 'stringifyPublicParams', () => stringifyPublicParams);
__get(__exports, 'stringifyPublicParamsUniquely', () => stringifyPublicParamsUniquely);
__get(__exports, 'stringifySingleParam', () => stringifySingleParam);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });let badParamValueChars, paramKeyIsPublic; __bind('common/internal/params_utils.js', (__m) => { ({ badParamValueChars, paramKeyIsPublic } = __m); });

let stringifyParamValue, stringifyParamValueUniquely; __bind('common/internal/query/json_param_value.js', (__m) => { ({ stringifyParamValue, stringifyParamValueUniquely } = __m); });
let kParamKVSeparator, kParamSeparator, kWildcard; __bind('common/internal/query/separators.js', (__m) => { ({ kParamKVSeparator, kParamSeparator, kWildcard } = __m); });

function stringifyPublicParams(p, addWildcard = false) {
  const parts = Object.keys(p).
  filter((k) => paramKeyIsPublic(k)).
  map((k) => stringifySingleParam(k, p[k]));

  if (addWildcard) parts.push(kWildcard);

  return parts.join(kParamSeparator);
}

/**
 * An _approximately_ unique string representing a CaseParams value.
 */
function stringifyPublicParamsUniquely(p) {
  const keys = Object.keys(p).sort();
  return keys.
  filter((k) => paramKeyIsPublic(k)).
  map((k) => stringifySingleParamUniquely(k, p[k])).
  join(kParamSeparator);
}

function stringifySingleParam(k, v) {
  return `${k}${kParamKVSeparator}${stringifySingleParamValue(v)}`;
}

function stringifySingleParamUniquely(k, v) {
  return `${k}${kParamKVSeparator}${stringifyParamValueUniquely(v)}`;
}

function stringifySingleParamValue(v) {
  const s = stringifyParamValue(v);
  assert(
    !badParamValueChars.test(s),
    `JSON.stringified param value must not match ${badParamValueChars} - was ${s}`
  );
  return s;
}
});
__define('common/internal/query/validQueryPart.js', function (__exports) {
'use strict';
__get(__exports, 'validQueryPart', () => validQueryPart);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /** Applies to group parts, test parts, params keys. */const validQueryPart = /^[a-zA-Z0-9_]+$/;
});
__define('common/internal/tree.js', function (__exports) {
'use strict';
__get(__exports, 'TestTree', () => TestTree);
__get(__exports, 'loadTreeForQuery', () => loadTreeForQuery);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });let assert, now; __bind('common/util/util.js', (__m) => { ({ assert, now } = __m); });



let comparePublicParamsPaths, compareQueries, Ordering; __bind('common/internal/query/compare.js', (__m) => { ({ comparePublicParamsPaths, compareQueries, Ordering } = __m); });
let TestQueryMultiCase, TestQuerySingleCase, TestQueryMultiFile, TestQueryMultiTest; __bind('common/internal/query/query.js', (__m) => { ({ TestQueryMultiCase, TestQuerySingleCase, TestQueryMultiFile, TestQueryMultiTest } = __m); });
let kBigSeparator, kWildcard, kPathSeparator, kParamSeparator; __bind('common/internal/query/separators.js', (__m) => { ({ kBigSeparator, kWildcard, kPathSeparator, kParamSeparator } = __m); });
let stringifySingleParam; __bind('common/internal/query/stringify_params.js', (__m) => { ({ stringifySingleParam } = __m); });
let StacklessError; __bind('common/internal/util.js', (__m) => { ({ StacklessError } = __m); });

// `loadTreeForQuery()` loads a TestTree for a given queryToLoad.
// The resulting tree is a linked-list all the way from `suite:*` to queryToLoad,
// and under queryToLoad is a tree containing every case matched by queryToLoad.
//
// `subqueriesToExpand` influences the `collapsible` flag on nodes in the resulting tree.
// A node is considered "collapsible" if none of the subqueriesToExpand is a StrictSubset
// of that node.
//
// In WebKit/Blink-style web_tests, an expectation file marks individual cts.https.html "variants
// as "Failure", "Crash", etc. By passing in the list of expectations as the subqueriesToExpand,
// we can programmatically subdivide the cts.https.html "variants" list to be able to implement
// arbitrarily-fine suppressions (instead of having to suppress entire test files, which would
// lose a lot of coverage).
//
// `iterateCollapsedNodes()` produces the list of queries for the variants list.
//
// Though somewhat complicated, this system has important benefits:
//   - Avoids having to suppress entire test files, which would cause large test coverage loss.
//   - Minimizes the number of page loads needed for fine-grained suppressions.
//     (In the naive case, we could do one page load per test case - but the test suite would
//     take impossibly long to run.)
//   - Enables developers to put any number of tests in one file as appropriate, without worrying
//     about expectation granularity.





























/**
 * When iterating through "collapsed" tree nodes, indicates how many "query levels" to traverse
 * through before starting to collapse nodes.
 *
 * Corresponds with TestQueryLevel, but excludes 4 (SingleCase):
 * - 1 = MultiFile. Expands so every file is in the collapsed tree.
 * - 2 = MultiTest. Expands so every test is in the collapsed tree.
 * - 3 = MultiCase. Expands so every case is in the collapsed tree (i.e. collapsing disabled).
 */


class TestTree {
  /**
   * The `queryToLoad` that this test tree was created for.
   * Test trees are always rooted at `suite:*`, but they only contain nodes that fit
   * within `forQuery`.
   *
   * This is used for `iterateCollapsedNodes` which only starts collapsing at the next
   * `TestQueryLevel` after `forQuery`.
   */



  constructor(forQuery, root) {
    this.forQuery = forQuery;
    this.root = root;
    assert(
      root.query.level === 1 && root.query.depthInLevel === 0,
      'TestTree root must be the root (suite:*)'
    );
  }

  static async create(forQuery, root) {
    await TestTree.propagateCounts(root);
    return new TestTree(forQuery, root);
  }

  /**
   * Iterate through the leaves of a version of the tree which has been pruned to exclude
   * subtrees which:
   * - are at a deeper `TestQueryLevel` than `this.forQuery`, and
   * - were not a `Ordering.StrictSubset` of any of the `subqueriesToExpand` during tree creation.
   */
  iterateCollapsedNodes({
    includeIntermediateNodes = false,
    includeEmptySubtrees = false,
    alwaysExpandThroughLevel







  }) {
    const expandThroughLevel = Math.max(this.forQuery.level, alwaysExpandThroughLevel);
    return TestTree.iterateSubtreeNodes(this.root, {
      includeIntermediateNodes,
      includeEmptySubtrees,
      expandThroughLevel
    });
  }

  iterateLeaves() {
    return TestTree.iterateSubtreeLeaves(this.root);
  }

  /**
   * Dissolve nodes which have only one child, e.g.:
   *   a,* { a,b,* { a,b:* { ... } } }
   * collapses down into:
   *   a,* { a,b:* { ... } }
   * which is less needlessly verbose when displaying the tree in the standalone runner.
   */
  dissolveSingleChildTrees() {
    const newRoot = dissolveSingleChildTrees(this.root);
    assert(newRoot === this.root);
  }

  toString() {
    return TestTree.subtreeToString('(root)', this.root, '');
  }

  static *iterateSubtreeNodes(
  subtree,
  opts)




  {
    if (opts.includeIntermediateNodes) {
      yield subtree;
    }

    for (const [, child] of subtree.children) {
      if ('children' in child) {
        // Is a subtree
        const collapsible = child.collapsible && child.query.level > opts.expandThroughLevel;
        if (child.children.size > 0 && !collapsible) {
          yield* TestTree.iterateSubtreeNodes(child, opts);
        } else if (child.children.size > 0 || opts.includeEmptySubtrees) {
          // Don't yield empty subtrees (e.g. files with no tests) unless includeEmptySubtrees
          yield child;
        }
      } else {
        // Is a leaf
        yield child;
      }
    }
  }

  static *iterateSubtreeLeaves(subtree) {
    for (const [, child] of subtree.children) {
      if ('children' in child) {
        yield* TestTree.iterateSubtreeLeaves(child);
      } else {
        yield child;
      }
    }
  }

  /** Propagate the subtreeTODOs/subtreeTests state upward from leaves to parent nodes. */
  static async propagateCounts(
  subtree)
  {
    subtree.subtreeCounts ??= { tests: 0, nodesWithTODO: 0, totalTimeMS: 0 };
    subtree.subcaseCount = 0;
    for (const [, child] of subtree.children) {
      if ('children' in child) {
        const counts = await TestTree.propagateCounts(child);
        subtree.subtreeCounts.tests += counts.tests;
        subtree.subtreeCounts.nodesWithTODO += counts.nodesWithTODO;
        subtree.subtreeCounts.totalTimeMS += counts.totalTimeMS;
        subtree.subcaseCount += counts.subcaseCount;
      } else {
        subtree.subcaseCount = child.subcaseCount;
      }
    }

    return { ...subtree.subtreeCounts, subcaseCount: subtree.subcaseCount ?? 0 };
  }

  /** Displays counts in the format `(Nodes with TODOs) / (Total test count)`. */
  static countsToString(tree) {
    if (tree.subtreeCounts) {
      return `${tree.subtreeCounts.nodesWithTODO} / ${tree.subtreeCounts.tests}`;
    } else {
      return '';
    }
  }

  static subtreeToString(name, tree, indent) {
    const collapsible = 'run' in tree ? '>' : tree.collapsible ? '+' : '-';
    let s =
    indent +
    `${collapsible} ${TestTree.countsToString(tree)} ${JSON.stringify(name)} => ${tree.query}`;
    if ('children' in tree) {
      if (tree.description !== undefined) {
        s += `\n${indent}  | ${JSON.stringify(tree.description)}`;
      }

      for (const [name, child] of tree.children) {
        s += '\n' + TestTree.subtreeToString(name, child, indent + '  ');
      }
    }
    return s;
  }
}

// MAINTENANCE_TODO: Consider having subqueriesToExpand actually impact the depth-order of params
// in the tree.
async function loadTreeForQuery(
loader,
queryToLoad,
{
  subqueriesToExpand,
  fullyExpandSubtrees = []
})
{
  const suite = queryToLoad.suite;
  const specs = await loader.listing(suite);

  const subqueriesToExpandEntries = Array.from(subqueriesToExpand.entries());
  const seenSubqueriesToExpand = new Array(subqueriesToExpand.length);
  seenSubqueriesToExpand.fill(false);

  const isCollapsible = (subquery) =>
  subqueriesToExpandEntries.every(([i, toExpand]) => {
    const ordering = compareQueries(toExpand, subquery);

    // If toExpand == subquery, no expansion is needed (but it's still "seen").
    if (ordering === Ordering.Equal) seenSubqueriesToExpand[i] = true;
    return ordering !== Ordering.StrictSubset;
  }) &&
  fullyExpandSubtrees.every((toExpand) => {
    const ordering = compareQueries(toExpand, subquery);
    return ordering === Ordering.Unordered;
  });

  // L0 = suite-level, e.g. suite:*
  // L1 =  file-level, e.g. suite:a,b:*
  // L2 =  test-level, e.g. suite:a,b:c,d:*
  // L3 =  case-level, e.g. suite:a,b:c,d:
  let foundCase = false;
  // L0 is suite:*
  const subtreeL0 = makeTreeForSuite(suite, isCollapsible);

  const imports_start = now();
  const pEntriesWithImports = []; // Promise<entry with importedSpec>[]
  for (const entry of specs) {
    if (entry.file.length === 0 && 'readme' in entry) {
      // Suite-level readme.
      setSubtreeDescriptionAndCountTODOs(subtreeL0, entry.readme);
      continue;
    }

    {
      const queryL1 = new TestQueryMultiFile(suite, entry.file);
      const orderingL1 = compareQueries(queryL1, queryToLoad);
      if (orderingL1 === Ordering.Unordered) {
        // File path is not matched by this query.
        continue;
      }
    }

    // We're going to be fetching+importing a bunch of things, so do it in async.
    const pEntryWithImport = (async () => {
      if ('readme' in entry) {
        return entry;
      } else {
        return {
          ...entry,
          importedSpec: await loader.importSpecFile(queryToLoad.suite, entry.file)
        };
      }
    })();

    const kForceSerialImporting = false;
    if (kForceSerialImporting) {
      await pEntryWithImport;
    }
    pEntriesWithImports.push(pEntryWithImport);
  }

  const entriesWithImports = await Promise.all(pEntriesWithImports);
  if (globalTestConfig.frameworkDebugLog) {
    const imported_time = performance.now() - imports_start;
    globalTestConfig.frameworkDebugLog(
      `Imported importedSpecFiles[${entriesWithImports.length}] in ${imported_time}ms.`
    );
  }

  for (const entry of entriesWithImports) {
    if ('readme' in entry) {
      // Entry is a README that is an ancestor or descendant of the query.
      // (It's included for display in the standalone runner.)

      // readmeSubtree is suite:a,b,*
      // (This is always going to dedup with a file path, if there are any test spec files under
      // the directory that has the README).
      const readmeSubtree = addSubtreeForDirPath(
        subtreeL0,
        entry.file,
        isCollapsible
      );
      setSubtreeDescriptionAndCountTODOs(readmeSubtree, entry.readme);
      continue;
    }

    // Entry is a spec file.
    const spec = entry.importedSpec;
    // subtreeL1 is suite:a,b:*
    const subtreeL1 = addSubtreeForFilePath(
      subtreeL0,
      entry.file,
      isCollapsible
    );
    setSubtreeDescriptionAndCountTODOs(subtreeL1, spec.description);

    let groupHasTests = false;
    for (const t of spec.g.iterate()) {
      groupHasTests = true;
      {
        const queryL2 = new TestQueryMultiCase(suite, entry.file, t.testPath, {});
        const orderingL2 = compareQueries(queryL2, queryToLoad);
        if (orderingL2 === Ordering.Unordered) {
          // Test path is not matched by this query.
          continue;
        }
      }

      // subtreeL2 is suite:a,b:c,d:*
      const subtreeL2 = addSubtreeForTestPath(
        subtreeL1,
        t.testPath,
        t.testCreationStack,
        isCollapsible
      );
      // This is 1 test. Set tests=1 then count TODOs.
      subtreeL2.subtreeCounts ??= { tests: 1, nodesWithTODO: 0, totalTimeMS: 0 };
      if (t.description) setSubtreeDescriptionAndCountTODOs(subtreeL2, t.description);

      let caseFilter = null;
      if ('params' in queryToLoad) {
        caseFilter = queryToLoad.params;
      }

      // MAINTENANCE_TODO: If tree generation gets too slow, avoid actually iterating the cases in a
      // file if there's no need to (based on the subqueriesToExpand).
      for (const c of t.iterate(caseFilter)) {
        // iterate() guarantees c's query is equal to or a subset of queryToLoad.

        if (queryToLoad instanceof TestQuerySingleCase) {
          // A subset is OK if it's TestQueryMultiCase, but for SingleCase it must match exactly.
          const ordering = comparePublicParamsPaths(c.id.params, queryToLoad.params);
          if (ordering !== Ordering.Equal) {
            continue;
          }
        }

        // Leaf for case is suite:a,b:c,d:x=1;y=2
        addLeafForCase(subtreeL2, c, isCollapsible);
        foundCase = true;
      }
    }
    if (!groupHasTests && !subtreeL1.subtreeCounts) {
      throw new StacklessError(
        `${subtreeL1.query} has no tests - it must have "TODO" in its description`
      );
    }
  }

  for (const [i, sq] of subqueriesToExpandEntries) {
    const subquerySeen = seenSubqueriesToExpand[i];
    if (!subquerySeen) {
      throw new StacklessError(
        `subqueriesToExpand entry did not match anything \
(could be wrong, or could be redundant with a previous subquery):\n  ${sq.toString()}`
      );
    }
  }
  assert(foundCase, `Query \`${queryToLoad.toString()}\` does not match any cases`);

  return TestTree.create(queryToLoad, subtreeL0);
}

function setSubtreeDescriptionAndCountTODOs(
subtree,
description)
{
  assert(subtree.description === undefined);
  subtree.description = description.trim();
  subtree.subtreeCounts ??= { tests: 0, nodesWithTODO: 0, totalTimeMS: 0 };
  if (subtree.description.indexOf('TODO') !== -1) {
    subtree.subtreeCounts.nodesWithTODO++;
  }
}

function makeTreeForSuite(
suite,
isCollapsible)
{
  const query = new TestQueryMultiFile(suite, []);
  return {
    readableRelativeName: suite + kBigSeparator,
    query,
    children: new Map(),
    collapsible: isCollapsible(query)
  };
}

function addSubtreeForDirPath(
tree,
file,
isCollapsible)
{
  const subqueryFile = [];
  // To start, tree is suite:*
  // This loop goes from that -> suite:a,* -> suite:a,b,*
  for (const part of file) {
    subqueryFile.push(part);
    tree = getOrInsertSubtree(part, tree, () => {
      const query = new TestQueryMultiFile(tree.query.suite, subqueryFile);
      return {
        readableRelativeName: part + kPathSeparator + kWildcard,
        query,
        collapsible: isCollapsible(query)
      };
    });
  }
  return tree;
}

function addSubtreeForFilePath(
tree,
file,
isCollapsible)
{
  // To start, tree is suite:*
  // This goes from that -> suite:a,* -> suite:a,b,*
  tree = addSubtreeForDirPath(tree, file, isCollapsible);
  // This goes from that -> suite:a,b:*
  const subtree = getOrInsertSubtree('', tree, () => {
    const query = new TestQueryMultiTest(tree.query.suite, tree.query.filePathParts, []);
    assert(file.length > 0, 'file path is empty');
    return {
      readableRelativeName: file[file.length - 1] + kBigSeparator + kWildcard,
      query,
      collapsible: isCollapsible(query)
    };
  });
  return subtree;
}

function addSubtreeForTestPath(
tree,
test,
testCreationStack,
isCollapsible)
{
  const subqueryTest = [];
  // To start, tree is suite:a,b:*
  // This loop goes from that -> suite:a,b:c,* -> suite:a,b:c,d,*
  for (const part of test) {
    subqueryTest.push(part);
    tree = getOrInsertSubtree(part, tree, () => {
      const query = new TestQueryMultiTest(
        tree.query.suite,
        tree.query.filePathParts,
        subqueryTest
      );
      return {
        readableRelativeName: part + kPathSeparator + kWildcard,
        query,
        collapsible: isCollapsible(query)
      };
    });
  }
  // This goes from that -> suite:a,b:c,d:*
  return getOrInsertSubtree('', tree, () => {
    const query = new TestQueryMultiCase(
      tree.query.suite,
      tree.query.filePathParts,
      subqueryTest,
      {}
    );
    assert(subqueryTest.length > 0, 'subqueryTest is empty');
    return {
      readableRelativeName: subqueryTest[subqueryTest.length - 1] + kBigSeparator + kWildcard,
      kWildcard,
      query,
      testCreationStack,
      collapsible: isCollapsible(query)
    };
  });
}

function addLeafForCase(
tree,
t,
checkCollapsible)
{
  const query = tree.query;
  let name = '';
  const subqueryParams = {};

  // To start, tree is suite:a,b:c,d:*
  // This loop goes from that -> suite:a,b:c,d:x=1;* -> suite:a,b:c,d:x=1;y=2;*
  for (const [k, v] of Object.entries(t.id.params)) {
    name = stringifySingleParam(k, v);
    subqueryParams[k] = v;

    tree = getOrInsertSubtree(name, tree, () => {
      const subquery = new TestQueryMultiCase(
        query.suite,
        query.filePathParts,
        query.testPathParts,
        subqueryParams
      );
      return {
        readableRelativeName: name + kParamSeparator + kWildcard,
        query: subquery,
        collapsible: checkCollapsible(subquery)
      };
    });
  }

  // This goes from that -> suite:a,b:c,d:x=1;y=2
  const subquery = new TestQuerySingleCase(
    query.suite,
    query.filePathParts,
    query.testPathParts,
    subqueryParams
  );
  checkCollapsible(subquery); // mark seenSubqueriesToExpand
  insertLeaf(tree, subquery, t);
}

function getOrInsertSubtree(
key,
parent,
createSubtree)
{
  let v;
  const child = parent.children.get(key);
  if (child !== undefined) {
    assert('children' in child); // Make sure cached subtree is not actually a leaf
    v = child;
  } else {
    v = { ...createSubtree(), children: new Map() };
    parent.children.set(key, v);
  }
  return v;
}

function insertLeaf(parent, query, t) {
  const leaf = {
    readableRelativeName: readableNameForCase(query),
    query,
    run: (rec, expectations) => t.run(rec, query, expectations || []),
    isUnimplemented: t.isUnimplemented,
    subcaseCount: t.computeSubcaseCount()
  };

  // This is a leaf (e.g. s:f:t:x=1;* -> s:f:t:x=1). The key is always ''.
  const key = '';
  assert(!parent.children.has(key), `Duplicate testcase: ${query}`);
  parent.children.set(key, leaf);
}

function dissolveSingleChildTrees(tree) {
  if ('children' in tree) {
    const shouldDissolveThisTree =
    tree.children.size === 1 && tree.query.depthInLevel !== 0 && tree.description === undefined;
    if (shouldDissolveThisTree) {
      // Loops exactly once
      for (const [, child] of tree.children) {
        // Recurse on child
        return dissolveSingleChildTrees(child);
      }
    }

    for (const [k, child] of tree.children) {
      // Recurse on each child
      const newChild = dissolveSingleChildTrees(child);
      if (newChild !== child) {
        tree.children.set(k, newChild);
      }
    }
  }
  return tree;
}

/** Generate a readable relative name for a case (used in standalone). */
function readableNameForCase(query) {
  const paramsKeys = Object.keys(query.params);
  if (paramsKeys.length === 0) {
    return query.testPathParts[query.testPathParts.length - 1] + kBigSeparator;
  } else {
    const lastKey = paramsKeys[paramsKeys.length - 1];
    return stringifySingleParam(lastKey, query.params[lastKey]);
  }
}
});
__define('common/internal/util.js', function (__exports) {
'use strict';
__get(__exports, 'StacklessError', () => StacklessError);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /**
 * Error without a stack, which can be used to fatally exit from `tool/` scripts with a
 * user-friendly message (and no confusing stack).
 */class StacklessError extends Error {constructor(message) {
    super(message);
    this.stack = undefined;
  }
}
});
__define('common/internal/logging/logger.js', function (__exports) {
'use strict';
__get(__exports, 'Logger', () => Logger);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });let version; __bind('common/internal/version.js', (__m) => { ({ version } = __m); });

let TestCaseRecorder; __bind('common/internal/logging/test_case_recorder.js', (__m) => { ({ TestCaseRecorder } = __m); });



class Logger {

  results = new Map();


  constructor({ overrideDebugMode } = {}) {
    this.overriddenDebugMode = overrideDebugMode;
  }

  record(name) {
    const result = { status: 'running', timems: -1 };
    this.results.set(name, result);
    return [
    new TestCaseRecorder(result, this.overriddenDebugMode ?? globalTestConfig.enableDebugLogs),
    result];

  }

  asJSON(space, predFunc) {
    return JSON.stringify(
      {
        version,
        defaultDevice: this.defaultDeviceDescription,
        results: Array.from(
          new Map(
            [...this.results].filter(([key, value]) => predFunc ? predFunc(key, value) : true)
          )
        )
      },
      undefined,
      space
    );
  }
}
});
__define('common/internal/version.js', function (__exports) {
'use strict';
__get(__exports, 'version', () => version);
// AUTO-GENERATED - DO NOT EDIT. See tools/gen_version.

const version = 'd116bcccd1833369fa3715a6df503c0dd9b0d7fa';

});
__define('common/runtime/helper/test_worker.js', function (__exports) {
'use strict';
__get(__exports, 'TestDedicatedWorker', () => TestDedicatedWorker);
__get(__exports, 'TestWorker', () => TestWorker);
__get(__exports, 'TestSharedWorker', () => TestSharedWorker);
__get(__exports, 'TestServiceWorker', () => TestServiceWorker);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let registerShutdownTask, runShutdownTasks; __bind('common/framework/on_shutdown.js', (__m) => { ({ registerShutdownTask, runShutdownTasks } = __m); });let LogMessageWithStack; __bind('common/internal/logging/log_message.js', (__m) => { ({ LogMessageWithStack } = __m); });


let timeout; __bind('common/util/timeout.js', (__m) => { ({ timeout } = __m); });
let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });

let kDefaultCTSOptions; __bind('common/runtime/helper/options.js', (__m) => { ({ kDefaultCTSOptions } = __m); });


/** Query all currently-registered service workers, and unregister them. */
function unregisterAllServiceWorkers() {
  void navigator.serviceWorker.getRegistrations().then((registrations) => {
    for (const registration of registrations) {
      void registration.unregister();
    }
  });
}

// In practice all Web-based runtimes should be including this file, so take the opportunity to set
// up runShutdownTasks. None of these events are guaranteed to happen, but cleaning up is very
// important, so we try our best (and don't worry about shutdown performance or disabling bfcache).
// (We could try 'visibilitychange', but since it can happen in the middle of the page lifetime,
// it is more likely to have unintended consequences and would need to do different stuff.)
// - 'unload' supposedly always disables the bfcache, but is deprecated in Chrome.
// - 'beforeunload' may disable the bfcache but may be called more reliably than 'unload'.
window.addEventListener('beforeunload', runShutdownTasks);
// - 'pagehide' won't disable the bfcache but may be called more reliably than the others.
window.addEventListener('pagehide', runShutdownTasks);

class TestBaseWorker {

  resolvers = new Map();

  constructor(worker, ctsOptions) {
    this.ctsOptions = { ...(ctsOptions || kDefaultCTSOptions), ...{ worker } };
  }

  onmessage(ev) {
    const query = ev.data.query;
    const transferredResult = ev.data.result;

    const result = {
      status: transferredResult.status,
      timems: transferredResult.timems,
      logs: transferredResult.logs?.map((l) => new LogMessageWithStack(l))
    };

    this.resolvers.get(query)(result);
    this.resolvers.delete(query);

    // MAINTENANCE_TODO(kainino0x): update the Logger with this result (or don't have a logger and
    // update the entire results JSON somehow at some point).
  }

  makeRequestAndRecordResult(
  target,
  query,
  expectations)
  {
    const request = {
      query,
      expectations,
      ctsOptions: this.ctsOptions
    };
    target.postMessage(request);

    return new Promise((resolve) => {
      assert(!this.resolvers.has(query), "can't request same query twice simultaneously");
      this.resolvers.set(query, resolve);
    });
  }

  async run(
  rec,
  query,
  expectations = [])
  {
    try {
      rec.injectResult(await this.runImpl(query, expectations));
    } catch (ex) {
      rec.start();
      rec.threw(ex);
      rec.finish();
    }
  }





}

class TestDedicatedWorker extends TestBaseWorker {


  constructor(ctsOptions) {
    super('dedicated', ctsOptions);
    try {
      if (typeof Worker === 'undefined') {
        throw new Error('Dedicated Workers not available');
      }

      const selfPath = __url('common/runtime/helper/test_worker.js');
      const selfPathDir = selfPath.substring(0, selfPath.lastIndexOf('/'));
      const workerPath = selfPathDir + '/test_worker-worker.js';
      const worker = new Worker(workerPath, { type: 'module' });
      this.worker = worker;
      this.worker.onmessage = (ev) => this.onmessage(ev);

      // Try to send a shutdown signal to the worker on shutdown.
      registerShutdownTask(() => worker.postMessage('shutdown'));
    } catch (ex) {
      assert(ex instanceof Error);
      // Save the exception to re-throw in runImpl().
      this.worker = ex;
    }
  }

  runImpl(query, expectations = []) {
    if (this.worker instanceof Worker) {
      return this.makeRequestAndRecordResult(this.worker, query, expectations);
    } else {
      throw this.worker;
    }
  }
}

/** @deprecated Use TestDedicatedWorker instead. */
class TestWorker extends TestDedicatedWorker {}

class TestSharedWorker extends TestBaseWorker {
  /** MessagePort to the SharedWorker, or an Error if it couldn't be initialized. */


  constructor(ctsOptions) {
    super('shared', ctsOptions);
    try {
      if (typeof SharedWorker === 'undefined') {
        throw new Error('Shared Workers not available');
      }

      const selfPath = __url('common/runtime/helper/test_worker.js');
      const selfPathDir = selfPath.substring(0, selfPath.lastIndexOf('/'));
      const workerPath = selfPathDir + '/test_worker-worker.js';
      const worker = new SharedWorker(workerPath, { type: 'module' });
      this.port = worker.port;
      this.port.start();
      this.port.onmessage = (ev) => this.onmessage(ev);

      // Try to send a shutdown signal to the worker on shutdown.
      registerShutdownTask(() => worker.port.postMessage('shutdown'));
    } catch (ex) {
      assert(ex instanceof Error);
      // Save the exception to re-throw in runImpl().
      this.port = ex;
    }
  }

  runImpl(query, expectations = []) {
    if (this.port instanceof MessagePort) {
      return this.makeRequestAndRecordResult(this.port, query, expectations);
    } else {
      throw this.port;
    }
  }
}

class TestServiceWorker extends TestBaseWorker {
  lastServiceWorker = null;

  constructor(ctsOptions) {
    super('service', ctsOptions);

    // If the runtime is trying to use service workers, first clean up any service workers that may
    // have leaked from previous incarnations of the page. Service workers *shouldn't* affect
    // worker=none/dedicated/shared, so it should be OK if this constructor isn't called.
    unregisterAllServiceWorkers();

    // Try to send a shutdown signal to the worker on shutdown.
    registerShutdownTask(() => {
      this.runShutdownTasksOnLastServiceWorker();
      unregisterAllServiceWorkers();
    });
  }

  runShutdownTasksOnLastServiceWorker() {
    this.lastServiceWorker?.postMessage('shutdown');
  }

  async runImpl(query, expectations = []) {
    if (!('serviceWorker' in navigator)) {
      throw new Error('Service Workers not available');
    }
    const [suite, name] = query.split(':', 2);
    const fileName = name.split(',').join('/');

    const selfPath = __url('common/runtime/helper/test_worker.js');
    const selfPathDir = selfPath.substring(0, selfPath.lastIndexOf('/'));
    // Construct the path to the worker file, then use URL to resolve the `../` components.
    const serviceWorkerURL = new URL(
      `${selfPathDir}/../../../${suite}/webworker/${fileName}.as_worker.js`
    ).toString();

    // Ensure the correct service worker is registered.
    const registration = await navigator.serviceWorker.register(serviceWorkerURL, {
      type: 'module'
    });
    const registrationPending = () =>
    !registration.active || registration.active.scriptURL !== serviceWorkerURL;

    // Ensure the registration we just requested is active. (We don't worry about it being
    // outdated from a previous page load, because we wipe all service workers on shutdown/startup.)
    const isNewInstance = registrationPending();
    if (isNewInstance) {
      this.runShutdownTasksOnLastServiceWorker();
      do {
        await new Promise((resolve) => timeout(resolve, 0));
      } while (registrationPending());
    }
    const serviceWorker = registration.active;
    this.lastServiceWorker = serviceWorker;

    navigator.serviceWorker.onmessage = (ev) => this.onmessage(ev);
    return this.makeRequestAndRecordResult(serviceWorker, query, expectations);
  }
}
});
__define('common/framework/on_shutdown.js', function (__exports) {
'use strict';
__get(__exports, 'registerShutdownTask', () => registerShutdownTask);
__get(__exports, 'runShutdownTasks', () => runShutdownTasks);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const shutdownTasks = []; /**
 * Register a callback to be run during program shutdown (triggered, e.g., by the 'beforeunload'
 * event when the webpage is being closed).
 *
 * Note such tasks should be synchronous functions; otherwise, they probably won't complete.
 */
function registerShutdownTask(task) {
  shutdownTasks.push(task);
}

/** Run all shutdown tasks. Should only be called during program shutdown. */
function runShutdownTasks() {
  for (const task of shutdownTasks) {
    task();
  }
  shutdownTasks.length = 0;
}
});
__define('webgpu/listing.js', function (__exports) {
'use strict';
__get(__exports, 'listing', () => listing);
// AUTO-GENERATED - DO NOT EDIT. See src/common/tools/gen_listings_and_webworkers.ts.

const listing = [
  {
    "file": [],
    "readme": "WebGPU conformance test suite."
  },
  {
    "file": [
      "api"
    ],
    "readme": "Tests for full coverage of the Javascript API surface of WebGPU."
  },
  {
    "file": [
      "api",
      "operation"
    ],
    "readme": "Tests that check the result of performing valid WebGPU operations, taking advantage of\nparameterization to exercise interactions between features."
  },
  {
    "file": [
      "api",
      "operation",
      "adapter",
      "info"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "adapter",
      "requestAdapter"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "adapter",
      "requestDevice"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "async_ordering"
    ],
    "readme": "Test ordering of async resolutions between promises returned by the following calls (and possibly\nbetween multiple of the same call), where there are constraints on the ordering.\nSpec issue: https://github.com/gpuweb/gpuweb/issues/962\n\nTODO: plan and implement\n- createReadyPipeline() (not sure if this actually has any ordering constraints)\n- cmdbuf.executionTime\n- device.popErrorScope()\n- device.lost\n- queue.onSubmittedWorkDone()\n- buffer.mapAsync()\n- shadermodule.getCompilationInfo()"
  },
  {
    "file": [
      "api",
      "operation",
      "buffers"
    ],
    "readme": "GPUBuffer tests."
  },
  {
    "file": [
      "api",
      "operation",
      "buffers",
      "createBindGroup"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "buffers",
      "map"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "buffers",
      "map_ArrayBuffer"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "buffers",
      "map_detach"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "buffers",
      "map_oom"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "buffers",
      "threading"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "basic"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "clearBuffer"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "copyBufferToBuffer"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "copyTextureToTexture"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "image_copy"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "programmable",
      "immediate"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "programmable",
      "state_tracking"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "queries"
    ],
    "readme": "TODO: test the behavior of creating/using/resolving queries.\n- timestamp\n- nested (e.g. timestamp inside occlusion query), if any such cases are valid. Try\n  writing to the same query set (at same or different indices), if valid. Check results make sense.\n- start a query (all types) with no draw calls"
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "queries",
      "occlusionQuery"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "queries",
      "timestampQuery"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "render",
      "dynamic_state"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "render",
      "render_bundle"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "command_buffer",
      "render",
      "state_tracking"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "compute",
      "basic"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "compute_pipeline",
      "entry_point_name"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "compute_pipeline",
      "overrides"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "device",
      "all_limits_and_features"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "device",
      "lost"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "labels"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "limits",
      "max_combined_limits"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "memory_allocation"
    ],
    "readme": "Try to stress memory allocators in the implementation and driver.\n\nTODO: plan and implement\n- Tests which (pseudo-randomly?) allocate a bunch of memory and then assert things about the memory\n  (it's not aliased, it's valid to read and write in various ways, accesses read/write the correct data)\n    - Possibly also with OOB accesses/robust buffer access?\n- Tests which are targeted against particular known implementation details"
  },
  {
    "file": [
      "api",
      "operation",
      "memory_sync",
      "buffer",
      "multiple_buffers"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "memory_sync",
      "buffer",
      "single_buffer"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "memory_sync",
      "texture",
      "readonly_depth_stencil"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "memory_sync",
      "texture",
      "same_subresource"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "onSubmittedWorkDone"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "pipeline",
      "default_layout"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "pipeline",
      "pipeline_layout_created_with_null_bind_group_layout"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "queue",
      "writeBuffer"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "reflection"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pass"
    ],
    "readme": "Render pass stuff other than commands (which are in command_buffer/)."
  },
  {
    "file": [
      "api",
      "operation",
      "render_pass",
      "clear_value"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pass",
      "resolve"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pass",
      "storeOp"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pass",
      "storeop2"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pass",
      "transient_attachment"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pipeline",
      "culling_tests"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pipeline",
      "overrides"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pipeline",
      "pipeline_output_targets"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pipeline",
      "primitive_topology"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pipeline",
      "sample_mask"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "render_pipeline",
      "vertex_only_render_pipeline"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "3d_texture_slices"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "basic"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "color_target_state"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "depth"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "depth_bias"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "depth_clip_clamp"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "draw"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "indirect_draw"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "robust_access_index"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "rendering",
      "stencil"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "resource_init",
      "buffer"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "resource_init",
      "texture_zero"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "sampling",
      "anisotropy"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "sampling",
      "filter_mode"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "sampling",
      "sampler_texture"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "shader_module",
      "compilation_info"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "storage_texture",
      "read_only"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "storage_texture",
      "read_write"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "texture_view",
      "format_reinterpretation"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "texture_view",
      "texture_component_swizzle"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "texture_view",
      "write"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "threading"
    ],
    "readme": "Tests for behavior with multiple threads (main thread + workers).\n\nTODO: plan and implement\n- 'postMessage'\n  Try postMessage'ing an object of every type (to same or different thread)\n    - {main -> main, main -> worker, worker -> main, worker1 -> worker1, worker1 -> worker2}\n    - through {global postMessage, MessageChannel}\n    - {in, not in} transferrable object list, when valid\n- 'concurrency'\n  Short tight loop doing many of an action from two threads at the same time\n    - e.g. {create {buffer, texture, shader, pipeline}}"
  },
  {
    "file": [
      "api",
      "operation",
      "uncapturederror"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "vertex_state",
      "correctness"
    ]
  },
  {
    "file": [
      "api",
      "operation",
      "vertex_state",
      "index_format"
    ]
  },
  {
    "file": [
      "api",
      "regression"
    ],
    "readme": "One-off tests that reproduce API bugs found in implementations to prevent the bugs from\nappearing again."
  },
  {
    "file": [
      "api",
      "validation"
    ],
    "readme": "Positive and negative tests for all the validation rules of the API."
  },
  {
    "file": [
      "api",
      "validation",
      "buffer",
      "create"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "buffer",
      "destroy"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "buffer",
      "mapping"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "buffer",
      "threading"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features"
    ],
    "readme": "Test every method or option that shouldn't be allowed without a feature enabled.\nIf the feature is not enabled, any use of an enum value added by a feature must be an\n*exception*, per <https://github.com/gpuweb/gpuweb/blob/main/design/ErrorConventions.md>.\n\n- x= that feature {enabled, disabled}\n\nGenerally one file for each feature name, but some may be grouped (e.g. one file for all optional\nquery types, one file for all optional texture formats).\n\nTODO: implement"
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features",
      "clip_distances"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features",
      "query_types"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features",
      "subgroup_size_control"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features",
      "texture_component_swizzle"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features",
      "texture_compression_unaligned"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features",
      "texture_formats"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features",
      "texture_formats_tier1"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "features",
      "texture_formats_tier2"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits"
    ],
    "readme": "Test everything that shouldn't be valid without a higher-than-specified limit.\n\nOne file for each limit name.\n\nTODO: Test that \"alignment\" limits require a power of 2."
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxBindGroups"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxBindGroupsPlusVertexBuffers"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxBindingsPerBindGroup"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxBufferSize"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxColorAttachmentBytesPerSample"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxColorAttachments"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxComputeInvocationsPerWorkgroup"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxComputeWorkgroupSizeX"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxComputeWorkgroupSizeY"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxComputeWorkgroupSizeZ"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxComputeWorkgroupStorageSize"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxComputeWorkgroupsPerDimension"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxDynamicStorageBuffersPerPipelineLayout"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxDynamicUniformBuffersPerPipelineLayout"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxInterStageShaderVariables"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxSampledTexturesPerShaderStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxSamplersPerShaderStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxStorageBufferBindingSize"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxStorageBuffersInFragmentStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxStorageBuffersInVertexStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxStorageBuffersPerShaderStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxStorageTexturesInFragmentStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxStorageTexturesInVertexStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxStorageTexturesPerShaderStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxTextureArrayLayers"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxTextureDimension1D"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxTextureDimension2D"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxTextureDimension3D"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxUniformBufferBindingSize"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxUniformBuffersPerShaderStage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxVertexAttributes"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxVertexBufferArrayStride"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "maxVertexBuffers"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "minStorageBufferOffsetAlignment"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "capability_checks",
      "limits",
      "minUniformBufferOffsetAlignment"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "compute_pipeline"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "createBindGroup"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "createBindGroupLayout"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "createPipelineLayout"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "createSampler"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "createTexture"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "createView"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "debugMarker"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "dispatch"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "beginComputePass"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "beginRenderPass"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "clearBuffer"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "compute_pass"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "copyBufferToBuffer"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "copyTextureToTexture"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "debug"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "index_access"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render",
      "draw"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render",
      "dynamic_state"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render",
      "indirect_draw"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render",
      "indirect_multi_draw"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render",
      "setIndexBuffer"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render",
      "setPipeline"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render",
      "setVertexBuffer"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render",
      "state_tracking"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "render_pass"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "setBindGroup"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "cmds",
      "setImmediates"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "createRenderBundleEncoder"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "encoder_open_state"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "encoder_state"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "programmable",
      "pipeline_bind_group_compat"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "programmable",
      "pipeline_immediate"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "queries",
      "begin_end"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "queries",
      "general"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "queries",
      "resolveQuerySet"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "encoding",
      "render_bundle"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "error_scope"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "getBindGroupLayout"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "gpu_external_texture_expiration"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "image_copy"
    ],
    "readme": "writeTexture + copyBufferToTexture + copyTextureToBuffer validation tests.\n\nTest coverage:\n* resource usages:\n  - texture_usage_must_be_valid: for GPUTextureUsage::COPY_SRC, GPUTextureUsage::COPY_DST flags.\n  - buffer_usage_must_be_valid: for GPUBufferUsage::COPY_SRC, GPUBufferUsage::COPY_DST flags.\n\n* textureCopyView:\n  - texture_must_be_valid: for valid, destroyed, error textures.\n  - sample_count_must_be_1: for sample count 1 and 4.\n  - mip_level_must_be_in_range: for various combinations of mipLevel and mipLevelCount.\n  - format: for all formats with full and non-full copies on width, height, and depth.\n  - texel_block_alignment_on_origin: for all formats and coordinates.\n\n* bufferCopyView:\n  - buffer_must_be_valid: for valid, destroyed, error buffers.\n  - bytes_per_row_alignment: for bytesPerRow to be 256-byte aligned or not, and bytesPerRow is required or not.\n\n* linear texture data:\n  - bound_on_rows_per_image: for various combinations of copyDepth (1, >1), copyHeight, rowsPerImage.\n  - offset_plus_required_bytes_in_copy_overflow\n  - required_bytes_in_copy: testing minimal data size and data size too small for various combinations of bytesPerRow, rowsPerImage, copyExtent and offset. for the copy method, bytesPerRow is computed as bytesInACompleteRow aligned to be a multiple of 256 + bytesPerRowPadding * 256.\n  - texel_block_alignment_on_rows_per_image: for all formats.\n  - offset_alignment: for all formats.\n  - bound_on_offset: for various combinations of offset and dataSize.\n\n* texture copy range:\n  - 1d_texture: copyExtent.height isn't 1, copyExtent.depthOrArrayLayers isn't 1.\n  - texel_block_alignment_on_size: for all formats and coordinates.\n  - texture_range_conditons: for all coordinate and various combinations of origin, copyExtent, textureSize and mipLevel.\n\nTODO: more test coverage for 1D and 3D textures."
  },
  {
    "file": [
      "api",
      "validation",
      "image_copy",
      "buffer_related"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "image_copy",
      "buffer_texture_copies"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "image_copy",
      "layout_related"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "image_copy",
      "texture_related"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "layout_shader_compat"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "min_buffer_binding_size"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "non_filterable_texture"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "pipeline",
      "immediates"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "query_set",
      "create"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "query_set",
      "destroy"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "queue"
    ],
    "readme": "Tests for validation that occurs inside queued operations\n(submit, writeBuffer, writeTexture, copyExternalImageToTexture).\n\nBufferMapStatesToTest = {\n  mapped -> unmapped,\n  mapped at creation -> unmapped,\n  mapping pending -> unmapped,\n  pending -> mapped (await map),\n  unmapped -> pending (noawait map),\n  created mapped-at-creation,\n}\n\nNote writeTexture is tested in image_copy."
  },
  {
    "file": [
      "api",
      "validation",
      "queue",
      "buffer_mapped"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "queue",
      "copyToTexture",
      "CopyExternalImageToTexture"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "queue",
      "destroyed",
      "buffer"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "queue",
      "destroyed",
      "query_set"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "queue",
      "destroyed",
      "texture"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "queue",
      "submit"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "queue",
      "writeBuffer"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "queue",
      "writeTexture"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pass"
    ],
    "readme": "Render pass stuff other than commands (which are in encoding/cmds/)."
  },
  {
    "file": [
      "api",
      "validation",
      "render_pass",
      "attachment_compatibility"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pass",
      "render_pass_descriptor"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pass",
      "resolve"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "depth_stencil_state"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "float32_blendable"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "fragment_state"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "inter_stage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "misc"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "multisample_state"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "overrides"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "primitive_state"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "resource_compatibility"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "shader_module"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "render_pipeline",
      "vertex_state"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "resource_usages",
      "buffer"
    ],
    "readme": "TODO: look at texture,*"
  },
  {
    "file": [
      "api",
      "validation",
      "resource_usages",
      "buffer",
      "in_pass_encoder"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "resource_usages",
      "buffer",
      "in_pass_misc"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "resource_usages",
      "texture",
      "in_pass_encoder"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "resource_usages",
      "texture",
      "in_render_common"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "resource_usages",
      "texture",
      "in_render_misc"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "shader_module",
      "entry_point"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "shader_module",
      "overrides"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "state",
      "device_lost"
    ],
    "readme": "Tests of behavior while the device is lost.\n\n- x= every method in the API.\n\nTODO: implement"
  },
  {
    "file": [
      "api",
      "validation",
      "state",
      "device_lost",
      "destroy"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "texture",
      "bgra8unorm_storage"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "texture",
      "destroy"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "texture",
      "float32_filterable"
    ]
  },
  {
    "file": [
      "api",
      "validation",
      "texture",
      "rg11b10ufloat_renderable"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "createBindGroup"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "createBindGroupLayout"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "createBindGroupLayout_limits"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "createPipelineLayout"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "encoding",
      "cmds",
      "copyTextureToBuffer"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "encoding",
      "cmds",
      "copyTextureToTexture"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "encoding",
      "programmable",
      "pipeline_bind_group_compat"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "pipeline_creation"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "render_pipeline",
      "depth_stencil_state"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "render_pipeline",
      "fragment_state"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "render_pipeline",
      "in_stage_limits"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "render_pipeline",
      "unsupported_wgsl"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "render_pipeline",
      "vertex_state"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "texture",
      "createTexture"
    ]
  },
  {
    "file": [
      "compat",
      "api",
      "validation",
      "texture",
      "cubeArray"
    ]
  },
  {
    "file": [
      "examples"
    ]
  },
  {
    "file": [
      "idl"
    ],
    "readme": "Tests to check that the WebGPU IDL is correctly implemented, for examples that objects exposed\nexactly the correct members, and that methods throw when passed incomplete dictionaries.\n\nSee https://github.com/gpuweb/cts/issues/332\n\nTODO: exposed.html.ts: Test all WebGPU interfaces instead of just some of them.\nTODO: Check prototype chains. (Add a helper in IDLTest for this.)"
  },
  {
    "file": [
      "idl",
      "constants",
      "flags"
    ]
  },
  {
    "file": [
      "idl",
      "constructable"
    ]
  },
  {
    "file": [
      "idl",
      "javascript"
    ]
  },
  {
    "file": [
      "print_environment"
    ]
  },
  {
    "file": [
      "shader"
    ],
    "readme": "Tests for full coverage of the shaders that can be passed to WebGPU."
  },
  {
    "file": [
      "shader",
      "execution"
    ],
    "readme": "Tests that check the result of valid shader execution."
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "access",
      "array",
      "index"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "access",
      "matrix",
      "index"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "access",
      "structure",
      "index"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "access",
      "vector",
      "components"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "access",
      "vector",
      "index"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_addition"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_comparison"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_division"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_matrix_addition"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_matrix_matrix_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_matrix_scalar_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_matrix_subtraction"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_matrix_vector_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_remainder"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "af_subtraction"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "ai_arithmetic"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "ai_comparison"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "bitwise"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "bitwise_shift"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "bool_logical"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_addition"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_comparison"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_division"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_matrix_addition"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_matrix_matrix_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_matrix_scalar_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_matrix_subtraction"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_matrix_vector_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_remainder"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f16_subtraction"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_addition"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_comparison"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_division"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_matrix_addition"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_matrix_matrix_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_matrix_scalar_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_matrix_subtraction"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_matrix_vector_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_multiplication"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_remainder"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "f32_subtraction"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "i32_arithmetic"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "i32_comparison"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "u32_arithmetic"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "binary",
      "u32_comparison"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "abs"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "acos"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "acosh"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "all"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "any"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "arrayLength"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "asin"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "asinh"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atan"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atan2"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atanh"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicAdd"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicAnd"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicCompareExchangeWeak"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicExchange"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicLoad"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicMax"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicMin"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicOr"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicStore"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicStoreMax"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicStoreMin"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicSub"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "atomics",
      "atomicXor"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "bitcast"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "bufferArrayView"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "bufferLength"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "bufferView"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "ceil"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "clamp"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "cos"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "cosh"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "countLeadingZeros"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "countOneBits"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "countTrailingZeros"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "cross"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "degrees"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "determinant"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "distance"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dot"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dot4I8Packed"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dot4U8Packed"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dpdx"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dpdxCoarse"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dpdxFine"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dpdy"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dpdyCoarse"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "dpdyFine"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "exp"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "exp2"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "extractBits"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "faceForward"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "firstLeadingBit"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "firstTrailingBit"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "floor"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "fma"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "fract"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "frexp"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "fwidth"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "fwidthCoarse"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "fwidthFine"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "insertBits"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "inversesqrt"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "ldexp"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "length"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "log"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "log2"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "max"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "min"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "mix"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "modf"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "normalize"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack2x16float"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack2x16snorm"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack2x16unorm"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack4x8snorm"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack4x8unorm"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack4xI8"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack4xI8Clamp"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack4xU8"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pack4xU8Clamp"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "pow"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "quadBroadcast"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "quadSwap"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "quantizeToF16"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "radians"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "reflect"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "refract"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "reverseBits"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "round"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "saturate"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "select"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "sign"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "sin"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "sinh"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "smoothstep"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "sqrt"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "step"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "storageBarrier"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupAdd"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupAll"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupAny"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupBallot"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupBitwise"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupBroadcast"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupElect"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupMinMax"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupMul"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "subgroupShuffle"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "tan"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "tanh"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureDimensions"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureGather"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureGatherCompare"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureLoad"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureNumLayers"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureNumLevels"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureNumSamples"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureSample"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureSampleBaseClampToEdge"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureSampleBias"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureSampleCompare"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureSampleCompareLevel"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureSampleGrad"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureSampleLevel"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "textureStore"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "texture_utils"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "transpose"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "trunc"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "unpack2x16float"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "unpack2x16snorm"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "unpack2x16unorm"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "unpack4x8snorm"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "unpack4x8unorm"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "unpack4xI8"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "unpack4xU8"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "workgroupBarrier"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "builtin",
      "workgroupUniformLoad"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "user",
      "ptr_aliasing"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "call",
      "user",
      "ptr_params"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "constructor",
      "non_zero"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "constructor",
      "zero_value"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "precedence"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "address_of_and_indirection"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "af_arithmetic"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "af_assignment"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "ai_arithmetic"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "ai_assignment"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "ai_complement"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "bool_conversion"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "bool_logical"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "f16_arithmetic"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "f16_conversion"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "f32_arithmetic"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "f32_conversion"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "i32_arithmetic"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "i32_complement"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "i32_conversion"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "u32_complement"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "expression",
      "unary",
      "u32_conversion"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "float_parse"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "call"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "complex"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "eval_order"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "for"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "if"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "loop"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "phony"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "return"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "switch"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "flow_control",
      "while"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "limits"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "memory_layout"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "memory_model",
      "adjacent"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "memory_model",
      "atomicity"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "memory_model",
      "barrier"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "memory_model",
      "coherence"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "memory_model",
      "texture_intra_invocation_coherence"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "memory_model",
      "weak"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "override"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "padding"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "robust_access"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "robust_access_vertex"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "shader_io",
      "compute_builtins"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "shader_io",
      "fragment_builtins"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "shader_io",
      "shared_structs"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "shader_io",
      "user_io"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "shader_io",
      "vertex_builtins"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "shader_io",
      "workgroup_size"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "shadow"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "stage"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "statement",
      "assignment",
      "vector"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "statement",
      "compound"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "statement",
      "discard"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "statement",
      "increment_decrement"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "statement",
      "phony"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "statement",
      "swizzle_assignment"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "value_init"
    ]
  },
  {
    "file": [
      "shader",
      "execution",
      "zero_init"
    ]
  },
  {
    "file": [
      "shader",
      "regression"
    ],
    "readme": "One-off tests that reproduce shader bugs found in implementations to prevent the bugs from\nappearing again."
  },
  {
    "file": [
      "shader",
      "validation"
    ],
    "readme": "Positive and negative tests for all the validation rules of the shading language."
  },
  {
    "file": [
      "shader",
      "validation",
      "const_assert",
      "const_assert"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "decl",
      "assignment_statement"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "decl",
      "compound_statement"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "decl",
      "const"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "decl",
      "context_dependent_resolution"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "decl",
      "immediate"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "decl",
      "let"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "decl",
      "override"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "decl",
      "var"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "access",
      "array"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "access",
      "matrix"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "access",
      "structure"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "access",
      "vector"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "binary",
      "add_sub_mul"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "binary",
      "and_or_xor"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "binary",
      "bitwise_shift"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "binary",
      "comparison"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "binary",
      "div_rem"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "binary",
      "parse"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "binary",
      "short_circuiting_and_or"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "abs"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "acos"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "acosh"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "all"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "any"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "arrayLength"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "asin"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "asinh"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "atan"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "atan2"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "atanh"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "atomicStoreMinMax"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "atomics"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "barriers"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "bitcast"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "bufferArrayView"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "bufferLength"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "bufferView"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "ceil"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "clamp"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "cos"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "cosh"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "countLeadingZeros"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "countOneBits"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "countTrailingZeros"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "cross"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "degrees"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "derivatives"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "determinant"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "distance"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "dot"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "dot4I8Packed"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "dot4U8Packed"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "exp"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "exp2"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "extractBits"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "faceForward"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "firstLeadingBit"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "firstTrailingBit"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "floor"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "fma"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "fract"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "frexp"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "insertBits"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "inverseSqrt"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "ldexp"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "length"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "log"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "log2"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "max"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "min"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "mix"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "modf"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "normalize"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack2x16float"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack2x16snorm"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack2x16unorm"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack4x8snorm"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack4x8unorm"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack4xI8"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack4xI8Clamp"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack4xU8"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pack4xU8Clamp"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "pow"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "quadBroadcast"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "quadSwap"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "quantizeToF16"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "radians"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "reflect"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "refract"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "reverseBits"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "round"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "saturate"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "select"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "sign"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "sin"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "sinh"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "smoothstep"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "sqrt"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "step"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupAdd"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupAnyAll"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupBallot"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupBitwise"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupBroadcast"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupBroadcastFirst"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupElect"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupMinMax"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupMul"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "subgroupShuffle"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "tan"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "tanh"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureDimensions"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureGather"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureGatherCompare"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureLoad"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureNumLayers"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureNumLevels"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureNumSamples"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureSample"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureSampleBaseClampToEdge"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureSampleBias"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureSampleCompare"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureSampleCompareLevel"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureSampleGrad"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureSampleLevel"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "textureStore"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "transpose"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "trunc"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "unpack2x16float"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "unpack2x16snorm"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "unpack2x16unorm"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "unpack4x8snorm"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "unpack4x8unorm"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "unpack4xI8"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "unpack4xU8"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "value_constructor"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "call",
      "builtin",
      "workgroupUniformLoad"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "early_evaluation"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "matrix",
      "add_sub"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "matrix",
      "and_or_xor"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "matrix",
      "bitwise_shift"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "matrix",
      "comparison"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "matrix",
      "div_rem"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "matrix",
      "mul"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "overload_resolution"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "precedence"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "unary",
      "address_of_and_indirection"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "unary",
      "arithmetic_negation"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "unary",
      "bitwise_complement"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "expression",
      "unary",
      "logical_negation"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "extension",
      "clip_distances"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "extension",
      "dual_source_blending"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "extension",
      "pointer_composite_access"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "extension",
      "readonly_and_readwrite_storage_textures"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "extension",
      "subgroup_size_control"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "functions",
      "alias_analysis"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "functions",
      "restrictions"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "attribute"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "blankspace"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "comments"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "diagnostic"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "enable"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "identifiers"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "literal"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "must_use"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "requires"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "semicolon"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "shadow_builtins"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "parse",
      "source"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "align"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "binding"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "builtins"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "entry_point"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "group"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "group_and_binding"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "id"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "interpolate"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "invariant"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "layout_constraints"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "locations"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "pipeline_stage"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "size"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "shader_io",
      "workgroup_size"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "break"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "break_if"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "compound"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "const_assert"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "continue"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "continuing"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "discard"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "for"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "if"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "increment_decrement"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "loop"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "phony"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "return"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "statement_behavior"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "switch"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "swizzle_assignment"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "statement",
      "while"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "alias"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "array"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "atomics"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "buffer"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "enumerant"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "matrix"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "pointer"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "ref"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "struct"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "textures"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "types",
      "vector"
    ]
  },
  {
    "file": [
      "shader",
      "validation",
      "uniformity",
      "uniformity"
    ]
  },
  {
    "file": [
      "util",
      "texture",
      "color_space_conversions"
    ]
  },
  {
    "file": [
      "util",
      "texture",
      "texel_data"
    ]
  },
  {
    "file": [
      "util",
      "texture",
      "texture_ok"
    ]
  },
  {
    "file": [
      "web_platform"
    ],
    "readme": "Tests for Web platform-specific interactions like GPUCanvasContext and canvas, WebXR,\nImageBitmaps, and video APIs.\n\nTODO(#922): Also hopefully tests for user-initiated readbacks from WebGPU canvases\n(printing, save image as, etc.)"
  },
  {
    "file": [
      "web_platform",
      "canvas"
    ],
    "readme": "Tests for WebGPU <canvas> and OffscreenCanvas presentation."
  },
  {
    "file": [
      "web_platform",
      "canvas",
      "configure"
    ]
  },
  {
    "file": [
      "web_platform",
      "canvas",
      "context_creation"
    ]
  },
  {
    "file": [
      "web_platform",
      "canvas",
      "getCurrentTexture"
    ]
  },
  {
    "file": [
      "web_platform",
      "canvas",
      "getPreferredCanvasFormat"
    ]
  },
  {
    "file": [
      "web_platform",
      "canvas",
      "readbackFromWebGPUCanvas"
    ]
  },
  {
    "file": [
      "web_platform",
      "copyToTexture",
      "ImageBitmap"
    ]
  },
  {
    "file": [
      "web_platform",
      "copyToTexture",
      "ImageData"
    ]
  },
  {
    "file": [
      "web_platform",
      "copyToTexture"
    ],
    "readme": "Tests for copyToTexture from all possible sources (video, canvas, ImageBitmap, ...)"
  },
  {
    "file": [
      "web_platform",
      "copyToTexture",
      "canvas"
    ]
  },
  {
    "file": [
      "web_platform",
      "copyToTexture",
      "image"
    ]
  },
  {
    "file": [
      "web_platform",
      "copyToTexture",
      "image_file"
    ]
  },
  {
    "file": [
      "web_platform",
      "copyToTexture",
      "video"
    ]
  },
  {
    "file": [
      "web_platform",
      "external_texture"
    ],
    "readme": "Tests for external textures."
  },
  {
    "file": [
      "web_platform",
      "external_texture",
      "video"
    ]
  },
  {
    "file": [
      "web_platform",
      "reftests"
    ],
    "readme": "Reference tests (reftests) for WebGPU canvas presentation.\n\nThese render some contents to a canvas using WebGPU, and WPT compares the rendering result with\nthe \"reference\" versions (in `ref/`) which render with 2D canvas.\n\nThis tests things like:\n- The canvas has the correct orientation.\n- The canvas renders with the correct transfer function.\n- The canvas blends and interpolates in the correct color encoding.\n\nTODO(#918): Test all possible color spaces (once we have more than 1)\nTODO(#921): Why is there sometimes a difference of 1 (e.g. 3f vs 40) in canvas_size_different_with_back_buffer_size?\nAnd why does chromium's image_diff show diffs on other pixels that don't seem to have diffs?\nTODO(#1093): Test rgba16float values which are out of gamut of the canvas but under SDR luminance.\nTODO(#1093): Test rgba16float values which are above SDR luminance.\nTODO(#1116): Test canvas scaling.\nTODO: Test transferControlToOffscreen, used from {the same,another} thread"
  },
  {
    "file": [
      "web_platform",
      "worker",
      "worker"
    ]
  }
];

});
__define('webgpu/api/operation/adapter/requestDevice.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Test GPUAdapter.requestDevice.

Note tests explicitly destroy created devices so that tests don't have to wait for GC to clean up
potentially limited native resources.
`;let Fixture; __bind('common/framework/fixture.js', (__m) => { ({ Fixture } = __m); });
let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let getGPU; __bind('common/util/navigator_gpu.js', (__m) => { ({ getGPU } = __m); });
let assert, assertReject, hasFeature, typedEntries; __bind('common/util/util.js', (__m) => { ({ assert, assertReject, hasFeature, typedEntries } = __m); });
let getDefaultLimitsForCTS, kFeatureNames, kLimitClasses, kPossibleLimits; __bind('webgpu/capability_info.js', (__m) => { ({ getDefaultLimitsForCTS, kFeatureNames, kLimitClasses, kPossibleLimits } = __m); });
let clamp, isPowerOfTwo; __bind('webgpu/util/math.js', (__m) => { ({ clamp, isPowerOfTwo } = __m); });

const g = makeTestGroup(Fixture);

g.test('default').
desc(
  `
    Test requesting the device with a variation of default parameters.
    - No features listed in default device
    - Default limits`
).
paramsSubcasesOnly((u) =>
u.combine('args', [
[],
[undefined],
[{}],
[{ requiredFeatures: [], requiredLimits: {} }]]
)
).
fn(async (t) => {
  const { args } = t.params;
  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);
  const device = await t.requestDeviceTracked(adapter, ...args);
  assert(device !== null);

  if (device.features.size === 1) {
    t.expect(
      hasFeature(device.features, 'core-features-and-limits'),
      'Default device should not have any features other than "core-features-and-limits"'
    );
  } else {
    t.expect(
      device.features.size === 0,
      'Default device should not have any features other than "core-features-and-limits"'
    );
  }
  // All limits should be defaults.
  const limitInfos = getDefaultLimitsForCTS();
  for (const [limit, limitInfo] of typedEntries(limitInfos)) {
    t.expect(
      device.limits[limit] === limitInfo.default,
      `Expected ${limit} == default: ${device.limits[limit]} != ${limitInfo.default}`
    );
  }
});

g.test('invalid').
desc(
  `
    Test that requesting device on an invalid adapter resolves with lost device.
    - Induce invalid adapter via a device lost from a device.destroy()
    - Check the device is lost with reason 'destroyed'
    - Try creating another device on the now-stale adapter fails.
    `
).
fn(async (t) => {
  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);

  {
    // Request a device and destroy it immediately afterwards.
    const device = await t.requestDeviceTracked(adapter);
    assert(device !== null);
    device.destroy();
    const lostInfo = await device.lost;
    t.expect(lostInfo.reason === 'destroyed');
  }

  // The adapter should now be invalid since a device was lost. Requesting another device is not possible anymore.
  t.shouldReject('OperationError', t.requestDeviceTracked(adapter));
});

g.test('stale').
desc(
  `
    Test that adapter.requestDevice() can successfully return a device once, and once only.
    - Tests that we can successfully resolve after serial and concurrent rejections.
    - Tests that consecutive valid attempts only succeeds the first time, throwing OperationError
      afterward, when the adapter state is "consumed". (Note it's not possibe for CTS to test the
      adapter state "expired" since there's no way to lose an adapter in the API.)`
).
paramsSubcasesOnly((u) =>
u.
combine('initialError', [undefined, 'TypeError', 'OperationError']).
combine('awaitInitialError', [true, false]).
combine('awaitSuccess', [true, false]).
unless(
  ({ initialError, awaitInitialError }) => initialError === undefined && awaitInitialError
)
).
fn(async (t) => {
  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);

  const { initialError, awaitInitialError, awaitSuccess } = t.params;

  switch (initialError) {
    case undefined:
      break;
    case 'TypeError':
      // Cause a type error by requesting with an unknown feature.
      if (awaitInitialError) {
        await assertReject(
          'TypeError',
          t.requestDeviceTracked(adapter, {
            requiredFeatures: ['unknown-feature']
          })
        );
      } else {
        t.shouldReject(
          'TypeError',
          t.requestDeviceTracked(adapter, {
            requiredFeatures: ['unknown-feature']
          })
        );
      }
      break;
    case 'OperationError':
      // Cause an operation error by requesting with an alignment limit that is not a power of 2.
      if (awaitInitialError) {
        await assertReject(
          'OperationError',
          t.requestDeviceTracked(adapter, {
            requiredLimits: { minUniformBufferOffsetAlignment: 255 }
          })
        );
      } else {
        t.shouldReject(
          'OperationError',
          t.requestDeviceTracked(adapter, {
            requiredLimits: { minUniformBufferOffsetAlignment: 255 }
          })
        );
      }
      break;
  }

  let device = undefined;
  const promise = t.requestDeviceTracked(adapter);
  if (awaitSuccess) {
    device = await promise;
    assert(device !== null);
  } else {
    t.shouldResolve(
      (async () => {
        const device = await promise;
        device.destroy();
      })()
    );
  }

  // Since the adapter is consumed now, requesting another device is not possible anymore.
  t.shouldReject('OperationError', t.requestDeviceTracked(adapter));
});

g.test('features,unknown').
desc(
  `
    Test requesting device with an unknown feature.`
).
fn(async (t) => {
  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);

  t.shouldReject(
    'TypeError',
    t.requestDeviceTracked(adapter, { requiredFeatures: ['unknown-feature'] })
  );
});

g.test('features,known').
desc(
  `
    Test requesting device with all features.
    - Succeeds with device supporting feature if adapter supports the feature.
    - Rejects if the adapter does not support the feature.`
).
params((u) => u.combine('feature', kFeatureNames)).
fn(async (t) => {
  const { feature } = t.params;

  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);

  const promise = t.requestDeviceTracked(adapter, { requiredFeatures: [feature] });
  if (hasFeature(adapter.features, feature)) {
    const device = await promise;
    t.expect(hasFeature(device.features, feature), 'Device should include the required feature');
  } else {
    t.shouldReject('TypeError', promise);
  }
});

g.test('limits,unknown').
desc(
  `
    Test that specifying limits that aren't part of the supported limit set causes
    requestDevice to reject unless the value is undefined.
    Also tests that the invalid requestDevice() call does not expire the adapter.`
).
fn(async (t) => {
  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);

  t.shouldReject(
    'OperationError',
    t.requestDeviceTracked(adapter, { requiredLimits: { unknownLimitName: 9000 } })
  );
  // Adapter is still alive because the requestDevice() call was invalid.

  const device = await t.requestDeviceTracked(adapter, {
    requiredLimits: { unknownLimitName: undefined }
  });
  assert(device !== null);
});

g.test('limits,supported').
desc(
  `
    Test that each supported limit can be specified with valid values.
    - Tests each limit with the default values given by the spec
    - Tests each limit with the supported values given by the adapter
    - Tests each limit with undefined`
).
params((u) =>
u.
combine('limit', kPossibleLimits).
beginSubcases().
combine('limitValue', ['default', 'adapter', 'undefined'])
).
fn(async (t) => {
  const { limit, limitValue } = t.params;

  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);

  const limitInfo = getDefaultLimitsForCTS()[limit];
  let value = -1;
  let result = -1;
  switch (limitValue) {
    case 'default':
      value = limitInfo.default;
      result = value;
      break;
    case 'adapter':
      value = adapter.limits[limit];
      result = value;
      break;
    case 'undefined':
      value = undefined;
      result = limitInfo.default;
      break;
  }

  const requiredLimits = { [limit]: value };

  if (
  limit === 'maxStorageBuffersInFragmentStage' ||
  limit === 'maxStorageBuffersInVertexStage')
  {
    requiredLimits['maxStorageBuffersPerShaderStage'] = value;
  }

  if (
  limit === 'maxStorageTexturesInFragmentStage' ||
  limit === 'maxStorageTexturesInVertexStage')
  {
    requiredLimits['maxStorageTexturesPerShaderStage'] = value;
  }

  const device = await t.requestDeviceTracked(adapter, { requiredLimits });
  assert(device !== null);
  t.expect(
    device.limits[limit] === result,
    `Devices reported limit for ${limit}(${device.limits[limit]}) should match the required limit (${result})`
  );
});

g.test('limit,better_than_supported').
desc(
  `
    Test that specifying a better limit than what the adapter supports causes requestDevice to
    reject.
    - Tests each limit
    - Tests requesting better limits by various amounts`
).
params((u) =>
u.
combine('limit', kPossibleLimits).
beginSubcases().
expandWithParams((p) => {
  switch (kLimitClasses[p.limit]) {
    case 'maximum':
      return [
      { mul: 1, add: 1 },
      { mul: 1, add: 100 }];

    case 'alignment':
      return [
      { mul: 1, add: -1 },
      { mul: 1 / 2, add: 0 },
      { mul: 1 / 1024, add: 0 }];

  }
})
).
fn(async (t) => {
  const { limit, mul, add } = t.params;

  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);

  const limitInfo = getDefaultLimitsForCTS();
  const value = adapter.limits[limit] * mul + add;
  const requiredLimits = {
    [limit]: clamp(value, { min: 0, max: limitInfo[limit].maximumValue })
  };

  t.shouldReject('OperationError', t.requestDeviceTracked(adapter, { requiredLimits }));
});

g.test('limit,out_of_range').
desc(
  `
    Test that specifying limits that are out of range (<0, >MAX_SAFE_INTEGER, >2**31-2 for 32-bit
    limits, =0 for alignment limits) produce the appropriate error (TypeError or OperationError).
    `
).
params((u) =>
u.
combine('limit', kPossibleLimits).
beginSubcases().
expand('value', function* () {
  yield -(2 ** 64);
  yield Number.MIN_SAFE_INTEGER - 3;
  yield Number.MIN_SAFE_INTEGER - 1;
  yield Number.MIN_SAFE_INTEGER;
  yield -(2 ** 32);
  yield -1;
  yield 0;
  yield 2 ** 32 - 2;
  yield 2 ** 32 - 1;
  yield 2 ** 32;
  yield 2 ** 32 + 1;
  yield 2 ** 32 + 2;
  yield Number.MAX_SAFE_INTEGER;
  yield Number.MAX_SAFE_INTEGER + 1;
  yield Number.MAX_SAFE_INTEGER + 3;
  yield 2 ** 64;
  yield Number.MAX_VALUE;
})
).
fn(async (t) => {
  const { limit, value } = t.params;

  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);
  const limitInfo = getDefaultLimitsForCTS()[limit];
  const requiredLimits = {
    [limit]: value
  };

  const errorName =
  value < 0 || value > Number.MAX_SAFE_INTEGER ?
  'TypeError' :
  limitInfo.class === 'maximum' && value > adapter.limits[limit] ?
  'OperationError' :
  limitInfo.class === 'alignment' && (value > 2 ** 31 || !isPowerOfTwo(value)) ?
  'OperationError' :
  false;

  const devicePromise = t.requestDeviceTracked(adapter, { requiredLimits });
  if (errorName) {
    t.shouldReject(errorName, devicePromise);
  } else {
    await devicePromise;
  }
});

g.test('limit,worse_than_default').
desc(
  `
    Test that specifying a worse limit than the default values required by the spec cause the value
    to clamp.
    - Tests each limit
    - Tests requesting worse limits by various amounts`
).
params((u) =>
u.
combine('limit', kPossibleLimits).
beginSubcases().
expandWithParams((p) => {
  switch (kLimitClasses[p.limit]) {
    case 'maximum':
      return [
      { mul: 1, add: -1 },
      { mul: 1, add: -100 }];

    case 'alignment':
      return [
      { mul: 1, add: 1 },
      { mul: 2, add: 0 },
      { mul: 1024, add: 0 }];

  }
})
).
fn(async (t) => {
  const { limit, mul, add } = t.params;

  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter();
  assert(adapter !== null);

  const limitInfo = getDefaultLimitsForCTS()[limit];
  const value = limitInfo.default * mul + add;
  const requiredLimits = {
    [limit]: clamp(value, { min: 0, max: limitInfo.maximumValue })
  };

  let success;
  switch (limitInfo.class) {
    case 'alignment':
      success = isPowerOfTwo(value);
      break;
    case 'maximum':
      success = true;
      break;
  }

  const devicePromise = t.requestDeviceTracked(adapter, { requiredLimits });
  if (success) {
    const device = await devicePromise;
    assert(device !== null);
    t.expect(
      device.limits[limit] === limitInfo.default,
      'Devices reported limit should match the default limit'
    );
    device.destroy();
  } else {
    t.shouldReject('OperationError', devicePromise);
  }
});

g.test('always_returns_device').
desc(
  `
    Test that if requestAdapter returns an adapter then requestDevice must return a device.

    requestAdapter -> null = ok
    requestAdapter -> adapter, requestDevice -> device (lost or not) = ok
    requestAdapter -> adapter, requestDevice = null = Invalid: not spec compliant.

    Note: requestDevice can throw for invalid parameters like requesting features not
    in the adapter, reqesting limits not in the adapter, requesting limits larger than
    the maximum for the adapter. Otherwise it does not throw.

    Note: This is a regression test for a Chrome bug crbug.com/349062459
    Checking that a requestDevice always return a device is checked in other tests above
    but those tests have 'featureLevel: "compatibility"' set for them by the API that getGPU
    returns when the test suite is run in compatibility mode.

    This test tries to force both compat and core separately so both code paths are
    tested in the same browser configuration.
  `
).
params((u) => u.combine('featureLevel', ['core', 'compatibility'])).
fn(async (t) => {
  const { featureLevel } = t.params;
  const gpu = getGPU(t.rec);
  const adapter = await gpu.requestAdapter({
    featureLevel
  });
  if (adapter) {
    const device = await t.requestDeviceTracked(adapter);
    assert(device instanceof GPUDevice, 'requestDevice must return a device or throw');

    if (featureLevel === 'core' && hasFeature(adapter.features, 'core-features-and-limits')) {
      // Check if the device supports core, when featureLevel is core and adapter supports core.
      // This check is to make sure something lower-level is not forcing compatibility mode.

      t.expect(
        hasFeature(device.features, 'core-features-and-limits'),
        'must not get a Compatibility adapter if not requested'
      );
    }
  }
});
});
__define('common/framework/test_group.js', function (__exports) {
'use strict';
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/__get(__exports, 'makeTestGroup', () => __require('common/internal/test_group.js').makeTestGroup);
});
__define('common/internal/test_group.js', function (__exports) {
'use strict';
__get(__exports, 'makeTestGroup', () => makeTestGroup);
__get(__exports, 'makeTestGroupForUnitTesting', () => makeTestGroupForUnitTesting);
__get(__exports, 'kQueryMaxLength', () => kQueryMaxLength);
__get(__exports, 'TestGroup', () => TestGroup);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let SkipTestCase, UnexpectedPassError; __bind('common/framework/fixture.js', (__m) => { ({ SkipTestCase, UnexpectedPassError } = __m); });
let builderIterateCasesWithSubcases, kUnitCaseParamsBuilder; __bind('common/framework/params_builder.js', (__m) => { ({ builderIterateCasesWithSubcases, kUnitCaseParamsBuilder } = __m); });
let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });


let extractPublicParams, mergeParams; __bind('common/internal/params_utils.js', (__m) => { ({ extractPublicParams, mergeParams } = __m); });
let compareQueries, Ordering; __bind('common/internal/query/compare.js', (__m) => { ({ compareQueries, Ordering } = __m); });
let TestQueryMultiTest, TestQuerySingleCase; __bind('common/internal/query/query.js', (__m) => { ({ TestQueryMultiTest, TestQuerySingleCase } = __m); });
let kPathSeparator; __bind('common/internal/query/separators.js', (__m) => { ({ kPathSeparator } = __m); });
let stringifyPublicParams, stringifyPublicParamsUniquely; __bind('common/internal/query/stringify_params.js', (__m) => { ({ stringifyPublicParams, stringifyPublicParamsUniquely } = __m); });
let validQueryPart; __bind('common/internal/query/validQueryPart.js', (__m) => { ({ validQueryPart } = __m); });
let attemptGarbageCollection; __bind('common/util/collect_garbage.js', (__m) => { ({ attemptGarbageCollection } = __m); });

let assert, unreachable; __bind('common/util/util.js', (__m) => { ({ assert, unreachable } = __m); });

let logToWebSocket; __bind('common/internal/websocket_logger.js', (__m) => { ({ logToWebSocket } = __m); });






















// Interface for defining tests



function makeTestGroup(fixture) {
  return new TestGroup(fixture);
}

// Interfaces for running tests











function makeTestGroupForUnitTesting(
fixture)
{
  return new TestGroup(fixture);
}

/** The maximum allowed length of a test query string. Checked by tools/validate. */
const kQueryMaxLength = 375;

/** Parameter name for batch number (see also TestBuilder.batch). */
const kBatchParamName = 'batch__';








class TestGroup {

  seen = new Set();
  tests = [];

  constructor(fixture) {
    this.fixture = fixture;
  }

  iterate() {
    return this.tests;
  }

  checkName(name) {
    assert(
      // Shouldn't happen due to the rule above. Just makes sure that treating
      // unencoded strings as encoded strings is OK.
      name === decodeURIComponent(name),
      `Not decodeURIComponent-idempotent: ${name} !== ${decodeURIComponent(name)}`
    );
    assert(!this.seen.has(name), `Duplicate test name: ${name}`);

    this.seen.add(name);
  }

  test(name) {
    const testCreationStack = new Error(`Test created: ${name}`);

    this.checkName(name);

    const parts = name.split(kPathSeparator);
    for (const p of parts) {
      assert(validQueryPart.test(p), `Invalid test name part ${p}; must match ${validQueryPart}`);
    }

    const test = new TestBuilder(parts, this.fixture, testCreationStack);
    this.tests.push(test);
    return test;
  }

  validate(fileQuery) {
    for (const test of this.tests) {
      const testQuery = new TestQueryMultiTest(
        fileQuery.suite,
        fileQuery.filePathParts,
        test.testPath
      );
      test.validate(testQuery);
    }
  }

  collectNonEmptyTests() {
    const testPaths = [];
    for (const test of this.tests) {
      if (test.computeCaseCount() > 0) {
        testPaths.push({ testPath: test.testPath });
      }
    }
    return testPaths;
  }
}
















































































class TestBuilder {








  testCases = undefined;
  batchSize = 0;

  constructor(testPath, fixture, testCreationStack) {
    this.testPath = testPath;
    this.isUnimplemented = false;
    this.fixture = fixture;
    this.testCreationStack = testCreationStack;
  }

  desc(description) {
    this.description = description.trim();
    return this;
  }

  specURL(_url) {
    return this;
  }

  beforeAllSubcases(fn) {
    assert(this.beforeFn === undefined);
    this.beforeFn = fn;
    return this;
  }

  fn(fn) {

    // MAINTENANCE_TODO: add "TODO" if there's no description? (and make sure it only ends up on
    // actual tests, not on test parents in the tree, which is what happens if you do it here, not
    // sure why)
    assert(this.testFn === undefined);
    this.testFn = fn;
  }

  batch(b) {
    this.batchSize = b;
    return this;
  }

  unimplemented() {
    assert(this.testFn === undefined);

    this.description =
    (this.description ? this.description + '\n\n' : '') + 'TODO: .unimplemented()';
    this.isUnimplemented = true;

    // Use the beforeFn to skip the test, so we don't have to iterate the subcases.
    this.beforeFn = () => {
      throw new SkipTestCase('test unimplemented');
    };
    this.testFn = () => {};
  }

  /** Perform various validation/"lint" chenks. */
  validate(testQuery) {
    const testPathString = this.testPath.join(kPathSeparator);
    assert(this.testFn !== undefined, () => {
      let s = `Test is missing .fn(): ${testPathString}`;
      if (this.testCreationStack.stack) {
        s += `\n-> test created at:\n${this.testCreationStack.stack}`;
      }
      return s;
    });

    assert(
      testQuery.toString().length <= kQueryMaxLength,
      () =>
      `Test query ${testQuery} is too long. Max length is ${kQueryMaxLength} characters. Please shorten names or reduce parameters.`
    );

    if (this.testCases === undefined) {
      return;
    }

    const seen = new Set();
    for (const [caseParams, subcases] of builderIterateCasesWithSubcases(this.testCases, null)) {
      const caseQuery = new TestQuerySingleCase(
        testQuery.suite,
        testQuery.filePathParts,
        testQuery.testPathParts,
        caseParams
      ).toString();
      assert(
        caseQuery.length <= kQueryMaxLength,
        () =>
        `Case query ${caseQuery} is too long. Max length is ${kQueryMaxLength} characters. Please shorten names or reduce parameters.`
      );

      for (const subcaseParams of subcases ?? [{}]) {
        const params = mergeParams(caseParams, subcaseParams);
        assert(this.batchSize === 0 || !(kBatchParamName in params));

        // stringifyPublicParams also checks for invalid params values
        let testcaseString;
        try {
          testcaseString = stringifyPublicParams(params);
        } catch (e) {
          throw new Error(`${e}: ${testPathString}`);
        }

        // A (hopefully) unique representation of a params value.
        const testcaseStringUnique = stringifyPublicParamsUniquely(params);
        assert(
          !seen.has(testcaseStringUnique),
          `Duplicate public test case+subcase params for test ${testPathString}: ${testcaseString} (${caseQuery})`
        );
        seen.add(testcaseStringUnique);
      }
    }
  }

  computeCaseCount() {
    if (this.testCases === undefined) {
      return 1;
    }

    let caseCount = 0;
    for (const [_caseParams, _subcases] of builderIterateCasesWithSubcases(this.testCases, null)) {
      caseCount++;
    }
    return caseCount;
  }

  params(
  cases)
  {
    assert(this.testCases === undefined, 'test case is already parameterized');
    if (cases instanceof Function) {
      this.testCases = cases(kUnitCaseParamsBuilder);
    } else {
      this.testCases = cases;
    }
    return this;
  }

  paramsSimple(cases) {
    assert(this.testCases === undefined, 'test case is already parameterized');
    this.testCases = kUnitCaseParamsBuilder.combineWithParams(cases);
    return this;
  }

  paramsSubcasesOnly(
  subcases)
  {
    if (subcases instanceof Function) {
      return this.params(subcases(kUnitCaseParamsBuilder.beginSubcases()));
    } else {
      return this.params(kUnitCaseParamsBuilder.beginSubcases().combineWithParams(subcases));
    }
  }

  makeCaseSpecific(params, subcases) {
    assert(this.testFn !== undefined, 'No test function (.fn()) for test');
    return new RunCaseSpecific(
      this.testPath,
      params,
      this.isUnimplemented,
      subcases,
      this.fixture,
      this.testFn,
      this.beforeFn,
      this.testCreationStack
    );
  }

  *iterate(caseFilter) {
    this.testCases ??= kUnitCaseParamsBuilder;

    // Remove the batch__ from the caseFilter because the params builder doesn't
    // know about it (we don't add it until later in this function).
    let filterToBatch;
    const caseFilterWithoutBatch = caseFilter ? { ...caseFilter } : null;
    if (caseFilterWithoutBatch && kBatchParamName in caseFilterWithoutBatch) {
      const batchParam = caseFilterWithoutBatch[kBatchParamName];
      assert(typeof batchParam === 'number');
      filterToBatch = batchParam;
      delete caseFilterWithoutBatch[kBatchParamName];
    }

    for (const [caseParams, subcases] of builderIterateCasesWithSubcases(
      this.testCases,
      caseFilterWithoutBatch
    )) {
      // If batches are not used, yield just one case.
      if (this.batchSize === 0 || subcases === undefined) {
        yield this.makeCaseSpecific(caseParams, subcases);
        continue;
      }

      // Same if there ends up being only one batch.
      const subcaseArray = Array.from(subcases);
      if (subcaseArray.length <= this.batchSize) {
        yield this.makeCaseSpecific(caseParams, subcaseArray);
        continue;
      }

      // There are multiple batches. Helper function for this case:
      const makeCaseForBatch = (batch) => {
        const sliceStart = batch * this.batchSize;
        return this.makeCaseSpecific(
          { ...caseParams, [kBatchParamName]: batch },
          subcaseArray.slice(sliceStart, Math.min(subcaseArray.length, sliceStart + this.batchSize))
        );
      };

      // If we filter to just one batch, yield it.
      if (filterToBatch !== undefined) {
        yield makeCaseForBatch(filterToBatch);
        continue;
      }

      // Finally, if not, yield all of the batches.
      for (let batch = 0; batch * this.batchSize < subcaseArray.length; ++batch) {
        yield makeCaseForBatch(batch);
      }
    }
  }
}

class RunCaseSpecific {










  constructor(
  testPath,
  params,
  isUnimplemented,
  subcases,
  fixture,
  fn,
  beforeFn,
  testCreationStack)
  {
    this.id = { test: testPath, params: extractPublicParams(params) };
    this.isUnimplemented = isUnimplemented;
    this.params = params;
    this.subcases = subcases;
    this.fixture = fixture;
    this.fn = fn;
    this.beforeFn = beforeFn;
    this.testCreationStack = testCreationStack;
  }

  computeSubcaseCount() {
    if (this.subcases) {
      let count = 0;
      for (const _subcase of this.subcases) {
        count++;
      }
      return count;
    } else {
      return 1;
    }
  }

  async runTest(
  rec,
  sharedState,
  params,
  throwSkip,
  expectedStatus)
  {
    try {
      rec.beginSubCase();
      if (expectedStatus === 'skip') {
        throw new SkipTestCase('Skipped by expectations');
      }

      const inst = new this.fixture(sharedState, rec, params);
      try {
        await inst.init();
        await this.fn(inst);
        rec.passed();
      } finally {
        // Runs as long as constructor succeeded, even if initialization or the test failed.
        await inst.finalize();
      }
    } catch (ex) {
      // There was an exception from constructor, init, test, or finalize.
      // An error from init or test may have been a SkipTestCase.
      // An error from finalize may have been an eventualAsyncExpectation failure
      // or unexpected validation/OOM error from the GPUDevice.
      rec.threw(ex);
      if (throwSkip && ex instanceof SkipTestCase) {
        throw ex;
      }
    } finally {
      try {
        rec.endSubCase(expectedStatus);
      } catch (ex) {
        assert(ex instanceof UnexpectedPassError);
        ex.message = `Testcase passed unexpectedly.`;
        ex.stack = this.testCreationStack.stack;
        rec.warn(ex);
      }
    }
  }

  async run(
  rec,
  selfQuery,
  expectations)
  {
    const getExpectedStatus = (selfQueryWithSubParams) => {
      let didSeeFail = false;
      for (const exp of expectations) {
        const ordering = compareQueries(exp.query, selfQueryWithSubParams);
        if (ordering === Ordering.Unordered || ordering === Ordering.StrictSubset) {
          continue;
        }

        switch (exp.expectation) {
          // Skip takes precedence. If there is any expectation indicating a skip,
          // signal it immediately.
          case 'skip':
            return 'skip';
          case 'fail':
            // Otherwise, indicate that we might expect a failure.
            didSeeFail = true;
            break;
          default:
            unreachable();
        }
      }
      return didSeeFail ? 'fail' : 'pass';
    };

    const { testHeartbeatCallback, maxSubcasesInFlight } = globalTestConfig;
    try {
      rec.start();
      const sharedState = this.fixture.MakeSharedState(rec, this.params);
      try {
        await sharedState.init();
        if (this.beforeFn) {
          await this.beforeFn(sharedState);
        }
        await sharedState.postInit();
        testHeartbeatCallback();

        let allPreviousSubcasesFinalizedPromise = Promise.resolve();
        if (this.subcases) {
          let totalCount = 0;
          let skipCount = 0;

          // If there are too many subcases in flight, starting the next subcase will register
          // `resolvePromiseBlockingSubcase` and wait until `subcaseFinishedCallback` is called.
          let subcasesInFlight = 0;
          let resolvePromiseBlockingSubcase = undefined;
          const subcaseFinishedCallback = () => {
            subcasesInFlight -= 1;
            // If there is any subcase waiting on a previous subcase to finish,
            // unblock it now, and clear the resolve callback.
            if (resolvePromiseBlockingSubcase) {
              resolvePromiseBlockingSubcase();
              resolvePromiseBlockingSubcase = undefined;
            }
          };

          for (const subParams of this.subcases) {
            // Defer subcase logs so that they appear in the correct order.
            const subRec = rec.makeDeferredSubRecorder(
              `(in subcase: ${stringifyPublicParams(subParams)}) `,
              allPreviousSubcasesFinalizedPromise
            );

            const params = mergeParams(this.params, subParams);
            const subcaseQuery = new TestQuerySingleCase(
              selfQuery.suite,
              selfQuery.filePathParts,
              selfQuery.testPathParts,
              params
            );

            // Limit the maximum number of subcases in flight.
            if (subcasesInFlight >= maxSubcasesInFlight) {
              await new Promise((resolve) => {
                // There should only be one subcase waiting at a time.
                assert(resolvePromiseBlockingSubcase === undefined);
                resolvePromiseBlockingSubcase = resolve;
              });
            }

            subcasesInFlight += 1;
            // Runs async without waiting so that subsequent subcases can start.
            // All finalization steps will be waited on at the end of the testcase.
            const finalizePromise = this.runTest(
              subRec,
              sharedState,
              params,
              /* throwSkip */true,
              getExpectedStatus(subcaseQuery)
            ).
            then(() => {
              subRec.info(new Error('subcase ran'));
            }).
            catch((ex) => {
              if (ex instanceof SkipTestCase) {
                // Convert SkipTestCase to an info message so it won't skip the whole test
                ex.message = 'subcase skipped: ' + ex.message;
                subRec.info(new Error('subcase skipped'));
                ++skipCount;
              } else {
                // We are catching all other errors inside runTest(), so this should never happen
                subRec.threw(ex);
              }
            }).
            finally(attemptGarbageCollectionIfDue).
            finally(subcaseFinishedCallback);

            allPreviousSubcasesFinalizedPromise = allPreviousSubcasesFinalizedPromise.then(
              () => finalizePromise
            );
            ++totalCount;
          }

          // Wait for all subcases to finalize and report their results.
          await allPreviousSubcasesFinalizedPromise;

          if (skipCount === totalCount) {
            rec.skipped(new SkipTestCase('all subcases were skipped'));
          }
        } else {
          try {
            await this.runTest(
              rec,
              sharedState,
              this.params,
              /* throwSkip */false,
              getExpectedStatus(selfQuery)
            );
          } finally {
            await attemptGarbageCollectionIfDue();
          }
        }
      } finally {
        testHeartbeatCallback();
        // Runs as long as the shared state constructor succeeded, even if initialization or a test failed.
        await sharedState.finalize();
        testHeartbeatCallback();
      }
    } catch (ex) {
      // There was an exception from sharedState/fixture constructor, init, beforeFn, or test.
      // An error from beforeFn may have been SkipTestCase.
      // An error from finalize may have been an eventualAsyncExpectation failure
      // or unexpected validation/OOM error from the GPUDevice.
      rec.threw(ex);
    } finally {
      rec.finish();

      const msg = {
        q: selfQuery.toString(),
        timems: rec.result.timems,
        nonskippedSubcaseCount: rec.nonskippedSubcaseCount
      };
      logToWebSocket(JSON.stringify(msg));
    }
  }
}












/** Every `subcasesBetweenAttemptingGC` calls to this function will `attemptGarbageCollection()`. */
const attemptGarbageCollectionIfDue = (() => {
  // This state is global because garbage is global.
  let subcasesSinceLastGC = 0;

  return async function attemptGarbageCollectionIfDue() {
    subcasesSinceLastGC++;
    if (subcasesSinceLastGC >= globalTestConfig.subcasesBetweenAttemptingGC) {
      subcasesSinceLastGC = 0;
      return attemptGarbageCollection();
    }
  };
})();
});
__define('common/framework/params_builder.js', function (__exports) {
'use strict';
__get(__exports, 'ParamsBuilderBase', () => ParamsBuilderBase);
__get(__exports, 'builderIterateCasesWithSubcases', () => builderIterateCasesWithSubcases);
__get(__exports, 'CaseParamsBuilder', () => CaseParamsBuilder);
__get(__exports, 'kUnitCaseParamsBuilder', () => kUnitCaseParamsBuilder);
__get(__exports, 'SubcaseParamsBuilder', () => SubcaseParamsBuilder);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let mergeParams, mergeParamsChecked; __bind('common/internal/params_utils.js', (__m) => { ({ mergeParams, mergeParamsChecked } = __m); });let comparePublicParamsPaths, Ordering; __bind('common/internal/query/compare.js', (__m) => { ({ comparePublicParamsPaths, Ordering } = __m); });let stringifyPublicParams; __bind('common/internal/query/stringify_params.js', (__m) => { ({ stringifyPublicParams } = __m); });

let assert, mapLazy, objectEquals; __bind('common/util/util.js', (__m) => { ({ assert, mapLazy, objectEquals } = __m); });



// ================================================================
// "Public" ParamsBuilder API / Documentation
// ================================================================

/**
 * Provides doc comments for the methods of CaseParamsBuilder and SubcaseParamsBuilder.
 * (Also enforces rough interface match between them.)
 */






























































/**
 * Determines the resulting parameter object type which would be generated by an object of
 * the given ParamsBuilder type.
 */









// ================================================================
// Implementation
// ================================================================

/**
 * Iterable over pairs of either:
 * - `[case params, Iterable<subcase params>]` if there are subcases.
 * - `[case params, undefined]` if not.
 */




/**
 * Base class for `CaseParamsBuilder` and `SubcaseParamsBuilder`.
 */
class ParamsBuilderBase {


  constructor(cases) {
    this.cases = cases;
  }

  /**
   * Hidden from test files. Use `builderIterateCasesWithSubcases` to access this.
   */



}

/**
 * Calls the (normally hidden) `iterateCasesWithSubcases()` method.
 */
function builderIterateCasesWithSubcases(
builder,
caseFilter)
{




  return builder.iterateCasesWithSubcases(caseFilter);
}

/**
 * Builder for combinatorial test **case** parameters.
 *
 * CaseParamsBuilder is immutable. Each method call returns a new, immutable object,
 * modifying the list of cases according to the method called.
 *
 * This means, for example, that the `unit` passed into `TestBuilder.params()` can be reused.
 */
class CaseParamsBuilder extends
ParamsBuilderBase

{
  *iterateCasesWithSubcases(caseFilter) {
    for (const caseP of this.cases(caseFilter)) {
      if (caseFilter) {
        // this.cases() only filters out cases which conflict with caseFilter. Now that we have
        // the final caseP, filter out cases which are missing keys that caseFilter requires.
        const ordering = comparePublicParamsPaths(caseP, caseFilter);
        if (ordering === Ordering.StrictSuperset || ordering === Ordering.Unordered) {
          continue;
        }
      }

      yield [caseP, undefined];
    }
  }

  [Symbol.iterator]() {
    return this.cases(null);
  }

  /** @inheritDoc */
  expandWithParams(
  expander)
  {
    const baseGenerator = this.cases;
    return new CaseParamsBuilder(function* (caseFilter) {
      for (const a of baseGenerator(caseFilter)) {
        for (const b of expander(a)) {
          if (caseFilter) {
            // If the expander generated any key-value pair that conflicts with caseFilter, skip.
            const kvPairs = Object.entries(b);
            if (kvPairs.some(([k, v]) => k in caseFilter && !objectEquals(caseFilter[k], v))) {
              continue;
            }
          }

          yield mergeParamsChecked(a, b);
        }
      }
    });
  }

  /** @inheritDoc */
  expand(
  key,
  expander)
  {
    const baseGenerator = this.cases;
    return new CaseParamsBuilder(function* (caseFilter) {
      for (const a of baseGenerator(caseFilter)) {
        assert(!(key in a), `New key '${key}' already exists in ${JSON.stringify(a)}`);

        for (const v of expander(a)) {
          // If the expander generated a value for this key that conflicts with caseFilter, skip.
          if (caseFilter && key in caseFilter) {
            if (!objectEquals(caseFilter[key], v)) {
              continue;
            }
          }
          yield { ...a, [key]: v };
        }
      }
    });
  }

  /** @inheritDoc */
  combineWithParams(
  newParams)
  {
    assertNotGenerator(newParams);
    const seenValues = new Set();
    for (const params of newParams) {
      const paramsStr = stringifyPublicParams(params);
      assert(!seenValues.has(paramsStr), `Duplicate entry in combine[WithParams]: ${paramsStr}`);
      seenValues.add(paramsStr);
    }

    return this.expandWithParams(() => newParams);
  }

  /** @inheritDoc */
  combine(
  key,
  values)
  {
    assertNotGenerator(values);
    const mapped = mapLazy(values, (v) => ({ [key]: v }));
    return this.combineWithParams(mapped);
  }

  /** @inheritDoc */
  filter(pred) {
    const baseGenerator = this.cases;
    return new CaseParamsBuilder(function* (caseFilter) {
      for (const a of baseGenerator(caseFilter)) {
        if (pred(a)) yield a;
      }
    });
  }

  /** @inheritDoc */
  unless(pred) {
    return this.filter((x) => !pred(x));
  }

  /**
   * "Finalize" the list of cases and begin defining subcases.
   * Returns a new SubcaseParamsBuilder. Methods called on SubcaseParamsBuilder
   * generate new subcases instead of new cases.
   */
  beginSubcases() {
    return new SubcaseParamsBuilder(this.cases, function* () {
      yield {};
    });
  }
}

/**
 * The unit CaseParamsBuilder, representing a single case with no params: `[ {} ]`.
 *
 * `punit` is passed to every `.params()`/`.paramsSubcasesOnly()` call, so `kUnitCaseParamsBuilder`
 * is only explicitly needed if constructing a ParamsBuilder outside of a test builder.
 */
const kUnitCaseParamsBuilder = new CaseParamsBuilder(function* () {
  yield {};
});

/**
 * Builder for combinatorial test _subcase_ parameters.
 *
 * SubcaseParamsBuilder is immutable. Each method call returns a new, immutable object,
 * modifying the list of subcases according to the method called.
 */
class SubcaseParamsBuilder extends
ParamsBuilderBase

{


  constructor(
  cases,
  generator)
  {
    super(cases);
    this.subcases = generator;
  }

  *iterateCasesWithSubcases(caseFilter) {
    for (const caseP of this.cases(caseFilter)) {
      if (caseFilter) {
        // this.cases() only filters out cases which conflict with caseFilter. Now that we have
        // the final caseP, filter out cases which are missing keys that caseFilter requires.
        const ordering = comparePublicParamsPaths(caseP, caseFilter);
        if (ordering === Ordering.StrictSuperset || ordering === Ordering.Unordered) {
          continue;
        }
      }

      const subcases = Array.from(this.subcases(caseP));
      if (subcases.length) {
        yield [
        caseP,
        subcases];

      }
    }
  }

  /** @inheritDoc */
  expandWithParams(
  expander)
  {
    const baseGenerator = this.subcases;
    return new SubcaseParamsBuilder(this.cases, function* (base) {
      for (const a of baseGenerator(base)) {
        for (const b of expander(mergeParams(base, a))) {
          yield mergeParamsChecked(a, b);
        }
      }
    });
  }

  /** @inheritDoc */
  expand(
  key,
  expander)
  {
    const baseGenerator = this.subcases;
    return new SubcaseParamsBuilder(this.cases, function* (base) {
      for (const a of baseGenerator(base)) {
        const before = mergeParams(base, a);
        assert(!(key in before), () => `Key '${key}' already exists in ${JSON.stringify(before)}`);

        for (const v of expander(before)) {
          yield { ...a, [key]: v };
        }
      }
    });
  }

  /** @inheritDoc */
  combineWithParams(
  newParams)
  {
    assertNotGenerator(newParams);
    return this.expandWithParams(() => newParams);
  }

  /** @inheritDoc */
  combine(
  key,
  values)
  {
    assertNotGenerator(values);
    return this.expand(key, () => values);
  }

  /** @inheritDoc */
  filter(pred) {
    const baseGenerator = this.subcases;
    return new SubcaseParamsBuilder(this.cases, function* (base) {
      for (const a of baseGenerator(base)) {
        if (pred(mergeParams(base, a))) yield a;
      }
    });
  }

  /** @inheritDoc */
  unless(pred) {
    return this.filter((x) => !pred(x));
  }
}

/** Assert an object is not a Generator (a thing returned from a generator function). */
function assertNotGenerator(x) {
  if ('constructor' in x) {
    assert(
      x.constructor !== function* () {}().constructor,
      'Argument must not be a generator, as generators are not reusable'
    );
  }
}
});
__define('common/util/collect_garbage.js', function (__exports) {
'use strict';
__get(__exports, 'attemptGarbageCollection', () => attemptGarbageCollection);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let resolveOnTimeout; __bind('common/util/util.js', (__m) => { ({ resolveOnTimeout } = __m); });


/**
 * Attempts to trigger JavaScript garbage collection, either using explicit methods if exposed
 * (may be available in testing environments with special browser runtime flags set), or using
 * some weird tricks to incur GC pressure. Adopted from the WebGL CTS.
 */
async function attemptGarbageCollection() {

  const w = globalThis;
  if (w.GCController) {
    w.GCController.collect();
    return;
  }

  if (w.opera && w.opera.collect) {
    w.opera.collect();
    return;
  }

  try {
    w.QueryInterface(Components.interfaces.nsIInterfaceRequestor).
    getInterface(Components.interfaces.nsIDOMWindowUtils).
    garbageCollect();
    return;
  } catch (e) {

    // ignore any failure
  }
  if (w.gc) {
    w.gc();
    return;
  }

  if (w.CollectGarbage) {
    w.CollectGarbage();
    return;
  }

  let i;
  function gcRec(n) {
    if (n < 1) return;

    let temp = { i: 'ab' + i + i / 100000 };

    temp = temp + 'foo';
    temp; // dummy use of unused variable
    gcRec(n - 1);
  }
  for (i = 0; i < 1000; i++) {
    gcRec(10);
  }

  return resolveOnTimeout(35); // Let the event loop run a few frames in case it helps.
}
});
__define('common/internal/websocket_logger.js', function (__exports) {
'use strict';
__get(__exports, 'logToWebSocket', () => logToWebSocket);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); }); /**
 * - 'uninitialized' means we haven't tried to connect yet
 * - Promise means it's pending
 * - 'failed' means it failed (this is the most common case, where the logger isn't running)
 * - WebSocket means it succeeded
 */
let connection =
'uninitialized';

/**
 * If the logToWebSocket option is enabled (?log_to_web_socket=1 in browser,
 * --log-to-web-socket on command line, or enable it by default in options.ts),
 * log a string to a websocket at `localhost:59497`. See `tools/websocket-logger`.
 *
 * This does nothing if a logToWebSocket is not enabled, or if a connection
 * couldn't be established on the first call.
 */
function logToWebSocket(msg) {
  if (!globalTestConfig.logToWebSocket || connection === 'failed') {
    return;
  }

  if (connection === 'uninitialized') {
    connection = new Promise((resolve) => {
      if (typeof WebSocket === 'undefined') {
        resolve('failed');
        return;
      }

      const ws = new WebSocket('ws://localhost:59497/optional_cts_websocket_logger');
      ws.onopen = () => {
        resolve(ws);
      };
      ws.onerror = () => {
        connection = 'failed';
        resolve('failed');
      };
      ws.onclose = () => {
        connection = 'failed';
        resolve('failed');
      };
    });
    void connection.then((resolved) => {
      connection = resolved;
    });
  }

  void (async () => {
    // connection may be a promise or a value here. Either is OK to await.
    const ws = await connection;
    if (ws !== 'failed') {
      ws.send(msg);
    }
  })();
}
});
__define('common/util/navigator_gpu.js', function (__exports) {
'use strict';
__get(__exports, 'setGPUProvider', () => setGPUProvider);
__get(__exports, 'setDefaultRequestAdapterOptions', () => setDefaultRequestAdapterOptions);
__get(__exports, 'getDefaultRequestAdapterOptions', () => getDefaultRequestAdapterOptions);
__get(__exports, 'getGPU', () => getGPU);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });

let ErrorWithExtra, assert, hasFeature, objectEquals; __bind('common/util/util.js', (__m) => { ({ ErrorWithExtra, assert, hasFeature, objectEquals } = __m); });

/**
 * Finds and returns the `navigator.gpu` object (or equivalent, for non-browser implementations).
 * Throws an exception if not found.
 */
function defaultGPUProvider() {
  assert(
    typeof navigator !== 'undefined' && navigator.gpu !== undefined,
    'No WebGPU implementation found'
  );
  return navigator.gpu;
}

/**
 * GPUProvider is a function that creates and returns a new GPU instance.
 * May throw an exception if a GPU cannot be created.
 */


let gpuProvider = defaultGPUProvider;

/**
 * Sets the function to create and return a new GPU instance.
 */
function setGPUProvider(provider) {
  assert(impl === undefined, 'setGPUProvider() should not be after getGPU()');
  gpuProvider = provider;
}

let impl = undefined;
let s_defaultLimits = undefined;

let defaultRequestAdapterOptions;

function setDefaultRequestAdapterOptions(options) {
  // It's okay to call this if you don't change the options
  if (objectEquals(options, defaultRequestAdapterOptions)) {
    return;
  }
  if (impl) {
    throw new Error('must call setDefaultRequestAdapterOptions before getGPU');
  }
  defaultRequestAdapterOptions = { ...options };
}

function getDefaultRequestAdapterOptions() {
  return defaultRequestAdapterOptions;
}

function copyLimits(objLike) {
  const obj = {};
  for (const key in objLike) {
    obj[key] = objLike[key];
  }
  return obj;
}

/**
 * Finds and returns the `navigator.gpu` object (or equivalent, for non-browser implementations).
 * Throws an exception if not found.
 */
function getGPU(recorder) {
  if (impl) {
    return impl;
  }

  impl = gpuProvider();

  if (globalTestConfig.enforceDefaultLimits) {

    const origRequestAdapterFn = impl.requestAdapter;

    const origRequestDeviceFn = GPUAdapter.prototype.requestDevice;

    Object.defineProperty(impl, 'requestAdapter', {
      configurable: true,
      async value(options) {
        if (!s_defaultLimits) {
          const tempAdapter = await origRequestAdapterFn.call(this, {
            ...defaultRequestAdapterOptions,
            ...options
          });

          const tempDevice = await tempAdapter?.requestDevice();
          s_defaultLimits = copyLimits(tempDevice.limits);
          tempDevice?.destroy();
        }
        const adapter = await origRequestAdapterFn.call(this, {
          ...defaultRequestAdapterOptions,
          ...options
        });
        if (adapter) {
          const limits = Object.fromEntries(
            Object.entries(s_defaultLimits).map(([key, v]) => [key, v])
          );

          Object.defineProperty(adapter, 'limits', {
            get() {
              return limits;
            }
          });
        }
        return adapter;
      }
    });

    const enforceDefaultLimits = (adapter, desc) => {
      if (desc?.requiredLimits) {
        for (const [key, value] of Object.entries(desc.requiredLimits)) {
          const limit = s_defaultLimits[key];
          if (limit !== undefined && value !== undefined) {
            const [beyondLimit, condition] = key.startsWith('max') ?
            [value > limit, 'greater'] :
            [value < limit, 'less'];
            if (beyondLimit) {
              throw new DOMException(
                `requestedLimit ${value} for ${key} is ${condition} than adapter limit ${limit}`,
                'OperationError'
              );
            }
          }
        }
      }
    };

    GPUAdapter.prototype.requestDevice = async function (

    desc)
    {
      // We need to enforce the default limits because even though we patched the adapter to
      // show defaults for adapter.limits, there are tests that test we throw when we request more than the max.
      // In other words.
      //
      //   adapter.requestDevice({ requiredLimits: {
      //     maxXXX: adapter.limits.maxXXX + 1,  // should throw
      //   });
      //
      // But unless we enforce this manually, it won't actually throw if the adapter's
      // true limits are higher than we patched above.
      enforceDefaultLimits(this, desc);
      return await origRequestDeviceFn.call(this, desc);
    };
  }

  if (globalTestConfig.blockAllFeatures) {

    const origRequestAdapterFn = impl.requestAdapter;

    const origRequestDeviceFn = GPUAdapter.prototype.requestDevice;

    Object.defineProperty(impl, 'requestAdapter', {
      configurable: true,
      async value(options) {
        const adapter = await origRequestAdapterFn.call(this, {
          ...defaultRequestAdapterOptions,
          ...options
        });
        if (adapter) {
          Object.defineProperty(adapter, 'features', {
            enumerable: false,
            value: new Set(
              hasFeature(adapter.features, 'core-features-and-limits') ?
              ['core-features-and-limits'] :
              []
            )
          });
        }
        return adapter;
      }
    });

    const enforceBlockedFeatures = (adapter, desc) => {
      if (desc?.requiredFeatures) {
        for (const [feature] of desc.requiredFeatures) {
          // Note: This adapter has had its features property over-ridden and will only return
          // have nothing or 'core-features-and-limits'.

          if (!adapter.features.has(feature)) {
            throw new TypeError(`requested feature ${feature} does not exist on adapter`);
          }
        }
      }
    };

    GPUAdapter.prototype.requestDevice = async function (

    desc)
    {
      // We need to enforce the feature block because even though we patched the adapter to
      // advertise no features they still exist on the real adapter.
      enforceBlockedFeatures(this, desc);
      return await origRequestDeviceFn.call(this, desc);
    };
  }

  if (defaultRequestAdapterOptions) {

    const origRequestAdapterFn = impl.requestAdapter;


    Object.defineProperty(impl, 'requestAdapter', {
      configurable: true,
      async value(options) {
        const adapter = await origRequestAdapterFn.call(this, {
          ...defaultRequestAdapterOptions,
          ...options
        });
        if (recorder && adapter) {
          const adapterInfo = adapter.info;
          const infoString = `Adapter: ${adapterInfo.vendor} / ${adapterInfo.architecture} / ${adapterInfo.device}`;
          recorder.debug(new ErrorWithExtra(infoString, () => ({ adapterInfo })));
        }
        return adapter;
      }
    });
  }

  return impl;
}
});
__define('webgpu/capability_info.js', function (__exports) {
'use strict';
__get(__exports, 'kMaxQueryCount', () => kMaxQueryCount);
__get(__exports, 'kQueryTypeInfo', () => kQueryTypeInfo);
__get(__exports, 'kQueryTypes', () => kQueryTypes);
__get(__exports, 'kBufferSizeAlignment', () => kBufferSizeAlignment);
__get(__exports, 'kBufferUsageCopyInfo', () => kBufferUsageCopyInfo);
__get(__exports, 'kBufferUsageCopy', () => kBufferUsageCopy);
__get(__exports, 'kBufferUsageKeys', () => kBufferUsageKeys);
__get(__exports, 'kBufferUsageInfo', () => kBufferUsageInfo);
__get(__exports, 'kBufferUsages', () => kBufferUsages);
__get(__exports, 'kAllBufferUsageBits', () => kAllBufferUsageBits);
__get(__exports, 'kSomeBogusBufferUsage', () => kSomeBogusBufferUsage);
__get(__exports, 'kErrorScopeFilterInfo', () => kErrorScopeFilterInfo);
__get(__exports, 'kErrorScopeFilters', () => kErrorScopeFilters);
__get(__exports, 'kGeneratableErrorScopeFilters', () => kGeneratableErrorScopeFilters);
__get(__exports, 'kCanvasTextureFormats', () => kCanvasTextureFormats);
__get(__exports, 'kCanvasAlphaModesInfo', () => kCanvasAlphaModesInfo);
__get(__exports, 'kCanvasAlphaModes', () => kCanvasAlphaModes);
__get(__exports, 'kCanvasColorSpacesInfo', () => kCanvasColorSpacesInfo);
__get(__exports, 'kCanvasColorSpaces', () => kCanvasColorSpaces);
__get(__exports, 'kTextureDimensionInfo', () => kTextureDimensionInfo);
__get(__exports, 'kTextureDimensions', () => kTextureDimensions);
__get(__exports, 'kTextureAspectInfo', () => kTextureAspectInfo);
__get(__exports, 'kTextureAspects', () => kTextureAspects);
__get(__exports, 'kCompareFunctionInfo', () => kCompareFunctionInfo);
__get(__exports, 'kCompareFunctions', () => kCompareFunctions);
__get(__exports, 'kStencilOperationInfo', () => kStencilOperationInfo);
__get(__exports, 'kStencilOperations', () => kStencilOperations);
__get(__exports, 'kTextureUsageTypeInfo', () => kTextureUsageTypeInfo);
__get(__exports, 'kTextureUsageType', () => kTextureUsageType);
__get(__exports, 'kTextureUsageCopyInfo', () => kTextureUsageCopyInfo);
__get(__exports, 'kTextureUsageCopy', () => kTextureUsageCopy);
__get(__exports, 'kTextureUsages', () => kTextureUsages);
__get(__exports, 'kAllTextureUsages', () => kAllTextureUsages);
__get(__exports, 'kSomeBogusTextureUsage', () => kSomeBogusTextureUsage);
__get(__exports, 'isValidTextureUsageCombination', () => isValidTextureUsageCombination);
__get(__exports, 'usageIsTypeErrorForConfigure', () => usageIsTypeErrorForConfigure);
__get(__exports, 'kValidCombinationsOfOneOrTwoTextureUsages', () => kValidCombinationsOfOneOrTwoTextureUsages);
__get(__exports, 'kTextureViewDimensionInfo', () => kTextureViewDimensionInfo);
__get(__exports, 'kTextureViewDimensions', () => kTextureViewDimensions);
__get(__exports, 'kVertexFormatInfo', () => kVertexFormatInfo);
__get(__exports, 'kVertexFormats', () => kVertexFormats);
__get(__exports, 'kBindableResources', () => kBindableResources);
__get(__exports, 'kMinDynamicBufferOffsetAlignment', () => kMinDynamicBufferOffsetAlignment);
__get(__exports, 'kPerStageBindingLimits', () => kPerStageBindingLimits);
__get(__exports, 'kPerPipelineBindingLimits', () => kPerPipelineBindingLimits);
__get(__exports, 'bufferBindingTypeInfo', () => bufferBindingTypeInfo);
__get(__exports, 'kBufferBindingTypes', () => kBufferBindingTypes);
__get(__exports, 'samplerBindingTypeInfo', () => samplerBindingTypeInfo);
__get(__exports, 'kSamplerBindingTypes', () => kSamplerBindingTypes);
__get(__exports, 'sampledTextureBindingTypeInfo', () => sampledTextureBindingTypeInfo);
__get(__exports, 'kTextureSampleTypes', () => kTextureSampleTypes);
__get(__exports, 'storageTextureBindingTypeInfo', () => storageTextureBindingTypeInfo);
__get(__exports, 'kStorageTextureAccessValues', () => kStorageTextureAccessValues);
__get(__exports, 'texBindingTypeInfo', () => texBindingTypeInfo);
__get(__exports, 'bindingTypeInfo', () => bindingTypeInfo);
__get(__exports, 'bufferBindingEntries', () => bufferBindingEntries);
__get(__exports, 'samplerBindingEntries', () => samplerBindingEntries);
__get(__exports, 'textureBindingEntries', () => textureBindingEntries);
__get(__exports, 'storageTextureBindingEntries', () => storageTextureBindingEntries);
__get(__exports, 'sampledAndStorageBindingEntries', () => sampledAndStorageBindingEntries);
__get(__exports, 'allBindingEntries', () => allBindingEntries);
__get(__exports, 'kShaderStageKeys', () => kShaderStageKeys);
__get(__exports, 'kShaderStages', () => kShaderStages);
__get(__exports, 'kShaderStageCombinations', () => kShaderStageCombinations);
__get(__exports, 'kShaderStageCombinationsWithStage', () => kShaderStageCombinationsWithStage);
__get(__exports, 'kTextureSampleCounts', () => kTextureSampleCounts);
__get(__exports, 'kMipmapFilterModes', () => kMipmapFilterModes);
__get(__exports, 'kAddressModes', () => kAddressModes);
__get(__exports, 'kBlendFactors', () => kBlendFactors);
__get(__exports, 'IsDualSourceBlendingFactor', () => IsDualSourceBlendingFactor);
__get(__exports, 'kBlendOperations', () => kBlendOperations);
__get(__exports, 'kPrimitiveTopology', () => kPrimitiveTopology);
__get(__exports, 'kIndexFormat', () => kIndexFormat);
__get(__exports, 'kFeatureLevels', () => kFeatureLevels);
__get(__exports, 'kLimitClasses', () => kLimitClasses);
__get(__exports, 'getDefaultLimits', () => getDefaultLimits);
__get(__exports, 'getDefaultLimitsForCTS', () => getDefaultLimitsForCTS);
__get(__exports, 'getDefaultLimitsForDevice', () => getDefaultLimitsForDevice);
__get(__exports, 'getBindingLimitForBindingType', () => getBindingLimitForBindingType);
__get(__exports, 'kPossibleLimits', () => kPossibleLimits);
__get(__exports, 'kMaxColorAttachmentsToTest', () => kMaxColorAttachmentsToTest);
__get(__exports, 'kDrawIndirectParametersSize', () => kDrawIndirectParametersSize);
__get(__exports, 'kDrawIndexedIndirectParametersSize', () => kDrawIndexedIndirectParametersSize);
__get(__exports, 'kFeatureNameInfo', () => kFeatureNameInfo);
__get(__exports, 'kFeatureNames', () => kFeatureNames);
__get(__exports, 'kKnownWGSLLanguageFeatures', () => kKnownWGSLLanguageFeatures);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ // MAINTENANCE_TODO: The generated Typedoc for this file is hard to navigate because it's
// alphabetized. Consider using namespaces or renames to fix this?

let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });
let keysOf, makeTable, makeTableRenameAndFilter, numericKeysOf; __bind('common/util/data_tables.js', (__m) => { ({ keysOf, makeTable, makeTableRenameAndFilter, numericKeysOf } = __m); });
let assertTypeTrue; __bind('common/util/types.js', (__m) => { ({ assertTypeTrue } = __m); });
let assert, combinationsOfOneOrTwoUsages, hasFeature, unreachable; __bind('common/util/util.js', (__m) => { ({ assert, combinationsOfOneOrTwoUsages, hasFeature, unreachable } = __m); });

let GPUConst, kMaxUnsignedLongValue, kMaxUnsignedLongLongValue; __bind('webgpu/constants.js', (__m) => { ({ GPUConst, kMaxUnsignedLongValue, kMaxUnsignedLongLongValue } = __m); });

// Base device limits can be found in constants.ts.

// Queries

/** Maximum number of queries in GPUQuerySet, by spec. */
const kMaxQueryCount = 4096;
/** Per-GPUQueryType info. */





const kQueryTypeInfo =


{
  'occlusion': { feature: undefined },
  'timestamp': { feature: 'timestamp-query' }
};
/** List of all GPUQueryType values. */
const kQueryTypes = keysOf(kQueryTypeInfo);

// Buffers

/** Required alignment of a GPUBuffer size, by spec. */
const kBufferSizeAlignment = 4;

/** Per-GPUBufferUsage copy info. */
const kBufferUsageCopyInfo =


{
  'COPY_NONE': 0,
  'COPY_SRC': GPUConst.BufferUsage.COPY_SRC,
  'COPY_DST': GPUConst.BufferUsage.COPY_DST,
  'COPY_SRC_DST': GPUConst.BufferUsage.COPY_SRC | GPUConst.BufferUsage.COPY_DST
};
/** List of all GPUBufferUsage copy values. */
const kBufferUsageCopy = keysOf(kBufferUsageCopyInfo);

/** Per-GPUBufferUsage keys and info. */

const kBufferUsageKeys = keysOf(GPUConst.BufferUsage);
const kBufferUsageInfo =

{
  ...GPUConst.BufferUsage
};

/** List of all GPUBufferUsage values. */
const kBufferUsages = Object.values(GPUConst.BufferUsage);
const kAllBufferUsageBits = kBufferUsages.reduce(
  (previousSet, currentUsage) => previousSet | currentUsage,
  0
);

/** An arbitrary invalid buffer usage bit. */
const kSomeBogusBufferUsage = 0x4000_0000;
assert((kSomeBogusBufferUsage & kAllBufferUsageBits) === 0);

// Errors

/** Per-GPUErrorFilter info. */
const kErrorScopeFilterInfo =




{
  'internal': { generatable: false },
  'out-of-memory': { generatable: true },
  'validation': { generatable: true }
};
/** List of all GPUErrorFilter values. */
const kErrorScopeFilters = keysOf(kErrorScopeFilterInfo);
const kGeneratableErrorScopeFilters = kErrorScopeFilters.filter(
  (e) => kErrorScopeFilterInfo[e].generatable
);

// Canvases

// The formats of GPUTextureFormat for canvas context.
const kCanvasTextureFormats = ['bgra8unorm', 'rgba8unorm', 'rgba16float'];

// The alpha mode for canvas context.
const kCanvasAlphaModesInfo =

{
  'opaque': {},
  'premultiplied': {}
};
const kCanvasAlphaModes = keysOf(kCanvasAlphaModesInfo);

// The color spaces for canvas context
const kCanvasColorSpacesInfo =

{
  'srgb': {},
  'display-p3': {}
};
const kCanvasColorSpaces = keysOf(kCanvasColorSpacesInfo);

// Textures (except for texture format info)

/** Per-GPUTextureDimension info. */
const kTextureDimensionInfo =

{
  '1d': {},
  '2d': {},
  '3d': {}
};
/** List of all GPUTextureDimension values. */
const kTextureDimensions = keysOf(kTextureDimensionInfo);

/** Per-GPUTextureAspect info. */
const kTextureAspectInfo =

{
  'all': {},
  'depth-only': {},
  'stencil-only': {}
};
/** List of all GPUTextureAspect values. */
const kTextureAspects = keysOf(kTextureAspectInfo);

// Misc

/** Per-GPUCompareFunction info. */
const kCompareFunctionInfo =


{
  'never': {},
  'less': {},
  'equal': {},
  'less-equal': {},
  'greater': {},
  'not-equal': {},
  'greater-equal': {},
  'always': {}
};
/** List of all GPUCompareFunction values. */
const kCompareFunctions = keysOf(kCompareFunctionInfo);

/** Per-GPUStencilOperation info. */
const kStencilOperationInfo =


{
  'keep': {},
  'zero': {},
  'replace': {},
  'invert': {},
  'increment-clamp': {},
  'decrement-clamp': {},
  'increment-wrap': {},
  'decrement-wrap': {}
};
/** List of all GPUStencilOperation values. */
const kStencilOperations = keysOf(kStencilOperationInfo);

// More textures (except for texture format info)

/** Per-GPUTextureUsage type info. */
const kTextureUsageTypeInfo =


{
  'texture': Number(GPUConst.TextureUsage.TEXTURE_BINDING),
  'storage': Number(GPUConst.TextureUsage.STORAGE_BINDING),
  'render': Number(GPUConst.TextureUsage.RENDER_ATTACHMENT)
};
/** List of all GPUTextureUsage type values. */
const kTextureUsageType = keysOf(kTextureUsageTypeInfo);

/** Per-GPUTextureUsage copy info. */
const kTextureUsageCopyInfo =


{
  'none': 0,
  'src': Number(GPUConst.TextureUsage.COPY_SRC),
  'dst': Number(GPUConst.TextureUsage.COPY_DST),
  'src-dest': Number(GPUConst.TextureUsage.COPY_SRC) | Number(GPUConst.TextureUsage.COPY_DST)
};
/** List of all GPUTextureUsage copy values. */
const kTextureUsageCopy = keysOf(kTextureUsageCopyInfo);


/** Per-GPUTextureUsage info. */
const kTextureUsageInfo =





{
  [GPUConst.TextureUsage.COPY_SRC]: { typeErrorForConfigure: false },
  [GPUConst.TextureUsage.COPY_DST]: { typeErrorForConfigure: false },
  [GPUConst.TextureUsage.TEXTURE_BINDING]: { typeErrorForConfigure: false },
  [GPUConst.TextureUsage.STORAGE_BINDING]: { typeErrorForConfigure: false },
  [GPUConst.TextureUsage.RENDER_ATTACHMENT]: { typeErrorForConfigure: false },
  [GPUConst.TextureUsage.TRANSIENT_ATTACHMENT]: { typeErrorForConfigure: true }
};
/** List of all GPUTextureUsage values. */
const kTextureUsages = numericKeysOf(kTextureUsageInfo);
/** Bitmask of all known texture usages. */
const kAllTextureUsages = kTextureUsages.reduce((acc, usage) => acc | usage, 0);

/** An arbitrary invalid texture usage bit. */
const kSomeBogusTextureUsage = 0x4000_0000;
assert((kSomeBogusTextureUsage & kAllTextureUsages) === 0);

/**
 * Check usage is valid for createTexture(): is non-zero, has only defined bits,
 * and follows rules for TRANSIENT_ATTACHMENT.
 */
function isValidTextureUsageCombination(usage) {
  if (usage === 0) return false;

  if (usage & GPUConst.TextureUsage.TRANSIENT_ATTACHMENT) {
    return (
      usage === (
      GPUConst.TextureUsage.TRANSIENT_ATTACHMENT | GPUConst.TextureUsage.RENDER_ATTACHMENT));

  }

  return (usage & ~kAllTextureUsages) === 0;
}

/** Check if usage contains a bit that is supposed to cause configure() to TypeError. */
function usageIsTypeErrorForConfigure(usage) {
  for (const bit of kTextureUsages) {
    if ((usage & bit) !== 0 && kTextureUsageInfo[bit].typeErrorForConfigure) {
      return true;
    }
  }
  return false;
}

/** List of all combinations of 1-2 known texture usages, that are valid for createTexture(). */
const kValidCombinationsOfOneOrTwoTextureUsages = combinationsOfOneOrTwoUsages(
  kTextureUsages
).filter(isValidTextureUsageCombination);

// Texture View

/** Per-GPUTextureViewDimension info. */





/** Per-GPUTextureViewDimension info. */
const kTextureViewDimensionInfo =


{
  '1d': { storage: true },
  '2d': { storage: true },
  '2d-array': { storage: true },
  'cube': { storage: false },
  'cube-array': { storage: false },
  '3d': { storage: true }
};
/** List of all GPUTextureDimension values. */
const kTextureViewDimensions = keysOf(kTextureViewDimensionInfo);

// Vertex formats

/** Per-GPUVertexFormat info. */
// Exists just for documentation. Otherwise could be inferred by `makeTable`.

























/** Per-GPUVertexFormat info. */
const kVertexFormatInfo =


makeTable(
  ['bytesPerComponent', 'type', 'componentCount', 'byteSize', 'wgslType'],
  [,,,,], {
    // 8 bit components
    'uint8': [1, 'uint', 1, 1, 'u32'],
    'uint8x2': [1, 'uint', 2, 2, 'vec2<u32>'],
    'uint8x4': [1, 'uint', 4, 4, 'vec4<u32>'],
    'sint8': [1, 'sint', 1, 1, 'i32'],
    'sint8x2': [1, 'sint', 2, 2, 'vec2<i32>'],
    'sint8x4': [1, 'sint', 4, 4, 'vec4<i32>'],
    'unorm8': [1, 'unorm', 1, 1, 'f32'],
    'unorm8x2': [1, 'unorm', 2, 2, 'vec2<f32>'],
    'unorm8x4': [1, 'unorm', 4, 4, 'vec4<f32>'],
    'snorm8': [1, 'snorm', 1, 1, 'f32'],
    'snorm8x2': [1, 'snorm', 2, 2, 'vec2<f32>'],
    'snorm8x4': [1, 'snorm', 4, 4, 'vec4<f32>'],
    // 16 bit components
    'uint16': [2, 'uint', 1, 2, 'u32'],
    'uint16x2': [2, 'uint', 2, 4, 'vec2<u32>'],
    'uint16x4': [2, 'uint', 4, 8, 'vec4<u32>'],
    'sint16': [2, 'sint', 1, 2, 'i32'],
    'sint16x2': [2, 'sint', 2, 4, 'vec2<i32>'],
    'sint16x4': [2, 'sint', 4, 8, 'vec4<i32>'],
    'unorm16': [2, 'unorm', 1, 2, 'f32'],
    'unorm16x2': [2, 'unorm', 2, 4, 'vec2<f32>'],
    'unorm16x4': [2, 'unorm', 4, 8, 'vec4<f32>'],
    'snorm16': [2, 'snorm', 1, 2, 'f32'],
    'snorm16x2': [2, 'snorm', 2, 4, 'vec2<f32>'],
    'snorm16x4': [2, 'snorm', 4, 8, 'vec4<f32>'],
    'float16': [2, 'float', 1, 2, 'f32'],
    'float16x2': [2, 'float', 2, 4, 'vec2<f32>'],
    'float16x4': [2, 'float', 4, 8, 'vec4<f32>'],
    // 32 bit components
    'float32': [4, 'float', 1, 4, 'f32'],
    'float32x2': [4, 'float', 2, 8, 'vec2<f32>'],
    'float32x3': [4, 'float', 3, 12, 'vec3<f32>'],
    'float32x4': [4, 'float', 4, 16, 'vec4<f32>'],
    'uint32': [4, 'uint', 1, 4, 'u32'],
    'uint32x2': [4, 'uint', 2, 8, 'vec2<u32>'],
    'uint32x3': [4, 'uint', 3, 12, 'vec3<u32>'],
    'uint32x4': [4, 'uint', 4, 16, 'vec4<u32>'],
    'sint32': [4, 'sint', 1, 4, 'i32'],
    'sint32x2': [4, 'sint', 2, 8, 'vec2<i32>'],
    'sint32x3': [4, 'sint', 3, 12, 'vec3<i32>'],
    'sint32x4': [4, 'sint', 4, 16, 'vec4<i32>'],
    // 32 bit packed
    'unorm10-10-10-2': ['packed', 'unorm', 4, 4, 'vec4<f32>'],
    'unorm8x4-bgra': ['packed', 'unorm', 4, 4, 'vec4<f32>'],
    'snorm10-10-10-2': ['packed', 'snorm', 4, 4, 'vec4<f32>']
  });
/** List of all GPUVertexFormat values. */
const kVertexFormats = keysOf(kVertexFormatInfo);

// Typedefs for bindings

/**
 * Classes of `PerShaderStage` binding limits. Two bindings with the same class
 * count toward the same `PerShaderStage` limit(s) in the spec (if any).
 */








/**
 * Classes of `PerPipelineLayout` binding limits. Two bindings with the same class
 * count toward the same `PerPipelineLayout` limit(s) in the spec (if any).
 */















/**
 * Types of resource binding which have distinct binding rules, by spec
 * (e.g. filtering vs non-filtering sampler, multisample vs non-multisample texture).
 */

const kBindableResources = [
'uniformBuf',
'storageBuf',
'filtSamp',
'nonFiltSamp',
'compareSamp',
'sampledTex',
'sampledTexMS',
'readonlyStorageTex',
'writeonlyStorageTex',
'readwriteStorageTex',
'errorBuf',
'errorSamp',
'errorTex'];

assertTypeTrue();

// Bindings

/** Dynamic buffer offsets require offset to be divisible by 256, by spec. */
const kMinDynamicBufferOffsetAlignment = 256;

/** Default `PerShaderStage` binding limits, by spec. */
const kPerStageBindingLimits =








{
  'uniformBuf': { class: 'uniformBuf', maxLimits: { COMPUTE: 'maxUniformBuffersPerShaderStage', FRAGMENT: 'maxUniformBuffersPerShaderStage', VERTEX: 'maxUniformBuffersPerShaderStage' } },
  'storageBuf': { class: 'storageBuf', maxLimits: { COMPUTE: 'maxStorageBuffersPerShaderStage', FRAGMENT: 'maxStorageBuffersInFragmentStage', VERTEX: 'maxStorageBuffersInVertexStage' } },
  'sampler': { class: 'sampler', maxLimits: { COMPUTE: 'maxSamplersPerShaderStage', FRAGMENT: 'maxSamplersPerShaderStage', VERTEX: 'maxSamplersPerShaderStage' } },
  'sampledTex': { class: 'sampledTex', maxLimits: { COMPUTE: 'maxSampledTexturesPerShaderStage', FRAGMENT: 'maxSampledTexturesPerShaderStage', VERTEX: 'maxSampledTexturesPerShaderStage' } },
  'readonlyStorageTex': { class: 'readonlyStorageTex', maxLimits: { COMPUTE: 'maxStorageTexturesPerShaderStage', FRAGMENT: 'maxStorageTexturesInFragmentStage', VERTEX: 'maxStorageTexturesInVertexStage' } },
  'writeonlyStorageTex': { class: 'writeonlyStorageTex', maxLimits: { COMPUTE: 'maxStorageTexturesPerShaderStage', FRAGMENT: 'maxStorageTexturesInFragmentStage', VERTEX: 'maxStorageTexturesInVertexStage' } },
  'readwriteStorageTex': { class: 'readwriteStorageTex', maxLimits: { COMPUTE: 'maxStorageTexturesPerShaderStage', FRAGMENT: 'maxStorageTexturesInFragmentStage', VERTEX: 'maxStorageTexturesInVertexStage' } }
};

/**
 * Default `PerPipelineLayout` binding limits, by spec.
 */
const kPerPipelineBindingLimits =










{
  'uniformBuf': { class: 'uniformBuf', maxDynamicLimit: 'maxDynamicUniformBuffersPerPipelineLayout' },
  'storageBuf': { class: 'storageBuf', maxDynamicLimit: 'maxDynamicStorageBuffersPerPipelineLayout' },
  'sampler': { class: 'sampler', maxDynamicLimit: '' },
  'sampledTex': { class: 'sampledTex', maxDynamicLimit: '' },
  'readonlyStorageTex': { class: 'readonlyStorageTex', maxDynamicLimit: '' },
  'writeonlyStorageTex': { class: 'writeonlyStorageTex', maxDynamicLimit: '' },
  'readwriteStorageTex': { class: 'readwriteStorageTex', maxDynamicLimit: '' }
};








const kBindingKind =


{
  uniformBuf: { resource: 'uniformBuf', perStageLimitClass: kPerStageBindingLimits.uniformBuf, perPipelineLimitClass: kPerPipelineBindingLimits.uniformBuf },
  storageBuf: { resource: 'storageBuf', perStageLimitClass: kPerStageBindingLimits.storageBuf, perPipelineLimitClass: kPerPipelineBindingLimits.storageBuf },
  filtSamp: { resource: 'filtSamp', perStageLimitClass: kPerStageBindingLimits.sampler, perPipelineLimitClass: kPerPipelineBindingLimits.sampler },
  nonFiltSamp: { resource: 'nonFiltSamp', perStageLimitClass: kPerStageBindingLimits.sampler, perPipelineLimitClass: kPerPipelineBindingLimits.sampler },
  compareSamp: { resource: 'compareSamp', perStageLimitClass: kPerStageBindingLimits.sampler, perPipelineLimitClass: kPerPipelineBindingLimits.sampler },
  sampledTex: { resource: 'sampledTex', perStageLimitClass: kPerStageBindingLimits.sampledTex, perPipelineLimitClass: kPerPipelineBindingLimits.sampledTex },
  sampledTexMS: { resource: 'sampledTexMS', perStageLimitClass: kPerStageBindingLimits.sampledTex, perPipelineLimitClass: kPerPipelineBindingLimits.sampledTex },
  readonlyStorageTex: { resource: 'readonlyStorageTex', perStageLimitClass: kPerStageBindingLimits.readonlyStorageTex, perPipelineLimitClass: kPerPipelineBindingLimits.readonlyStorageTex },
  writeonlyStorageTex: { resource: 'writeonlyStorageTex', perStageLimitClass: kPerStageBindingLimits.writeonlyStorageTex, perPipelineLimitClass: kPerPipelineBindingLimits.writeonlyStorageTex },
  readwriteStorageTex: { resource: 'readwriteStorageTex', perStageLimitClass: kPerStageBindingLimits.readwriteStorageTex, perPipelineLimitClass: kPerPipelineBindingLimits.readwriteStorageTex }
};

// Binding type info

const kValidStagesAll = {
  validStages:
  GPUConst.ShaderStage.VERTEX | GPUConst.ShaderStage.FRAGMENT | GPUConst.ShaderStage.COMPUTE
};
const kValidStagesStorageWrite = {
  validStages: GPUConst.ShaderStage.FRAGMENT | GPUConst.ShaderStage.COMPUTE
};

/** Binding type info (including class limits) for the specified GPUBufferBindingLayout. */
function bufferBindingTypeInfo(d) {

  switch (d.type ?? 'uniform') {
    case 'uniform':return { usage: GPUConst.BufferUsage.UNIFORM, ...kBindingKind.uniformBuf, ...kValidStagesAll };
    case 'storage':return { usage: GPUConst.BufferUsage.STORAGE, ...kBindingKind.storageBuf, ...kValidStagesStorageWrite };
    case 'read-only-storage':return { usage: GPUConst.BufferUsage.STORAGE, ...kBindingKind.storageBuf, ...kValidStagesAll };
  }
}
/** List of all GPUBufferBindingType values. */
const kBufferBindingTypes = ['uniform', 'storage', 'read-only-storage'];
assertTypeTrue();

/** Binding type info (including class limits) for the specified GPUSamplerBindingLayout. */
function samplerBindingTypeInfo(d) {

  switch (d.type ?? 'filtering') {
    case 'filtering':return { ...kBindingKind.filtSamp, ...kValidStagesAll };
    case 'non-filtering':return { ...kBindingKind.nonFiltSamp, ...kValidStagesAll };
    case 'comparison':return { ...kBindingKind.compareSamp, ...kValidStagesAll };
  }
}
/** List of all GPUSamplerBindingType values. */
const kSamplerBindingTypes = ['filtering', 'non-filtering', 'comparison'];
assertTypeTrue();

/** Binding type info (including class limits) for the specified GPUTextureBindingLayout. */
function sampledTextureBindingTypeInfo(d) {

  if (d.multisampled) {
    return { usage: GPUConst.TextureUsage.TEXTURE_BINDING, ...kBindingKind.sampledTexMS, ...kValidStagesAll };
  } else {
    return { usage: GPUConst.TextureUsage.TEXTURE_BINDING, ...kBindingKind.sampledTex, ...kValidStagesAll };
  }
}
/** List of all GPUTextureSampleType values. */
const kTextureSampleTypes = [
'float',
'unfilterable-float',
'depth',
'sint',
'uint'];

assertTypeTrue();

/** Binding type info (including class limits) for the specified GPUStorageTextureAccess. */
function storageTextureBindingTypeInfo(d) {
  switch (d.access) {
    case undefined:
    case 'write-only':
      return {
        wgslAccess: 'write',
        usage: GPUConst.TextureUsage.STORAGE_BINDING,
        ...kBindingKind.writeonlyStorageTex,
        ...kValidStagesStorageWrite
      };
    case 'read-only':
      return {
        wgslAccess: 'read',
        usage: GPUConst.TextureUsage.STORAGE_BINDING,
        ...kBindingKind.readonlyStorageTex,
        ...kValidStagesAll
      };
    case 'read-write':
      return {
        wgslAccess: 'read_write',
        usage: GPUConst.TextureUsage.STORAGE_BINDING,
        ...kBindingKind.readwriteStorageTex,
        ...kValidStagesStorageWrite
      };
  }
}
/** List of all GPUStorageTextureAccess values. */
const kStorageTextureAccessValues = ['read-only', 'read-write', 'write-only'];
assertTypeTrue();

/** GPUBindGroupLayoutEntry, but only the "union" fields, not the common fields. */

/** Binding type info (including class limits) for the specified BGLEntry. */
function texBindingTypeInfo(e) {
  if (e.texture !== undefined) return sampledTextureBindingTypeInfo(e.texture);
  if (e.storageTexture !== undefined) return storageTextureBindingTypeInfo(e.storageTexture);
  unreachable();
}
/** BindingTypeInfo (including class limits) for the specified BGLEntry. */
function bindingTypeInfo(e) {
  if (e.buffer !== undefined) return bufferBindingTypeInfo(e.buffer);
  if (e.texture !== undefined) return sampledTextureBindingTypeInfo(e.texture);
  if (e.sampler !== undefined) return samplerBindingTypeInfo(e.sampler);
  if (e.storageTexture !== undefined) return storageTextureBindingTypeInfo(e.storageTexture);
  unreachable('GPUBindGroupLayoutEntry has no BindingLayout');
}

/**
 * Generate a list of possible buffer-typed BGLEntry values.
 *
 * Note: Generates different `type` options, but not `hasDynamicOffset` options.
 */
function bufferBindingEntries(includeUndefined) {
  return [
  ...(includeUndefined ? [{ buffer: { type: undefined } }] : []),
  { buffer: { type: 'uniform' } },
  { buffer: { type: 'storage' } },
  { buffer: { type: 'read-only-storage' } }];

}
/** Generate a list of possible sampler-typed BGLEntry values. */
function samplerBindingEntries(includeUndefined) {
  return [
  ...(includeUndefined ? [{ sampler: { type: undefined } }] : []),
  { sampler: { type: 'comparison' } },
  { sampler: { type: 'filtering' } },
  { sampler: { type: 'non-filtering' } }];

}
/**
 * Generate a list of possible texture-typed BGLEntry values.
 *
 * Note: Generates different `multisampled` options, but not `sampleType` or `viewDimension` options.
 */
function textureBindingEntries(includeUndefined) {
  return [
  ...(includeUndefined ?
  [{ texture: { multisampled: undefined, sampleType: 'unfilterable-float' } }] :
  []),
  { texture: { multisampled: false, sampleType: 'unfilterable-float' } },
  { texture: { multisampled: true, sampleType: 'unfilterable-float' } }];

}
/**
 * Generate a list of possible storageTexture-typed BGLEntry values.
 *
 * Note: Generates different `access` options, but not `format` or `viewDimension` options.
 */
function storageTextureBindingEntries(format) {
  return [
  { storageTexture: { access: 'write-only', format } },
  { storageTexture: { access: 'read-only', format } },
  { storageTexture: { access: 'read-write', format } }];

}
/** Generate a list of possible texture-or-storageTexture-typed BGLEntry values. */
function sampledAndStorageBindingEntries(
includeUndefined,
format = 'r32float')
{
  return [
  ...textureBindingEntries(includeUndefined),
  ...storageTextureBindingEntries(format)];

}
/**
 * Generate a list of possible BGLEntry values of every type, but not variants with different:
 * - buffer.hasDynamicOffset
 * - texture.sampleType
 * - texture.viewDimension
 * - storageTexture.viewDimension
 */
function allBindingEntries(
includeUndefined,
format = 'r32float')
{
  return [
  ...bufferBindingEntries(includeUndefined),
  ...samplerBindingEntries(includeUndefined),
  ...sampledAndStorageBindingEntries(includeUndefined, format)];

}

// Shader stages

/** List of all GPUShaderStage values. */

const kShaderStageKeys = Object.keys(GPUConst.ShaderStage);
const kShaderStages = [
GPUConst.ShaderStage.VERTEX,
GPUConst.ShaderStage.FRAGMENT,
GPUConst.ShaderStage.COMPUTE];

/** List of all possible combinations of GPUShaderStage values. */
const kShaderStageCombinations = [0, 1, 2, 3, 4, 5, 6, 7];
const kShaderStageCombinationsWithStage = [
1, 2, 3, 4, 5, 6, 7];


/**
 * List of all possible texture sampleCount values.
 *
 * MAINTENANCE_TODO: Switch existing tests to use kTextureSampleCounts
 */
const kTextureSampleCounts = [1, 4];

// Sampler info

/** List of all mipmap filter modes. */
const kMipmapFilterModes = ['nearest', 'linear'];
assertTypeTrue();

/** List of address modes. */
const kAddressModes = [
'clamp-to-edge',
'repeat',
'mirror-repeat'];

assertTypeTrue();

// Blend factors and Blend components

/** List of all GPUBlendFactor values. */
const kBlendFactors = [
'zero',
'one',
'src',
'one-minus-src',
'src-alpha',
'one-minus-src-alpha',
'dst',
'one-minus-dst',
'dst-alpha',
'one-minus-dst-alpha',
'src-alpha-saturated',
'constant',
'one-minus-constant',
'src1',
'one-minus-src1',
'src1-alpha',
'one-minus-src1-alpha'];


/** Check if `blendFactor` belongs to the blend factors in the extension "dual-source-blending". */
function IsDualSourceBlendingFactor(blendFactor) {
  switch (blendFactor) {
    case 'src1':
    case 'one-minus-src1':
    case 'src1-alpha':
    case 'one-minus-src1-alpha':
      return true;
    default:
      return false;
  }
}

/** List of all GPUBlendOperation values. */
const kBlendOperations = [
'add',
'subtract',
'reverse-subtract',
'min',
'max'];


// Primitive topologies
const kPrimitiveTopology = [
'point-list',
'line-list',
'line-strip',
'triangle-list',
'triangle-strip'];

assertTypeTrue();

const kIndexFormat = ['uint16', 'uint32'];
assertTypeTrue();

/** Info for each entry of GPUSupportedLimits */
const [kLimitInfoKeys, kLimitInfoDefaults, kLimitInfoData] =
[
['class', 'core', 'compatibility', 'maximumValue'],
['maximum',,, kMaxUnsignedLongValue], {
  'maxTextureDimension1D': [, 8192, 4096],
  'maxTextureDimension2D': [, 8192, 4096],
  'maxTextureDimension3D': [, 2048, 2048],
  'maxTextureArrayLayers': [, 256, 256],

  'maxBindGroups': [, 4, 4],
  'maxBindGroupsPlusVertexBuffers': [, 24, 24],
  'maxBindingsPerBindGroup': [, 1000, 1000],
  'maxDynamicUniformBuffersPerPipelineLayout': [, 8, 8],
  'maxDynamicStorageBuffersPerPipelineLayout': [, 4, 4],
  'maxSampledTexturesPerShaderStage': [, 16, 16],
  'maxSamplersPerShaderStage': [, 16, 16],
  'maxStorageBuffersInFragmentStage': [, 8, 4],
  'maxStorageBuffersInVertexStage': [, 8, 0],
  'maxStorageBuffersPerShaderStage': [, 8, 8],
  'maxStorageTexturesInFragmentStage': [, 4, 4],
  'maxStorageTexturesInVertexStage': [, 4, 0],
  'maxStorageTexturesPerShaderStage': [, 4, 4],
  'maxUniformBuffersPerShaderStage': [, 12, 12],

  'maxUniformBufferBindingSize': [, 65536, 16384, kMaxUnsignedLongLongValue],
  'maxStorageBufferBindingSize': [, 134217728, 134217728, kMaxUnsignedLongLongValue],
  'minUniformBufferOffsetAlignment': ['alignment', 256, 256],
  'minStorageBufferOffsetAlignment': ['alignment', 256, 256],

  'maxVertexBuffers': [, 8, 8],
  'maxBufferSize': [, 268435456, 268435456, kMaxUnsignedLongLongValue],
  'maxVertexAttributes': [, 16, 16],
  'maxVertexBufferArrayStride': [, 2048, 2048],
  'maxInterStageShaderVariables': [, 16, 15],

  'maxColorAttachments': [, 8, 4],
  'maxColorAttachmentBytesPerSample': [, 32, 32],

  'maxComputeWorkgroupStorageSize': [, 16384, 16384],
  'maxComputeInvocationsPerWorkgroup': [, 256, 128],
  'maxComputeWorkgroupSizeX': [, 256, 128],
  'maxComputeWorkgroupSizeY': [, 256, 128],
  'maxComputeWorkgroupSizeZ': [, 64, 64],
  'maxComputeWorkgroupsPerDimension': [, 65535, 65535]
  // MAINTENANCE_TODO(4535): Consider allowing optional non-conforming limits. Currently they are not allowed.
  // Any limit here is immediately required by all implementations.
  // Also, consider having this table statically check that all limits listed in @webgpu/types exist in
  // this table.
}];

/**
 * Feature levels corresponding to core WebGPU and WebGPU
 * in compatibility mode. They can be passed to
 * getDefaultLimits though if you have access to an adapter
 * it's preferred to use getDefaultLimits or getDefaultLimitsForCTS
 */
const kFeatureLevels = ['core', 'compatibility'];


const kLimitKeys = ['class', 'default', 'maximumValue'];

const kLimitInfoCore = makeTableRenameAndFilter(
  { default: 'core' },
  kLimitKeys,
  kLimitInfoKeys,
  kLimitInfoDefaults,
  kLimitInfoData
);

const kLimitInfoCompatibility = makeTableRenameAndFilter(
  { default: 'compatibility' },
  kLimitKeys,
  kLimitInfoKeys,
  kLimitInfoDefaults,
  kLimitInfoData
);

const kLimitInfos = {
  core: kLimitInfoCore,
  compatibility: kLimitInfoCompatibility
};

const kLimitClasses = Object.fromEntries(
  Object.entries(kLimitInfoCore).map(([k, { class: c }]) => [k, c])
);

function getDefaultLimits(featureLevel) {
  return kLimitInfos[featureLevel];
}

/**
 * The CTS is generally designed to run in a single feature level.
 * Use this function get the default limits for the CTS's feature level
 * This is needed if you can not use the device limits as you have not yet
 * created a device. An adapter can not tell you if it supports compatibility
 * mode. The only way to know is to request a device without `core-features-and-limits`.
 * If the device you get back doesn't have `core-features-and-limits` then it's
 * a compatibility device.
 */
function getDefaultLimitsForCTS() {
  return getDefaultLimits(globalTestConfig.compatibility ? 'compatibility' : 'core');
}

function getDefaultLimitsForDevice(device) {
  const featureLevel = hasFeature(device.features, 'core-features-and-limits') ?
  'core' :
  'compatibility';
  return getDefaultLimits(featureLevel);
}

const kEachStage = [
GPUConst.ShaderStage.COMPUTE,
GPUConst.ShaderStage.FRAGMENT,
GPUConst.ShaderStage.VERTEX];

function shaderStageFlagToStageName(stage) {
  switch (stage) {
    case GPUConst.ShaderStage.COMPUTE:
      return 'COMPUTE';
    case GPUConst.ShaderStage.FRAGMENT:
      return 'FRAGMENT';
    case GPUConst.ShaderStage.VERTEX:
      return 'VERTEX';
    default:
      unreachable();
  }
}

/**
 * Get the limit of the number of things you can bind for
 * a given BGLEntry given the specified visibility. This is
 * the minimum across stages for the given visibility.
 */
function getBindingLimitForBindingType(
device,
visibility,
e)
{
  const info = bindingTypeInfo(e);
  const maxLimits = info.perStageLimitClass.maxLimits;
  const limits = kEachStage.
  filter((stage) => stage & visibility).
  map((stage) => device.limits[maxLimits[shaderStageFlagToStageName(stage)]]);
  return limits.length > 0 ? Math.min(...limits) : 0;
}

/** List of all possible entries of GPUSupportedLimits. */
const kPossibleLimits = keysOf(kLimitInfoCore);

/**
 * The number of color attachments to test.
 * The CTS needs to generate a consistent list of tests.
 * We can't use any default limits since they different from core to compat mode
 * So, tests should use this value and filter out any values that are out of
 * range for the current device.
 *
 * The test in maxColorAttachments.spec.ts tests that kMaxColorAttachmentsToTest
 * is large enough to cover all devices tested.
 */
const kMaxColorAttachmentsToTest = 32;

/** The size of indirect draw parameters in the indirectBuffer of drawIndirect */
const kDrawIndirectParametersSize = 4;
/** The size of indirect drawIndexed parameters in the indirectBuffer of drawIndexedIndirect */
const kDrawIndexedIndirectParametersSize = 5;

/** Per-GPUFeatureName info. */
const kFeatureNameInfo =


{
  'bgra8unorm-storage': {},
  'depth-clip-control': {},
  'depth32float-stencil8': {},
  'texture-compression-bc': {},
  'texture-compression-bc-sliced-3d': {},
  'texture-compression-etc2': {},
  'texture-compression-astc': {},
  'texture-compression-astc-sliced-3d': {},
  'timestamp-query': {},
  'indirect-first-instance': {},
  'shader-f16': {},
  'rg11b10ufloat-renderable': {},
  'float32-filterable': {},
  'float32-blendable': {},
  'clip-distances': {},
  'dual-source-blending': {},
  'subgroups': {},
  'core-features-and-limits': {},
  'texture-formats-tier1': {},
  'texture-formats-tier2': {},
  'primitive-index': {},
  'texture-component-swizzle': {},
  'subgroup-size-control': {},
  'atomic-vec2u-min-max': {},
  'texture-compression-unaligned': {}
};
/** List of all GPUFeatureName values. */
const kFeatureNames = keysOf(kFeatureNameInfo);

/** List of all known WGSL language features */
const kKnownWGSLLanguageFeatures = [
'readonly_and_readwrite_storage_textures',
'packed_4x8_integer_dot_product',
'unrestricted_pointer_parameters',
'pointer_composite_access',
'uniform_buffer_standard_layout',
'texture_and_sampler_let',
'subgroup_id',
'subgroup_uniformity',
'swizzle_assignment',
'linear_indexing',
'texture_formats_tier1',
'immediate_address_space',
'fragment_depth',
'buffer_view',
'unrestricted_aliasing'];
});
__define('common/util/types.js', function (__exports) {
'use strict';
__get(__exports, 'assertTypeTrue', () => assertTypeTrue);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /** Forces a type to resolve its type definitions, to make it readable/debuggable. */




/** Returns the type `true` iff X and Y are exactly equal */





function assertTypeTrue() {}

/** `ReadonlyArray` of `ReadonlyArray`s. */

/** `ReadonlyArray` of `ReadonlyArray`s of `ReadonlyArray`s. */


/**
 * Deep version of the Readonly<> type, with support for tuples (up to length 7).
 * <https://gist.github.com/masterkidan/7322752f569b1bba53e0426266768623>
 */






























/**
 * Computes the intersection of a set of types, given the union of those types.
 *
 * From: https://stackoverflow.com/a/56375136
 */




/** "Type asserts" that `X` is a subtype of `Y`. */






/**
 * Zips a key tuple type and a value tuple type together into an object.
 *
 * @template Keys Keys of the resulting object.
 * @template Values Values of the resulting object. If a key corresponds to a `Values` member that
 *   is undefined or past the end, it defaults to the corresponding `Defaults` member.
 * @template Defaults Default values. If a key corresponds to a `Defaults` member that is past the
 *   end, the default falls back to `undefined`.
 */

















// K exhausted
});
__define('webgpu/constants.js', function (__exports) {
'use strict';
__get(__exports, 'GPUConst', () => GPUConst);
__get(__exports, 'kMaxUnsignedLongValue', () => kMaxUnsignedLongValue);
__get(__exports, 'kMaxUnsignedLongLongValue', () => kMaxUnsignedLongLongValue);
__get(__exports, 'kInterpolationSampling', () => kInterpolationSampling);
__get(__exports, 'kInterpolationType', () => kInterpolationType);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ // Note: Types ensure every field is specified.
function checkType(_) {}

const BufferUsage = {
  MAP_READ: 0x0001,
  MAP_WRITE: 0x0002,
  COPY_SRC: 0x0004,
  COPY_DST: 0x0008,
  INDEX: 0x0010,
  VERTEX: 0x0020,
  UNIFORM: 0x0040,
  STORAGE: 0x0080,
  INDIRECT: 0x0100,
  QUERY_RESOLVE: 0x0200
};
checkType(BufferUsage);








const TextureUsage = {
  COPY_SRC: 0x01,
  COPY_DST: 0x02,
  TEXTURE_BINDING: 0x04,
  STORAGE_BINDING: 0x08,
  RENDER_ATTACHMENT: 0x10,
  TRANSIENT_ATTACHMENT: 0x20
};
checkType(TextureUsage);

const ColorWrite = {
  RED: 0x1,
  GREEN: 0x2,
  BLUE: 0x4,
  ALPHA: 0x8,
  ALL: 0xf
};
checkType(ColorWrite);

const ShaderStage = {
  VERTEX: 0x1,
  FRAGMENT: 0x2,
  COMPUTE: 0x4
};
checkType(ShaderStage);

const MapMode = {
  READ: 0x1,
  WRITE: 0x2
};
checkType(MapMode);

const GPUConst = {
  BufferUsage,
  TextureUsage,
  ColorWrite,
  ShaderStage,
  MapMode
};

const kMaxUnsignedLongValue = 4294967295;
const kMaxUnsignedLongLongValue = Number.MAX_SAFE_INTEGER;

const kInterpolationSampling = ['center', 'centroid', 'sample', 'first', 'either'];
const kInterpolationType = ['perspective', 'linear', 'flat'];
});
__define('webgpu/util/math.js', function (__exports) {
'use strict';
__get(__exports, 'kMaxSafeMultipleOf8', () => kMaxSafeMultipleOf8);
__get(__exports, 'align', () => align);
__get(__exports, 'roundDown', () => roundDown);
__get(__exports, 'clamp', () => clamp);
__get(__exports, 'flushSubnormalNumberF64', () => flushSubnormalNumberF64);
__get(__exports, 'isSubnormalNumberF64', () => isSubnormalNumberF64);
__get(__exports, 'flushSubnormalNumberF32', () => flushSubnormalNumberF32);
__get(__exports, 'isSubnormalNumberF32', () => isSubnormalNumberF32);
__get(__exports, 'isFiniteF32', () => isFiniteF32);
__get(__exports, 'flushSubnormalNumberF16', () => flushSubnormalNumberF16);
__get(__exports, 'isSubnormalNumberF16', () => isSubnormalNumberF16);
__get(__exports, 'isFiniteF16', () => isFiniteF16);
__get(__exports, 'nextAfterF64', () => nextAfterF64);
__get(__exports, 'nextAfterF32', () => nextAfterF32);
__get(__exports, 'nextAfterF16', () => nextAfterF16);
__get(__exports, 'oneULPF64', () => oneULPF64);
__get(__exports, 'oneULPF32', () => oneULPF32);
__get(__exports, 'hashU32', () => hashU32);
__get(__exports, 'oneULPF16', () => oneULPF16);
__get(__exports, 'correctlyRoundedF64', () => correctlyRoundedF64);
__get(__exports, 'correctlyRoundedF32', () => correctlyRoundedF32);
__get(__exports, 'correctlyRoundedF16', () => correctlyRoundedF16);
__get(__exports, 'frexp', () => frexp);
__get(__exports, 'lerp', () => lerp);
__get(__exports, 'lerpBigInt', () => lerpBigInt);
__get(__exports, 'linearRange', () => linearRange);
__get(__exports, 'linearRangeBigInt', () => linearRangeBigInt);
__get(__exports, 'biasedRange', () => biasedRange);
__get(__exports, 'biasedRangeBigInt', () => biasedRangeBigInt);
__get(__exports, 'scalarF32Range', () => scalarF32Range);
__get(__exports, 'filteredScalarF32Range', () => filteredScalarF32Range);
__get(__exports, 'scalarF16Range', () => scalarF16Range);
__get(__exports, 'scalarF64Range', () => scalarF64Range);
__get(__exports, 'limitedScalarF64Range', () => limitedScalarF64Range);
__get(__exports, 'sparseI32Range', () => sparseI32Range);
__get(__exports, 'vectorI32Range', () => vectorI32Range);
__get(__exports, 'sparseVectorI32Range', () => sparseVectorI32Range);
__get(__exports, 'fullI32Range', () => fullI32Range);
__get(__exports, 'sparseU32Range', () => sparseU32Range);
__get(__exports, 'vectorU32Range', () => vectorU32Range);
__get(__exports, 'sparseVectorU32Range', () => sparseVectorU32Range);
__get(__exports, 'fullU32Range', () => fullU32Range);
__get(__exports, 'sparseI64Range', () => sparseI64Range);
__get(__exports, 'vectorI64Range', () => vectorI64Range);
__get(__exports, 'sparseVectorI64Range', () => sparseVectorI64Range);
__get(__exports, 'fullI64Range', () => fullI64Range);
__get(__exports, 'sparseScalarF32Range', () => sparseScalarF32Range);
__get(__exports, 'vectorF32Range', () => vectorF32Range);
__get(__exports, 'sparseVectorF32Range', () => sparseVectorF32Range);
__get(__exports, 'sparseMatrixF32Range', () => sparseMatrixF32Range);
__get(__exports, 'sparseScalarF16Range', () => sparseScalarF16Range);
__get(__exports, 'vectorF16Range', () => vectorF16Range);
__get(__exports, 'sparseVectorF16Range', () => sparseVectorF16Range);
__get(__exports, 'sparseMatrixF16Range', () => sparseMatrixF16Range);
__get(__exports, 'sparseScalarF64Range', () => sparseScalarF64Range);
__get(__exports, 'vectorF64Range', () => vectorF64Range);
__get(__exports, 'sparseVectorF64Range', () => sparseVectorF64Range);
__get(__exports, 'sparseMatrixF64Range', () => sparseMatrixF64Range);
__get(__exports, 'multiplyMatrices', () => multiplyMatrices);
__get(__exports, 'signExtend', () => signExtend);
__get(__exports, 'quantizeToF32', () => quantizeToF32);
__get(__exports, 'quantizeToF16', () => quantizeToF16);
__get(__exports, 'quantizeToI32', () => quantizeToI32);
__get(__exports, 'quantizeToU32', () => quantizeToU32);
__get(__exports, 'quantizeToI64', () => quantizeToI64);
__get(__exports, 'isPowerOfTwo', () => isPowerOfTwo);
__get(__exports, 'gcd', () => gcd);
__get(__exports, 'lcm', () => lcm);
__get(__exports, 'cartesianProduct', () => cartesianProduct);
__get(__exports, 'calculatePermutations', () => calculatePermutations);
__get(__exports, 'flatten2DArray', () => flatten2DArray);
__get(__exports, 'unflatten2DArray', () => unflatten2DArray);
__get(__exports, 'map2DArray', () => map2DArray);
__get(__exports, 'every2DArray', () => every2DArray);
__get(__exports, 'subtractVectors', () => subtractVectors);
__get(__exports, 'dotProduct', () => dotProduct);
__get(__exports, 'absBigInt', () => absBigInt);
__get(__exports, 'maxBigInt', () => maxBigInt);
__get(__exports, 'minBigInt', () => minBigInt);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });let Float16Array, getFloat16, hfround, setFloat16; __bind('external/petamoriken/float16/float16.js', (__m) => { ({ Float16Array, getFloat16, hfround, setFloat16 } = __m); });

let kBit, kValue; __bind('webgpu/util/constants.js', (__m) => { ({ kBit, kValue } = __m); });
let reinterpretF64AsU64, reinterpretU64AsF64, reinterpretU32AsF32, reinterpretU16AsF16; __bind('webgpu/util/reinterpret.js', (__m) => { ({ reinterpretF64AsU64, reinterpretU64AsF64, reinterpretU32AsF32, reinterpretU16AsF16 } = __m); });

/**
 * A multiple of 8 guaranteed to be way too large to allocate (just under 8 pebibytes).
 * This is a "safe" integer (ULP <= 1.0) very close to MAX_SAFE_INTEGER.
 *
 * Note: allocations of this size are likely to exceed limitations other than just the system's
 * physical memory, so test cases are also needed to try to trigger "true" OOM.
 */
const kMaxSafeMultipleOf8 = Number.MAX_SAFE_INTEGER - 7;

/** Round `n` up to the next multiple of `alignment` (inclusive). */
// MAINTENANCE_TODO: Rename to `roundUp`
function align(n, alignment) {
  assert(Number.isInteger(n) && n >= 0, 'n must be a non-negative integer');
  assert(Number.isInteger(alignment) && alignment > 0, 'alignment must be a positive integer');
  return Math.ceil(n / alignment) * alignment;
}

/** Round `n` down to the next multiple of `alignment` (inclusive). */
function roundDown(n, alignment) {
  assert(Number.isInteger(n) && n >= 0, 'n must be a non-negative integer');
  assert(Number.isInteger(alignment) && alignment > 0, 'alignment must be a positive integer');
  return Math.floor(n / alignment) * alignment;
}

/** Clamp a number to the provided range. */
function clamp(n, { min, max }) {
  assert(max >= min);
  return Math.min(Math.max(n, min), max);
}

/** @returns 0 if |val| is a subnormal f64 number, otherwise returns |val| */
function flushSubnormalNumberF64(val) {
  return isSubnormalNumberF64(val) ? 0 : val;
}

/** @returns if number is within subnormal range of f64 */
function isSubnormalNumberF64(n) {
  return n > kValue.f64.negative.max && n < kValue.f64.positive.min;
}

/** @returns 0 if |val| is a subnormal f32 number, otherwise returns |val| */
function flushSubnormalNumberF32(val) {
  return isSubnormalNumberF32(val) ? 0 : val;
}

/** @returns if number is within subnormal range of f32 */
function isSubnormalNumberF32(n) {
  return n > kValue.f32.negative.max && n < kValue.f32.positive.min;
}

/** @returns if number is in the finite range of f32 */
function isFiniteF32(n) {
  return n >= kValue.f32.negative.min && n <= kValue.f32.positive.max;
}

/** @returns 0 if |val| is a subnormal f16 number, otherwise returns |val| */
function flushSubnormalNumberF16(val) {
  return isSubnormalNumberF16(val) ? 0 : val;
}

/** @returns if number is within subnormal range of f16 */
function isSubnormalNumberF16(n) {
  return n > kValue.f16.negative.max && n < kValue.f16.positive.min;
}

/** @returns if number is in the finite range of f16 */
function isFiniteF16(n) {
  return n >= kValue.f16.negative.min && n <= kValue.f16.positive.max;
}

/** Should FTZ occur during calculations or not */


/** Should nextAfter calculate towards positive infinity or negative infinity */


/**
 * Once-allocated ArrayBuffer/views to avoid overhead of allocation when
 * converting between numeric formats
 *
 * Usage of a once-allocated pattern like this makes nextAfterF64 non-reentrant,
 * so cannot call itself directly or indirectly.
 */
const nextAfterF64Data = new ArrayBuffer(8);
const nextAfterF64Int = new BigUint64Array(nextAfterF64Data);
const nextAfterF64Float = new Float64Array(nextAfterF64Data);

/**
 * @returns the next f64 value after |val|, towards +inf or -inf as specified by |dir|.

 * If |mode| is 'flush', all subnormal values will be flushed to 0,
 * before processing and for -/+0 the nextAfterF64 will be the closest normal in
 * the correct direction.

 * If |mode| is 'no-flush', the next subnormal will be calculated when appropriate,
 * and for -/+0 the nextAfterF64 will be the closest subnormal in the correct
 * direction.
 *
 * val needs to be in [min f64, max f64]
 */
function nextAfterF64(val, dir, mode) {
  if (Number.isNaN(val)) {
    return val;
  }

  if (val === Number.POSITIVE_INFINITY) {
    return kValue.f64.positive.infinity;
  }

  if (val === Number.NEGATIVE_INFINITY) {
    return kValue.f64.negative.infinity;
  }

  assert(
    val <= kValue.f64.positive.max && val >= kValue.f64.negative.min,
    `${val} is not in the range of f64`
  );

  val = mode === 'flush' ? flushSubnormalNumberF64(val) : val;

  // -/+0 === 0 returns true
  if (val === 0) {
    if (dir === 'positive') {
      return mode === 'flush' ? kValue.f64.positive.min : kValue.f64.positive.subnormal.min;
    } else {
      return mode === 'flush' ? kValue.f64.negative.max : kValue.f64.negative.subnormal.max;
    }
  }

  nextAfterF64Float[0] = val;
  const is_positive = (nextAfterF64Int[0] & 0x8000_0000_0000_0000n) === 0n;
  if (is_positive === (dir === 'positive')) {
    nextAfterF64Int[0] += 1n;
  } else {
    nextAfterF64Int[0] -= 1n;
  }

  // Checking for overflow
  if ((nextAfterF64Int[0] & 0x7ff0_0000_0000_0000n) === 0x7ff0_0000_0000_0000n) {
    if (dir === 'positive') {
      return kValue.f64.positive.infinity;
    } else {
      return kValue.f64.negative.infinity;
    }
  }

  return mode === 'flush' ? flushSubnormalNumberF64(nextAfterF64Float[0]) : nextAfterF64Float[0];
}

/**
 * Once-allocated ArrayBuffer/views to avoid overhead of allocation when
 * converting between numeric formats
 *
 * Usage of a once-allocated pattern like this makes nextAfterF32 non-reentrant,
 * so cannot call itself directly or indirectly.
 */
const nextAfterF32Data = new ArrayBuffer(4);
const nextAfterF32Int = new Uint32Array(nextAfterF32Data);
const nextAfterF32Float = new Float32Array(nextAfterF32Data);

/**
 * @returns the next f32 value after |val|, towards +inf or -inf as specified by |dir|.

 * If |mode| is 'flush', all subnormal values will be flushed to 0,
 * before processing and for -/+0 the nextAfterF32 will be the closest normal in
 * the correct direction.

 * If |mode| is 'no-flush', the next subnormal will be calculated when appropriate,
 * and for -/+0 the nextAfterF32 will be the closest subnormal in the correct
 * direction.
 *
 * val needs to be in [min f32, max f32]
 */
function nextAfterF32(val, dir, mode) {
  if (Number.isNaN(val)) {
    return val;
  }

  if (val === Number.POSITIVE_INFINITY) {
    return kValue.f32.positive.infinity;
  }

  if (val === Number.NEGATIVE_INFINITY) {
    return kValue.f32.negative.infinity;
  }

  assert(
    val <= kValue.f32.positive.max && val >= kValue.f32.negative.min,
    `${val} is not in the range of f32`
  );

  val = mode === 'flush' ? flushSubnormalNumberF32(val) : val;

  // -/+0 === 0 returns true
  if (val === 0) {
    if (dir === 'positive') {
      return mode === 'flush' ? kValue.f32.positive.min : kValue.f32.positive.subnormal.min;
    } else {
      return mode === 'flush' ? kValue.f32.negative.max : kValue.f32.negative.subnormal.max;
    }
  }

  nextAfterF32Float[0] = val; // This quantizes from number (f64) to f32
  if (
  dir === 'positive' && nextAfterF32Float[0] <= val ||
  dir === 'negative' && nextAfterF32Float[0] >= val)
  {
    // val is either f32 precise or quantizing rounded in the opposite direction
    // from what is needed, so need to calculate the value in the correct
    // direction.
    const is_positive = (nextAfterF32Int[0] & 0x80000000) === 0;
    if (is_positive === (dir === 'positive')) {
      nextAfterF32Int[0] += 1;
    } else {
      nextAfterF32Int[0] -= 1;
    }
  }

  // Checking for overflow
  if ((nextAfterF32Int[0] & 0x7f800000) === 0x7f800000) {
    if (dir === 'positive') {
      return kValue.f32.positive.infinity;
    } else {
      return kValue.f32.negative.infinity;
    }
  }

  return mode === 'flush' ? flushSubnormalNumberF32(nextAfterF32Float[0]) : nextAfterF32Float[0];
}

/**
 * Once-allocated ArrayBuffer/views to avoid overhead of allocation when
 * converting between numeric formats
 *
 * Usage of a once-allocated pattern like this makes nextAfterF16 non-reentrant,
 * so cannot call itself directly or indirectly.
 */
const nextAfterF16Data = new ArrayBuffer(2);
const nextAfterF16Hex = new Uint16Array(nextAfterF16Data);
const nextAfterF16Float = new Float16Array(nextAfterF16Data);

/**
 * @returns the next f16 value after |val|, towards +inf or -inf as specified by |dir|.

 * If |mode| is 'flush', all subnormal values will be flushed to 0,
 * before processing and for -/+0 the nextAfterF16 will be the closest normal in
 * the correct direction.

 * If |mode| is 'no-flush', the next subnormal will be calculated when appropriate,
 * and for -/+0 the nextAfterF16 will be the closest subnormal in the correct
 * direction.
 *
 * val needs to be in [min f16, max f16]
 */
function nextAfterF16(val, dir, mode) {
  if (Number.isNaN(val)) {
    return val;
  }

  if (val === Number.POSITIVE_INFINITY) {
    return kValue.f16.positive.infinity;
  }

  if (val === Number.NEGATIVE_INFINITY) {
    return kValue.f16.negative.infinity;
  }

  assert(
    val <= kValue.f16.positive.max && val >= kValue.f16.negative.min,
    `${val} is not in the range of f16`
  );

  val = mode === 'flush' ? flushSubnormalNumberF16(val) : val;

  // -/+0 === 0 returns true
  if (val === 0) {
    if (dir === 'positive') {
      return mode === 'flush' ? kValue.f16.positive.min : kValue.f16.positive.subnormal.min;
    } else {
      return mode === 'flush' ? kValue.f16.negative.max : kValue.f16.negative.subnormal.max;
    }
  }

  nextAfterF16Float[0] = val; // This quantizes from number (f64) to f16
  if (
  dir === 'positive' && nextAfterF16Float[0] <= val ||
  dir === 'negative' && nextAfterF16Float[0] >= val)
  {
    // val is either f16 precise or quantizing rounded in the opposite direction
    // from what is needed, so need to calculate the value in the correct
    // direction.
    const is_positive = (nextAfterF16Hex[0] & 0x8000) === 0;
    if (is_positive === (dir === 'positive')) {
      nextAfterF16Hex[0] += 1;
    } else {
      nextAfterF16Hex[0] -= 1;
    }
  }

  // Checking for overflow
  if ((nextAfterF16Hex[0] & 0x7c00) === 0x7c00) {
    if (dir === 'positive') {
      return kValue.f16.positive.infinity;
    } else {
      return kValue.f16.negative.infinity;
    }
  }

  return mode === 'flush' ? flushSubnormalNumberF16(nextAfterF16Float[0]) : nextAfterF16Float[0];
}

/**
 * @returns ulp(x), the unit of least precision for a specific number as a 64-bit float
 *
 * ulp(x) is the distance between the two floating point numbers nearest x.
 * This value is also called unit of last place, ULP, and 1 ULP.
 * See the WGSL spec and http://www.ens-lyon.fr/LIP/Pub/Rapports/RR/RR2005/RR2005-09.pdf
 * for a more detailed/nuanced discussion of the definition of ulp(x).
 *
 * @param target number to calculate ULP for
 * @param mode should FTZ occurring during calculation or not
 */
function oneULPF64(target, mode = 'flush') {
  if (Number.isNaN(target)) {
    return Number.NaN;
  }

  target = mode === 'flush' ? flushSubnormalNumberF64(target) : target;

  // For values out of bounds for f64 ulp(x) is defined as the
  // distance between the two nearest f64 representable numbers to the
  // appropriate edge, which also happens to be the maximum possible ULP.
  if (
  target === Number.POSITIVE_INFINITY ||
  target >= kValue.f64.positive.max ||
  target === Number.NEGATIVE_INFINITY ||
  target <= kValue.f64.negative.min)
  {
    return kValue.f64.max_ulp;
  }

  // ulp(x) is min(after - before), where
  //     before <= x <= after
  //     before =/= after
  //     before and after are f64 representable
  const before = nextAfterF64(target, 'negative', mode);
  const after = nextAfterF64(target, 'positive', mode);
  // Since number is internally a f64, |target| is always f64 representable, so
  // either before or after will be x
  return Math.min(target - before, after - target);
}

/**
 * @returns ulp(x), the unit of least precision for a specific number as a 32-bit float
 *
 * ulp(x) is the distance between the two floating point numbers nearest x.
 * This value is also called unit of last place, ULP, and 1 ULP.
 * See the WGSL spec and http://www.ens-lyon.fr/LIP/Pub/Rapports/RR/RR2005/RR2005-09.pdf
 * for a more detailed/nuanced discussion of the definition of ulp(x).
 *
 * @param target number to calculate ULP for
 * @param mode should FTZ occurring during calculation or not
 */
function oneULPF32(target, mode = 'flush') {
  if (Number.isNaN(target)) {
    return Number.NaN;
  }

  target = mode === 'flush' ? flushSubnormalNumberF32(target) : target;

  // For values out of bounds for f32 ulp(x) is defined as the
  // distance between the two nearest f32 representable numbers to the
  // appropriate edge, which also happens to be the maximum possible ULP.
  if (
  target === Number.POSITIVE_INFINITY ||
  target >= kValue.f32.positive.max ||
  target === Number.NEGATIVE_INFINITY ||
  target <= kValue.f32.negative.min)
  {
    return kValue.f32.max_ulp;
  }

  // ulp(x) is min(after - before), where
  //     before <= x <= after
  //     before =/= after
  //     before and after are f32 representable
  const before = nextAfterF32(target, 'negative', mode);
  const after = nextAfterF32(target, 'positive', mode);
  const converted = quantizeToF32(target);
  if (converted === target) {
    // |target| is f32 representable, so either before or after will be x
    return Math.min(target - before, after - target);
  } else {
    // |target| is not f32 representable so taking distance of neighbouring f32s.
    return after - before;
  }
}

/**
 * @returns an integer value between 0..0xffffffff using a simple non-cryptographic hash function
 * @param values integers to generate hash from.
 */
function hashU32(...values) {
  let n = 0x3504_f333;
  for (const v of values) {
    n = v + (n << 7) + (n >>> 1);
    n = n * 0x29493 & 0xffff_ffff;
  }
  n ^= n >>> 8;
  n += n << 15;
  n = n & 0xffff_ffff;
  if (n < 0) {
    n = ~n * 2 + 1;
  }
  return n;
}

/**
 * @returns ulp(x), the unit of least precision for a specific number as a 32-bit float
 *
 * ulp(x) is the distance between the two floating point numbers nearest x.
 * This value is also called unit of last place, ULP, and 1 ULP.
 * See the WGSL spec and http://www.ens-lyon.fr/LIP/Pub/Rapports/RR/RR2005/RR2005-09.pdf
 * for a more detailed/nuanced discussion of the definition of ulp(x).
 *
 * @param target number to calculate ULP for
 * @param mode should FTZ occurring during calculation or not
 */
function oneULPF16(target, mode = 'flush') {
  if (Number.isNaN(target)) {
    return Number.NaN;
  }

  target = mode === 'flush' ? flushSubnormalNumberF16(target) : target;

  // For values out of bounds for f16 ulp(x) is defined as the
  // distance between the two nearest f16 representable numbers to the
  // appropriate edge, which also happens to be the maximum possible ULP.
  if (
  target === Number.POSITIVE_INFINITY ||
  target >= kValue.f16.positive.max ||
  target === Number.NEGATIVE_INFINITY ||
  target <= kValue.f16.negative.min)
  {
    return kValue.f16.max_ulp;
  }

  // ulp(x) is min(after - before), where
  //     before <= x <= after
  //     before =/= after
  //     before and after are f16 representable
  const before = nextAfterF16(target, 'negative', mode);
  const after = nextAfterF16(target, 'positive', mode);
  const converted = quantizeToF16(target);
  if (converted === target) {
    // |target| is f16 representable, so either before or after will be x
    return Math.min(target - before, after - target);
  } else {
    // |target| is not f16 representable so taking distance of neighbouring f16s.
    return after - before;
  }
}

/**
 * Calculate the valid roundings when quantizing to 64-bit floats
 *
 * TS/JS's number type is internally a f64, so the supplied value will be
 * quanitized by definition. The only corner cases occur if a non-finite value
 * is provided, since the valid roundings include the appropriate min or max
 * value.
 *
 * @param n number to be quantized
 * @returns all of the acceptable roundings for quantizing to 64-bits in
 *          ascending order.
 */
function correctlyRoundedF64(n) {
  assert(!Number.isNaN(n), `correctlyRoundedF32 not defined for NaN`);
  // Above f64 range
  if (n === Number.POSITIVE_INFINITY) {
    return [kValue.f64.positive.max, Number.POSITIVE_INFINITY];
  }

  // Below f64 range
  if (n === Number.NEGATIVE_INFINITY) {
    return [Number.NEGATIVE_INFINITY, kValue.f64.negative.min];
  }

  return [n];
}

/**
 * Calculate the valid roundings when quantizing to 32-bit floats
 *
 * TS/JS's number type is internally a f64, so quantization needs to occur when
 * converting to f32 for WGSL. WGSL does not specify a specific rounding mode,
 * so if a number is not precisely representable in 32-bits, but in the
 * range, there are two possible valid quantizations. If it is precisely
 * representable, there is only one valid quantization. This function calculates
 * the valid roundings and returns them in an array.
 *
 * This function does not consider flushing mode, so subnormals are maintained.
 * The caller is responsible to flushing before and after as appropriate.
 *
 * Out of bounds values need to consider how they interact with the overflow
 * rules.
 *  * If a value is OOB but not too far out, an implementation may choose to round
 * to nearest finite value or the correct infinity. This boundary is at
 * 2^(f32.emax + 1) and -(2^(f32.emax + 1)) respectively.
 * Values that are at or beyond these limits must be rounded towards the
 * appropriate infinity.
 *
 * @param n number to be quantized
 * @returns all of the acceptable roundings for quantizing to 32-bits in
 *          ascending order.
 */
function correctlyRoundedF32(n) {
  if (Number.isNaN(n)) {
    return [n];
  }

  // Greater than or equal to the upper overflow boundry
  if (n >= 2 ** (kValue.f32.emax + 1)) {
    return [Number.POSITIVE_INFINITY];
  }

  // OOB, but less than the upper overflow boundary
  if (n > kValue.f32.positive.max) {
    return [kValue.f32.positive.max, Number.POSITIVE_INFINITY];
  }

  // f32 finite
  if (n <= kValue.f32.positive.max && n >= kValue.f32.negative.min) {
    const n_32 = quantizeToF32(n);
    if (n === n_32) {
      // n is precisely expressible as a f32, so should not be rounded
      return [n];
    }

    if (n_32 > n) {
      // n_32 rounded towards +inf, so is after n
      const other = nextAfterF32(n_32, 'negative', 'no-flush');
      return [other, n_32];
    } else {
      // n_32 rounded towards -inf, so is before n
      const other = nextAfterF32(n_32, 'positive', 'no-flush');
      return [n_32, other];
    }
  }

  // OOB, but greater the lower overflow boundary
  if (n > -(2 ** (kValue.f32.emax + 1))) {
    return [Number.NEGATIVE_INFINITY, kValue.f32.negative.min];
  }

  // Less than or equal to the lower overflow boundary
  return [Number.NEGATIVE_INFINITY];
}

/**
 * Calculate the valid roundings when quantizing to 16-bit floats
 *
 * TS/JS's number type is internally a f64, so quantization needs to occur when
 * converting to f16 for WGSL. WGSL does not specify a specific rounding mode,
 * so if a number is not precisely representable in 16-bits, but in the
 * range, there are two possible valid quantizations. If it is precisely
 * representable, there is only one valid quantization. This function calculates
 * the valid roundings and returns them in an array.
 *
 * This function does not consider flushing mode, so subnormals are maintained.
 * The caller is responsible to flushing before and after as appropriate.
 *
 * Out of bounds values need to consider how they interact with the overflow
 * rules.
 *  * If a value is OOB but not too far out, an implementation may choose to round
 * to nearest finite value or the correct infinity. This boundary is at
 * 2^(f16.emax + 1) and -(2^(f16.emax + 1)) respectively.
 * Values that are at or beyond these limits must be rounded towards the
 * appropriate infinity.
 *
 * @param n number to be quantized
 * @returns all of the acceptable roundings for quantizing to 16-bits in
 *          ascending order.
 */
function correctlyRoundedF16(n) {
  if (Number.isNaN(n)) {
    return [n];
  }

  // Greater than or equal to the upper overflow boundry
  if (n >= 2 ** (kValue.f16.emax + 1)) {
    return [Number.POSITIVE_INFINITY];
  }

  // OOB, but less than the upper overflow boundary
  if (n > kValue.f16.positive.max) {
    return [kValue.f16.positive.max, Number.POSITIVE_INFINITY];
  }

  // f16 finite
  if (n <= kValue.f16.positive.max && n >= kValue.f16.negative.min) {
    const n_16 = quantizeToF16(n);
    if (n === n_16) {
      // n is precisely expressible as a f16, so should not be rounded
      return [n];
    }

    if (n_16 > n) {
      // n_16 rounded towards +inf, so is after n
      const other = nextAfterF16(n_16, 'negative', 'no-flush');
      return [other, n_16];
    } else {
      // n_16 rounded towards -inf, so is before n
      const other = nextAfterF16(n_16, 'positive', 'no-flush');
      return [n_16, other];
    }
  }

  // OOB, but greater the lower overflow boundary
  if (n > -(2 ** (kValue.f16.emax + 1))) {
    return [Number.NEGATIVE_INFINITY, kValue.f16.negative.min];
  }

  // Less than or equal to the lower overflow boundary
  return [Number.NEGATIVE_INFINITY];
}

/**
 * Calculates WGSL frexp
 *
 * Splits val into a fraction and an exponent so that
 * val = fraction * 2 ^ exponent.
 * The fraction is 0.0 or its magnitude is in the range [0.5, 1.0).
 *
 * @param val the float to split
 * @param trait the float type, f32 or f16 or f64
 * @returns the results of splitting val
 */
function frexp(val, trait) {
  const buffer = new ArrayBuffer(8);
  const dataView = new DataView(buffer);

  // expBitCount and fractBitCount is the bitwidth of exponent and fractional part of the given FP type.
  // expBias is the bias constant of exponent of the given FP type.
  // Biased exponent (unsigned integer, i.e. the exponent part of float) = unbiased exponent (signed integer) + expBias.
  let expBitCount, fractBitCount, expBias;
  // To handle the exponent bits of given FP types (f16, f32, and f64), considering the highest 16
  // bits is enough.
  // expMaskForHigh16Bits indicates the exponent bitfield in the highest 16 bits of the given FP
  // type, and targetExpBitsForHigh16Bits is the exponent bits that corresponding to unbiased
  // exponent -1, i.e. the exponent bits when the FP values is in range [0.5, 1.0).
  let expMaskForHigh16Bits, targetExpBitsForHigh16Bits;
  // Helper function that store the given FP value into buffer as the given FP types
  let setFloatToBuffer;
  // Helper function that read back FP value from buffer as the given FP types
  let getFloatFromBuffer;

  let isFinite;
  let isSubnormal;

  if (trait === 'f32') {
    // f32 bit pattern: s_eeeeeeee_fffffff_ffffffffffffffff
    expBitCount = 8;
    fractBitCount = 23;
    expBias = 127;
    // The exponent bitmask for high 16 bits of f32.
    expMaskForHigh16Bits = 0x7f80;
    // The target exponent bits is equal to those for f32 0.5 = 0x3f000000.
    targetExpBitsForHigh16Bits = 0x3f00;
    isFinite = isFiniteF32;
    isSubnormal = isSubnormalNumberF32;
    // Enforce big-endian so that offset 0 is highest byte.
    setFloatToBuffer = (v) => dataView.setFloat32(0, v, false);
    getFloatFromBuffer = () => dataView.getFloat32(0, false);
  } else if (trait === 'f16') {
    // f16 bit pattern: s_eeeee_ffffffffff
    expBitCount = 5;
    fractBitCount = 10;
    expBias = 15;
    // The exponent bitmask for 16 bits of f16.
    expMaskForHigh16Bits = 0x7c00;
    // The target exponent bits is equal to those for f16 0.5 = 0x3800.
    targetExpBitsForHigh16Bits = 0x3800;
    isFinite = isFiniteF16;
    isSubnormal = isSubnormalNumberF16;
    // Enforce big-endian so that offset 0 is highest byte.
    setFloatToBuffer = (v) => setFloat16(dataView, 0, v, false);
    getFloatFromBuffer = () => getFloat16(dataView, 0, false);
  } else {
    assert(trait === 'f64');
    // f64 bit pattern: s_eeeeeeeeeee_ffff_ffffffffffffffffffffffffffffffffffffffffffffffff
    expBitCount = 11;
    fractBitCount = 52;
    expBias = 1023;
    // The exponent bitmask for 16 bits of f64.
    expMaskForHigh16Bits = 0x7ff0;
    // The target exponent bits is equal to those for f64 0.5 = 0x3fe0_0000_0000_0000.
    targetExpBitsForHigh16Bits = 0x3fe0;
    isFinite = Number.isFinite;
    isSubnormal = isSubnormalNumberF64;
    // Enforce big-endian so that offset 0 is highest byte.
    setFloatToBuffer = (v) => dataView.setFloat64(0, v, false);
    getFloatFromBuffer = () => dataView.getFloat64(0, false);
  }
  // Helper function that extract the unbiased exponent of the float in buffer.
  const extractUnbiasedExpFromNormalFloatInBuffer = () => {
    // Assert the float in buffer is finite normal float.
    assert(isFinite(getFloatFromBuffer()) && !isSubnormal(getFloatFromBuffer()));
    // Get the highest 16 bits of float as uint16, which can contain the whole exponent part for both f16, f32, and f64.
    const high16BitsAsUint16 = dataView.getUint16(0, false);
    // Return the unbiased exp by masking, shifting and unbiasing.
    return ((high16BitsAsUint16 & expMaskForHigh16Bits) >> 16 - 1 - expBitCount) - expBias;
  };
  // Helper function that modify the exponent of float in buffer to make it in range [0.5, 1.0).
  // By setting the unbiased exponent to -1, the fp value will be in range 2**-1 * [1.0, 2.0), i.e. [0.5, 1.0).
  const modifyExpOfNormalFloatInBuffer = () => {
    // Assert the float in buffer is finite normal float.
    assert(isFinite(getFloatFromBuffer()) && !isSubnormal(getFloatFromBuffer()));
    // Get the highest 16 bits of float as uint16, which contains the whole exponent part for both f16, f32, and f64.
    const high16BitsAsUint16 = dataView.getUint16(0, false);
    // Modify the exponent bits.
    const modifiedHigh16Bits =
    high16BitsAsUint16 & ~expMaskForHigh16Bits | targetExpBitsForHigh16Bits;
    // Set back to buffer
    dataView.setUint16(0, modifiedHigh16Bits, false);
  };

  // +/- 0.0
  if (val === 0) {
    return { fract: val, exp: 0 };
  }
  // NaN and Inf
  if (!isFinite(val)) {
    return { fract: val, exp: 0 };
  }

  setFloatToBuffer(val);
  // Don't use val below. Use helper functions working with buffer instead.

  let exp = 0;
  // Normailze the value if it is subnormal. Increase the exponent by multiplying a subnormal value
  // with 2**fractBitCount will result in a finite normal FP value of the given FP type.
  if (isSubnormal(getFloatFromBuffer())) {
    setFloatToBuffer(getFloatFromBuffer() * 2 ** fractBitCount);
    exp = -fractBitCount;
  }
  // A normal FP value v is represented as v = ((-1)**s)*(2**(unbiased exponent))*f, where f is in
  // range [1.0, 2.0). By moving a factor 2 from f to exponent, we have
  // v = ((-1)**s)*(2**(unbiased exponent + 1))*(f / 2), where (f / 2) is in range [0.5, 1.0), so
  // the exp = (unbiased exponent + 1) and fract = ((-1)**s)*(f / 2) is what we expect to get from
  // frexp function. Note that fract and v only differs in exponent bitfield as long as v is normal.
  // Calc the result exp by getting the unbiased float exponent and plus 1.
  exp += extractUnbiasedExpFromNormalFloatInBuffer() + 1;
  // Modify the exponent of float in buffer to make it be in range [0.5, 1.0) to get fract.
  modifyExpOfNormalFloatInBuffer();

  return { fract: getFloatFromBuffer(), exp };
}

/**
 * Calculates the linear interpolation between two values of a given fractional.
 *
 * If |t| is 0, |a| is returned, if |t| is 1, |b| is returned, otherwise
 * interpolation/extrapolation equivalent to a + t(b - a) is performed.
 *
 * Numerical stable version is adapted from http://www.open-std.org/jtc1/sc22/wg21/docs/papers/2018/p0811r2.html
 */
function lerp(a, b, t) {
  if (!Number.isFinite(a) || !Number.isFinite(b)) {
    return Number.NaN;
  }

  if (a <= 0.0 && b >= 0.0 || a >= 0.0 && b <= 0.0) {
    return t * b + (1 - t) * a;
  }

  if (t === 1.0) {
    return b;
  }

  const x = a + t * (b - a);
  return t > 1.0 === b > a ? Math.max(b, x) : Math.min(b, x);
}

/**
 * Version of lerp that operates on bigint values
 *
 * lerp was not made into a generic or to take in (number|bigint), because that
 * introduces a bunch of complexity overhead related to type differentiation
 */
function lerpBigInt(a, b, idx, steps) {
  assert(Math.trunc(idx) === idx);
  assert(Math.trunc(steps) === steps);

  // This constrains t to [0.0, 1.0]
  assert(idx >= 0);
  assert(steps > 0);
  assert(idx < steps);

  if (steps === 1) {
    return a;
  }
  if (idx === 0) {
    return a;
  }
  if (idx === steps - 1) {
    return b;
  }

  const min = (x, y) => {
    return x < y ? x : y;
  };
  const max = (x, y) => {
    return x > y ? x : y;
  };

  // For number the variable t is used, there t = idx / (steps - 1),
  // but that is a fraction on [0, 1], so becomes either 0 or 1 when converted
  // to bigint, so need to expand things out.
  const big_idx = BigInt(idx);
  const big_steps = BigInt(steps);
  if (a <= 0n && b >= 0n || a >= 0n && b <= 0n) {
    return b * big_idx / (big_steps - 1n) + (a - a * big_idx / (big_steps - 1n));
  }

  const x = a + b * big_idx / (big_steps - 1n) - a * big_idx / (big_steps - 1n);
  return !(b > a) ? max(b, x) : min(b, x);
}

/** @returns a linear increasing range of numbers. */
function linearRange(a, b, num_steps) {
  if (num_steps <= 0) {
    return [];
  }

  // Avoid division by 0
  if (num_steps === 1) {
    return [a];
  }

  return Array.from(Array(num_steps).keys()).map((i) => lerp(a, b, i / (num_steps - 1)));
}

/**
 * Version of linearRange that operates on bigint values
 *
 * linearRange was not made into a generic or to take in (number|bigint),
 * because that introduces a bunch of complexity overhead related to type
 * differentiation
 */
function linearRangeBigInt(a, b, num_steps) {
  if (num_steps <= 0) {
    return [];
  }

  // Avoid division by 0
  if (num_steps === 1) {
    return [a];
  }

  return Array.from(Array(num_steps).keys()).map((i) => lerpBigInt(a, b, i, num_steps));
}

/**
 * @returns a non-linear increasing range of numbers, with a bias towards the beginning.
 *
 * Generates a linear range on [0,1] with |num_steps|, then squares all the values to make the curve be quadratic,
 * thus biasing towards 0, but remaining on the [0, 1] range.
 * This biased range is then scaled to the desired range using lerp.
 * Different curves could be generated by changing c, where greater values of c will bias more towards 0.
 */
function biasedRange(a, b, num_steps) {
  const c = 2;
  if (num_steps <= 0) {
    return [];
  }

  // Avoid division by 0
  if (num_steps === 1) {
    return [a];
  }

  return Array.from(Array(num_steps).keys()).map((i) => lerp(a, b, Math.pow(i / (num_steps - 1), c)));
}

/**
 * Version of biasedRange that operates on bigint values
 *
 * biasedRange was not made into a generic or to take in (number|bigint),
 * because that introduces a bunch of complexity overhead related to type
 * differentiation.
 *
 * Scaling is used internally so that the number of possible indices is
 * significantly larger than num_steps. This is done to avoid duplicate entries
 * in the resulting range due to quantizing to integers during the calculation.
 *
 *  If a and b are close together, such that the number of integers between them
 *  is close to num_steps, then duplicates will occur regardless of scaling.
 */
function biasedRangeBigInt(a, b, num_steps) {
  if (num_steps <= 0) {
    return [];
  }

  // Avoid division by 0
  if (num_steps === 1) {
    return [a];
  }

  const c = 2;
  const scaling = 1000;
  const scaled_num_steps = num_steps * scaling;

  return Array.from(Array(num_steps).keys()).map((i) => {
    const biased_i = Math.pow(i / (num_steps - 1), c); // Floating Point on [0, 1]
    const scaled_i = Math.trunc((scaled_num_steps - 1) * biased_i); // Integer on [0, scaled_num_steps - 1]
    return lerpBigInt(a, b, scaled_i, scaled_num_steps);
  });
}

/**
 * @returns an ascending sorted array of numbers spread over the entire range of 32-bit floats
 *
 * Numbers are divided into 4 regions: negative normals, negative subnormals, positive subnormals & positive normals.
 * Zero is included.
 *
 * Numbers are generated via taking a linear spread of the bit field representations of the values in each region. This
 * means that number of precise f32 values between each returned value in a region should be about the same. This allows
 * for a wide range of magnitudes to be generated, instead of being extremely biased towards the edges of the f32 range.
 *
 * This function is intended to provide dense coverage of the f32 range, for a minimal list of values to use to cover
 * f32 behaviour, use sparseScalarF32Range instead.
 *
 * @param counts structure param with 4 entries indicating the number of entries to be generated each region, entries
 *               must be 0 or greater.
 */
function scalarF32Range(
counts =




{ pos_sub: 10, pos_norm: 50 })
{
  counts.neg_norm = counts.neg_norm === undefined ? counts.pos_norm : counts.neg_norm;
  counts.neg_sub = counts.neg_sub === undefined ? counts.pos_sub : counts.neg_sub;

  let special_pos = [];
  // The first interior point for 'pos_norm' is at 3. Because we have two special values we start allowing these
  // special values as soon as they will fit as interior values.
  if (counts.pos_norm >= 4) {
    special_pos = [
    // Largest float as signed integer
    0x4effffff,
    // Largest float as unsigned integer
    0x4f7fffff];

  }
  // Generating bit fields first and then converting to f32, so that the spread across the possible f32 values is more
  // even. Generating against the bounds of f32 values directly results in the values being extremely biased towards the
  // extremes, since they are so much larger.
  const bit_fields = [
  ...linearRange(kBit.f32.negative.min, kBit.f32.negative.max, counts.neg_norm),
  ...linearRange(
    kBit.f32.negative.subnormal.min,
    kBit.f32.negative.subnormal.max,
    counts.neg_sub
  ),
  // -0.0
  0x80000000,
  // +0.0
  0,
  ...linearRange(
    kBit.f32.positive.subnormal.min,
    kBit.f32.positive.subnormal.max,
    counts.pos_sub
  ),
  ...[
  ...linearRange(
    kBit.f32.positive.min,
    kBit.f32.positive.max,
    counts.pos_norm - special_pos.length
  ),
  ...special_pos].
  sort((n1, n2) => n1 - n2)].
  map(Math.trunc);
  return bit_fields.map(reinterpretU32AsF32);
}

/**
 * @returns an ascending sorted array of numbers.
 *
 * The numbers returned are based on the `full32Range` as described above. The difference comes depending
 * on the `source` parameter. If the `source` is `const` then the numbers will be restricted to be
 * in the range `[low, high]`. This allows filtering out a set of `f32` values which are invalid for
 * const-evaluation but are needed to test the non-const implementation.
 *
 * @param source the input source for the test. If the `source` is `const` then the return will be filtered
 * @param low the lowest f32 value to permit when filtered
 * @param high the highest f32 value to permit when filtered
 */
function filteredScalarF32Range(source, low, high) {
  return scalarF32Range().filter((x) => source !== 'const' || x >= low && x <= high);
}

/**
 * @returns an ascending sorted array of numbers spread over the entire range of 16-bit floats
 *
 * Numbers are divided into 4 regions: negative normals, negative subnormals, positive subnormals & positive normals.
 * Zero is included.
 *
 * Numbers are generated via taking a linear spread of the bit field representations of the values in each region. This
 * means that number of precise f16 values between each returned value in a region should be about the same. This allows
 * for a wide range of magnitudes to be generated, instead of being extremely biased towards the edges of the f16 range.
 *
 * This function is intended to provide dense coverage of the f16 range, for a minimal list of values to use to cover
 * f16 behaviour, use sparseScalarF16Range instead.
 *
 * @param counts structure param with 4 entries indicating the number of entries to be generated each region, entries
 *               must be 0 or greater.
 */
function scalarF16Range(
counts =




{ pos_sub: 10, pos_norm: 50 })
{
  counts.neg_norm = counts.neg_norm === undefined ? counts.pos_norm : counts.neg_norm;
  counts.neg_sub = counts.neg_sub === undefined ? counts.pos_sub : counts.neg_sub;

  // Generating bit fields first and then converting to f16, so that the spread across the possible f16 values is more
  // even. Generating against the bounds of f16 values directly results in the values being extremely biased towards the
  // extremes, since they are so much larger.
  const bit_fields = [
  ...linearRange(kBit.f16.negative.min, kBit.f16.negative.max, counts.neg_norm),
  ...linearRange(
    kBit.f16.negative.subnormal.min,
    kBit.f16.negative.subnormal.max,
    counts.neg_sub
  ),
  // -0.0
  0x8000,
  // +0.0
  0,
  ...linearRange(
    kBit.f16.positive.subnormal.min,
    kBit.f16.positive.subnormal.max,
    counts.pos_sub
  ),
  ...linearRange(kBit.f16.positive.min, kBit.f16.positive.max, counts.pos_norm)].
  map(Math.trunc);
  return bit_fields.map(reinterpretU16AsF16);
}

/**
 * @returns an ascending sorted array of numbers spread over the entire range of 64-bit floats
 *
 * Numbers are divided into 4 regions: negative normals, negative subnormals, positive subnormals & positive normals.
 * Zero is included.
 *
 * Numbers are generated via taking a linear spread of the bit field representations of the values in each region. This
 * means that number of precise f64 values between each returned value in a region should be about the same. This allows
 * for a wide range of magnitudes to be generated, instead of being extremely biased towards the edges of the f64 range.
 *
 * This function is intended to provide dense coverage of the f64 range, for a minimal list of values to use to cover
 * f64 behaviour, use sparseScalarF64Range instead.
 *
 * @param counts structure param with 4 entries indicating the number of entries to be generated each region, entries
 *               must be 0 or greater.
 */
function scalarF64Range(
counts =




{ pos_sub: 10, pos_norm: 50 })
{
  counts.neg_norm = counts.neg_norm === undefined ? counts.pos_norm : counts.neg_norm;
  counts.neg_sub = counts.neg_sub === undefined ? counts.pos_sub : counts.neg_sub;

  // Generating bit fields first and then converting to f64, so that the spread across the possible f64 values is more
  // even. Generating against the bounds of f64 values directly results in the values being extremely biased towards the
  // extremes, since they are so much larger.
  const bit_fields = [
  ...linearRangeBigInt(kBit.f64.negative.min, kBit.f64.negative.max, counts.neg_norm),
  ...linearRangeBigInt(
    kBit.f64.negative.subnormal.min,
    kBit.f64.negative.subnormal.max,
    counts.neg_sub
  ),
  // -0.0
  0x8000_0000_0000_0000n,
  // +0.0
  0n,
  ...linearRangeBigInt(
    kBit.f64.positive.subnormal.min,
    kBit.f64.positive.subnormal.max,
    counts.pos_sub
  ),
  ...linearRangeBigInt(kBit.f64.positive.min, kBit.f64.positive.max, counts.pos_norm)];

  return bit_fields.map(reinterpretU64AsF64);
}

/**
 * @returns an ascending sorted array of f64 values spread over specific range of f64 normal floats
 *
 * Numbers are divided into 4 regions: negative 64-bit normals, negative 64-bit subnormals, positive 64-bit subnormals &
 * positive 64-bit normals.
 * Zero is included.
 *
 * Numbers are generated via taking a linear spread of the bit field representations of the values in each region. This
 * means that number of precise f64 values between each returned value in a region should be about the same. This allows
 * for a wide range of magnitudes to be generated, instead of being extremely biased towards the edges of the range.
 *
 * @param begin a negative f64 normal float value
 * @param end a positive f64 normal float value
 * @param counts structure param with 4 entries indicating the number of entries
 *               to be generated each region, entries must be 0 or greater.
 */
function limitedScalarF64Range(
begin,
end,
counts = {
  pos_sub: 10,
  pos_norm: 50
})
{
  assert(
    begin <= kValue.f64.negative.max,
    `Beginning of range ${begin} must be negative f64 normal`
  );
  assert(end >= kValue.f64.positive.min, `Ending of range ${end} must be positive f64 normal`);

  counts.neg_norm = counts.neg_norm === undefined ? counts.pos_norm : counts.neg_norm;
  counts.neg_sub = counts.neg_sub === undefined ? counts.pos_sub : counts.neg_sub;

  const u64_begin = reinterpretF64AsU64(begin);
  const u64_end = reinterpretF64AsU64(end);
  // Generating bit fields first and then converting to f64, so that the spread across the possible f64 values is more
  // even. Generating against the bounds of f64 values directly results in the values being extremely biased towards the
  // extremes, since they are so much larger.
  const bit_fields = [
  ...linearRangeBigInt(u64_begin, kBit.f64.negative.max, counts.neg_norm),
  ...linearRangeBigInt(
    kBit.f64.negative.subnormal.min,
    kBit.f64.negative.subnormal.max,
    counts.neg_sub
  ),
  // -0.0
  0x8000_0000_0000_0000n,
  // +0.0
  0n,
  ...linearRangeBigInt(
    kBit.f64.positive.subnormal.min,
    kBit.f64.positive.subnormal.max,
    counts.pos_sub
  ),
  ...linearRangeBigInt(kBit.f64.positive.min, u64_end, counts.pos_norm)];

  return bit_fields.map(reinterpretU64AsF64);
}

/** Short list of i32 values of interest to test against */
const kInterestingI32Values = [
kValue.i32.negative.max,
Math.trunc(kValue.i32.negative.max / 2),
-256,
-10,
-1,
0,
1,
10,
256,
Math.trunc(kValue.i32.positive.max / 2),
kValue.i32.positive.max];


/** @returns minimal i32 values that cover the entire range of i32 behaviours
 *
 * This is used instead of fullI32Range when the number of test cases being
 * generated is a super linear function of the length of i32 values which is
 * leading to time outs.
 */
function sparseI32Range() {
  return kInterestingI32Values;
}

const kVectorI32Values = {
  2: kInterestingI32Values.flatMap((f) => [
  [f, 1],
  [-1, f]]
  ),
  3: kInterestingI32Values.flatMap((f) => [
  [f, 1, -2],
  [-1, f, 2],
  [1, -2, f]]
  ),
  4: kInterestingI32Values.flatMap((f) => [
  [f, -1, 2, 3],
  [1, f, -2, 3],
  [1, 2, f, -3],
  [-1, 2, -3, f]]
  )
};

/**
 * Returns set of vectors, indexed by dimension containing interesting i32
 * values.
 *
 * The tests do not do the simple option for coverage of computing the cartesian
 * product of all of the interesting i32 values N times for vecN tests,
 * because that creates a huge number of tests for vec3 and vec4, leading to
 * time outs.
 *
 * Instead they insert the interesting i32 values into each location of the
 * vector to get a spread of testing over the entire range. This reduces the
 * number of cases being run substantially, but maintains coverage.
 */
function vectorI32Range(dim) {
  assert(dim === 2 || dim === 3 || dim === 4, 'vectorI32Range only accepts dimensions 2, 3, and 4');
  return kVectorI32Values[dim];
}

const kSparseVectorI32Values = {
  2: sparseI32Range().map((i, idx) => [idx % 2 === 0 ? i : idx, idx % 2 === 1 ? i : -idx]),
  3: sparseI32Range().map((i, idx) => [
  idx % 3 === 0 ? i : idx,
  idx % 3 === 1 ? i : -idx,
  idx % 3 === 2 ? i : idx]
  ),
  4: sparseI32Range().map((i, idx) => [
  idx % 4 === 0 ? i : idx,
  idx % 4 === 1 ? i : -idx,
  idx % 4 === 2 ? i : idx,
  idx % 4 === 3 ? i : -idx]
  )
};

/**
 * Minimal set of vectors, indexed by dimension, that contain interesting
 * abstract integer values.
 *
 * This is an even more stripped down version of `vectorI32Range` for when
 * pairs of vectors are being tested.
 * All interesting integers from sparseI32Range are guaranteed to be
 * tested, but not in every position.
 */
function sparseVectorI32Range(dim) {
  assert(
    dim === 2 || dim === 3 || dim === 4,
    'sparseVectorI32Range only accepts dimensions 2, 3, and 4'
  );
  return kSparseVectorI32Values[dim];
}

/**
 * @returns an ascending sorted array of numbers spread over the entire range of 32-bit signed ints
 *
 * Numbers are divided into 2 regions: negatives, and positives, with their spreads biased towards 0
 * Zero is included in range.
 *
 * @param counts structure param with 2 entries indicating the number of entries to be generated each region, values must be 0 or greater.
 */
function fullI32Range(
counts =


{ positive: 50 })
{
  counts.negative = counts.negative === undefined ? counts.positive : counts.negative;
  return [
  ...biasedRange(kValue.i32.negative.min, -1, counts.negative),
  0,
  ...biasedRange(1, kValue.i32.positive.max, counts.positive)].
  map(Math.trunc);
}

/** Short list of u32 values of interest to test against */
const kInterestingU32Values = [
0,
1,
10,
256,
Math.trunc(kValue.u32.max / 2),
kValue.u32.max];


/** @returns minimal u32 values that cover the entire range of u32 behaviours
 *
 * This is used instead of fullU32Range when the number of test cases being
 * generated is a super linear function of the length of u32 values which is
 * leading to time outs.
 */
function sparseU32Range() {
  return kInterestingU32Values;
}

const kVectorU32Values = {
  2: kInterestingU32Values.flatMap((f) => [
  [f, 1],
  [1, f]]
  ),
  3: kInterestingU32Values.flatMap((f) => [
  [f, 1, 2],
  [1, f, 2],
  [1, 2, f]]
  ),
  4: kInterestingU32Values.flatMap((f) => [
  [f, 1, 2, 3],
  [1, f, 2, 3],
  [1, 2, f, 3],
  [1, 2, 3, f]]
  )
};

/**
 * Returns set of vectors, indexed by dimension containing interesting u32
 * values.
 *
 * The tests do not do the simple option for coverage of computing the cartesian
 * product of all of the interesting u32 values N times for vecN tests,
 * because that creates a huge number of tests for vec3 and vec4, leading to
 * time outs.
 *
 * Instead they insert the interesting u32 values into each location of the
 * vector to get a spread of testing over the entire range. This reduces the
 * number of cases being run substantially, but maintains coverage.
 */
function vectorU32Range(dim) {
  assert(dim === 2 || dim === 3 || dim === 4, 'vectorU32Range only accepts dimensions 2, 3, and 4');
  return kVectorU32Values[dim];
}

const kSparseVectorU32Values = {
  2: sparseU32Range().map((i, idx) => [idx % 2 === 0 ? i : idx, idx % 2 === 1 ? i : -idx]),
  3: sparseU32Range().map((i, idx) => [
  idx % 3 === 0 ? i : idx,
  idx % 3 === 1 ? i : -idx,
  idx % 3 === 2 ? i : idx]
  ),
  4: sparseU32Range().map((i, idx) => [
  idx % 4 === 0 ? i : idx,
  idx % 4 === 1 ? i : -idx,
  idx % 4 === 2 ? i : idx,
  idx % 4 === 3 ? i : -idx]
  )
};

/**
 * Minimal set of vectors, indexed by dimension, that contain interesting
 * abstract integer values.
 *
 * This is an even more stripped down version of `vectorU32Range` for when
 * pairs of vectors are being tested.
 * All interesting integers from sparseU32Range are guaranteed to be
 * tested, but not in every position.
 */
function sparseVectorU32Range(dim) {
  assert(
    dim === 2 || dim === 3 || dim === 4,
    'sparseVectorU32Range only accepts dimensions 2, 3, and 4'
  );
  return kSparseVectorU32Values[dim];
}

/**
 * @returns an ascending sorted array of numbers spread over the entire range of 32-bit unsigned ints
 *
 * Numbers are biased towards 0, and 0 is included in the range.
 *
 * @param count number of entries to include in the range, in addition to 0, must be greater than 0, defaults to 50
 */
function fullU32Range(count = 50) {
  return [0, ...biasedRange(1, kValue.u32.max, count)].map(Math.trunc);
}

/** Short list of i64 values of interest to test against */
const kInterestingI64Values = [
kValue.i64.negative.max,
kValue.i64.negative.max / 2n,
-256n,
-10n,
-1n,
0n,
1n,
10n,
256n,
kValue.i64.positive.max / 2n,
kValue.i64.positive.max];


/** @returns minimal i64 values that cover the entire range of i64 behaviours
 *
 * This is used instead of fullI64Range when the number of test cases being
 * generated is a super linear function of the length of i64 values which is
 * leading to time outs.
 */
function sparseI64Range() {
  return kInterestingI64Values;
}

const kVectorI64Values = {
  2: kInterestingI64Values.flatMap((f) => [
  [f, 1n],
  [-1n, f]]
  ),
  3: kInterestingI64Values.flatMap((f) => [
  [f, 1n, -2n],
  [-1n, f, 2n],
  [1n, -2n, f]]
  ),
  4: kInterestingI64Values.flatMap((f) => [
  [f, -1n, 2n, 3n],
  [1n, f, -2n, 3n],
  [1n, 2n, f, -3n],
  [-1n, 2n, -3n, f]]
  )
};

/**
 * Returns set of vectors, indexed by dimension containing interesting i64
 * values.
 *
 * The tests do not do the simple option for coverage of computing the cartesian
 * product of all of the interesting i64 values N times for vecN tests,
 * because that creates a huge number of tests for vec3 and vec4, leading to
 * time outs.
 *
 * Instead they insert the interesting i64 values into each location of the
 * vector to get a spread of testing over the entire range. This reduces the
 * number of cases being run substantially, but maintains coverage.
 */
function vectorI64Range(dim) {
  assert(dim === 2 || dim === 3 || dim === 4, 'vectorI64Range only accepts dimensions 2, 3, and 4');
  return kVectorI64Values[dim];
}

const kSparseVectorI64Values = {
  2: sparseI64Range().map((i, idx) => [
  idx % 2 === 0 ? i : BigInt(idx),
  idx % 2 === 1 ? i : -BigInt(idx)]
  ),
  3: sparseI64Range().map((i, idx) => [
  idx % 3 === 0 ? i : BigInt(idx),
  idx % 3 === 1 ? i : -BigInt(idx),
  idx % 3 === 2 ? i : BigInt(idx)]
  ),
  4: sparseI64Range().map((i, idx) => [
  idx % 4 === 0 ? i : BigInt(idx),
  idx % 4 === 1 ? i : -BigInt(idx),
  idx % 4 === 2 ? i : BigInt(idx),
  idx % 4 === 3 ? i : -BigInt(idx)]
  )
};

/**
 * Minimal set of vectors, indexed by dimension, that contain interesting
 * abstract integer values.
 *
 * This is an even more stripped down version of `vectorI64Range` for when
 * pairs of vectors are being tested.
 * All interesting integers from sparseI64Range are guaranteed to be
 * tested, but not in every position.
 */
function sparseVectorI64Range(dim) {
  assert(
    dim === 2 || dim === 3 || dim === 4,
    'sparseVectorI64Range only accepts dimensions 2, 3, and 4'
  );
  return kSparseVectorI64Values[dim];
}

/**
 * @returns an ascending sorted array of numbers spread over the entire range of 64-bit signed ints
 *
 * Numbers are divided into 2 regions: negatives, and positives, with their spreads biased towards 0
 * Zero is included in range.
 *
 * @param counts structure param with 2 entries indicating the number of entries to be generated each region, values must be 0 or greater.
 */
function fullI64Range(
counts =


{ positive: 50 })
{
  counts.negative = counts.negative === undefined ? counts.positive : counts.negative;
  return [
  ...biasedRangeBigInt(kValue.i64.negative.min, -1n, counts.negative),
  0n,
  ...biasedRangeBigInt(1n, kValue.i64.positive.max, counts.positive)];

}

/** Short list of f32 values of interest to test against */
const kInterestingF32Values = [
kValue.f32.negative.min,
-10.0,
-1.0,
-0.125,
kValue.f32.negative.max,
kValue.f32.negative.subnormal.min,
kValue.f32.negative.subnormal.max,
-0.0,
0.0,
kValue.f32.positive.subnormal.min,
kValue.f32.positive.subnormal.max,
kValue.f32.positive.min,
0.125,
1.0,
10.0,
kValue.f32.positive.max];


/** @returns minimal f32 values that cover the entire range of f32 behaviours
 *
 * Has specially selected values that cover edge cases, normals, and subnormals.
 * This is used instead of fullF32Range when the number of test cases being
 * generated is a super linear function of the length of f32 values which is
 * leading to time outs.
 *
 * These values have been chosen to attempt to test the widest range of f32
 * behaviours in the lowest number of entries, so may potentially miss function
 * specific values of interest. If there are known values of interest they
 * should be appended to this list in the test generation code.
 */
function sparseScalarF32Range() {
  return kInterestingF32Values;
}

const kVectorF32Values = {
  2: kInterestingF32Values.flatMap((f) => [
  [f, 1.0],
  [-1.0, f]]
  ),
  3: kInterestingF32Values.flatMap((f) => [
  [f, 1.0, -2.0],
  [-1.0, f, 2.0],
  [1.0, -2.0, f]]
  ),
  4: kInterestingF32Values.flatMap((f) => [
  [f, -1.0, 2.0, 3.0],
  [1.0, f, -2.0, 3.0],
  [1.0, 2.0, f, -3.0],
  [-1.0, 2.0, -3.0, f]]
  )
};

/**
 * Returns set of vectors, indexed by dimension containing interesting float
 * values.
 *
 * The tests do not do the simple option for coverage of computing the cartesian
 * product of all of the interesting float values N times for vecN tests,
 * because that creates a huge number of tests for vec3 and vec4, leading to
 * time outs.
 *
 * Instead they insert the interesting f32 values into each location of the
 * vector to get a spread of testing over the entire range. This reduces the
 * number of cases being run substantially, but maintains coverage.
 */
function vectorF32Range(dim) {
  assert(dim === 2 || dim === 3 || dim === 4, 'vectorF32Range only accepts dimensions 2, 3, and 4');
  return kVectorF32Values[dim];
}

const kSparseVectorF32Values = {
  2: sparseScalarF32Range().map((f, idx) => [idx % 2 === 0 ? f : idx, idx % 2 === 1 ? f : -idx]),
  3: sparseScalarF32Range().map((f, idx) => [
  idx % 3 === 0 ? f : idx,
  idx % 3 === 1 ? f : -idx,
  idx % 3 === 2 ? f : idx]
  ),
  4: sparseScalarF32Range().map((f, idx) => [
  idx % 4 === 0 ? f : idx,
  idx % 4 === 1 ? f : -idx,
  idx % 4 === 2 ? f : idx,
  idx % 4 === 3 ? f : -idx]
  )
};

/**
 * Minimal set of vectors, indexed by dimension, that contain interesting float
 * values.
 *
 * This is an even more stripped down version of `vectorF32Range` for when
 * pairs of vectors are being tested.
 * All of the interesting floats from sparseScalarF32 are guaranteed to be
 * tested, but not in every position.
 */
function sparseVectorF32Range(dim) {
  assert(
    dim === 2 || dim === 3 || dim === 4,
    'sparseVectorF32Range only accepts dimensions 2, 3, and 4'
  );
  return kSparseVectorF32Values[dim];
}

const kSparseMatrixF32Values = {
  2: {
    2: kInterestingF32Values.map((f, idx) => [
    [idx % 4 === 0 ? f : idx, idx % 4 === 1 ? f : -idx],
    [idx % 4 === 2 ? f : -idx, idx % 4 === 3 ? f : idx]]
    ),
    3: kInterestingF32Values.map((f, idx) => [
    [idx % 6 === 0 ? f : idx, idx % 6 === 1 ? f : -idx, idx % 6 === 2 ? f : idx],
    [idx % 6 === 3 ? f : -idx, idx % 6 === 4 ? f : idx, idx % 6 === 5 ? f : -idx]]
    ),
    4: kInterestingF32Values.map((f, idx) => [
    [
    idx % 8 === 0 ? f : idx,
    idx % 8 === 1 ? f : -idx,
    idx % 8 === 2 ? f : idx,
    idx % 8 === 3 ? f : -idx],

    [
    idx % 8 === 4 ? f : -idx,
    idx % 8 === 5 ? f : idx,
    idx % 8 === 6 ? f : -idx,
    idx % 8 === 7 ? f : idx]]

    )
  },
  3: {
    2: kInterestingF32Values.map((f, idx) => [
    [idx % 6 === 0 ? f : idx, idx % 6 === 1 ? f : -idx],
    [idx % 6 === 2 ? f : -idx, idx % 6 === 3 ? f : idx],
    [idx % 6 === 4 ? f : idx, idx % 6 === 5 ? f : -idx]]
    ),
    3: kInterestingF32Values.map((f, idx) => [
    [idx % 9 === 0 ? f : idx, idx % 9 === 1 ? f : -idx, idx % 9 === 2 ? f : idx],
    [idx % 9 === 3 ? f : -idx, idx % 9 === 4 ? f : idx, idx % 9 === 5 ? f : -idx],
    [idx % 9 === 6 ? f : idx, idx % 9 === 7 ? f : -idx, idx % 9 === 8 ? f : idx]]
    ),
    4: kInterestingF32Values.map((f, idx) => [
    [
    idx % 12 === 0 ? f : idx,
    idx % 12 === 1 ? f : -idx,
    idx % 12 === 2 ? f : idx,
    idx % 12 === 3 ? f : -idx],

    [
    idx % 12 === 4 ? f : -idx,
    idx % 12 === 5 ? f : idx,
    idx % 12 === 6 ? f : -idx,
    idx % 12 === 7 ? f : idx],

    [
    idx % 12 === 8 ? f : idx,
    idx % 12 === 9 ? f : -idx,
    idx % 12 === 10 ? f : idx,
    idx % 12 === 11 ? f : -idx]]

    )
  },
  4: {
    2: kInterestingF32Values.map((f, idx) => [
    [idx % 8 === 0 ? f : idx, idx % 8 === 1 ? f : -idx],
    [idx % 8 === 2 ? f : -idx, idx % 8 === 3 ? f : idx],
    [idx % 8 === 4 ? f : idx, idx % 8 === 5 ? f : -idx],
    [idx % 8 === 6 ? f : -idx, idx % 8 === 7 ? f : idx]]
    ),
    3: kInterestingF32Values.map((f, idx) => [
    [idx % 12 === 0 ? f : idx, idx % 12 === 1 ? f : -idx, idx % 12 === 2 ? f : idx],
    [idx % 12 === 3 ? f : -idx, idx % 12 === 4 ? f : idx, idx % 12 === 5 ? f : -idx],
    [idx % 12 === 6 ? f : idx, idx % 12 === 7 ? f : -idx, idx % 12 === 8 ? f : idx],
    [idx % 12 === 9 ? f : -idx, idx % 12 === 10 ? f : idx, idx % 12 === 11 ? f : -idx]]
    ),
    4: kInterestingF32Values.map((f, idx) => [
    [
    idx % 16 === 0 ? f : idx,
    idx % 16 === 1 ? f : -idx,
    idx % 16 === 2 ? f : idx,
    idx % 16 === 3 ? f : -idx],

    [
    idx % 16 === 4 ? f : -idx,
    idx % 16 === 5 ? f : idx,
    idx % 16 === 6 ? f : -idx,
    idx % 16 === 7 ? f : idx],

    [
    idx % 16 === 8 ? f : idx,
    idx % 16 === 9 ? f : -idx,
    idx % 16 === 10 ? f : idx,
    idx % 16 === 11 ? f : -idx],

    [
    idx % 16 === 12 ? f : -idx,
    idx % 16 === 13 ? f : idx,
    idx % 16 === 14 ? f : -idx,
    idx % 16 === 15 ? f : idx]]

    )
  }
};

/**
 * Returns a minimal set of matrices, indexed by dimension containing
 * interesting float values.
 *
 * This is the matrix analogue of `sparseVectorF32Range`, so it is producing a
 * minimal coverage set of matrices that test all of the interesting f32 values.
 * There is not a more expansive set of matrices, since matrices are even more
 * expensive than vectors for increasing runtime with coverage.
 *
 * All of the interesting floats from sparseScalarF32 are guaranteed to be
 * tested, but not in every position.
 */
function sparseMatrixF32Range(c, r) {
  assert(
    c === 2 || c === 3 || c === 4,
    'sparseMatrixF32Range only accepts column counts of 2, 3, and 4'
  );
  assert(
    r === 2 || r === 3 || r === 4,
    'sparseMatrixF32Range only accepts row counts of 2, 3, and 4'
  );
  return kSparseMatrixF32Values[c][r];
}

/** Short list of f16 values of interest to test against */
const kInterestingF16Values = [
kValue.f16.negative.min,
-10.0,
-1.0,
-0.125,
kValue.f16.negative.max,
kValue.f16.negative.subnormal.min,
kValue.f16.negative.subnormal.max,
-0.0,
0.0,
kValue.f16.positive.subnormal.min,
kValue.f16.positive.subnormal.max,
kValue.f16.positive.min,
0.125,
1.0,
10.0,
kValue.f16.positive.max];


/** @returns minimal f16 values that cover the entire range of f16 behaviours
 *
 * Has specially selected values that cover edge cases, normals, and subnormals.
 * This is used instead of fullF16Range when the number of test cases being
 * generated is a super linear function of the length of f16 values which is
 * leading to time outs.
 *
 * These values have been chosen to attempt to test the widest range of f16
 * behaviours in the lowest number of entries, so may potentially miss function
 * specific values of interest. If there are known values of interest they
 * should be appended to this list in the test generation code.
 */
function sparseScalarF16Range() {
  return kInterestingF16Values;
}

const kVectorF16Values = {
  2: kInterestingF16Values.flatMap((f) => [
  [f, 1.0],
  [-1.0, f]]
  ),
  3: kInterestingF16Values.flatMap((f) => [
  [f, 1.0, -2.0],
  [-1.0, f, 2.0],
  [1.0, -2.0, f]]
  ),
  4: kInterestingF16Values.flatMap((f) => [
  [f, -1.0, 2.0, 3.0],
  [1.0, f, -2.0, 3.0],
  [1.0, 2.0, f, -3.0],
  [-1.0, 2.0, -3.0, f]]
  )
};

/**
 * Returns set of vectors, indexed by dimension containing interesting f16
 * values.
 *
 * The tests do not do the simple option for coverage of computing the cartesian
 * product of all of the interesting float values N times for vecN tests,
 * because that creates a huge number of tests for vec3 and vec4, leading to
 * time outs.
 *
 * Instead they insert the interesting f16 values into each location of the
 * vector to get a spread of testing over the entire range. This reduces the
 * number of cases being run substantially, but maintains coverage.
 */
function vectorF16Range(dim) {
  assert(dim === 2 || dim === 3 || dim === 4, 'vectorF16Range only accepts dimensions 2, 3, and 4');
  return kVectorF16Values[dim];
}

const kSparseVectorF16Values = {
  2: sparseScalarF16Range().map((f, idx) => [idx % 2 === 0 ? f : idx, idx % 2 === 1 ? f : -idx]),
  3: sparseScalarF16Range().map((f, idx) => [
  idx % 3 === 0 ? f : idx,
  idx % 3 === 1 ? f : -idx,
  idx % 3 === 2 ? f : idx]
  ),
  4: sparseScalarF16Range().map((f, idx) => [
  idx % 4 === 0 ? f : idx,
  idx % 4 === 1 ? f : -idx,
  idx % 4 === 2 ? f : idx,
  idx % 4 === 3 ? f : -idx]
  )
};

/**
 * Minimal set of vectors, indexed by dimension, that contain interesting f16
 * values.
 *
 * This is an even more stripped down version of `vectorF16Range` for when
 * pairs of vectors are being tested.
 * All of the interesting floats from sparseScalarF16 are guaranteed to be
 * tested, but not in every position.
 */
function sparseVectorF16Range(dim) {
  assert(
    dim === 2 || dim === 3 || dim === 4,
    'sparseVectorF16Range only accepts dimensions 2, 3, and 4'
  );
  return kSparseVectorF16Values[dim];
}

const kSparseMatrixF16Values = {
  2: {
    2: kInterestingF16Values.map((f, idx) => [
    [idx % 4 === 0 ? f : idx, idx % 4 === 1 ? f : -idx],
    [idx % 4 === 2 ? f : -idx, idx % 4 === 3 ? f : idx]]
    ),
    3: kInterestingF16Values.map((f, idx) => [
    [idx % 6 === 0 ? f : idx, idx % 6 === 1 ? f : -idx, idx % 6 === 2 ? f : idx],
    [idx % 6 === 3 ? f : -idx, idx % 6 === 4 ? f : idx, idx % 6 === 5 ? f : -idx]]
    ),
    4: kInterestingF16Values.map((f, idx) => [
    [
    idx % 8 === 0 ? f : idx,
    idx % 8 === 1 ? f : -idx,
    idx % 8 === 2 ? f : idx,
    idx % 8 === 3 ? f : -idx],

    [
    idx % 8 === 4 ? f : -idx,
    idx % 8 === 5 ? f : idx,
    idx % 8 === 6 ? f : -idx,
    idx % 8 === 7 ? f : idx]]

    )
  },
  3: {
    2: kInterestingF16Values.map((f, idx) => [
    [idx % 6 === 0 ? f : idx, idx % 6 === 1 ? f : -idx],
    [idx % 6 === 2 ? f : -idx, idx % 6 === 3 ? f : idx],
    [idx % 6 === 4 ? f : idx, idx % 6 === 5 ? f : -idx]]
    ),
    3: kInterestingF16Values.map((f, idx) => [
    [idx % 9 === 0 ? f : idx, idx % 9 === 1 ? f : -idx, idx % 9 === 2 ? f : idx],
    [idx % 9 === 3 ? f : -idx, idx % 9 === 4 ? f : idx, idx % 9 === 5 ? f : -idx],
    [idx % 9 === 6 ? f : idx, idx % 9 === 7 ? f : -idx, idx % 9 === 8 ? f : idx]]
    ),
    4: kInterestingF16Values.map((f, idx) => [
    [
    idx % 12 === 0 ? f : idx,
    idx % 12 === 1 ? f : -idx,
    idx % 12 === 2 ? f : idx,
    idx % 12 === 3 ? f : -idx],

    [
    idx % 12 === 4 ? f : -idx,
    idx % 12 === 5 ? f : idx,
    idx % 12 === 6 ? f : -idx,
    idx % 12 === 7 ? f : idx],

    [
    idx % 12 === 8 ? f : idx,
    idx % 12 === 9 ? f : -idx,
    idx % 12 === 10 ? f : idx,
    idx % 12 === 11 ? f : -idx]]

    )
  },
  4: {
    2: kInterestingF16Values.map((f, idx) => [
    [idx % 8 === 0 ? f : idx, idx % 8 === 1 ? f : -idx],
    [idx % 8 === 2 ? f : -idx, idx % 8 === 3 ? f : idx],
    [idx % 8 === 4 ? f : idx, idx % 8 === 5 ? f : -idx],
    [idx % 8 === 6 ? f : -idx, idx % 8 === 7 ? f : idx]]
    ),
    3: kInterestingF16Values.map((f, idx) => [
    [idx % 12 === 0 ? f : idx, idx % 12 === 1 ? f : -idx, idx % 12 === 2 ? f : idx],
    [idx % 12 === 3 ? f : -idx, idx % 12 === 4 ? f : idx, idx % 12 === 5 ? f : -idx],
    [idx % 12 === 6 ? f : idx, idx % 12 === 7 ? f : -idx, idx % 12 === 8 ? f : idx],
    [idx % 12 === 9 ? f : -idx, idx % 12 === 10 ? f : idx, idx % 12 === 11 ? f : -idx]]
    ),
    4: kInterestingF16Values.map((f, idx) => [
    [
    idx % 16 === 0 ? f : idx,
    idx % 16 === 1 ? f : -idx,
    idx % 16 === 2 ? f : idx,
    idx % 16 === 3 ? f : -idx],

    [
    idx % 16 === 4 ? f : -idx,
    idx % 16 === 5 ? f : idx,
    idx % 16 === 6 ? f : -idx,
    idx % 16 === 7 ? f : idx],

    [
    idx % 16 === 8 ? f : idx,
    idx % 16 === 9 ? f : -idx,
    idx % 16 === 10 ? f : idx,
    idx % 16 === 11 ? f : -idx],

    [
    idx % 16 === 12 ? f : -idx,
    idx % 16 === 13 ? f : idx,
    idx % 16 === 14 ? f : -idx,
    idx % 16 === 15 ? f : idx]]

    )
  }
};

/**
 * Returns a minimal set of matrices, indexed by dimension containing interesting
 * f16 values.
 *
 * This is the matrix analogue of `sparseVectorF16Range`, so it is producing a
 * minimal coverage set of matrices that test all of the interesting f16 values.
 * There is not a more expansive set of matrices, since matrices are even more
 * expensive than vectors for increasing runtime with coverage.
 *
 * All of the interesting floats from sparseScalarF16 are guaranteed to be tested, but
 * not in every position.
 */
function sparseMatrixF16Range(c, r) {
  assert(
    c === 2 || c === 3 || c === 4,
    'sparseMatrixF16Range only accepts column counts of 2, 3, and 4'
  );
  assert(
    r === 2 || r === 3 || r === 4,
    'sparseMatrixF16Range only accepts row counts of 2, 3, and 4'
  );
  return kSparseMatrixF16Values[c][r];
}

/** Short list of f64 values of interest to test against */
const kInterestingF64Values = [
kValue.f64.negative.min,
-10.0,
-1.0,
-0.125,
kValue.f64.negative.max,
kValue.f64.negative.subnormal.min,
kValue.f64.negative.subnormal.max,
-0.0,
0.0,
kValue.f64.positive.subnormal.min,
kValue.f64.positive.subnormal.max,
kValue.f64.positive.min,
0.125,
1.0,
10.0,
kValue.f64.positive.max];


/** @returns minimal F64 values that cover the entire range of F64 behaviours
 *
 * Has specially selected values that cover edge cases, normals, and subnormals.
 * This is used instead of fullF64Range when the number of test cases being
 * generated is a super linear function of the length of F64 values which is
 * leading to time outs.
 *
 * These values have been chosen to attempt to test the widest range of F64
 * behaviours in the lowest number of entries, so may potentially miss function
 * specific values of interest. If there are known values of interest they
 * should be appended to this list in the test generation code.
 */
function sparseScalarF64Range() {
  return kInterestingF64Values;
}

const kVectorF64Values = {
  2: kInterestingF64Values.flatMap((f) => [
  [f, 1.0],
  [-1.0, f]]
  ),
  3: kInterestingF64Values.flatMap((f) => [
  [f, 1.0, -2.0],
  [-1.0, f, 2.0],
  [1.0, -2.0, f]]
  ),
  4: kInterestingF64Values.flatMap((f) => [
  [f, -1.0, 2.0, 3.0],
  [1.0, f, -2.0, 3.0],
  [1.0, 2.0, f, -3.0],
  [-1.0, 2.0, -3.0, f]]
  )
};

/**
 * Returns set of vectors, indexed by dimension containing interesting float
 * values.
 *
 * The tests do not do the simple option for coverage of computing the cartesian
 * product of all of the interesting float values N times for vecN tests,
 * because that creates a huge number of tests for vec3 and vec4, leading to
 * time outs.
 *
 * Instead they insert the interesting F64 values into each location of the
 * vector to get a spread of testing over the entire range. This reduces the
 * number of cases being run substantially, but maintains coverage.
 */
function vectorF64Range(dim) {
  assert(dim === 2 || dim === 3 || dim === 4, 'vectorF64Range only accepts dimensions 2, 3, and 4');
  return kVectorF64Values[dim];
}

const kSparseVectorF64Values = {
  2: sparseScalarF64Range().map((f, idx) => [idx % 2 === 0 ? f : idx, idx % 2 === 1 ? f : -idx]),
  3: sparseScalarF64Range().map((f, idx) => [
  idx % 3 === 0 ? f : idx,
  idx % 3 === 1 ? f : -idx,
  idx % 3 === 2 ? f : idx]
  ),
  4: sparseScalarF64Range().map((f, idx) => [
  idx % 4 === 0 ? f : idx,
  idx % 4 === 1 ? f : -idx,
  idx % 4 === 2 ? f : idx,
  idx % 4 === 3 ? f : -idx]
  )
};

/**
 * Minimal set of vectors, indexed by dimension, that contain interesting f64
 * values.
 *
 * This is an even more stripped down version of `vectorF64Range` for when
 * pairs of vectors are being tested.
 * All the interesting floats from sparseScalarF64 are guaranteed to be tested, but
 * not in every position.
 */
function sparseVectorF64Range(dim) {
  assert(
    dim === 2 || dim === 3 || dim === 4,
    'sparseVectorF64Range only accepts dimensions 2, 3, and 4'
  );
  return kSparseVectorF64Values[dim];
}

const kSparseMatrixF64Values = {
  2: {
    2: kInterestingF64Values.map((f, idx) => [
    [idx % 4 === 0 ? f : idx, idx % 4 === 1 ? f : -idx],
    [idx % 4 === 2 ? f : -idx, idx % 4 === 3 ? f : idx]]
    ),
    3: kInterestingF64Values.map((f, idx) => [
    [idx % 6 === 0 ? f : idx, idx % 6 === 1 ? f : -idx, idx % 6 === 2 ? f : idx],
    [idx % 6 === 3 ? f : -idx, idx % 6 === 4 ? f : idx, idx % 6 === 5 ? f : -idx]]
    ),
    4: kInterestingF64Values.map((f, idx) => [
    [
    idx % 8 === 0 ? f : idx,
    idx % 8 === 1 ? f : -idx,
    idx % 8 === 2 ? f : idx,
    idx % 8 === 3 ? f : -idx],

    [
    idx % 8 === 4 ? f : -idx,
    idx % 8 === 5 ? f : idx,
    idx % 8 === 6 ? f : -idx,
    idx % 8 === 7 ? f : idx]]

    )
  },
  3: {
    2: kInterestingF64Values.map((f, idx) => [
    [idx % 6 === 0 ? f : idx, idx % 6 === 1 ? f : -idx],
    [idx % 6 === 2 ? f : -idx, idx % 6 === 3 ? f : idx],
    [idx % 6 === 4 ? f : idx, idx % 6 === 5 ? f : -idx]]
    ),
    3: kInterestingF64Values.map((f, idx) => [
    [idx % 9 === 0 ? f : idx, idx % 9 === 1 ? f : -idx, idx % 9 === 2 ? f : idx],
    [idx % 9 === 3 ? f : -idx, idx % 9 === 4 ? f : idx, idx % 9 === 5 ? f : -idx],
    [idx % 9 === 6 ? f : idx, idx % 9 === 7 ? f : -idx, idx % 9 === 8 ? f : idx]]
    ),
    4: kInterestingF64Values.map((f, idx) => [
    [
    idx % 12 === 0 ? f : idx,
    idx % 12 === 1 ? f : -idx,
    idx % 12 === 2 ? f : idx,
    idx % 12 === 3 ? f : -idx],

    [
    idx % 12 === 4 ? f : -idx,
    idx % 12 === 5 ? f : idx,
    idx % 12 === 6 ? f : -idx,
    idx % 12 === 7 ? f : idx],

    [
    idx % 12 === 8 ? f : idx,
    idx % 12 === 9 ? f : -idx,
    idx % 12 === 10 ? f : idx,
    idx % 12 === 11 ? f : -idx]]

    )
  },
  4: {
    2: kInterestingF64Values.map((f, idx) => [
    [idx % 8 === 0 ? f : idx, idx % 8 === 1 ? f : -idx],
    [idx % 8 === 2 ? f : -idx, idx % 8 === 3 ? f : idx],
    [idx % 8 === 4 ? f : idx, idx % 8 === 5 ? f : -idx],
    [idx % 8 === 6 ? f : -idx, idx % 8 === 7 ? f : idx]]
    ),
    3: kInterestingF64Values.map((f, idx) => [
    [idx % 12 === 0 ? f : idx, idx % 12 === 1 ? f : -idx, idx % 12 === 2 ? f : idx],
    [idx % 12 === 3 ? f : -idx, idx % 12 === 4 ? f : idx, idx % 12 === 5 ? f : -idx],
    [idx % 12 === 6 ? f : idx, idx % 12 === 7 ? f : -idx, idx % 12 === 8 ? f : idx],
    [idx % 12 === 9 ? f : -idx, idx % 12 === 10 ? f : idx, idx % 12 === 11 ? f : -idx]]
    ),
    4: kInterestingF64Values.map((f, idx) => [
    [
    idx % 16 === 0 ? f : idx,
    idx % 16 === 1 ? f : -idx,
    idx % 16 === 2 ? f : idx,
    idx % 16 === 3 ? f : -idx],

    [
    idx % 16 === 4 ? f : -idx,
    idx % 16 === 5 ? f : idx,
    idx % 16 === 6 ? f : -idx,
    idx % 16 === 7 ? f : idx],

    [
    idx % 16 === 8 ? f : idx,
    idx % 16 === 9 ? f : -idx,
    idx % 16 === 10 ? f : idx,
    idx % 16 === 11 ? f : -idx],

    [
    idx % 16 === 12 ? f : -idx,
    idx % 16 === 13 ? f : idx,
    idx % 16 === 14 ? f : -idx,
    idx % 16 === 15 ? f : idx]]

    )
  }
};

/**
 * Returns a minimal set of matrices, indexed by dimension containing interesting
 * float values.
 *
 * This is the matrix analogue of `sparseVectorF64Range`, so it is producing a
 * minimal coverage set of matrices that test all the interesting f64 values.
 * There is not a more expansive set of matrices, since matrices are even more
 * expensive than vectors for increasing runtime with coverage.
 *
 * All the interesting floats from sparseScalarF64 are guaranteed to be tested, but
 * not in every position.
 */
function sparseMatrixF64Range(cols, rows) {
  assert(
    cols === 2 || cols === 3 || cols === 4,
    'sparseMatrixF64Range only accepts column counts of 2, 3, and 4'
  );
  assert(
    rows === 2 || rows === 3 || rows === 4,
    'sparseMatrixF64Range only accepts row counts of 2, 3, and 4'
  );
  return kSparseMatrixF64Values[cols][rows];
}

/**
 * @returns the result matrix in Array<Array<number>> type.
 *
 * Matrix multiplication. A is m x n and B is n x p. Returns
 * m x p result.
 */
// A is m x n. B is n x p. product is m x p.
function multiplyMatrices(
A,
B)
{
  assert(A.length > 0 && B.length > 0 && B[0].length > 0 && A[0].length === B.length);
  const product = new Array(A.length);
  for (let i = 0; i < product.length; ++i) {
    product[i] = new Array(B[0].length).fill(0);
  }

  for (let m = 0; m < A.length; ++m) {
    for (let p = 0; p < B[0].length; ++p) {
      for (let n = 0; n < B.length; ++n) {
        product[m][p] += A[m][n] * B[n][p];
      }
    }
  }

  return product;
}

/** Sign-extend the `bits`-bit number `n` to a 32-bit signed integer. */
function signExtend(n, bits) {
  const shift = 32 - bits;
  return n << shift >> shift;
}





/** @returns the closest 32-bit floating point value to the input */
function quantizeToF32(num) {
  return Math.fround(num);
}

/** @returns the closest 16-bit floating point value to the input */
function quantizeToF16(num) {
  return hfround(num);
}

/**
 * @returns the closest 32-bit signed integer value to the input, rounding
 * towards 0, if not already an integer
 */
function quantizeToI32(num) {
  if (num >= kValue.i32.positive.max) {
    return kValue.i32.positive.max;
  }
  if (num <= kValue.i32.negative.min) {
    return kValue.i32.negative.min;
  }
  return Math.trunc(num);
}

/**
 * @returns the closest 32-bit unsigned integer value to the input, rounding
 * towards 0, if not already an integer
 */
function quantizeToU32(num) {
  if (num >= kValue.u32.max) {
    return kValue.u32.max;
  }
  if (num <= 0) {
    return 0;
  }
  return Math.trunc(num);
}

/**
 * @returns the closest 64-bit signed integer value to the input.
 */
function quantizeToI64(num) {
  if (num >= kValue.i64.positive.max) {
    return kValue.i64.positive.max;
  }
  if (num <= kValue.i64.negative.min) {
    return kValue.i64.negative.min;
  }
  return num;
}

/** @returns whether the number is an integer and a power of two */
function isPowerOfTwo(n) {
  if (!Number.isInteger(n)) {
    return false;
  }
  assert((n | 0) === n, 'isPowerOfTwo only supports 32-bit numbers');
  return n !== 0 && (n & n - 1) === 0;
}

/** @returns the Greatest Common Divisor (GCD) of the inputs */
function gcd(a, b) {
  assert(Number.isInteger(a) && a > 0);
  assert(Number.isInteger(b) && b > 0);

  while (b !== 0) {
    const bTemp = b;
    b = a % b;
    a = bTemp;
  }

  return a;
}

/** @returns the Least Common Multiplier (LCM) of the inputs */
function lcm(a, b) {
  return a * b / gcd(a, b);
}

/** @returns the cross of an array with the intermediate result of cartesianProduct
 *
 * @param elements array of values to cross with the intermediate result of
 *                 cartesianProduct
 * @param intermediate arrays of values representing the partial result of
 *                     cartesianProduct
 */
function cartesianProductImpl(
elements,
intermediate)
{
  const result = [];
  elements.forEach((e) => {
    if (intermediate.length > 0) {
      intermediate.forEach((i) => {
        result.push([...i, e]);
      });
    } else {
      result.push([e]);
    }
  });
  return result;
}

/** @returns the cartesian product (NxMx...) of a set of arrays
 *
 * This is implemented by calculating the cross of a single input against an
 * intermediate result for each input to build up the final array of arrays.
 *
 * There are examples of doing this more succinctly using map & reduce online,
 * but they are a bit more opaque to read.
 *
 * @param inputs arrays of numbers to calculate cartesian product over
 */
function cartesianProduct(...inputs) {
  let result = [];
  inputs.forEach((i) => {
    result = cartesianProductImpl(i, result);
  });

  return result;
}

/** @returns all of the permutations of an array
 *
 * Recursively calculates all of the permutations, does not cull duplicate
 * entries.
 *
 * Only feasible for inputs of lengths 5 or so, since the number of permutations
 * is (input.length)!, so will cause the stack to explode for longer inputs.
 *
 * This code could be made iterative using something like
 * Steinhaus\u2013Johnson\u2013Trotter and additionally turned into a generator to reduce
 * the stack size, but there is still a fundamental combinatorial explosion
 * here that will affect runtime.
 *
 * @param input the array to get permutations of
 */
function calculatePermutations(input) {
  if (input.length === 0) {
    return [];
  }

  if (input.length === 1) {
    return [input];
  }

  if (input.length === 2) {
    return [input, [input[1], input[0]]];
  }

  const result = [];
  input.forEach((head, idx) => {
    const tail = input.slice(0, idx).concat(input.slice(idx + 1));
    const permutations = calculatePermutations(tail);
    permutations.forEach((p) => {
      result.push([head, ...p]);
    });
  });

  return result;
}

/**
 * Convert an Array of Arrays to linear array
 *
 * Caller is responsible to retaining the dimensions of the array for later
 * unflattening
 *
 * @param m Matrix to convert
 */
function flatten2DArray(m) {
  const c = m.length;
  const r = m[0].length;
  assert(
    m.every((c) => c.length === r),
    `Unexpectedly received jagged array to flatten`
  );
  const result = Array(c * r);
  for (let i = 0; i < c; i++) {
    for (let j = 0; j < r; j++) {
      result[j + i * r] = m[i][j];
    }
  }
  return result;
}

/**
 * Convert linear array to an Array of Arrays
 * @param n an array to convert
 * @param c number of elements in the array containing arrays
 * @param r number of elements in the arrays that are contained
 */
function unflatten2DArray(n, c, r) {
  assert(
    c > 0 && Number.isInteger(c) && r > 0 && Number.isInteger(r),
    `columns (${c}) and rows (${r}) need to be positive integers`
  );
  assert(n.length === c * r, `m.length(${n.length}) should equal c * r (${c * r})`);
  const result = [...Array(c)].map((_) => [...Array(r)]);
  for (let i = 0; i < c; i++) {
    for (let j = 0; j < r; j++) {
      result[i][j] = n[j + i * r];
    }
  }
  return result;
}

/**
 * Performs a .map over a matrix and return the result
 * The shape of the input and output matrices will be the same
 *
 * @param m input matrix of type T
 * @param op operation that converts an element of type T to one of type S
 * @returns a matrix with elements of type S that are calculated by applying op element by element
 */
function map2DArray(m, op) {
  const c = m.length;
  const r = m[0].length;
  assert(
    m.every((c) => c.length === r),
    `Unexpectedly received jagged array to map`
  );
  const result = [...Array(c)].map((_) => [...Array(r)]);
  for (let i = 0; i < c; i++) {
    for (let j = 0; j < r; j++) {
      result[i][j] = op(m[i][j]);
    }
  }
  return result;
}

/**
 * Performs a .every over a matrix and return the result
 *
 * @param m input matrix of type T
 * @param op operation that performs a test on an element
 * @returns a boolean indicating if the test passed for every element
 */
function every2DArray(m, op) {
  const r = m[0].length;
  assert(
    m.every((c) => c.length === r),
    `Unexpectedly received jagged array to map`
  );
  return m.every((col) => col.every((el) => op(el)));
}

/**
 * Subtracts 2 vectors
 */
function subtractVectors(v1, v2) {
  return v1.map((v, i) => v - v2[i]);
}

/**
 * Computes the dot product of 2 vectors
 */
function dotProduct(v1, v2) {
  return v1.reduce((a, v, i) => a + v * v2[i], 0);
}

/** @returns the absolute value of a bigint */
function absBigInt(v) {
  return v < 0n ? -v : v;
}

/** @returns the maximum from a list of bigints */
function maxBigInt(...vals) {
  return vals.reduce((prev, cur) => cur > prev ? cur : prev);
}

/** @returns the minimum from a list of bigints */
function minBigInt(...vals) {
  return vals.reduce((prev, cur) => cur < prev ? cur : prev);
}
});
__define('webgpu/util/constants.js', function (__exports) {
'use strict';
__get(__exports, 'kBit', () => kBit);
__get(__exports, 'kValue', () => kValue);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let reinterpretU64AsF64, reinterpretF64AsU64, reinterpretU32AsF32, reinterpretU16AsF16; __bind('webgpu/util/reinterpret.js', (__m) => { ({ reinterpretU64AsF64, reinterpretF64AsU64, reinterpretU32AsF32, reinterpretU16AsF16 } = __m); });

const kBit = {
  // Limits of int32
  i32: {
    positive: {
      min: 0x0000_0000, // 0
      max: 0x7fff_ffff // 2147483647
    },
    negative: {
      min: 0x8000_0000, // -2147483648
      max: 0x0000_0000 // 0
    }
  },

  // Limits of uint32
  u32: {
    min: 0x0000_0000,
    max: 0xffff_ffff
  },

  // Limits of f64
  // Have to be stored as a BigInt hex value, since number is a f64 internally,
  // so 64-bit hex values are not guaranteed to be precisely representable.
  f64: {
    positive: {
      min: BigInt(0x0010_0000_0000_0000n),
      max: BigInt(0x7fef_ffff_ffff_ffffn),
      zero: BigInt(0x0000_0000_0000_0000n),
      subnormal: {
        min: BigInt(0x0000_0000_0000_0001n),
        max: BigInt(0x000f_ffff_ffff_ffffn)
      },
      infinity: BigInt(0x7ff0_0000_0000_0000n),
      nearest_max: BigInt(0x7fef_ffff_ffff_fffen),
      less_than_one: BigInt(0x3fef_ffff_ffff_ffffn),
      pi: {
        whole: BigInt(0x4009_21fb_5444_2d18n),
        three_quarters: BigInt(0x4002_d97c_7f33_21d2n),
        half: BigInt(0x3ff9_21fb_5444_2d18n),
        third: BigInt(0x3ff0_c152_382d_7365n),
        quarter: BigInt(0x3fe9_21fb_5444_2d18n),
        sixth: BigInt(0x3fe0_c152_382d_7365n)
      },
      e: BigInt(0x4005_bf0a_8b14_5769n)
    },
    negative: {
      max: BigInt(0x8010_0000_0000_0000n),
      min: BigInt(0xffef_ffff_ffff_ffffn),
      zero: BigInt(0x8000_0000_0000_0000n),
      subnormal: {
        max: BigInt(0x8000_0000_0000_0001n),
        min: BigInt(0x800f_ffff_ffff_ffffn)
      },
      infinity: BigInt(0xfff0_0000_0000_0000n),
      nearest_min: BigInt(0xffef_ffff_ffff_fffen),
      less_than_one: BigInt(0xbfef_ffff_ffff_ffffn),
      pi: {
        whole: BigInt(0xc009_21fb_5444_2d18n),
        three_quarters: BigInt(0xc002_d97c_7f33_21d2n),
        half: BigInt(0xbff9_21fb_5444_2d18n),
        third: BigInt(0xbff0_c152_382d_7365n),
        quarter: BigInt(0xbfe9_21fb_5444_2d18n),
        sixth: BigInt(0xbfe0_c152_382d_7365n)
      }
    },
    max_ulp: BigInt(0x7ca0_0000_0000_0000n)
  },

  // Limits of f32
  f32: {
    positive: {
      min: 0x0080_0000,
      max: 0x7f7f_ffff,
      zero: 0x0000_0000,
      subnormal: {
        min: 0x0000_0001,
        max: 0x007f_ffff
      },
      infinity: 0x7f80_0000,
      nearest_max: 0x7f7f_fffe,
      less_than_one: 0x3f7f_ffff,
      pi: {
        whole: 0x4049_0fdb,
        three_quarters: 0x4016_cbe4,
        half: 0x3fc9_0fdb,
        third: 0x3f86_0a92,
        quarter: 0x3f49_0fdb,
        sixth: 0x3f06_0a92
      },
      e: 0x402d_f854
    },
    negative: {
      max: 0x8080_0000,
      min: 0xff7f_ffff,
      zero: 0x8000_0000,
      subnormal: {
        max: 0x8000_0001,
        min: 0x807f_ffff
      },
      infinity: 0xff80_0000,
      nearest_min: 0xff7f_fffe,
      less_than_one: 0xbf7f_ffff,
      pi: {
        whole: 0xc04_90fdb,
        three_quarters: 0xc016_cbe4,
        half: 0xbfc9_0fdb,
        third: 0xbf86_0a92,
        quarter: 0xbf49_0fdb,
        sixth: 0xbf06_0a92
      }
    },
    max_ulp: 0x7380_0000
  },

  // Limits of f16
  f16: {
    positive: {
      min: 0x0400,
      max: 0x7bff,
      zero: 0x0000,
      subnormal: {
        min: 0x0001,
        max: 0x03ff
      },
      infinity: 0x7c00,
      nearest_max: 0x7bfe,
      less_than_one: 0x3bff,
      pi: {
        whole: 0x4248,
        three_quarters: 0x40b6,
        half: 0x3e48,
        third: 0x3c30,
        quarter: 0x3a48,
        sixth: 0x3830
      },
      e: 0x416f
    },
    negative: {
      max: 0x8400,
      min: 0xfbff,
      zero: 0x8000,
      subnormal: {
        max: 0x8001,
        min: 0x83ff
      },
      infinity: 0xfc00,
      nearest_min: 0xfbfe,
      less_than_one: 0xbbff,
      pi: {
        whole: 0xc248,
        three_quarters: 0xc0b6,
        half: 0xbe48,
        third: 0xbc30,
        quarter: 0xba48,
        sixth: 0xb830
      }
    },
    max_ulp: 0x5000
  },

  // Uint32 representation of power(2, n) n = {0, ..., 31}
  // Stored as a JS `number`
  // {to0, ..., to31} ie. {0, ..., 31}
  powTwo: {
    to0: 0x0000_0001,
    to1: 0x0000_0002,
    to2: 0x0000_0004,
    to3: 0x0000_0008,
    to4: 0x0000_0010,
    to5: 0x0000_0020,
    to6: 0x0000_0040,
    to7: 0x0000_0080,
    to8: 0x0000_0100,
    to9: 0x0000_0200,
    to10: 0x0000_0400,
    to11: 0x0000_0800,
    to12: 0x0000_1000,
    to13: 0x0000_2000,
    to14: 0x0000_4000,
    to15: 0x0000_8000,
    to16: 0x0001_0000,
    to17: 0x0002_0000,
    to18: 0x0004_0000,
    to19: 0x0008_0000,
    to20: 0x0010_0000,
    to21: 0x0020_0000,
    to22: 0x0040_0000,
    to23: 0x0080_0000,
    to24: 0x0100_0000,
    to25: 0x0200_0000,
    to26: 0x0400_0000,
    to27: 0x0800_0000,
    to28: 0x1000_0000,
    to29: 0x2000_0000,
    to30: 0x4000_0000,
    to31: 0x8000_0000
  },

  // Int32 representation of  of -1 * power(2, n) n = {0, ..., 31}
  // Stored as a JS `number`
  // {to0, ..., to31} ie. {0, ..., 31}
  negPowTwo: {
    to0: 0xffff_ffff,
    to1: 0xffff_fffe,
    to2: 0xffff_fffc,
    to3: 0xffff_fff8,
    to4: 0xffff_fff0,
    to5: 0xffff_ffe0,
    to6: 0xffff_ffc0,
    to7: 0xffff_ff80,
    to8: 0xffff_ff00,
    to9: 0xffff_fe00,
    to10: 0xffff_fc00,
    to11: 0xffff_f800,
    to12: 0xffff_f000,
    to13: 0xffff_e000,
    to14: 0xffff_c000,
    to15: 0xffff_8000,
    to16: 0xffff_0000,
    to17: 0xfffe_0000,
    to18: 0xfffc_0000,
    to19: 0xfff8_0000,
    to20: 0xfff0_0000,
    to21: 0xffe0_0000,
    to22: 0xffc0_0000,
    to23: 0xff80_0000,
    to24: 0xff00_0000,
    to25: 0xfe00_0000,
    to26: 0xfc00_0000,
    to27: 0xf800_0000,
    to28: 0xf000_0000,
    to29: 0xe000_0000,
    to30: 0xc000_0000,
    to31: 0x8000_0000
  }
};

const kValue = {
  // Limits of i64
  i64: {
    positive: {
      min: BigInt(0n),
      max: BigInt(9223372036854775807n)
    },
    negative: {
      min: BigInt(-9223372036854775808n),
      max: BigInt(0n)
    },
    isOOB: (val) => {
      return val > kValue.i64.positive.max || val < kValue.i64.negative.min;
    }
  },

  // Limits of i32
  i32: {
    positive: {
      min: 0,
      max: 2147483647
    },
    negative: {
      min: -2147483648,
      max: 0
    }
  },

  // Limits of u32
  u32: {
    min: 0,
    max: 4294967295
  },

  // Limits of f64
  f64: {
    positive: {
      min: reinterpretU64AsF64(kBit.f64.positive.min),
      max: reinterpretU64AsF64(kBit.f64.positive.max),
      zero: reinterpretU64AsF64(kBit.f64.positive.zero),
      subnormal: {
        min: reinterpretU64AsF64(kBit.f64.positive.subnormal.min),
        max: reinterpretU64AsF64(kBit.f64.positive.subnormal.max)
      },
      infinity: reinterpretU64AsF64(kBit.f64.positive.infinity),
      nearest_max: reinterpretU64AsF64(kBit.f64.positive.nearest_max),
      less_than_one: reinterpretU64AsF64(kBit.f64.positive.less_than_one),
      pi: {
        whole: reinterpretU64AsF64(kBit.f64.positive.pi.whole),
        three_quarters: reinterpretU64AsF64(kBit.f64.positive.pi.three_quarters),
        half: reinterpretU64AsF64(kBit.f64.positive.pi.half),
        third: reinterpretU64AsF64(kBit.f64.positive.pi.third),
        quarter: reinterpretU64AsF64(kBit.f64.positive.pi.quarter),
        sixth: reinterpretU64AsF64(kBit.f64.positive.pi.sixth)
      },
      e: reinterpretU64AsF64(kBit.f64.positive.e)
    },
    negative: {
      max: reinterpretU64AsF64(kBit.f64.negative.max),
      min: reinterpretU64AsF64(kBit.f64.negative.min),
      zero: reinterpretU64AsF64(kBit.f64.negative.zero),
      subnormal: {
        max: reinterpretU64AsF64(kBit.f64.negative.subnormal.max),
        min: reinterpretU64AsF64(kBit.f64.negative.subnormal.min)
      },
      infinity: reinterpretU64AsF64(kBit.f64.negative.infinity),
      nearest_min: reinterpretU64AsF64(kBit.f64.negative.nearest_min),
      less_than_one: reinterpretU64AsF64(kBit.f64.negative.less_than_one), // -0.999999940395
      pi: {
        whole: reinterpretU64AsF64(kBit.f64.negative.pi.whole),
        three_quarters: reinterpretU64AsF64(kBit.f64.negative.pi.three_quarters),
        half: reinterpretU64AsF64(kBit.f64.negative.pi.half),
        third: reinterpretU64AsF64(kBit.f64.negative.pi.third),
        quarter: reinterpretU64AsF64(kBit.f64.negative.pi.quarter),
        sixth: reinterpretU64AsF64(kBit.f64.negative.pi.sixth)
      }
    },
    max_ulp: reinterpretU64AsF64(kBit.f64.max_ulp)
  },

  // Limits of f32
  f32: {
    positive: {
      min: reinterpretU32AsF32(kBit.f32.positive.min),
      max: reinterpretU32AsF32(kBit.f32.positive.max),
      zero: reinterpretU32AsF32(kBit.f32.positive.zero),
      subnormal: {
        min: reinterpretU32AsF32(kBit.f32.positive.subnormal.min),
        max: reinterpretU32AsF32(kBit.f32.positive.subnormal.max)
      },
      infinity: reinterpretU32AsF32(kBit.f32.positive.infinity),

      nearest_max: reinterpretU32AsF32(kBit.f32.positive.nearest_max),
      less_than_one: reinterpretU32AsF32(kBit.f32.positive.less_than_one),
      pi: {
        whole: reinterpretU32AsF32(kBit.f32.positive.pi.whole),
        three_quarters: reinterpretU32AsF32(kBit.f32.positive.pi.three_quarters),
        half: reinterpretU32AsF32(kBit.f32.positive.pi.half),
        third: reinterpretU32AsF32(kBit.f32.positive.pi.third),
        quarter: reinterpretU32AsF32(kBit.f32.positive.pi.quarter),
        sixth: reinterpretU32AsF32(kBit.f32.positive.pi.sixth)
      },
      e: reinterpretU32AsF32(kBit.f32.positive.e),
      // The positive pipeline-overridable constant with the smallest magnitude
      // which when cast to f32 will produce infinity. This comes from WGSL
      // conversion rules and the rounding rules of WebIDL.
      first_non_castable_pipeline_override:
      reinterpretU32AsF32(kBit.f32.positive.max) / 2 + 2 ** 127,
      // The positive pipeline-overridable constant with the largest magnitude
      // which when cast to f32 will not produce infinity. This comes from WGSL
      // conversion rules and the rounding rules of WebIDL
      last_castable_pipeline_override: reinterpretU64AsF64(
        reinterpretF64AsU64(reinterpretU32AsF32(kBit.f32.positive.max) / 2 + 2 ** 127) - BigInt(1)
      )
    },
    negative: {
      max: reinterpretU32AsF32(kBit.f32.negative.max),
      min: reinterpretU32AsF32(kBit.f32.negative.min),
      zero: reinterpretU32AsF32(kBit.f32.negative.zero),
      subnormal: {
        max: reinterpretU32AsF32(kBit.f32.negative.subnormal.max),
        min: reinterpretU32AsF32(kBit.f32.negative.subnormal.min)
      },
      infinity: reinterpretU32AsF32(kBit.f32.negative.infinity),
      nearest_min: reinterpretU32AsF32(kBit.f32.negative.nearest_min),
      less_than_one: reinterpretU32AsF32(kBit.f32.negative.less_than_one), // -0.999999940395
      pi: {
        whole: reinterpretU32AsF32(kBit.f32.negative.pi.whole),
        three_quarters: reinterpretU32AsF32(kBit.f32.negative.pi.three_quarters),
        half: reinterpretU32AsF32(kBit.f32.negative.pi.half),
        third: reinterpretU32AsF32(kBit.f32.negative.pi.third),
        quarter: reinterpretU32AsF32(kBit.f32.negative.pi.quarter),
        sixth: reinterpretU32AsF32(kBit.f32.negative.pi.sixth)
      },
      // The negative pipeline-overridable constant with the smallest magnitude
      // which when cast to f32 will produce infinity. This comes from WGSL
      // conversion rules and the rounding rules of WebIDL.
      first_non_castable_pipeline_override: -(
      reinterpretU32AsF32(kBit.f32.positive.max) / 2 +
      2 ** 127),

      // The negative pipeline-overridable constant with the largest magnitude
      // which when cast to f32 will not produce infinity. This comes from WGSL
      // conversion rules and the rounding rules of WebIDL.
      last_castable_pipeline_override: -reinterpretU64AsF64(
        reinterpretF64AsU64(reinterpretU32AsF32(kBit.f32.positive.max) / 2 + 2 ** 127) - BigInt(1)
      )
    },
    max_ulp: reinterpretU32AsF32(kBit.f32.max_ulp),
    emax: 127
  },

  // Limits of i16
  i16: {
    positive: {
      min: 0,
      max: 32767
    },
    negative: {
      min: -32768,
      max: 0
    }
  },

  // Limits of u16
  u16: {
    min: 0,
    max: 65535
  },

  // Limits of f16
  f16: {
    positive: {
      min: reinterpretU16AsF16(kBit.f16.positive.min),
      max: reinterpretU16AsF16(kBit.f16.positive.max),
      zero: reinterpretU16AsF16(kBit.f16.positive.zero),
      subnormal: {
        min: reinterpretU16AsF16(kBit.f16.positive.subnormal.min),
        max: reinterpretU16AsF16(kBit.f16.positive.subnormal.max)
      },
      infinity: reinterpretU16AsF16(kBit.f16.positive.infinity),
      nearest_max: reinterpretU16AsF16(kBit.f16.positive.nearest_max),
      less_than_one: reinterpretU16AsF16(kBit.f16.positive.less_than_one),
      pi: {
        whole: reinterpretU16AsF16(kBit.f16.positive.pi.whole),
        three_quarters: reinterpretU16AsF16(kBit.f16.positive.pi.three_quarters),
        half: reinterpretU16AsF16(kBit.f16.positive.pi.half),
        third: reinterpretU16AsF16(kBit.f16.positive.pi.third),
        quarter: reinterpretU16AsF16(kBit.f16.positive.pi.quarter),
        sixth: reinterpretU16AsF16(kBit.f16.positive.pi.sixth)
      },
      e: reinterpretU16AsF16(kBit.f16.positive.e),
      // The positive pipeline-overridable constant with the smallest magnitude
      // which when cast to f16 will produce infinity. This comes from WGSL
      // conversion rules and the rounding rules of WebIDL.
      first_non_castable_pipeline_override:
      reinterpretU16AsF16(kBit.f16.positive.max) / 2 + 2 ** 15,
      // The positive pipeline-overridable constant with the largest magnitude
      // which when cast to f16 will not produce infinity. This comes from WGSL
      // conversion rules and the rounding rules of WebIDL
      last_castable_pipeline_override: reinterpretU64AsF64(
        reinterpretF64AsU64(reinterpretU16AsF16(kBit.f16.positive.max) / 2 + 2 ** 15) - BigInt(1)
      )
    },
    negative: {
      max: reinterpretU16AsF16(kBit.f16.negative.max),
      min: reinterpretU16AsF16(kBit.f16.negative.min),
      zero: reinterpretU16AsF16(kBit.f16.negative.zero),
      subnormal: {
        max: reinterpretU16AsF16(kBit.f16.negative.subnormal.max),
        min: reinterpretU16AsF16(kBit.f16.negative.subnormal.min)
      },
      infinity: reinterpretU16AsF16(kBit.f16.negative.infinity),
      nearest_min: reinterpretU16AsF16(kBit.f16.negative.nearest_min),
      less_than_one: reinterpretU16AsF16(kBit.f16.negative.less_than_one), // -0.9996
      pi: {
        whole: reinterpretU16AsF16(kBit.f16.negative.pi.whole),
        three_quarters: reinterpretU16AsF16(kBit.f16.negative.pi.three_quarters),
        half: reinterpretU16AsF16(kBit.f16.negative.pi.half),
        third: reinterpretU16AsF16(kBit.f16.negative.pi.third),
        quarter: reinterpretU16AsF16(kBit.f16.negative.pi.quarter),
        sixth: reinterpretU16AsF16(kBit.f16.negative.pi.sixth)
      },
      // The negative pipeline-overridable constant with the smallest magnitude
      // which when cast to f16 will produce infinity. This comes from WGSL
      // conversion rules and the rounding rules of WebIDL.
      first_non_castable_pipeline_override: -(
      reinterpretU16AsF16(kBit.f16.positive.max) / 2 +
      2 ** 15),

      // The negative pipeline-overridable constant with the largest magnitude
      // which when cast to f16 will not produce infinity. This comes from WGSL
      // conversion rules and the rounding rules of WebIDL.
      last_castable_pipeline_override: -reinterpretU64AsF64(
        reinterpretF64AsU64(reinterpretU16AsF16(kBit.f16.positive.max) / 2 + 2 ** 15) - BigInt(1)
      )
    },
    max_ulp: reinterpretU16AsF16(kBit.f16.max_ulp),
    emax: 15
  },

  // Limits of i8
  i8: {
    positive: {
      min: 0,
      max: 127
    },
    negative: {
      min: -128,
      max: 0
    }
  },

  // Limits of u8
  u8: {
    min: 0,
    max: 255
  }
};
});
__define('webgpu/util/reinterpret.js', function (__exports) {
'use strict';
__get(__exports, 'reinterpretU64AsF64', () => reinterpretU64AsF64);
__get(__exports, 'reinterpretF64AsU64', () => reinterpretF64AsU64);
__get(__exports, 'reinterpretF64AsU32s', () => reinterpretF64AsU32s);
__get(__exports, 'reinterpretU32sAsF64', () => reinterpretU32sAsF64);
__get(__exports, 'reinterpretF32AsU32', () => reinterpretF32AsU32);
__get(__exports, 'reinterpretF32AsI32', () => reinterpretF32AsI32);
__get(__exports, 'reinterpretU32AsF32', () => reinterpretU32AsF32);
__get(__exports, 'reinterpretU32AsI32', () => reinterpretU32AsI32);
__get(__exports, 'reinterpretI32AsU32', () => reinterpretI32AsU32);
__get(__exports, 'reinterpretI32AsF32', () => reinterpretI32AsF32);
__get(__exports, 'reinterpretF16AsU16', () => reinterpretF16AsU16);
__get(__exports, 'reinterpretU16AsF16', () => reinterpretU16AsF16);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let Float16Array; __bind('external/petamoriken/float16/float16.js', (__m) => { ({ Float16Array } = __m); }); /**
 * Once-allocated ArrayBuffer/views to avoid overhead of allocation when converting between numeric formats
 *
 * workingData* is shared between multiple functions in this file, so to avoid re-entrancy problems, make sure in
 * functions that use it that they don't call themselves or other functions that use workingData*.
 */
const workingData = new ArrayBuffer(8);
const workingDataU32 = new Uint32Array(workingData);
const workingDataU16 = new Uint16Array(workingData);
const workingDataF32 = new Float32Array(workingData);
const workingDataF16 = new Float16Array(workingData);
const workingDataI32 = new Int32Array(workingData);
const workingDataF64 = new Float64Array(workingData);
const workingDataU64 = new BigUint64Array(workingData);

/**
 * @returns a 64-bit float value via interpreting the input as the bit
 * representation as a 64-bit integer
 */
function reinterpretU64AsF64(input) {
  workingDataU64[0] = input;
  return workingDataF64[0];
}

/**
 * @returns the 64-bit integer bit representation of 64-bit float value
 */
function reinterpretF64AsU64(input) {
  workingDataF64[0] = input;
  return workingDataU64[0];
}

// Encoding to u32s, instead of BigInt, for serialization
function reinterpretF64AsU32s(f64) {
  workingDataF64[0] = f64;
  return [workingDataU32[0], workingDataU32[1]];
}

// De-encoding from u32s, instead of BigInt, for serialization
function reinterpretU32sAsF64(u32s) {
  workingDataU32[0] = u32s[0];
  workingDataU32[1] = u32s[1];
  return workingDataF64[0];
}

/**
 * @returns a number representing the u32 interpretation
 * of the bits of a number assumed to be an f32 value.
 */
function reinterpretF32AsU32(f32) {
  workingDataF32[0] = f32;
  return workingDataU32[0];
}

/**
 * @returns a number representing the i32 interpretation
 * of the bits of a number assumed to be an f32 value.
 */
function reinterpretF32AsI32(f32) {
  workingDataF32[0] = f32;
  return workingDataI32[0];
}

/**
 * @returns a number representing the f32 interpretation
 * of the bits of a number assumed to be an u32 value.
 */
function reinterpretU32AsF32(u32) {
  workingDataU32[0] = u32;
  return workingDataF32[0];
}

/**
 * @returns a number representing the i32 interpretation
 * of the bits of a number assumed to be an u32 value.
 */
function reinterpretU32AsI32(u32) {
  workingDataU32[0] = u32;
  return workingDataI32[0];
}

/**
 * @returns a number representing the u32 interpretation
 * of the bits of a number assumed to be an i32 value.
 */
function reinterpretI32AsU32(i32) {
  workingDataI32[0] = i32;
  return workingDataU32[0];
}

/**
 * @returns a number representing the f32 interpretation
 * of the bits of a number assumed to be an i32 value.
 */
function reinterpretI32AsF32(i32) {
  workingDataI32[0] = i32;
  return workingDataF32[0];
}

/**
 * @returns a number representing the u16 interpretation
 * of the bits of a number assumed to be an f16 value.
 */
function reinterpretF16AsU16(f16) {
  workingDataF16[0] = f16;
  return workingDataU16[0];
}

/**
 * @returns a number representing the f16 interpretation
 * of the bits of a number assumed to be an u16 value.
 */
function reinterpretU16AsF16(u16) {
  workingDataU16[0] = u16;
  return workingDataF16[0];
}
});
__define('webgpu/api/operation/buffers/map.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Test the operation of buffer mapping, specifically the data contents written via
map-write/mappedAtCreation, and the contents of buffers returned by getMappedRange on
buffers which are mapped-read/mapped-write/mappedAtCreation.

range: used for getMappedRange
mapRegion: used for mapAsync

mapRegionBoundModes is used to get mapRegion from range:
 - default-expand: expand mapRegion to buffer bound by setting offset/size to undefined
 - explicit-expand: expand mapRegion to buffer bound by explicitly calculating offset/size
 - minimal: make mapRegion to be the same as range which is the minimal range to make getMappedRange input valid
`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let assert, memcpy; __bind('common/util/util.js', (__m) => { ({ assert, memcpy } = __m); });
let checkElementsEqual; __bind('webgpu/util/check_contents.js', (__m) => { ({ checkElementsEqual } = __m); });

let MappingTest; __bind('webgpu/api/operation/buffers/mapping_test.js', (__m) => { ({ MappingTest } = __m); });

const g = makeTestGroup(MappingTest);

const kSubcases = [
{ size: 0, range: [] },
{ size: 0, range: [undefined] },
{ size: 0, range: [undefined, undefined] },
{ size: 0, range: [0] },
{ size: 0, range: [0, undefined] },
{ size: 0, range: [0, 0] },
{ size: 12, range: [] },
{ size: 12, range: [undefined] },
{ size: 12, range: [undefined, undefined] },
{ size: 12, range: [0] },
{ size: 12, range: [0, undefined] },
{ size: 12, range: [0, 12] },
{ size: 12, range: [0, 0] },
{ size: 12, range: [8] },
{ size: 12, range: [8, undefined] },
{ size: 12, range: [8, 4] },
{ size: 28, range: [8, 8] },
{ size: 28, range: [8, 12] },
{ size: 512 * 1024, range: [] }];


function reifyMapRange(bufferSize, range) {
  const offset = range[0] ?? 0;
  return [offset, range[1] ?? bufferSize - offset];
}

const mapRegionBoundModes = ['default-expand', 'explicit-expand', 'minimal'];


function getRegionForMap(
bufferSize,
range,
{
  mapAsyncRegionLeft,
  mapAsyncRegionRight



})
{
  const regionLeft = mapAsyncRegionLeft === 'minimal' ? range[0] : 0;
  const regionRight = mapAsyncRegionRight === 'minimal' ? range[0] + range[1] : bufferSize;
  return [
  mapAsyncRegionLeft === 'default-expand' ? undefined : regionLeft,
  mapAsyncRegionRight === 'default-expand' ? undefined : regionRight - regionLeft];

}

g.test('mapAsync,write').
desc(
  `Use map-write to write to various ranges of variously-sized buffers, then expectContents
(which does copyBufferToBuffer + map-read) to ensure the contents were written.`
).
params((u) =>
u.
combine('mapAsyncRegionLeft', mapRegionBoundModes).
combine('mapAsyncRegionRight', mapRegionBoundModes).
beginSubcases().
combineWithParams(kSubcases)
).
fn(async (t) => {
  const { size, range } = t.params;
  const [rangeOffset, rangeSize] = reifyMapRange(size, range);

  const buffer = t.createBufferTracked({
    size,
    usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.MAP_WRITE
  });

  const mapRegion = getRegionForMap(size, [rangeOffset, rangeSize], t.params);
  await buffer.mapAsync(GPUMapMode.WRITE, ...mapRegion);
  const arrayBuffer = buffer.getMappedRange(...range);
  t.checkMapWrite(buffer, rangeOffset, arrayBuffer, rangeSize);
});

g.test('mapAsync,write,unchanged_ranges_preserved').
desc(
  `Use mappedAtCreation or mapAsync to write to various ranges of variously-sized buffers, then
use mapAsync to map a different range and zero it out. Finally use expectGPUBufferValuesEqual
(which does copyBufferToBuffer + map-read) to verify that contents originally written outside the
second mapped range were not altered.`
).
params((u) =>
u.
beginSubcases().
combine('mappedAtCreation', [false, true]).
combineWithParams([
{ size: 12, range1: [], range2: [8] },
{ size: 12, range1: [], range2: [0, 8] },
{ size: 12, range1: [0, 8], range2: [8] },
{ size: 12, range1: [8], range2: [0, 8] },
{ size: 28, range1: [], range2: [8, 8] },
{ size: 28, range1: [8, 16], range2: [16, 8] },
{ size: 32, range1: [16, 12], range2: [8, 16] },
{ size: 32, range1: [8, 8], range2: [24, 4] }]
)
).
fn(async (t) => {
  const { size, range1, range2, mappedAtCreation } = t.params;
  const [rangeOffset1, rangeSize1] = reifyMapRange(size, range1);
  const [rangeOffset2, rangeSize2] = reifyMapRange(size, range2);

  const buffer = t.createBufferTracked({
    mappedAtCreation,
    size,
    usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.MAP_WRITE
  });

  // If the buffer is not mappedAtCreation map it now.
  if (!mappedAtCreation) {
    await buffer.mapAsync(GPUMapMode.WRITE);
  }

  // Set the initial contents of the buffer.
  const init = buffer.getMappedRange(...range1);

  assert(init.byteLength === rangeSize1);
  const expectedBuffer = new ArrayBuffer(size);
  const expected = new Uint32Array(
    expectedBuffer,
    rangeOffset1,
    rangeSize1 / Uint32Array.BYTES_PER_ELEMENT
  );
  const data = new Uint32Array(init);
  for (let i = 0; i < data.length; ++i) {
    data[i] = expected[i] = i + 1;
  }
  buffer.unmap();

  // Write to a second range of the buffer
  await buffer.mapAsync(GPUMapMode.WRITE, ...range2);
  const init2 = buffer.getMappedRange(...range2);

  assert(init2.byteLength === rangeSize2);
  const expected2 = new Uint32Array(
    expectedBuffer,
    rangeOffset2,
    rangeSize2 / Uint32Array.BYTES_PER_ELEMENT
  );
  const data2 = new Uint32Array(init2);
  for (let i = 0; i < data2.length; ++i) {
    data2[i] = expected2[i] = 0;
  }
  buffer.unmap();

  // Verify that the range of the buffer which was not overwritten was preserved.
  t.expectGPUBufferValuesEqual(buffer, expected, rangeOffset1);
});

g.test('mapAsync,read').
desc(
  `Use mappedAtCreation to initialize various ranges of variously-sized buffers, then
map-read and check the read-back result.`
).
params((u) =>
u.
combine('mapAsyncRegionLeft', mapRegionBoundModes).
combine('mapAsyncRegionRight', mapRegionBoundModes).
beginSubcases().
combineWithParams(kSubcases)
).
fn(async (t) => {
  const { size, range } = t.params;
  const [rangeOffset, rangeSize] = reifyMapRange(size, range);

  const buffer = t.createBufferTracked({
    mappedAtCreation: true,
    size,
    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ
  });
  const init = buffer.getMappedRange(...range);

  assert(init.byteLength === rangeSize);
  const expected = new Uint32Array(new ArrayBuffer(rangeSize));
  const data = new Uint32Array(init);
  for (let i = 0; i < data.length; ++i) {
    data[i] = expected[i] = i + 1;
  }
  buffer.unmap();

  const mapRegion = getRegionForMap(size, [rangeOffset, rangeSize], t.params);
  await buffer.mapAsync(GPUMapMode.READ, ...mapRegion);
  const actual = new Uint8Array(buffer.getMappedRange(...range));
  t.expectOK(checkElementsEqual(actual, new Uint8Array(expected.buffer)));
});

g.test('mapAsync,read,typedArrayAccess').
desc(`Use various TypedArray types to read back from a mapped buffer`).
params((u) =>
u.
combine('mapAsyncRegionLeft', mapRegionBoundModes).
combine('mapAsyncRegionRight', mapRegionBoundModes).
beginSubcases().
combineWithParams([
{ size: 80, range: [] },
{ size: 160, range: [] },
{ size: 160, range: [0, 80] },
{ size: 160, range: [80] },
{ size: 160, range: [40, 120] },
{ size: 160, range: [40] }]
)
).
fn(async (t) => {
  const { size, range } = t.params;
  const [rangeOffset, rangeSize] = reifyMapRange(size, range);

  // Fill an array buffer with a variety of values of different types.
  const expectedArrayBuffer = new ArrayBuffer(80);
  const uint8Expected = new Uint8Array(expectedArrayBuffer, 0, 2);
  uint8Expected[0] = 1;
  uint8Expected[1] = 255;

  const int8Expected = new Int8Array(expectedArrayBuffer, 2, 2);
  int8Expected[0] = -1;
  int8Expected[1] = 127;

  const uint16Expected = new Uint16Array(expectedArrayBuffer, 4, 2);
  uint16Expected[0] = 1;
  uint16Expected[1] = 65535;

  const int16Expected = new Int16Array(expectedArrayBuffer, 8, 2);
  int16Expected[0] = -1;
  int16Expected[1] = 32767;

  const uint32Expected = new Uint32Array(expectedArrayBuffer, 12, 2);
  uint32Expected[0] = 1;
  uint32Expected[1] = 4294967295;

  const int32Expected = new Int32Array(expectedArrayBuffer, 20, 2);
  int32Expected[2] = -1;
  int32Expected[3] = 2147483647;

  const float32Expected = new Float32Array(expectedArrayBuffer, 28, 3);
  float32Expected[0] = 1;
  float32Expected[1] = -1;
  float32Expected[2] = 12345.6789;

  const float64Expected = new Float64Array(expectedArrayBuffer, 40, 5);
  float64Expected[0] = 1;
  float64Expected[1] = -1;
  float64Expected[2] = 12345.6789;
  float64Expected[3] = Number.MAX_VALUE;
  float64Expected[4] = Number.MIN_VALUE;

  const buffer = t.createBufferTracked({
    mappedAtCreation: true,
    size,
    usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ
  });
  const init = buffer.getMappedRange(...range);

  // Copy the expected values into the mapped range.
  assert(init.byteLength === rangeSize);
  memcpy({ src: expectedArrayBuffer }, { dst: init });
  buffer.unmap();

  const mapRegion = getRegionForMap(size, [rangeOffset, rangeSize], t.params);
  await buffer.mapAsync(GPUMapMode.READ, ...mapRegion);
  const mappedArrayBuffer = buffer.getMappedRange(...range);
  t.expectOK(checkElementsEqual(new Uint8Array(mappedArrayBuffer, 0, 2), uint8Expected));
  t.expectOK(checkElementsEqual(new Int8Array(mappedArrayBuffer, 2, 2), int8Expected));
  t.expectOK(checkElementsEqual(new Uint16Array(mappedArrayBuffer, 4, 2), uint16Expected));
  t.expectOK(checkElementsEqual(new Int16Array(mappedArrayBuffer, 8, 2), int16Expected));
  t.expectOK(checkElementsEqual(new Uint32Array(mappedArrayBuffer, 12, 2), uint32Expected));
  t.expectOK(checkElementsEqual(new Int32Array(mappedArrayBuffer, 20, 2), int32Expected));
  t.expectOK(checkElementsEqual(new Float32Array(mappedArrayBuffer, 28, 3), float32Expected));
  t.expectOK(checkElementsEqual(new Float64Array(mappedArrayBuffer, 40, 5), float64Expected));
});

g.test('mappedAtCreation').
desc(
  `Use mappedAtCreation to write to various ranges of variously-sized buffers created either
with or without the MAP_WRITE usage (since this could affect the mappedAtCreation upload path),
then expectContents (which does copyBufferToBuffer + map-read) to ensure the contents were written.`
).
params((u) =>
u //
.combine('mappable', [false, true]).
beginSubcases().
combineWithParams(kSubcases)
).
fn((t) => {
  const { size, range, mappable } = t.params;
  const [, rangeSize] = reifyMapRange(size, range);

  const buffer = t.createBufferTracked({
    mappedAtCreation: true,
    size,
    usage: GPUBufferUsage.COPY_SRC | (mappable ? GPUBufferUsage.MAP_WRITE : 0)
  });
  const arrayBuffer = buffer.getMappedRange(...range);
  t.checkMapWrite(buffer, range[0] ?? 0, arrayBuffer, rangeSize);
});

g.test('remapped_for_write').
desc(
  `Use mappedAtCreation or mapAsync to write to various ranges of variously-sized buffers created
with the MAP_WRITE usage, then mapAsync again and ensure that the previously written values are
still present in the mapped buffer.`
).
params((u) =>
u //
.combine('mapAsyncRegionLeft', mapRegionBoundModes).
combine('mapAsyncRegionRight', mapRegionBoundModes).
beginSubcases().
combine('mappedAtCreation', [false, true]).
combineWithParams(kSubcases)
).
fn(async (t) => {
  const { size, range, mappedAtCreation } = t.params;
  const [rangeOffset, rangeSize] = reifyMapRange(size, range);

  const buffer = t.createBufferTracked({
    mappedAtCreation,
    size,
    usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.MAP_WRITE
  });

  // If the buffer is not mappedAtCreation map it now.
  if (!mappedAtCreation) {
    await buffer.mapAsync(GPUMapMode.WRITE);
  }

  // Set the initial contents of the buffer.
  const init = buffer.getMappedRange(...range);

  assert(init.byteLength === rangeSize);
  const expected = new Uint32Array(new ArrayBuffer(rangeSize));
  const data = new Uint32Array(init);
  for (let i = 0; i < data.length; ++i) {
    data[i] = expected[i] = i + 1;
  }
  buffer.unmap();

  // Check that upon remapping the for WRITE the values in the buffer are
  // still the same.
  const mapRegion = getRegionForMap(size, [rangeOffset, rangeSize], t.params);
  await buffer.mapAsync(GPUMapMode.WRITE, ...mapRegion);
  const actual = new Uint8Array(buffer.getMappedRange(...range));
  t.expectOK(checkElementsEqual(actual, new Uint8Array(expected.buffer)));
});

g.test('mappedAtCreation,mapState').
desc('Test that exposed map state of buffer created with mappedAtCreation has expected values.').
params((u) =>
u.
combine('usageType', ['invalid', 'read', 'write']).
combine('afterUnmap', [false, true]).
combine('afterDestroy', [false, true])
).
fn((t) => {
  const { usageType, afterUnmap, afterDestroy } = t.params;
  const usage =
  usageType === 'read' ?
  GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ :
  usageType === 'write' ?
  GPUBufferUsage.COPY_SRC | GPUBufferUsage.MAP_WRITE :
  0;
  const validationError = usage === 0;
  const size = 8;
  const range = [0, 8];

  let buffer;
  t.expectValidationError(() => {
    buffer = t.createBufferTracked({
      mappedAtCreation: true,
      size,
      usage
    });
  }, validationError);

  // mapState must be "mapped" regardless of validation error
  t.expect(buffer.mapState === 'mapped');

  // getMappedRange must not change the map state
  buffer.getMappedRange(...range);
  t.expect(buffer.mapState === 'mapped');

  if (afterUnmap) {
    buffer.unmap();
    t.expect(buffer.mapState === 'unmapped');
  }

  if (afterDestroy) {
    buffer.destroy();
    t.expect(buffer.mapState === 'unmapped');
  }
});

g.test('mapAsync,mapState').
desc('Test that exposed map state of buffer mapped with mapAsync has expected values.').
params((u) =>
u.
combine('usageType', ['invalid', 'read', 'write']).
combine('mapModeType', ['READ', 'WRITE']).
combine('beforeUnmap', [false, true]).
combine('beforeDestroy', [false, true]).
combine('afterUnmap', [false, true]).
combine('afterDestroy', [false, true])
).
fn(async (t) => {
  const { usageType, mapModeType, beforeUnmap, beforeDestroy, afterUnmap, afterDestroy } =
  t.params;
  const size = 8;
  const range = [0, 8];
  const usage =
  usageType === 'read' ?
  GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ :
  usageType === 'write' ?
  GPUBufferUsage.COPY_SRC | GPUBufferUsage.MAP_WRITE :
  0;
  const bufferCreationValidationError = usage === 0;
  const mapMode = GPUMapMode[mapModeType];

  let buffer;
  t.expectValidationError(() => {
    buffer = t.createBufferTracked({
      mappedAtCreation: false,
      size,
      usage
    });
  }, bufferCreationValidationError);

  t.expect(buffer.mapState === 'unmapped');

  {
    const mapAsyncValidationError =
    bufferCreationValidationError ||
    mapMode === GPUMapMode.READ && !(usage & GPUBufferUsage.MAP_READ) ||
    mapMode === GPUMapMode.WRITE && !(usage & GPUBufferUsage.MAP_WRITE);
    let promise;
    t.expectValidationError(() => {
      promise = buffer.mapAsync(mapMode);
    }, mapAsyncValidationError);
    t.expect(buffer.mapState === 'pending');

    try {
      if (beforeUnmap) {
        buffer.unmap();
        t.expect(buffer.mapState === 'unmapped');
      }
      if (beforeDestroy) {
        buffer.destroy();
        t.expect(buffer.mapState === 'unmapped');
      }

      await promise;
      t.expect(buffer.mapState === 'mapped');

      // getMappedRange must not change the map state
      buffer.getMappedRange(...range);
      t.expect(buffer.mapState === 'mapped');
    } catch {
      // unmapped before resolve, destroyed before resolve, or mapAsync validation error
      // will end up with rejection and 'unmapped'
      t.expect(buffer.mapState === 'unmapped');
    }
  }

  // If buffer is already mapped test mapAsync on already mapped buffer
  if (buffer.mapState === 'mapped') {
    // mapAsync on already mapped buffer must be rejected with a validation error
    // and the map state must keep 'mapped'
    let promise;
    t.expectValidationError(() => {
      promise = buffer.mapAsync(GPUMapMode.WRITE);
    }, true);
    t.expect(buffer.mapState === 'mapped');

    try {
      await promise;
      t.fail('mapAsync on already mapped buffer must not succeed.');
    } catch {
      t.expect(buffer.mapState === 'mapped');
    }
  }

  if (afterUnmap) {
    buffer.unmap();
    t.expect(buffer.mapState === 'unmapped');
  }

  if (afterDestroy) {
    buffer.destroy();
    t.expect(buffer.mapState === 'unmapped');
  }
});
});
__define('webgpu/util/check_contents.js', function (__exports) {
'use strict';
__get(__exports, 'checkElementsEqual', () => checkElementsEqual);
__get(__exports, 'checkElementsBetween', () => checkElementsBetween);
__get(__exports, 'checkElementsEqualEither', () => checkElementsEqualEither);
__get(__exports, 'checkElementsEqualGenerated', () => checkElementsEqualGenerated);
__get(__exports, 'checkElementsPassPredicate', () => checkElementsPassPredicate);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ // MAINTENANCE_TODO: The "checkThingTrue" naming is confusing; these must be used with `expectOK`
// or the result is dropped on the floor. Rename these to things like `typedArrayIsOK`(??) to
// make it clearer.
// MAINTENANCE_TODO: Also, audit to make sure we aren't dropping any on the floor. Consider a
// no-ignored-return lint check if we can find one that we can use.
let assert, ErrorWithExtra, iterRange, range; __bind('common/util/util.js', (__m) => { ({ assert, ErrorWithExtra, iterRange, range } = __m); });
let Float16Array; __bind('external/petamoriken/float16/float16.js', (__m) => { ({ Float16Array } = __m); });

let generatePrettyTable; __bind('webgpu/util/pretty_diff_tables.js', (__m) => { ({ generatePrettyTable } = __m); });

/** Generate an expected value at `index`, to test for equality with the actual value. */

/** Check whether the actual `value` at `index` is as expected. */

/**
 * Provides a pretty-printing implementation for a particular CheckElementsPredicate.
 * This is an array; each element provides info to print an additional row in the error message.
 */










/**
 * Check whether two `TypedArray`s have equal contents.
 * Returns `undefined` if the check passes, or an `Error` if not.
 */
function checkElementsEqual(
actual,
expected)
{
  assert(actual.constructor === expected.constructor, 'TypedArray type mismatch');
  assert(
    actual.length === expected.length,
    `length mismatch: expected ${expected.length} got ${actual.length}`
  );

  let failedElementsFirstMaybe = undefined;
  /** Sparse array with `true` for elements that failed. */
  const failedElements = [];
  for (let i = 0; i < actual.length; ++i) {
    if (actual[i] !== expected[i]) {
      failedElementsFirstMaybe ??= i;
      failedElements[i] = true;
    }
  }

  if (failedElementsFirstMaybe === undefined) {
    return undefined;
  }

  const failedElementsFirst = failedElementsFirstMaybe;
  return failCheckElements({
    actual,
    failedElements,
    failedElementsFirst,
    predicatePrinter: [{ leftHeader: 'expected ==', getValueForCell: (index) => expected[index] }]
  });
}

/**
 * Check whether each value in a `TypedArray` is between the two corresponding "expected" values
 * (either `a(i) <= actual[i] <= b(i)` or `a(i) >= actual[i] => b(i)`).
 */
function checkElementsBetween(
actual,
expected)
{
  const error = checkElementsPassPredicate(
    actual,
    (index, value) =>
    value >= Math.min(expected[0](index), expected[1](index)) &&
    value <= Math.max(expected[0](index), expected[1](index)),
    {
      predicatePrinter: [
      { leftHeader: 'between', getValueForCell: (index) => expected[0](index) },
      { leftHeader: 'and', getValueForCell: (index) => expected[1](index) }]

    }
  );
  // If there was an error, extend it with additional extras.
  return error ? new ErrorWithExtra(error, () => ({ expected })) : undefined;
}

/**
 * Check whether each value in a `TypedArray` is equal to one of the two corresponding "expected"
 * values (either `actual[i] === a[i]` or `actual[i] === b[i]`)
 */
function checkElementsEqualEither(
actual,
expected)
{
  const error = checkElementsPassPredicate(
    actual,
    (index, value) => value === expected[0][index] || value === expected[1][index],
    {
      predicatePrinter: [
      { leftHeader: 'either', getValueForCell: (index) => expected[0][index] },
      { leftHeader: 'or', getValueForCell: (index) => expected[1][index] }]

    }
  );
  // If there was an error, extend it with additional extras.
  return error ? new ErrorWithExtra(error, () => ({ expected })) : undefined;
}

/**
 * Check whether a `TypedArray`'s contents equal the values produced by a generator function.
 * Returns `undefined` if the check passes, or an `Error` if not.
 *
 * ```text
 * Array had unexpected contents at indices 2 through 19.
 *  Starting at index 1:
 *    actual == 0x: 00 fe ff 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f 00
 *    failed ->        xx xx    xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx
 *  expected ==     00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
 * ```
 *
 * ```text
 * Array had unexpected contents at indices 2 through 29.
 *  Starting at index 1:
 *    actual ==  0.000 -2.000e+100 -1.000e+100 0.000 1.000e+100 2.000e+100 3.000e+100 4.000e+100 5.000e+100 6.000e+100 7.000e+100 ...
 *    failed ->                 xx          xx               xx         xx         xx         xx         xx         xx         xx ...
 *  expected ==  0.000       0.000       0.000 0.000      0.000      0.000      0.000      0.000      0.000      0.000      0.000 ...
 * ```
 */
function checkElementsEqualGenerated(
actual,
generator)
{
  let failedElementsFirstMaybe = undefined;
  /** Sparse array with `true` for elements that failed. */
  const failedElements = [];
  for (let i = 0; i < actual.length; ++i) {
    if (actual[i] !== generator(i)) {
      failedElementsFirstMaybe ??= i;
      failedElements[i] = true;
    }
  }

  if (failedElementsFirstMaybe === undefined) {
    return undefined;
  }

  const failedElementsFirst = failedElementsFirstMaybe;
  const error = failCheckElements({
    actual,
    failedElements,
    failedElementsFirst,
    predicatePrinter: [{ leftHeader: 'expected ==', getValueForCell: (index) => generator(index) }]
  });
  // Add more extras to the error.
  return new ErrorWithExtra(error, () => ({ generator }));
}

/**
 * Check whether a `TypedArray`'s values pass the provided predicate function.
 * Returns `undefined` if the check passes, or an `Error` if not.
 */
function checkElementsPassPredicate(
actual,
predicate,
{ predicatePrinter })
{
  let failedElementsFirstMaybe = undefined;
  /** Sparse array with `true` for elements that failed. */
  const failedElements = [];
  for (let i = 0; i < actual.length; ++i) {
    if (!predicate(i, actual[i])) {
      failedElementsFirstMaybe ??= i;
      failedElements[i] = true;
    }
  }

  if (failedElementsFirstMaybe === undefined) {
    return undefined;
  }

  const failedElementsFirst = failedElementsFirstMaybe;
  return failCheckElements({ actual, failedElements, failedElementsFirst, predicatePrinter });
}








/**
 * Implements the failure case of some checkElementsX helpers above. This allows those functions to
 * implement their checks directly without too many function indirections in between.
 *
 * Note: Separating this into its own function significantly speeds up the non-error case in
 * Chromium (though this may be V8-specific behavior).
 */
function failCheckElements({
  actual,
  failedElements,
  failedElementsFirst,
  predicatePrinter
}) {
  const size = actual.length;
  const ctor = actual.constructor;
  const printAsFloat = ctor === Float16Array || ctor === Float32Array || ctor === Float64Array;

  const failedElementsLast = failedElements.length - 1;

  // Include one extra non-failed element at the beginning and end (if they exist), for context.
  const printElementsStart = Math.max(0, failedElementsFirst - 1);
  const printElementsEnd = Math.min(size, failedElementsLast + 2);
  const printElementsCount = printElementsEnd - printElementsStart;

  const numericToString = (val) => {
    if (typeof val === 'number' && printAsFloat) {
      return val.toPrecision(4);
    }
    return intToPaddedHex(val, { byteLength: ctor.BYTES_PER_ELEMENT });
  };

  const numberPrefix = printAsFloat ? '' : '0x:';

  const printActual = actual.subarray(printElementsStart, printElementsEnd);
  const printExpected = [];
  if (predicatePrinter) {
    for (const { leftHeader, getValueForCell: cell } of predicatePrinter) {
      printExpected.push(
        function* () {
          yield* [leftHeader, ''];
          yield* iterRange(printElementsCount, (i) => cell(printElementsStart + i));
        }()
      );
    }
  }

  const printFailedValueMarkers = function* () {
    yield* ['failed ->', ''];
    yield* range(printElementsCount, (i) => failedElements[printElementsStart + i] ? 'xx' : '');
  }();

  const opts = {
    fillToWidth: 120,
    numericToString
  };
  const msg = `Array had unexpected contents at indices ${failedElementsFirst} through ${failedElementsLast}.
 Starting at index ${printElementsStart}:
${generatePrettyTable(opts, [
  ['actual ==', numberPrefix, ...printActual],
  printFailedValueMarkers,
  ...printExpected]
  )}`;
  return new ErrorWithExtra(msg, () => ({
    actual: actual.slice()
  }));
}

// Helper helpers

/** Convert an integral `number` into a hex string, padded to the specified `byteLength`. */
function intToPaddedHex(val, { byteLength }) {
  assert(Number.isInteger(val), 'number must be integer');
  const is_negative = typeof val === 'number' ? val < 0 : val < 0n;
  let str = (is_negative ? -val : val).toString(16);
  if (byteLength) str = str.padStart(byteLength * 2, '0');
  if (is_negative) str = '-' + str;
  return str;
}
});
__define('webgpu/util/pretty_diff_tables.js', function (__exports) {
'use strict';
__get(__exports, 'numericToStringBuilder', () => numericToStringBuilder);
__get(__exports, 'generatePrettyTable', () => generatePrettyTable);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let range; __bind('common/util/util.js', (__m) => { ({ range } = __m); }); /**
 * @returns a function that converts numerics to strings, depending on if they
 * should be treated as integers or not.
 */
function numericToStringBuilder(is_integer) {
  if (is_integer) {
    return (val) => {
      if (typeof val === 'number') {
        return val.toFixed();
      }
      return val.toString();
    };
  }

  return (val) => {
    if (typeof val === 'number') {
      return val.toPrecision(6);
    }
    return val.toString();
  };
}

/**
 * Pretty-prints a "table" of cell values (each being `number | string`), right-aligned.
 * Each row may be any iterator, including lazily-generated (potentially infinite) rows.
 *
 * The first argument is the printing options:
 *  - fillToWidth: Keep printing columns (as long as there is data) until this width is passed.
 *    If there is more data, "..." is appended.
 *  - numberToString: if a cell value is a number, this is used to stringify it.
 *
 * Each remaining argument provides one row for the table.
 */
function generatePrettyTable(
{
  fillToWidth,
  numericToString
},
rows)
{
  const rowStrings = range(rows.length, () => '');
  let totalTableWidth = 0;
  const iters = rows.map((row) => row[Symbol.iterator]());

  // Loop over columns
  for (;;) {
    const cellsForColumn = iters.map((iter) => {
      const r = iter.next(); // Advance the iterator for each row, in lock-step.
      if (r.done) {
        return undefined;
      }
      if (typeof r.value === 'number' || typeof r.value === 'bigint') {
        return numericToString(r.value);
      }
      return r.value;
    });
    if (cellsForColumn.every((cell) => cell === undefined)) break;

    // Maximum width of any cell in this column, plus one for space between columns
    // (also inserts a space at the left of the first column).
    const colWidth = Math.max(...cellsForColumn.map((c) => c === undefined ? 0 : c.length)) + 1;
    for (let row = 0; row < rowStrings.length; ++row) {
      const cell = cellsForColumn[row];
      if (cell !== undefined) {
        rowStrings[row] += cell.padStart(colWidth);
      }
    }

    totalTableWidth += colWidth;
    if (totalTableWidth >= fillToWidth) {
      for (let row = 0; row < rowStrings.length; ++row) {
        if (cellsForColumn[row] !== undefined) {
          rowStrings[row] += ' ...';
        }
      }
      break;
    }
  }
  return rowStrings.join('\n');
}
});
__define('webgpu/api/operation/buffers/mapping_test.js', function (__exports) {
'use strict';
__get(__exports, 'MappingTest', () => MappingTest);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });let AllFeaturesMaxLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest } = __m); });
class MappingTest extends AllFeaturesMaxLimitsGPUTest {
  checkMapWrite(
  buffer,
  offset,
  mappedContents,
  size)
  {
    this.checkMapWriteZeroed(mappedContents, size);

    const mappedView = new Uint32Array(mappedContents);
    const expected = new Uint32Array(new ArrayBuffer(size));
    assert(mappedView.byteLength === size);
    for (let i = 0; i < mappedView.length; ++i) {
      mappedView[i] = expected[i] = i + 1;
    }
    buffer.unmap();

    this.expectGPUBufferValuesEqual(buffer, expected, offset);
  }

  checkMapWriteZeroed(arrayBuffer, expectedSize) {
    this.expect(arrayBuffer.byteLength === expectedSize);
    const view = new Uint8Array(arrayBuffer);
    this.expectZero(view);
  }

  expectZero(actual) {
    const size = actual.byteLength;
    for (let i = 0; i < size; ++i) {
      if (actual[i] !== 0) {
        this.fail(`at [${i}], expected zero, got ${actual[i]}`);
        break;
      }
    }
  }
}
});
__define('webgpu/gpu_test.js', function (__exports) {
'use strict';
__get(__exports, 'kResourceStates', () => kResourceStates);
__get(__exports, 'initUncanonicalizedDeviceDescriptor', () => initUncanonicalizedDeviceDescriptor);
__get(__exports, 'GPUTestSubcaseBatchState', () => GPUTestSubcaseBatchState);
__get(__exports, 'GPUTestBase', () => GPUTestBase);
__get(__exports, 'GPUTest', () => GPUTest);
__get(__exports, 'RequiredLimitsGPUTestSubcaseBatchState', () => RequiredLimitsGPUTestSubcaseBatchState);
__get(__exports, 'RequiredLimitsTestMixin', () => RequiredLimitsTestMixin);
__get(__exports, 'AllFeaturesMaxLimitsGPUTestSubcaseBatchState', () => AllFeaturesMaxLimitsGPUTestSubcaseBatchState);
__get(__exports, 'UniqueFeaturesOrLimitsGPUTest', () => UniqueFeaturesOrLimitsGPUTest);
__get(__exports, 'AllFeaturesMaxLimitsGPUTest', () => AllFeaturesMaxLimitsGPUTest);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let Fixture, SubcaseBatchState; __bind('common/framework/fixture.js', (__m) => { ({ Fixture, SubcaseBatchState } = __m); });
let registerShutdownTask; __bind('common/framework/on_shutdown.js', (__m) => { ({ registerShutdownTask } = __m); });
let globalTestConfig, isCompatibilityDevice; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig, isCompatibilityDevice } = __m); });
let getGPU; __bind('common/util/navigator_gpu.js', (__m) => { ({ getGPU } = __m); });
let assert, makeValueTestVariant, memcpy, range, unreachable, hasFeature; __bind('common/util/util.js', (__m) => { ({ assert, makeValueTestVariant, memcpy, range, unreachable, hasFeature } = __m); });

let kQueryTypeInfo; __bind('webgpu/capability_info.js', (__m) => { ({ kQueryTypeInfo } = __m); });

let resolvePerAspectFormat, isCompressedTextureFormat, getRequiredFeatureForTextureFormat, isTextureFormatUsableAsRenderAttachment, isTextureFormatMultisampled, isTextureFormatResolvable, isDepthTextureFormat, isStencilTextureFormat, textureViewDimensionAndFormatCompatibleForDevice, textureDimensionAndFormatCompatibleForDevice, isTextureFormatUsableWithStorageAccessMode, isTextureFormatUsableWithCopyExternalImageToTexture, isTextureFormatFilterable, isTextureFormatBlendable; __bind('webgpu/format_info.js', (__m) => { ({ resolvePerAspectFormat, isCompressedTextureFormat, getRequiredFeatureForTextureFormat, isTextureFormatUsableAsRenderAttachment, isTextureFormatMultisampled, isTextureFormatResolvable, isDepthTextureFormat, isStencilTextureFormat, textureViewDimensionAndFormatCompatibleForDevice, textureDimensionAndFormatCompatibleForDevice, isTextureFormatUsableWithStorageAccessMode, isTextureFormatUsableWithCopyExternalImageToTexture, isTextureFormatFilterable, isTextureFormatBlendable } = __m); });
let checkElementsEqual, checkElementsBetween; __bind('webgpu/util/check_contents.js', (__m) => { ({ checkElementsEqual, checkElementsBetween } = __m); });
let CommandBufferMaker; __bind('webgpu/util/command_buffer_maker.js', (__m) => { ({ CommandBufferMaker } = __m); });

let DevicePool; __bind('webgpu/util/device_pool.js', (__m) => { ({ DevicePool } = __m); });
let align, roundDown; __bind('webgpu/util/math.js', (__m) => { ({ align, roundDown } = __m); });
let getTextureCopyLayout, getTextureSubCopyLayout; __bind('webgpu/util/texture/layout.js', (__m) => { ({ getTextureCopyLayout, getTextureSubCopyLayout } = __m); });
let kTexelRepresentationInfo; __bind('webgpu/util/texture/texel_data.js', (__m) => { ({ kTexelRepresentationInfo } = __m); });
let reifyExtent3D, reifyOrigin3D; __bind('webgpu/util/unions.js', (__m) => { ({ reifyExtent3D, reifyOrigin3D } = __m); });

// Declarations for WebGPU items we want tests for that are not yet officially part of the spec.










const devicePool = new DevicePool();

// MAINTENANCE_TODO: When DevicePool becomes able to provide multiple devices at once, use the
// usual one instead of a new one.
const mismatchedDevicePool = new DevicePool();

// On shutdown, try to explicitly destroy() the device pools (and devices) used by GPUTest,
// so they don't keep using system resources until they're fully garbage collected.
registerShutdownTask(() => {
  devicePool.destroy();
  mismatchedDevicePool.destroy();
});

const kResourceStateValues = ['valid', 'invalid', 'destroyed'];

const kResourceStates = kResourceStateValues;

/** Various "convenient" shorthands for GPUDeviceDescriptors for selectDevice functions. */






function initUncanonicalizedDeviceDescriptor(
descriptor)
{
  if (typeof descriptor === 'string') {
    return { requiredFeatures: [descriptor] };
  } else if (descriptor instanceof Array) {
    return {
      requiredFeatures: descriptor.filter((f) => f !== undefined)
    };
  } else {
    return descriptor ?? {};
  }
}







function mergeDeviceSelectionDescriptorIntoDeviceDescriptor(
src,
dst)
{
  const srcFixed = initUncanonicalizedDeviceDescriptor(src);
  if (srcFixed) {
    dst.requiredFeatures.push(...(srcFixed.requiredFeatures ?? []));
    Object.assign(dst.requiredLimits, srcFixed.requiredLimits ?? {});
  }
}

class GPUTestSubcaseBatchState extends SubcaseBatchState {
  /** Provider for default device. */

  /** Provider for mismatched device. */

  /** The accumulated skip-if requirements for this subcase */
  skipIfRequirements = {
    requiredFeatures: [],
    requiredLimits: {},
    defaultQueue: {}
  };
  /** Whether or not to provide a mismatched device */
  useMismatchedDevice = false;

  async postInit() {
    // Skip all subcases if there's no device.
    await this.acquireProvider();
  }

  async finalize() {
    await super.finalize();

    // Ensure devicePool.release is called for both providers even if one rejects
    // and wait for both of them before proceeding.
    const results = await Promise.allSettled([
    this.provider?.then((x) => devicePool.release(x)),
    this.mismatchedProvider?.then((x) => mismatchedDevicePool.release(x))]
    );

    // If one of them rejected throw its reason. It should be an `Error`.
    for (const result of results) {
      if (result.status === 'rejected') throw result.reason;
    }
  }

  /** @internal MAINTENANCE_TODO: Make this not visible to test code? */
  acquireProvider() {
    if (this.provider === undefined) {
      this.requestDeviceWithRequiredParametersOrSkip(this.skipIfRequirements);
    }
    assert(this.provider !== undefined);
    assert(!this.useMismatchedDevice || this.mismatchedProvider !== undefined);
    return this.provider;
  }

  get isCompatibility() {
    return globalTestConfig.compatibility;
  }

  /**
   * Some tests or cases need particular feature flags or limits to be enabled.
   * Call this function with a descriptor or feature name (or `undefined`) to select a
   * GPUDevice with matching capabilities. If this isn't called, a default device is provided.
   *
   * If the request isn't supported, throws a SkipTestCase exception to skip the entire test case.
   */
  requestDeviceWithRequiredParametersOrSkip(
  descriptor,
  descriptorModifier)
  {
    assert(this.provider === undefined, "Can't selectDeviceOrSkipTestCase() multiple times");
    this.provider = devicePool.acquire(
      this.recorder,
      initUncanonicalizedDeviceDescriptor(descriptor),
      descriptorModifier
    );
    // Suppress uncaught promise rejection (we'll catch it later).
    this.provider.catch(() => {});

    if (this.useMismatchedDevice) {
      this.mismatchedProvider = mismatchedDevicePool.acquire(
        this.recorder,
        initUncanonicalizedDeviceDescriptor(descriptor),
        descriptorModifier
      );
      // Suppress uncaught promise rejection (we'll catch it later).
      this.mismatchedProvider.catch(() => {});
    }
  }

  /**
   * Some tests need a second device which is different from the first.
   * This requests a second device so it will be available during the test. If it is not called,
   * no second device will be available. The second device will be created with the
   * same features and limits as the first device.
   */
  usesMismatchedDevice() {
    assert(this.provider === undefined, 'Can not call usedMismatchedDevice after device creation');
    this.useMismatchedDevice = true;
  }

  /**
   * Some tests or cases need particular feature flags or limits to be enabled.
   * Call this function with a descriptor or feature name (or `undefined`) to add
   * features or limits required by the subcase. If the features or limits are not
   * available a SkipTestCase exception will be thrown to skip the entire test case.
   */
  selectDeviceOrSkipTestCase(descriptor) {
    mergeDeviceSelectionDescriptorIntoDeviceDescriptor(descriptor, this.skipIfRequirements);
  }

  /**
   * Convenience function for {@link selectDeviceOrSkipTestCase}.
   * Select a device with the features required by these texture format(s).
   * If the device creation fails, then skip the test case.
   */
  selectDeviceForTextureFormatOrSkipTestCase(
  formats)
  {
    if (!Array.isArray(formats)) {
      formats = [formats];
    }
    const features = new Set();
    for (const format of formats) {
      if (format !== undefined) {
        features.add(getRequiredFeatureForTextureFormat(format));
      }
    }

    this.selectDeviceOrSkipTestCase(Array.from(features));
  }

  /**
   * Convenience function for {@link selectDeviceOrSkipTestCase}.
   * Select a device with the features required by these query type(s).
   * If the device creation fails, then skip the test case.
   */
  selectDeviceForQueryTypeOrSkipTestCase(types) {
    if (!Array.isArray(types)) {
      types = [types];
    }
    const features = types.map((t) => kQueryTypeInfo[t].feature);
    this.selectDeviceOrSkipTestCase(features);
  }

  /** @internal MAINTENANCE_TODO: Make this not visible to test code? */
  acquireMismatchedProvider() {
    return this.mismatchedProvider;
  }

  skipIfCopyTextureToTextureNotSupportedForFormat(...formats) {
    if (this.isCompatibility) {
      for (const format of formats) {
        if (format && isCompressedTextureFormat(format)) {
          this.skip(`copyTextureToTexture with ${format} is not supported in compatibility mode`);
        }
      }
    }
  }

  /**
   * Skips test if the given interpolation type or sampling is not supported.
   */
  skipIfInterpolationTypeOrSamplingNotSupported({
    type,
    sampling



  }) {
    if (this.isCompatibility) {
      this.skipIf(
        type === 'linear',
        'interpolation type linear is not supported in compatibility mode'
      );
      this.skipIf(
        sampling === 'sample',
        'interpolation type linear is not supported in compatibility mode'
      );
      this.skipIf(
        type === 'flat' && (!sampling || sampling === 'first'),
        'interpolation type flat with sampling not set to either is not supported in compatibility mode'
      );
    }
  }

  /** Skips this test case if the `langFeature` is *not* supported. */
  skipIfLanguageFeatureNotSupported(langFeature) {
    if (!this.hasLanguageFeature(langFeature)) {
      this.skip(`WGSL language feature '${langFeature}' is not supported`);
    }
  }

  /** Skips this test case if the `langFeature` is supported. */
  skipIfLanguageFeatureSupported(langFeature) {
    if (this.hasLanguageFeature(langFeature)) {
      this.skip(`WGSL language feature '${langFeature}' is supported`);
    }
  }

  /** returns true iff the `langFeature` is supported  */
  hasLanguageFeature(langFeature) {
    const lf = getGPU(this.recorder).wgslLanguageFeatures;
    return lf !== undefined && lf.has(langFeature);
  }
}

/**
 * Base fixture for WebGPU tests.
 *
 * This class is a Fixture + a getter that returns a GPUDevice
 * as well as helpers that use that device.
 */
class GPUTestBase extends Fixture {
  static MakeSharedState(
  recorder,
  params)
  {
    return new GPUTestSubcaseBatchState(recorder, params);
  }

  // This must be overridden in derived classes
  get device() {
    unreachable();
    return null;
  }

  /** GPUQueue for the test to use. (Same as `t.device.queue`.) */
  get queue() {
    return this.device.queue;
  }

  get isCompatibility() {
    return globalTestConfig.compatibility;
  }

  makeLimitVariant(limit, variant) {
    return makeValueTestVariant(this.device.limits[limit], variant);
  }

  canCallCopyTextureToBufferWithTextureFormat(format) {
    return !this.isCompatibility || !isCompressedTextureFormat(format);
  }

  /** Snapshot a GPUBuffer's contents, returning a new GPUBuffer with the `MAP_READ` usage. */
  createCopyForMapRead(src, srcOffset, size) {
    assert(srcOffset % 4 === 0);
    assert(size % 4 === 0);

    const dst = this.createBufferTracked({
      label: 'createCopyForMapRead',
      size,
      usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST
    });

    const c = this.device.createCommandEncoder({ label: 'createCopyForMapRead' });
    c.copyBufferToBuffer(src, srcOffset, dst, 0, size);
    this.queue.submit([c.finish()]);

    return dst;
  }

  /**
   * Offset and size passed to createCopyForMapRead must be divisible by 4. For that
   * we might need to copy more bytes from the buffer than we want to map.
   * begin and end values represent the part of the copied buffer that stores the contents
   * we initially wanted to map.
   * The copy will not cause an OOB error because the buffer size must be 4-aligned.
   */
  createAlignedCopyForMapRead(
  src,
  size,
  offset)
  {
    const alignedOffset = roundDown(offset, 4);
    const subarrayByteStart = offset - alignedOffset;
    const alignedSize = align(size + subarrayByteStart, 4);
    const mappable = this.createCopyForMapRead(src, alignedOffset, alignedSize);
    return { mappable, subarrayByteStart };
  }

  /**
   * Snapshot the current contents of a range of a GPUBuffer, and return them as a TypedArray.
   * Also provides a cleanup() function to unmap and destroy the staging buffer.
   */
  async readGPUBufferRangeTyped(
  src,
  {
    srcByteOffset = 0,
    method = 'copy',
    type,
    typedLength





  })
  {
    assert(
      srcByteOffset % type.BYTES_PER_ELEMENT === 0,
      'srcByteOffset must be a multiple of BYTES_PER_ELEMENT'
    );

    const byteLength = typedLength * type.BYTES_PER_ELEMENT;
    let mappable;
    let mapOffset, mapSize, subarrayByteStart;
    if (method === 'copy') {
      ({ mappable, subarrayByteStart } = this.createAlignedCopyForMapRead(
        src,
        byteLength,
        srcByteOffset
      ));
    } else if (method === 'map') {
      mappable = src;
      mapOffset = roundDown(srcByteOffset, 8);
      mapSize = align(byteLength, 4);
      subarrayByteStart = srcByteOffset - mapOffset;
    } else {
      unreachable();
    }

    assert(subarrayByteStart % type.BYTES_PER_ELEMENT === 0);
    const subarrayStart = subarrayByteStart / type.BYTES_PER_ELEMENT;

    // 2. Map the staging buffer, and create the TypedArray from it.
    await mappable.mapAsync(GPUMapMode.READ, mapOffset, mapSize);
    const mapped = new type(mappable.getMappedRange(mapOffset, mapSize));
    const data = mapped.subarray(subarrayStart, typedLength);

    return {
      data,
      cleanup() {
        mappable.unmap();
        mappable.destroy();
      }
    };
  }

  /**
   * Skips test if device does not have feature.
   * Note: Try to use one of the more specific skipIf tests if possible.
   */
  skipIfDeviceDoesNotHaveFeature(feature) {
    this.skipIf(
      !hasFeature(this.device.features, feature),
      `device does not have feature: '${feature}'`
    );
  }

  /**
   * Skips test if device des not support query type.
   */
  skipIfDeviceDoesNotSupportQueryType(...types) {
    for (const type of types) {
      const feature = kQueryTypeInfo[type].feature;
      if (feature) {
        this.skipIfDeviceDoesNotHaveFeature(feature);
      }
    }
  }

  skipIfDepthTextureCanNotBeUsedWithNonComparisonSampler() {
    this.skipIf(
      this.isCompatibility,
      'depth textures are not usable with non-comparison samplers in compatibility mode'
    );
  }

  /**
   * Skips test if any format is not supported.
   */
  skipIfTextureFormatNotSupported(...formats) {
    for (const format of formats) {
      if (!format) {
        continue;
      }
      if (format === 'bgra8unorm-srgb') {
        if (isCompatibilityDevice(this.device)) {
          this.skip(`texture format '${format}' is not supported`);
        }
      }
      const feature = getRequiredFeatureForTextureFormat(format);
      this.skipIf(
        !!feature && !hasFeature(this.device.features, feature),
        `texture format '${format}' requires feature: '${feature}'`
      );
    }
  }

  skipIfTextureFormatAndViewDimensionNotCompatible(
  format,
  viewDimension)
  {
    this.skipIf(
      !textureViewDimensionAndFormatCompatibleForDevice(
        this.device.features,
        viewDimension,
        format
      ),
      `format: ${format} does not support viewDimension: ${viewDimension}`
    );
  }

  skipIfTextureFormatAndDimensionNotCompatible(
  format,
  dimension)
  {
    this.skipIf(
      !textureDimensionAndFormatCompatibleForDevice(this.device.features, dimension, format),
      `format: ${format} does not support dimension: ${dimension}`
    );
  }

  skipIfTextureFormatNotResolvable(...formats) {
    for (const format of formats) {
      if (format === undefined) continue;
      if (!isTextureFormatResolvable(this.device.features, format)) {
        this.skip(`texture format '${format}' is not resolvable`);
      }
    }
  }

  skipIfTextureViewDimensionNotSupported(...dimensions) {
    if (isCompatibilityDevice(this.device)) {
      for (const dimension of dimensions) {
        if (dimension === 'cube-array') {
          this.skip(`texture view dimension '${dimension}' is not supported`);
        }
      }
    }
  }

  skipIfCopyTextureToTextureNotSupportedForFormat(...formats) {
    if (isCompatibilityDevice(this.device)) {
      for (const format of formats) {
        if (format && isCompressedTextureFormat(format)) {
          this.skip(`copyTextureToTexture with ${format} is not supported`);
        }
      }
    }
  }

  skipIfTextureLoadNotSupportedForTextureType(...types) {
    if (this.isCompatibility) {
      for (const type of types) {
        switch (type) {
          case 'texture_depth_2d':
          case 'texture_depth_2d_array':
          case 'texture_depth_multisampled_2d':
            this.skip(`${type} is not supported by textureLoad in compatibility mode`);
        }
      }
    }
  }

  skipIfTextureFormatNotUsableWithStorageAccessMode(
  access,
  ...formats)
  {
    for (const format of formats) {
      if (!format) continue;

      if (!isTextureFormatUsableWithStorageAccessMode(this.device.features, format, access)) {
        this.skip(
          `Texture with ${format} is not usable as a storage texture with access ${access}`
        );
      }
    }
  }

  skipIfTextureFormatNotUsableAsRenderAttachment(...formats) {
    for (const format of formats) {
      if (format && !isTextureFormatUsableAsRenderAttachment(this.device.features, format)) {
        this.skip(`Texture with ${format} is not usable as a render attachment`);
      }
    }
  }

  skipIfTextureFormatNotMultisampled(...formats) {
    for (const format of formats) {
      if (format === undefined) continue;
      if (!isTextureFormatMultisampled(this.device.features, format)) {
        this.skip(`texture format '${format}' does not support multisampling`);
      }
    }
  }

  skipIfTextureFormatNotBlendable(...formats) {
    for (const format of formats) {
      if (format === undefined) continue;
      this.skipIf(
        !isTextureFormatBlendable(this.device.features, format),
        `${format} is not blendable`
      );
    }
  }

  skipIfTextureFormatNotFilterable(...formats) {
    for (const format of formats) {
      if (format === undefined) continue;
      this.skipIf(
        !isTextureFormatFilterable(this.device.features, format),
        `${format} is not filterable`
      );
    }
  }

  skipIfTextureFormatDoesNotSupportUsage(
  usage,
  ...formats)
  {
    for (const format of formats) {
      if (!format) continue;
      if (usage & GPUTextureUsage.RENDER_ATTACHMENT) {
        this.skipIfTextureFormatNotUsableAsRenderAttachment(format);
      }
      if (usage & GPUTextureUsage.STORAGE_BINDING) {
        this.skipIfTextureFormatNotUsableWithStorageAccessMode('write-only', format);
      }
    }
  }

  skipIfTextureFormatDoesNotSupportCopyTextureToBuffer(format) {
    this.skipIf(
      !this.canCallCopyTextureToBufferWithTextureFormat(format),
      `can not use copyTextureToBuffer with ${format}`
    );
  }

  skipIfTextureFormatPossiblyNotUsableWithCopyExternalImageToTexture(format) {
    this.skipIf(
      !isTextureFormatUsableWithCopyExternalImageToTexture(this.device.features, format),
      `can not use copyExternalImageToTexture with ${format}`
    );
  }

  /** Skips this test case if the `langFeature` is *not* supported. */
  skipIfLanguageFeatureNotSupported(langFeature) {
    if (!this.hasLanguageFeature(langFeature)) {
      this.skip(`WGSL language feature '${langFeature}' is not supported`);
    }
  }

  /** Skips this test case if the `langFeature` is supported. */
  skipIfLanguageFeatureSupported(langFeature) {
    if (this.hasLanguageFeature(langFeature)) {
      this.skip(`WGSL language feature '${langFeature}' is supported`);
    }
  }

  /** returns true if the `langFeature` is supported  */
  hasLanguageFeature(langFeature) {
    const lf = getGPU(this.rec).wgslLanguageFeatures;
    return lf !== undefined && lf.has(langFeature);
  }

  /** Skips this test case if the GPUTextureUsage `TRANSIENT_ATTACHMENT` is *not* supported. */
  // MAINTENANCE_TODO(#4509): Remove this after all implementations have TRANSIENT_ATTACHMENT.
  skipIfTransientAttachmentNotSupported() {
    const isTransientAttachmentSupported = ('TRANSIENT_ATTACHMENT' in GPUTextureUsage);
    this.skipIf(
      !isTransientAttachmentSupported,
      'GPUTextureUsage TRANSIENT_ATTACHMENT is not supported'
    );
  }

  /**
   * Expect a GPUBuffer's contents to pass the provided check.
   *
   * A library of checks can be found in {@link webgpu/util/check_contents}.
   */
  expectGPUBufferValuesPassCheck(
  src,
  check,
  {
    srcByteOffset = 0,
    type,
    typedLength,
    method = 'copy',
    mode = 'fail'






  })
  {
    const readbackPromise = this.readGPUBufferRangeTyped(src, {
      srcByteOffset,
      type,
      typedLength,
      method
    });
    this.eventualAsyncExpectation(async (niceStack) => {
      const readback = await readbackPromise;
      this.expectOK(check(readback.data), { mode, niceStack });
      readback.cleanup();
    });
  }

  /**
   * Expect a GPUBuffer's contents to equal the values in the provided TypedArray.
   */
  expectGPUBufferValuesEqual(
  src,
  expected,
  srcByteOffset = 0,
  { method = 'copy', mode = 'fail' } = {})
  {
    this.expectGPUBufferValuesPassCheck(src, (a) => checkElementsEqual(a, expected), {
      srcByteOffset,
      type: expected.constructor,
      typedLength: expected.length,
      method,
      mode
    });
  }

  /**
   * Expect a buffer to consist exclusively of rows of some repeated expected value. The size of
   * `expectedValue` must be 1, 2, or any multiple of 4 bytes. Rows in the buffer are expected to be
   * zero-padded out to `bytesPerRow`. `minBytesPerRow` is the number of bytes per row that contain
   * actual (non-padding) data and must be an exact multiple of the byte-length of `expectedValue`.
   */
  expectGPUBufferRepeatsSingleValue(
  buffer,
  {
    expectedValue,
    numRows,
    minBytesPerRow,
    bytesPerRow





  })
  {
    const valueSize = expectedValue.byteLength;
    assert(valueSize === 1 || valueSize === 2 || valueSize % 4 === 0);
    assert(minBytesPerRow % valueSize === 0);
    assert(bytesPerRow % 4 === 0);

    // If the buffer is small enough, just generate the full expected buffer contents and check
    // against them on the CPU.
    const kMaxBufferSizeToCheckOnCpu = 256 * 1024;
    const bufferSize = bytesPerRow * (numRows - 1) + minBytesPerRow;
    if (bufferSize <= kMaxBufferSizeToCheckOnCpu) {
      const valueBytes = Array.from(new Uint8Array(expectedValue));
      const rowValues = new Array(minBytesPerRow / valueSize).fill(valueBytes);
      const rowBytes = new Uint8Array([].concat(...rowValues));
      const expectedContents = new Uint8Array(bufferSize);
      range(numRows, (row) => expectedContents.set(rowBytes, row * bytesPerRow));
      this.expectGPUBufferValuesEqual(buffer, expectedContents);
      return;
    }

    // Copy into a buffer suitable for STORAGE usage.
    const storageBuffer = this.createBufferTracked({
      label: 'expectGPUBufferRepeatsSingleValue:storageBuffer',
      size: bufferSize,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST
    });

    // This buffer conveys the data we expect to see for a single value read. Since we read 32 bits at
    // a time, for values smaller than 32 bits we pad this expectation with repeated value data, or
    // with zeroes if the width of a row in the buffer is less than 4 bytes. For value sizes larger
    // than 32 bits, we assume they're a multiple of 32 bits and expect to read exact matches of
    // `expectedValue` as-is.
    const expectedDataSize = Math.max(4, valueSize);
    const expectedDataBuffer = this.createBufferTracked({
      label: 'expectGPUBufferRepeatsSingleValue:expectedDataBuffer',
      size: expectedDataSize,
      usage: GPUBufferUsage.STORAGE,
      mappedAtCreation: true
    });
    const expectedData = new Uint32Array(expectedDataBuffer.getMappedRange());
    if (valueSize === 1) {
      const value = new Uint8Array(expectedValue)[0];
      const values = new Array(Math.min(4, minBytesPerRow)).fill(value);
      const padding = new Array(Math.max(0, 4 - values.length)).fill(0);
      const expectedBytes = new Uint8Array(expectedData.buffer);
      expectedBytes.set([...values, ...padding]);
    } else if (valueSize === 2) {
      const value = new Uint16Array(expectedValue)[0];
      const expectedWords = new Uint16Array(expectedData.buffer);
      expectedWords.set([value, minBytesPerRow > 2 ? value : 0]);
    } else {
      expectedData.set(new Uint32Array(expectedValue));
    }
    expectedDataBuffer.unmap();

    // The output buffer has one 32-bit entry per buffer row. An entry's value will be 1 if every
    // read from the corresponding row matches the expected data derived above, or 0 otherwise.
    const resultBuffer = this.createBufferTracked({
      label: 'expectGPUBufferRepeatsSingleValue:resultBuffer',
      size: numRows * 4,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC
    });

    const readsPerRow = Math.ceil(minBytesPerRow / expectedDataSize);
    const reducer = `
    struct Buffer { data: array<u32>, };
    @group(0) @binding(0) var<storage, read> expected: Buffer;
    @group(0) @binding(1) var<storage, read> in: Buffer;
    @group(0) @binding(2) var<storage, read_write> out: Buffer;
    @compute @workgroup_size(1) fn reduce(
        @builtin(global_invocation_id) id: vec3<u32>) {
      let rowBaseIndex = id.x * ${bytesPerRow / 4}u;
      let readSize = ${expectedDataSize / 4}u;
      out.data[id.x] = 1u;
      for (var i: u32 = 0u; i < ${readsPerRow}u; i = i + 1u) {
        let elementBaseIndex = rowBaseIndex + i * readSize;
        for (var j: u32 = 0u; j < readSize; j = j + 1u) {
          if (in.data[elementBaseIndex + j] != expected.data[j]) {
            out.data[id.x] = 0u;
            return;
          }
        }
      }
    }
    `;

    const pipeline = this.device.createComputePipeline({
      layout: 'auto',
      compute: {
        module: this.device.createShaderModule({ code: reducer }),
        entryPoint: 'reduce'
      }
    });

    const bindGroup = this.device.createBindGroup({
      layout: pipeline.getBindGroupLayout(0),
      entries: [
      { binding: 0, resource: { buffer: expectedDataBuffer } },
      { binding: 1, resource: { buffer: storageBuffer } },
      { binding: 2, resource: { buffer: resultBuffer } }]

    });

    const commandEncoder = this.device.createCommandEncoder({
      label: 'expectGPUBufferRepeatsSingleValue'
    });
    commandEncoder.copyBufferToBuffer(buffer, 0, storageBuffer, 0, bufferSize);
    const pass = commandEncoder.beginComputePass();
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, bindGroup);
    pass.dispatchWorkgroups(numRows);
    pass.end();
    this.device.queue.submit([commandEncoder.finish()]);

    const expectedResults = new Array(numRows).fill(1);
    this.expectGPUBufferValuesEqual(resultBuffer, new Uint32Array(expectedResults));
  }

  // MAINTENANCE_TODO: add an expectContents for textures, which logs data: uris on failure

  /**
   * Expect an entire GPUTexture to have a single color at the given mip level (defaults to 0).
   * MAINTENANCE_TODO: Remove this and/or replace it with a helper in TextureTestMixin.
   */
  expectSingleColor(
  src,
  format,
  {
    size,
    exp,
    dimension = '2d',
    slice = 0,
    layout






  })
  {
    assert(
      slice === 0 || dimension === '2d',
      'texture slices are only implemented for 2d textures'
    );

    format = resolvePerAspectFormat(format, layout?.aspect);
    const { byteLength, minBytesPerRow, bytesPerRow, rowsPerImage, mipSize } = getTextureCopyLayout(
      format,
      dimension,
      size,
      layout
    );
    // MAINTENANCE_TODO: getTextureCopyLayout does not return the proper size for array textures,
    // i.e. it will leave the z/depth value as is instead of making it 1 when dealing with 2d
    // texture arrays. Since we are passing in the dimension, we should update it to return the
    // corrected size.
    const copySize = [
    mipSize[0],
    dimension !== '1d' ? mipSize[1] : 1,
    dimension === '3d' ? mipSize[2] : 1];


    const rep = kTexelRepresentationInfo[format];
    const expectedTexelData = rep.pack(rep.encode(exp));

    const buffer = this.createBufferTracked({
      label: 'expectSingleColor',
      size: byteLength,
      usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST
    });

    const commandEncoder = this.device.createCommandEncoder({ label: 'expectSingleColor' });
    commandEncoder.copyTextureToBuffer(
      {
        texture: src,
        mipLevel: layout?.mipLevel,
        origin: { x: 0, y: 0, z: slice },
        aspect: layout?.aspect
      },
      { buffer, bytesPerRow, rowsPerImage },
      copySize
    );
    this.queue.submit([commandEncoder.finish()]);

    this.expectGPUBufferRepeatsSingleValue(buffer, {
      expectedValue: expectedTexelData,
      numRows: rowsPerImage * copySize[2],
      minBytesPerRow,
      bytesPerRow
    });
  }

  /**
   * Return a GPUBuffer that data are going to be written into.
   * MAINTENANCE_TODO: Remove this once expectSinglePixelBetweenTwoValuesIn2DTexture is removed.
   */
  readSinglePixelFrom2DTexture(
  src,
  format,
  { x, y },
  { slice = 0, layout })
  {
    const { byteLength, bytesPerRow, rowsPerImage } = getTextureSubCopyLayout(
      format,
      [1, 1],
      layout
    );
    const buffer = this.createBufferTracked({
      label: 'readSinglePixelFrom2DTexture',
      size: byteLength,
      usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST
    });

    const commandEncoder = this.device.createCommandEncoder({
      label: 'readSinglePixelFrom2DTexture'
    });
    commandEncoder.copyTextureToBuffer(
      { texture: src, mipLevel: layout?.mipLevel, origin: { x, y, z: slice } },
      { buffer, bytesPerRow, rowsPerImage },
      [1, 1]
    );
    this.queue.submit([commandEncoder.finish()]);

    return buffer;
  }

  /**
   * Take a single pixel of a 2D texture, interpret it using a TypedArray of the `expected` type,
   * and expect each value in that array to be between the corresponding "expected" values
   * (either `a[i] <= actual[i] <= b[i]` or `a[i] >= actual[i] => b[i]`).
   * MAINTENANCE_TODO: Remove this once there is a way to deal with undefined lerp-ed values.
   */
  expectSinglePixelBetweenTwoValuesIn2DTexture(
  src,
  format,
  { x, y },
  {
    exp,
    slice = 0,
    layout,
    generateWarningOnly = false,
    checkElementsBetweenFn = (act, [a, b]) =>
    checkElementsBetween(act, [(i) => a[i], (i) => b[i]])









  })
  {
    assert(exp[0].constructor === exp[1].constructor);
    const constructor = exp[0].constructor;
    assert(exp[0].length === exp[1].length);
    const typedLength = exp[0].length;

    const buffer = this.readSinglePixelFrom2DTexture(src, format, { x, y }, { slice, layout });
    this.expectGPUBufferValuesPassCheck(buffer, (a) => checkElementsBetweenFn(a, exp), {
      type: constructor,
      typedLength,
      mode: generateWarningOnly ? 'warn' : 'fail'
    });
  }

  /**
   * Emulate a texture to buffer copy by using a compute shader
   * to load texture values of a subregion of a 2d texture and write to a storage buffer.
   * For sample count == 1, the buffer contains extent[0] * extent[1] of the sample.
   * For sample count > 1, the buffer contains extent[0] * extent[1] * (N = sampleCount) values sorted
   * in the order of their sample index [0, sampleCount - 1]
   *
   * This can be useful when the texture to buffer copy is not available to the texture format
   * e.g. (depth24plus), or when the texture is multisampled.
   *
   * MAINTENANCE_TODO: extend texture dimension to 1d and 3d.
   *
   * @returns storage buffer containing the copied value from the texture.
   */
  copy2DTextureToBufferUsingComputePass(
  type,
  componentCount,
  textureView,
  sampleCount = 1,
  extent_ = [1, 1, 1],
  origin_ = [0, 0, 0])
  {
    const origin = reifyOrigin3D(origin_);
    const extent = reifyExtent3D(extent_);
    const width = extent.width;
    const height = extent.height;
    const kWorkgroupSizeX = 8;
    const kWorkgroupSizeY = 8;
    const textureSrcCode =
    sampleCount === 1 ?
    `@group(0) @binding(0) var src: texture_2d<${type}>;` :
    `@group(0) @binding(0) var src: texture_multisampled_2d<${type}>;`;
    const code = `
      struct Buffer {
        data: array<${type}>,
      };

      ${textureSrcCode}
      @group(0) @binding(1) var<storage, read_write> dst : Buffer;

      struct Params {
        origin: vec2u,
        extent: vec2u,
      };
      @group(0) @binding(2) var<uniform> params : Params;

      @compute @workgroup_size(${kWorkgroupSizeX}, ${kWorkgroupSizeY}, 1) fn main(@builtin(global_invocation_id) id : vec3u) {
        let boundary = params.origin + params.extent;
        let coord = params.origin + id.xy;
        if (any(coord >= boundary)) {
          return;
        }
        let offset = (id.x + id.y * params.extent.x) * ${componentCount} * ${sampleCount};
        for (var sampleIndex = 0u; sampleIndex < ${sampleCount};
          sampleIndex = sampleIndex + 1) {
          let o = offset + sampleIndex * ${componentCount};
          let v = textureLoad(src, coord.xy, sampleIndex);
          for (var component = 0u; component < ${componentCount}; component = component + 1) {
            dst.data[o + component] = v[component];
          }
        }
      }
    `;
    const computePipeline = this.device.createComputePipeline({
      layout: 'auto',
      compute: {
        module: this.device.createShaderModule({
          code
        }),
        entryPoint: 'main'
      }
    });

    const storageBuffer = this.createBufferTracked({
      label: 'copy2DTextureToBufferUsingComputePass:storageBuffer',
      size: sampleCount * type.size * componentCount * width * height,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST | GPUBufferUsage.COPY_SRC
    });

    const uniformBuffer = this.makeBufferWithContents(
      new Uint32Array([origin.x, origin.y, width, height]),
      GPUBufferUsage.UNIFORM
    );

    const uniformBindGroup = this.device.createBindGroup({
      layout: computePipeline.getBindGroupLayout(0),
      entries: [
      {
        binding: 0,
        resource: textureView
      },
      {
        binding: 1,
        resource: {
          buffer: storageBuffer
        }
      },
      {
        binding: 2,
        resource: {
          buffer: uniformBuffer
        }
      }]

    });

    const encoder = this.device.createCommandEncoder({
      label: 'copy2DTextureToBufferUsingComputePass'
    });
    const pass = encoder.beginComputePass();
    pass.setPipeline(computePipeline);
    pass.setBindGroup(0, uniformBindGroup);
    pass.dispatchWorkgroups(
      Math.floor((width + kWorkgroupSizeX - 1) / kWorkgroupSizeX),
      Math.floor((height + kWorkgroupSizeY - 1) / kWorkgroupSizeY),
      1
    );
    pass.end();
    this.device.queue.submit([encoder.finish()]);

    return storageBuffer;
  }

  /**
   * Expect the specified WebGPU error to be generated when running the provided function.
   */
  expectGPUError(filter, fn, shouldError = true) {
    // If no error is expected, we let the scope surrounding the test catch it.
    if (!shouldError) {
      return fn();
    }

    this.device.pushErrorScope(filter);
    const returnValue = fn();
    const promise = this.device.popErrorScope();

    this.eventualAsyncExpectation(async (niceStack) => {
      const error = await promise;

      let failed = false;
      switch (filter) {
        case 'out-of-memory':
          failed = !(error instanceof GPUOutOfMemoryError);
          break;
        case 'validation':
          failed = !(error instanceof GPUValidationError);
          break;
      }

      if (failed) {
        niceStack.message = `Expected ${filter} error`;
        this.rec.expectationFailed(niceStack);
      } else {
        niceStack.message = `Captured ${filter} error`;
        if (error instanceof GPUValidationError) {
          niceStack.message += ` - ${error.message}`;
        }
        this.rec.debug(niceStack);
      }
    });

    return returnValue;
  }

  /**
   * Expect a validation error inside the callback.
   *
   * Tests should always do just one WebGPU call in the callback, to make sure that's what's tested.
   */
  expectValidationError(fn, shouldError = true) {
    // If no error is expected, we let the scope surrounding the test catch it.
    if (shouldError) {
      this.device.pushErrorScope('validation');
    }

    // Note: A return value is not allowed for the callback function. This is to avoid confusion
    // about what the actual behavior would be; either of the following could be reasonable:
    //   - Make expectValidationError async, and have it await on fn(). This causes an async split
    //     between pushErrorScope and popErrorScope, so if the caller doesn't `await` on
    //     expectValidationError (either accidentally or because it doesn't care to do so), then
    //     other test code will be (nondeterministically) caught by the error scope.
    //   - Make expectValidationError NOT await fn(), but just execute its first block (until the
    //     first await) and return the return value (a Promise). This would be confusing because it
    //     would look like the error scope includes the whole async function, but doesn't.
    // If we do decide we need to return a value, we should use the latter semantic.
    const returnValue = fn();
    assert(
      returnValue === undefined,
      'expectValidationError callback should not return a value (or be async)'
    );

    if (shouldError) {
      const promise = this.device.popErrorScope();

      this.eventualAsyncExpectation(async (niceStack) => {
        const gpuValidationError = await promise;
        if (!gpuValidationError) {
          niceStack.message = 'Validation succeeded unexpectedly.';
          this.rec.validationFailed(niceStack);
        } else if (gpuValidationError instanceof GPUValidationError) {
          niceStack.message = `Validation failed, as expected - ${gpuValidationError.message}`;
          this.rec.debug(niceStack);
        }
      });
    }
  }

  /**
   * Expect a validation error or exception inside the callback.
   *
   * Tests should always do just one WebGPU call in the callback, to make sure that's what's tested.
   */
  expectValidationErrorOrException(
  fn,
  shouldError = true,
  shouldThrow = true)
  {
    if (shouldThrow) {
      this.shouldThrow(shouldError, fn);
    } else {
      this.expectValidationError(fn, shouldError);
    }
  }

  /** Create a GPUBuffer and track it for cleanup at the end of the test. */
  createBufferTracked(descriptor) {
    return this.trackForCleanup(this.device.createBuffer(descriptor));
  }

  /** Create a GPUTexture and track it for cleanup at the end of the test. */
  createTextureTracked(descriptor) {
    return this.trackForCleanup(this.device.createTexture(descriptor));
  }

  /** Create a GPUQuerySet and track it for cleanup at the end of the test. */
  createQuerySetTracked(descriptor) {
    return this.trackForCleanup(this.device.createQuerySet(descriptor));
  }

  /**
   * Creates a buffer with the contents of some TypedArray.
   * The buffer size will always be aligned to 4 as we set mappedAtCreation === true when creating the
   * buffer.
   *
   * MAINTENANCE_TODO: Several call sites would be simplified if this took ArrayBuffer as well.
   */
  makeBufferWithContents(dataArray, usage) {
    const buffer = this.createBufferTracked({
      mappedAtCreation: true,
      size: align(dataArray.byteLength, 4),
      usage
    });
    memcpy({ src: dataArray }, { dst: buffer.getMappedRange() });
    buffer.unmap();
    return buffer;
  }

  /**
   * Returns a GPUCommandEncoder, GPUComputePassEncoder, GPURenderPassEncoder, or
   * GPURenderBundleEncoder, and a `finish` method returning a GPUCommandBuffer.
   * Allows testing methods which have the same signature across multiple encoder interfaces.
   *
   * @example
   * ```
   * g.test('popDebugGroup')
   *   .params(u => u.combine('encoderType', kEncoderTypes))
   *   .fn(t => {
   *     const { encoder, finish } = t.createEncoder(t.params.encoderType);
   *     encoder.popDebugGroup();
   *   });
   *
   * g.test('writeTimestamp')
   *   .params(u => u.combine('encoderType', ['non-pass', 'compute pass', 'render pass'] as const)
   *   .fn(t => {
   *     const { encoder, finish } = t.createEncoder(t.params.encoderType);
   *     // Encoder type is inferred, so `writeTimestamp` can be used even though it doesn't exist
   *     // on GPURenderBundleEncoder.
   *     encoder.writeTimestamp(args);
   *   });
   * ```
   */
  createEncoder(
  encoderType,
  {
    attachmentInfo,
    occlusionQuerySet,
    targets




  } = {})
  {
    const fullAttachmentInfo = {
      // Defaults if not overridden:
      colorFormats: ['rgba8unorm'],
      sampleCount: 1,
      // Passed values take precedent.
      ...attachmentInfo
    };

    switch (encoderType) {
      case 'non-pass':{
          const encoder = this.device.createCommandEncoder();

          return new CommandBufferMaker(this, encoder, () => {
            return encoder.finish();
          });
        }
      case 'render bundle':{
          const device = this.device;
          const rbEncoder = device.createRenderBundleEncoder(fullAttachmentInfo);
          const pass = this.createEncoder('render pass', { attachmentInfo, targets });

          return new CommandBufferMaker(this, rbEncoder, () => {
            pass.encoder.executeBundles([rbEncoder.finish()]);
            return pass.finish();
          });
        }
      case 'compute pass':{
          const commandEncoder = this.device.createCommandEncoder();
          const encoder = commandEncoder.beginComputePass();

          return new CommandBufferMaker(this, encoder, () => {
            encoder.end();
            return commandEncoder.finish();
          });
        }
      case 'render pass':{
          const makeAttachmentView = (format) =>
          this.createTextureTracked({
            size: [16, 16, 1],
            format,
            usage: GPUTextureUsage.RENDER_ATTACHMENT,
            sampleCount: fullAttachmentInfo.sampleCount
          }).createView();

          let depthStencilAttachment = undefined;
          if (fullAttachmentInfo.depthStencilFormat !== undefined) {
            depthStencilAttachment = {
              view: makeAttachmentView(fullAttachmentInfo.depthStencilFormat),
              depthReadOnly: fullAttachmentInfo.depthReadOnly,
              stencilReadOnly: fullAttachmentInfo.stencilReadOnly
            };
            if (
            isDepthTextureFormat(fullAttachmentInfo.depthStencilFormat) &&
            !fullAttachmentInfo.depthReadOnly)
            {
              depthStencilAttachment.depthClearValue = 0;
              depthStencilAttachment.depthLoadOp = 'clear';
              depthStencilAttachment.depthStoreOp = 'discard';
            }
            if (
            isStencilTextureFormat(fullAttachmentInfo.depthStencilFormat) &&
            !fullAttachmentInfo.stencilReadOnly)
            {
              depthStencilAttachment.stencilClearValue = 1;
              depthStencilAttachment.stencilLoadOp = 'clear';
              depthStencilAttachment.stencilStoreOp = 'discard';
            }
          }
          const passDesc = {
            colorAttachments: Array.from(fullAttachmentInfo.colorFormats, (format, i) =>
            format ?
            {
              view: targets ? targets[i] : makeAttachmentView(format),
              clearValue: [0, 0, 0, 0],
              loadOp: 'clear',
              storeOp: 'store'
            } :
            null
            ),
            depthStencilAttachment,
            occlusionQuerySet
          };

          const commandEncoder = this.device.createCommandEncoder();
          const encoder = commandEncoder.beginRenderPass(passDesc);
          return new CommandBufferMaker(this, encoder, () => {
            encoder.end();
            return commandEncoder.finish();
          });
        }
    }
    unreachable();
  }
}

/**
 * Fixture for WebGPU tests that uses a DeviceProvider
 */
class GPUTest extends GPUTestBase {
  // Should never be undefined in a test. If it is, init() must not have run/finished.



  async init() {
    await super.init();

    this.provider = await this.sharedState.acquireProvider();
    this.mismatchedProvider = await this.sharedState.acquireMismatchedProvider();
  }

  /** GPUAdapter that the device was created from. */
  get adapter() {
    assert(this.provider !== undefined, 'internal error: DeviceProvider missing');
    return this.provider.adapter;
  }

  /**
   * GPUDevice for the test to use.
   */
  get device() {
    assert(this.provider !== undefined, 'internal error: DeviceProvider missing');
    return this.provider.device;
  }

  /**
   * GPUDevice for tests requiring a second device different from the default one,
   * e.g. for creating objects for by device_mismatch validation tests.
   */
  get mismatchedDevice() {
    assert(
      this.mismatchedProvider !== undefined,
      'usesMismatchedDevice or selectMismatchedDeviceOrSkipTestCase was not called in beforeAllSubcases'
    );
    return this.mismatchedProvider.device;
  }

  /**
   * Expects that the device should be lost for a particular reason at the teardown of the test.
   */
  expectDeviceLost(reason) {
    assert(this.provider !== undefined, 'internal error: GPUDevice missing?');
    this.provider.expectDeviceLost(reason);
  }
}

/**
 * Gets the adapter limits as a standard JavaScript object.
 */
function getAdapterLimitsAsDeviceRequiredLimits(adapter) {
  const requiredLimits = {};
  const adapterLimits = adapter.limits;
  for (const key in adapter.limits) {
    // MAINTENANCE_TODO: Remove this once minSubgroupSize is removed from
    // chromium.
    if (key === 'maxSubgroupSize' || key === 'minSubgroupSize') {
      continue;
    }
    requiredLimits[key] = adapterLimits[key];
  }
  return requiredLimits;
}

/**
 * Removes limits that don't exist on the adapter.
 * A test might request a new limit that not all implementations support. The test itself
 * should check the requested limit using code that expects undefined.
 *
 * ```ts
 *    t.skipIf(limit < 2);     // BAD! Doesn't skip if unsupported because undefined is never less than 2.
 *    t.skipIf(!(limit >= 2)); // Good. Skips if limits is not >= 2. undefined is not >= 2.
 * ```
 */
function removeNonExistentLimits(adapter, limits) {
  const filteredLimits = {};
  const adapterLimits = adapter.limits;
  for (const [limit, value] of Object.entries(limits)) {
    if (adapterLimits[limit] !== undefined) {
      filteredLimits[limit] = value;
    }
  }
  return filteredLimits;
}

function applyLimitsToDescriptor(
adapter,
desc,
getRequiredLimits)
{
  const descWithMaxLimits = {
    requiredFeatures: [],
    defaultQueue: {},
    ...desc,
    requiredLimits: removeNonExistentLimits(adapter, getRequiredLimits(adapter))
  };
  return descWithMaxLimits;
}

function getAdapterFeaturesAsDeviceRequiredFeatures(adapter) {
  return [...adapter.features].filter(
    (f) => f !== 'core-features-and-limits'
  );
}

function applyFeaturesToDescriptor(
adapter,
desc,
getRequiredFeatures)
{
  const existingRequiredFeatures = (desc && desc?.requiredFeatures) ?? [];
  const descWithRequiredFeatures = {
    requiredLimits: {},
    defaultQueue: {},
    ...desc,
    requiredFeatures: [...existingRequiredFeatures, ...getRequiredFeatures(adapter)]
  };
  return descWithRequiredFeatures;
}

/**
 * Used by RequiredLimitsTestMixin to allow you to request specific limits
 *
 * Supply a `getRequiredLimits` function that given a GPUAdapter, turns the limits
 * you want.
 *
 * Also supply a key function that returns a device key. You should generally return
 * the name of each limit you request and any math you did on the limit. For example
 *
 * ```js
 * {
 *   getRequiredLimits(adapter) {
 *     return {
 *       maxBindGroups: adapter.limits.maxBindGroups / 2,
 *       maxTextureDimensions2D: Math.max(adapter.limits.maxTextureDimensions2D, 8192),
 *     },
 *   },
 *   key() {
 *     return `
 *       maxBindGroups / 2,
 *       max(maxTextureDimension2D, 8192),
 *     `;
 *   },
 * }
 * ```
 *
 * Its important to note, the key is used BEFORE knowing the adapter limits to get a device
 * that was already created with the same key.
 */





/**
 * Used by RequiredLimitsTest to request a device with all requested limits of the adapter.
 */
class RequiredLimitsGPUTestSubcaseBatchState extends GPUTestSubcaseBatchState {

  constructor(
  recorder,
  params,
  requiredLimitsHelper)
  {
    super(recorder, params);this.recorder = recorder;this.params = params;
    this.requiredLimitsHelper = requiredLimitsHelper;
  }
  requestDeviceWithRequiredParametersOrSkip(
  descriptor,
  descriptorModifier)
  {
    const requiredLimitsHelper = this.requiredLimitsHelper;
    const mod = {
      descriptorModifier(adapter, desc) {
        desc = descriptorModifier?.descriptorModifier ?
        descriptorModifier.descriptorModifier(adapter, desc) :
        desc;
        return applyLimitsToDescriptor(adapter, desc, requiredLimitsHelper.getRequiredLimits);
      },
      keyModifier(baseKey) {
        return `${baseKey}:${requiredLimitsHelper.key()}`;
      }
    };
    super.requestDeviceWithRequiredParametersOrSkip(
      initUncanonicalizedDeviceDescriptor(descriptor),
      mod
    );
  }
}





/**
 * A text mixin to make it relatively easy to request specific limits.
 */
function RequiredLimitsTestMixin(
Base,
requiredLimitsHelper)
{
  class RequiredLimitsImpl extends
  Base

  {
    //
    static MakeSharedState(
    recorder,
    params)
    {
      return new RequiredLimitsGPUTestSubcaseBatchState(recorder, params, requiredLimitsHelper);
    }
  }

  return RequiredLimitsImpl;
}

/**
 * Used by AllFeaturesMaxLimitsGPUTest to request a device with all limits and features of the adapter.
 */
class AllFeaturesMaxLimitsGPUTestSubcaseBatchState extends GPUTestSubcaseBatchState {
  constructor(
  recorder,
  params)
  {
    super(recorder, params);this.recorder = recorder;this.params = params;
  }
  requestDeviceWithRequiredParametersOrSkip(
  descriptor,
  descriptorModifier)
  {
    const mod = {
      descriptorModifier(adapter, desc) {
        desc = descriptorModifier?.descriptorModifier ?
        descriptorModifier.descriptorModifier(adapter, desc) :
        desc;
        desc = applyLimitsToDescriptor(adapter, desc, getAdapterLimitsAsDeviceRequiredLimits);
        desc = applyFeaturesToDescriptor(adapter, desc, getAdapterFeaturesAsDeviceRequiredFeatures);
        return desc;
      },
      keyModifier(baseKey) {
        return `${baseKey}:AllFeaturesMaxLimits`;
      }
    };
    super.requestDeviceWithRequiredParametersOrSkip(
      initUncanonicalizedDeviceDescriptor(descriptor),
      mod
    );
  }

  /**
   * Use skipIfDeviceDoesNotHaveFeature or similar. If you really need to test
   * lack of a feature (for example tests under webgpu/api/validation/capability_checks)
   * then use UniqueFeaturesOrLimitsGPUTest
   */
  selectDeviceOrSkipTestCase(descriptor) {
    unreachable('this function should not be called in AllFeaturesMaxLimitsGPUTest');
  }

  /**
   * Use skipIfDeviceDoesNotHaveFeature or similar.
   */
  selectDeviceForQueryTypeOrSkipTestCase(types) {
    unreachable('this function should not be called in AllFeaturesMaxLimitsGPUTest');
  }

  /**
   * Use skipIfDeviceDoesNotHaveFeature or skipIf(device.limits.maxXXX < requiredXXX) etc...
   */
  selectDeviceForTextureFormatOrSkipTestCase(
  formats)
  {
    unreachable('this function should not be called in AllFeaturesMaxLimitsGPUTest');
  }

  /**
   * Use skipIfDeviceDoesNotHaveFeature or skipIf(device.limits.maxXXX < requiredXXX) etc...
   */
  selectMismatchedDeviceOrSkipTestCase(descriptor) {
    unreachable('this function should not be called in AllFeaturesMaxLimitsGPUTest');
  }
}

/**
 * Most tests should be using `AllFeaturesMaxLimitsGPUTest`. The exceptions
 * are tests specifically validating limits like those under api/validation/capability_checks/limits
 * and those tests the specifically validate certain features fail validation if not enabled
 * like those under api/validation/capability_checks/feature.
 *
 * NOTE: The goal is to go through all existing tests and remove any direct use of GPUTest.
 * For each test, choose either AllFeaturesMaxLimitsGPUTest or UniqueFeaturesOrLimitsGPUTest.
 * This way we can track progress as we go through every test using GPUTest and check it is
 * testing everything it should test.
 */
class UniqueFeaturesOrLimitsGPUTest extends GPUTest {}

/**
 * A test that requests all features and maximum limits. This should be the default
 * test for the majority of tests, otherwise optional features will not be tested.
 * The exceptions are only tests that explicitly test the absence of a feature or
 * specific limits such as the tests under validation/capability_checks.
 *
 * As a concrete example to demonstrate the issue, texture format `rg11b10ufloat` is
 * optionally renderable and can optionally be used multisampled. Any test that tests
 * texture formats should test this format, skipping only if the feature is missing.
 * So, the default should be that the test tests `kAllTextureFormats` with the appropriate
 * filters from format_info.ts or the various helpers. This way, `rg11b10ufloat` will
 * included in the test and fail if not appropriately filtered. If instead you were
 * to use GPUTest then `rg11b10ufloat` would just be skipped as its never enabled.
 * You could enable it manually but that spreads enabling to every test instead of being
 * centralized in one place, here.
 */
class AllFeaturesMaxLimitsGPUTest extends GPUTest {
  static MakeSharedState(
  recorder,
  params)
  {
    return new AllFeaturesMaxLimitsGPUTestSubcaseBatchState(recorder, params);
  }
}
});
__define('webgpu/format_info.js', function (__exports) {
'use strict';
__get(__exports, 'kRegularTextureFormats', () => kRegularTextureFormats);
__get(__exports, 'kSizedDepthStencilFormats', () => kSizedDepthStencilFormats);
__get(__exports, 'kUnsizedDepthStencilFormats', () => kUnsizedDepthStencilFormats);
__get(__exports, 'kCompressedTextureFormats', () => kCompressedTextureFormats);
__get(__exports, 'kBCCompressedTextureFormats', () => kBCCompressedTextureFormats);
__get(__exports, 'kASTCCompressedTextureFormats', () => kASTCCompressedTextureFormats);
__get(__exports, 'kColorTextureFormats', () => kColorTextureFormats);
__get(__exports, 'kEncodableTextureFormats', () => kEncodableTextureFormats);
__get(__exports, 'kSizedTextureFormats', () => kSizedTextureFormats);
__get(__exports, 'kDepthStencilFormats', () => kDepthStencilFormats);
__get(__exports, 'kUncompressedTextureFormats', () => kUncompressedTextureFormats);
__get(__exports, 'kAllTextureFormats', () => kAllTextureFormats);
__get(__exports, 'kDepthTextureFormats', () => kDepthTextureFormats);
__get(__exports, 'kStencilTextureFormats', () => kStencilTextureFormats);
__get(__exports, 'kTextureFormatTier1AllowsResolve', () => kTextureFormatTier1AllowsResolve);
__get(__exports, 'kTextureFormatTier1ThrowsWhenNotEnabled', () => kTextureFormatTier1ThrowsWhenNotEnabled);
__get(__exports, 'kTextureFormatTier1AllowsRenderAttachmentBlendableMultisample', () => kTextureFormatTier1AllowsRenderAttachmentBlendableMultisample);
__get(__exports, 'kTextureFormatsTier1EnablesStorageReadOnlyWriteOnly', () => kTextureFormatsTier1EnablesStorageReadOnlyWriteOnly);
__get(__exports, 'kTextureFormatsTier2EnablesStorageReadWrite', () => kTextureFormatsTier2EnablesStorageReadWrite);
__get(__exports, 'kPossibleStorageTextureFormats', () => kPossibleStorageTextureFormats);
__get(__exports, 'kPossibleReadWriteStorageTextureFormats', () => kPossibleReadWriteStorageTextureFormats);
__get(__exports, 'kPossibleMultisampledTextureFormats', () => kPossibleMultisampledTextureFormats);
__get(__exports, 'kPossibleColorRenderableTextureFormats', () => kPossibleColorRenderableTextureFormats);
__get(__exports, 'kDifferentBaseFormatTextureFormats', () => kDifferentBaseFormatTextureFormats);
__get(__exports, 'kDifferentBaseFormatRegularTextureFormats', () => kDifferentBaseFormatRegularTextureFormats);
__get(__exports, 'kOptionalTextureFormats', () => kOptionalTextureFormats);
__get(__exports, 'isTextureFormatPossiblyUsableWithCopyExternalImageToTexture', () => isTextureFormatPossiblyUsableWithCopyExternalImageToTexture);
__get(__exports, 'isTextureFormatUsableWithCopyExternalImageToTexture', () => isTextureFormatUsableWithCopyExternalImageToTexture);
__get(__exports, 'kDepthStencilFormatResolvedAspect', () => kDepthStencilFormatResolvedAspect);
__get(__exports, 'resolvePerAspectFormat', () => resolvePerAspectFormat);
__get(__exports, 'sampleTypeForFormatAndAspect', () => sampleTypeForFormatAndAspect);
__get(__exports, 'depthStencilFormatCopyableAspects', () => depthStencilFormatCopyableAspects);
__get(__exports, 'depthStencilBufferTextureCopySupported', () => depthStencilBufferTextureCopySupported);
__get(__exports, 'depthStencilFormatAspectSize', () => depthStencilFormatAspectSize);
__get(__exports, 'textureFormatAndDimensionPossiblyCompatible', () => textureFormatAndDimensionPossiblyCompatible);
__get(__exports, 'textureDimensionAndFormatCompatibleForDevice', () => textureDimensionAndFormatCompatibleForDevice);
__get(__exports, 'textureViewDimensionAndFormatCompatibleForDevice', () => textureViewDimensionAndFormatCompatibleForDevice);
__get(__exports, 'textureFormatsAreViewCompatible', () => textureFormatsAreViewCompatible);
__get(__exports, 'getBlockInfoForColorTextureFormat', () => getBlockInfoForColorTextureFormat);
__get(__exports, 'getBlockInfoForSizedTextureFormat', () => getBlockInfoForSizedTextureFormat);
__get(__exports, 'getBlockInfoForEncodableTextureFormat', () => getBlockInfoForEncodableTextureFormat);
__get(__exports, 'getBlockInfoForTextureFormat', () => getBlockInfoForTextureFormat);
__get(__exports, 'getColorRenderByteCost', () => getColorRenderByteCost);
__get(__exports, 'getColorRenderAlignment', () => getColorRenderAlignment);
__get(__exports, 'getBaseFormatForTextureFormat', () => getBaseFormatForTextureFormat);
__get(__exports, 'getBaseFormatForRegularTextureFormat', () => getBaseFormatForRegularTextureFormat);
__get(__exports, 'getRequiredFeatureForTextureFormat', () => getRequiredFeatureForTextureFormat);
__get(__exports, 'getFeaturesForFormats', () => getFeaturesForFormats);
__get(__exports, 'filterFormatsByFeature', () => filterFormatsByFeature);
__get(__exports, 'canCopyToAspectOfTextureFormat', () => canCopyToAspectOfTextureFormat);
__get(__exports, 'canCopyFromAspectOfTextureFormat', () => canCopyFromAspectOfTextureFormat);
__get(__exports, 'canCopyToAllAspectsOfTextureFormat', () => canCopyToAllAspectsOfTextureFormat);
__get(__exports, 'canCopyFromAllAspectsOfTextureFormat', () => canCopyFromAllAspectsOfTextureFormat);
__get(__exports, 'isCompressedTextureFormat', () => isCompressedTextureFormat);
__get(__exports, 'isBCTextureFormat', () => isBCTextureFormat);
__get(__exports, 'isASTCTextureFormat', () => isASTCTextureFormat);
__get(__exports, 'isColorTextureFormat', () => isColorTextureFormat);
__get(__exports, 'isColorTextureFormatWithAlpha', () => isColorTextureFormatWithAlpha);
__get(__exports, 'isDepthTextureFormat', () => isDepthTextureFormat);
__get(__exports, 'isStencilTextureFormat', () => isStencilTextureFormat);
__get(__exports, 'isDepthStencilTextureFormat', () => isDepthStencilTextureFormat);
__get(__exports, 'isDepthOrStencilTextureFormat', () => isDepthOrStencilTextureFormat);
__get(__exports, 'isEncodableTextureFormat', () => isEncodableTextureFormat);
__get(__exports, 'isTextureFormatUsableAsRenderAttachment', () => isTextureFormatUsableAsRenderAttachment);
__get(__exports, 'isTextureFormatColorRenderable', () => isTextureFormatColorRenderable);
__get(__exports, 'isTextureFormatBlendable', () => isTextureFormatBlendable);
__get(__exports, 'isTextureFormatFilterable', () => isTextureFormatFilterable);
__get(__exports, 'getTextureFormatType', () => getTextureFormatType);
__get(__exports, 'getTextureFormatColorType', () => getTextureFormatColorType);
__get(__exports, 'isTextureFormatPossiblyUsableAsRenderAttachment', () => isTextureFormatPossiblyUsableAsRenderAttachment);
__get(__exports, 'isTextureFormatPossiblyUsableAsColorRenderAttachment', () => isTextureFormatPossiblyUsableAsColorRenderAttachment);
__get(__exports, 'isTextureFormatPossiblyMultisampled', () => isTextureFormatPossiblyMultisampled);
__get(__exports, 'isTextureFormatPossiblyResolvable', () => isTextureFormatPossiblyResolvable);
__get(__exports, 'isTextureFormatPossiblyStorageReadable', () => isTextureFormatPossiblyStorageReadable);
__get(__exports, 'isTextureFormatPossiblyStorageReadWritable', () => isTextureFormatPossiblyStorageReadWritable);
__get(__exports, 'is16Float', () => is16Float);
__get(__exports, 'is32Float', () => is32Float);
__get(__exports, 'isTextureFormatPossiblyFilterableAsTextureF32', () => isTextureFormatPossiblyFilterableAsTextureF32);
__get(__exports, 'kCompatModeUnsupportedStorageTextureFormats', () => kCompatModeUnsupportedStorageTextureFormats);
__get(__exports, 'isTextureFormatUsableWithStorageAccessMode', () => isTextureFormatUsableWithStorageAccessMode);
__get(__exports, 'isTextureFormatUsableAsStorageFormatInCreateShaderModule', () => isTextureFormatUsableAsStorageFormatInCreateShaderModule);
__get(__exports, 'isRegularTextureFormat', () => isRegularTextureFormat);
__get(__exports, 'isCompressedFloatTextureFormat', () => isCompressedFloatTextureFormat);
__get(__exports, 'isSintOrUintFormat', () => isSintOrUintFormat);
__get(__exports, 'kCompatModeUnsupportedMultisampledTextureFormats', () => kCompatModeUnsupportedMultisampledTextureFormats);
__get(__exports, 'isTextureFormatMultisampled', () => isTextureFormatMultisampled);
__get(__exports, 'isTextureFormatResolvable', () => isTextureFormatResolvable);
__get(__exports, 'kFeaturesForFormats', () => kFeaturesForFormats);
__get(__exports, 'computeBytesPerSampleFromFormats', () => computeBytesPerSampleFromFormats);
__get(__exports, 'computeBytesPerSample', () => computeBytesPerSample);
__get(__exports, 'getMaxValidTextureSizeForFormatAndDimension', () => getMaxValidTextureSizeForFormatAndDimension);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let isCompatibilityMode; __bind('common/framework/test_config.js', (__m) => { ({ isCompatibilityMode } = __m); });let keysOf; __bind('common/util/data_tables.js', (__m) => { ({ keysOf } = __m); });let assert, unreachable, hasFeature; __bind('common/util/util.js', (__m) => { ({ assert, unreachable, hasFeature } = __m); });

let align, roundDown; __bind('webgpu/util/math.js', (__m) => { ({ align, roundDown } = __m); });
let getTextureDimensionFromView; __bind('webgpu/util/texture/base.js', (__m) => { ({ getTextureDimensionFromView } = __m); });


//
// Texture format tables
//

/**
 * Defaults applied to all texture format tables automatically. Used only inside
 * `formatTableWithDefaults`. This ensures keys are never missing, always explicitly `undefined`.
 *
 * All top-level keys must be defined here, or they won't be exposed at all.
 * Documentation is also written here; this makes it propagate through to the end types.
 */
const kFormatUniversalDefaults = {
  /** Texel block width. */
  blockWidth: undefined,
  /** Texel block height. */
  blockHeight: undefined,
  color: undefined,
  depth: undefined,
  stencil: undefined,
  /**
   * Info when this format can be used as a color render target. The format may require a feature
   * to actually be used as a render target. Eg: rg11b10ufloat which requires rg11b10ufloat-renderable
   * Call {@link isTextureFormatPossiblyUsableAsColorRenderAttachment} before having a device
   * Call {@link isTextureFormatColorRenderable}(device, format) to find out for a particular device.
   * Use {@link kPossibleColorRenderableTextureFormats} for params.
   */
  colorRender: undefined,
  /**
   * Whether the format can possibly be used as a multisample texture. The format may require a
   * feature to actually multisampled. Eg: rg11b10ufloat which requires rg11b10ufloat-renderable
   * Call {@link isTextureFormatPossiblyMultisampled} before having a device
   * Call {@link isTextureFormatMultisampled}(device, format) to find out for a particular device.
   * Use {@link kPossibleMultisampledTextureFormats} for params.
   */
  multisample: undefined,
  /** Optional feature required to use this format, or `undefined` if none. */
  feature: undefined,
  /** The base format for srgb formats. Specified on both srgb and equivalent non-srgb formats. */
  baseFormat: undefined,

  /** @deprecated Use `.color.bytes`, `.depth.bytes`, or `.stencil.bytes`. */
  bytesPerBlock: undefined

  // IMPORTANT:
  // Add new top-level keys both here and in TextureFormatInfo_TypeCheck.
};
/**
 * Takes `table` and applies `defaults` to every row, i.e. for each row,
 * `{ ... kUniversalDefaults, ...defaults, ...row }`.
 * This only operates at the first level; it doesn't support defaults in nested objects.
 */
function formatTableWithDefaults({
  defaults,
  table



})







{
  return Object.fromEntries(
    Object.entries(table).map(([k, row]) => [
    k,
    { ...kFormatUniversalDefaults, ...defaults, ...row }]
    )

  );
}

/** "plain color formats", plus rgb9e5ufloat. */
const kRegularTextureFormatInfo = formatTableWithDefaults({
  defaults: { blockWidth: 1, blockHeight: 1 },
  table: {
    // plain, 8 bits per component

    r8unorm: {
      color: {
        type: 'float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 1
      },
      colorRender: { blend: true, resolve: true, byteCost: 1, alignment: 1 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    r8snorm: {
      color: {
        type: 'float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 1
      },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    r8uint: {
      color: {
        type: 'uint',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 1
      },
      colorRender: { blend: false, resolve: false, byteCost: 1, alignment: 1 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    r8sint: {
      color: {
        type: 'sint',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 1
      },
      colorRender: { blend: false, resolve: false, byteCost: 1, alignment: 1 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },

    rg8unorm: {
      color: {
        type: 'float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 2
      },
      colorRender: { blend: true, resolve: true, byteCost: 2, alignment: 1 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg8snorm: {
      color: {
        type: 'float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 2
      },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg8uint: {
      color: {
        type: 'uint',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 2
      },
      colorRender: { blend: false, resolve: false, byteCost: 2, alignment: 1 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg8sint: {
      color: {
        type: 'sint',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 2
      },
      colorRender: { blend: false, resolve: false, byteCost: 2, alignment: 1 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },

    rgba8unorm: {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: true, byteCost: 8, alignment: 1 },
      multisample: true,
      baseFormat: 'rgba8unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'rgba8unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: true, byteCost: 8, alignment: 1 },
      multisample: true,
      baseFormat: 'rgba8unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba8snorm: {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 4
      },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba8uint: {
      color: {
        type: 'uint',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: false, resolve: false, byteCost: 4, alignment: 1 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba8sint: {
      color: {
        type: 'sint',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: false, resolve: false, byteCost: 4, alignment: 1 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    bgra8unorm: {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: true, byteCost: 8, alignment: 1 },
      multisample: true,
      baseFormat: 'bgra8unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'bgra8unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: true, byteCost: 8, alignment: 1 },
      multisample: true,
      baseFormat: 'bgra8unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    // plain, 16 bits per component

    r16unorm: {
      color: {
        type: 'unfilterable-float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 2
      },
      colorRender: { blend: true, resolve: false, byteCost: 2, alignment: 2 },
      multisample: true,
      feature: 'texture-formats-tier1',
      get bytesPerBlock() {return this.color.bytes;}
    },
    r16snorm: {
      color: {
        type: 'unfilterable-float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 2
      },
      colorRender: { blend: true, resolve: false, byteCost: 2, alignment: 2 },
      multisample: true,
      feature: 'texture-formats-tier1',
      get bytesPerBlock() {return this.color.bytes;}
    },
    r16uint: {
      color: {
        type: 'uint',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 2
      },
      colorRender: { blend: false, resolve: false, byteCost: 2, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    r16sint: {
      color: {
        type: 'sint',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 2
      },
      colorRender: { blend: false, resolve: false, byteCost: 2, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    r16float: {
      color: {
        type: 'float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 2
      },
      colorRender: { blend: true, resolve: true, byteCost: 2, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },

    rg16unorm: {
      color: {
        type: 'unfilterable-float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: false, byteCost: 4, alignment: 2 },
      multisample: true,
      feature: 'texture-formats-tier1',
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg16snorm: {
      color: {
        type: 'unfilterable-float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: false, byteCost: 4, alignment: 2 },
      multisample: true,
      feature: 'texture-formats-tier1',
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg16uint: {
      color: {
        type: 'uint',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: false, resolve: false, byteCost: 4, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg16sint: {
      color: {
        type: 'sint',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: false, resolve: false, byteCost: 4, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg16float: {
      color: {
        type: 'float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: true, byteCost: 4, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },

    rgba16unorm: {
      color: {
        type: 'unfilterable-float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 8
      },
      colorRender: { blend: true, resolve: false, byteCost: 8, alignment: 4 },
      multisample: true,
      feature: 'texture-formats-tier1',
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba16snorm: {
      color: {
        type: 'unfilterable-float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 8
      },
      colorRender: { blend: true, resolve: false, byteCost: 8, alignment: 2 },
      multisample: true,
      feature: 'texture-formats-tier1',
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba16uint: {
      color: {
        type: 'uint',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 8
      },
      colorRender: { blend: false, resolve: false, byteCost: 8, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba16sint: {
      color: {
        type: 'sint',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 8
      },
      colorRender: { blend: false, resolve: false, byteCost: 8, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba16float: {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 8
      },
      colorRender: { blend: true, resolve: true, byteCost: 8, alignment: 2 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },

    // plain, 32 bits per component

    r32uint: {
      color: {
        type: 'uint',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: true,
        bytes: 4
      },
      colorRender: { blend: false, resolve: false, byteCost: 4, alignment: 4 },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    r32sint: {
      color: {
        type: 'sint',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: true,
        bytes: 4
      },
      colorRender: { blend: false, resolve: false, byteCost: 4, alignment: 4 },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    r32float: {
      color: {
        type: 'unfilterable-float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: true,
        bytes: 4
      },
      colorRender: { blend: false, resolve: false, byteCost: 4, alignment: 4 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },

    rg32uint: {
      color: {
        type: 'uint',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 8
      },
      colorRender: { blend: false, resolve: false, byteCost: 8, alignment: 4 },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg32sint: {
      color: {
        type: 'sint',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 8
      },
      colorRender: { blend: false, resolve: false, byteCost: 8, alignment: 4 },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg32float: {
      color: {
        type: 'unfilterable-float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 8
      },
      colorRender: { blend: false, resolve: false, byteCost: 8, alignment: 4 },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },

    rgba32uint: {
      color: {
        type: 'uint',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 16
      },
      colorRender: { blend: false, resolve: false, byteCost: 16, alignment: 4 },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba32sint: {
      color: {
        type: 'sint',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 16
      },
      colorRender: { blend: false, resolve: false, byteCost: 16, alignment: 4 },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgba32float: {
      color: {
        type: 'unfilterable-float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: true,
        readWriteStorage: false,
        bytes: 16
      },
      colorRender: { blend: false, resolve: false, byteCost: 16, alignment: 4 },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    },

    // plain, mixed component width, 32 bits per texel

    rgb10a2uint: {
      color: {
        type: 'uint',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: false, resolve: false, byteCost: 8, alignment: 4 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rgb10a2unorm: {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: true, byteCost: 8, alignment: 4 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },
    rg11b10ufloat: {
      color: {
        type: 'float',
        componentCount: 3,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      colorRender: { blend: true, resolve: true, byteCost: 8, alignment: 4 },
      multisample: true,
      get bytesPerBlock() {return this.color.bytes;}
    },

    // packed

    rgb9e5ufloat: {
      color: {
        type: 'float',
        componentCount: 3,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      multisample: false,
      get bytesPerBlock() {return this.color.bytes;}
    }
  }
});

// MAINTENANCE_TODO: Distinguishing "sized" and "unsized" depth stencil formats doesn't make sense
// because one aspect can be sized and one can be unsized. This should be cleaned up, but is kept
// this way during a migration phase.
const kSizedDepthStencilFormatInfo = formatTableWithDefaults({
  defaults: { blockWidth: 1, blockHeight: 1, multisample: true },
  table: {
    stencil8: {
      stencil: {
        type: 'uint',
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 1
      },
      bytesPerBlock: 1
    },
    depth16unorm: {
      depth: {
        type: 'depth',
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 2
      },
      bytesPerBlock: 2
    },
    depth32float: {
      depth: {
        type: 'depth',
        copySrc: true,
        copyDst: false,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      bytesPerBlock: 4
    }
  }
});
const kUnsizedDepthStencilFormatInfo = formatTableWithDefaults({
  defaults: { blockWidth: 1, blockHeight: 1, multisample: true },
  table: {
    depth24plus: {
      depth: {
        type: 'depth',
        copySrc: false,
        copyDst: false,
        storage: false,
        readWriteStorage: false,
        bytes: undefined
      }
    },
    'depth24plus-stencil8': {
      depth: {
        type: 'depth',
        copySrc: false,
        copyDst: false,
        storage: false,
        readWriteStorage: false,
        bytes: undefined
      },
      stencil: {
        type: 'uint',
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 1
      }
    },
    'depth32float-stencil8': {
      depth: {
        type: 'depth',
        copySrc: true,
        copyDst: false,
        storage: false,
        readWriteStorage: false,
        bytes: 4
      },
      stencil: {
        type: 'uint',
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 1
      },
      feature: 'depth32float-stencil8'
    }
  }
});

const kBCTextureFormatInfo = formatTableWithDefaults({
  defaults: {
    blockWidth: 4,
    blockHeight: 4,
    multisample: false,
    feature: 'texture-compression-bc'
  },
  table: {
    'bc1-rgba-unorm': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      baseFormat: 'bc1-rgba-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'bc1-rgba-unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      baseFormat: 'bc1-rgba-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'bc2-rgba-unorm': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'bc2-rgba-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'bc2-rgba-unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'bc2-rgba-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'bc3-rgba-unorm': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'bc3-rgba-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'bc3-rgba-unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'bc3-rgba-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'bc4-r-unorm': {
      color: {
        type: 'float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      get bytesPerBlock() {return this.color.bytes;}
    },
    'bc4-r-snorm': {
      color: {
        type: 'float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      get bytesPerBlock() {return this.color.bytes;}
    },

    'bc5-rg-unorm': {
      color: {
        type: 'float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      get bytesPerBlock() {return this.color.bytes;}
    },
    'bc5-rg-snorm': {
      color: {
        type: 'float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      get bytesPerBlock() {return this.color.bytes;}
    },

    'bc6h-rgb-ufloat': {
      color: {
        type: 'float',
        componentCount: 3,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      get bytesPerBlock() {return this.color.bytes;}
    },
    'bc6h-rgb-float': {
      color: {
        type: 'float',
        componentCount: 3,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      get bytesPerBlock() {return this.color.bytes;}
    },

    'bc7-rgba-unorm': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'bc7-rgba-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'bc7-rgba-unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'bc7-rgba-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    }
  }
});

const kETC2TextureFormatInfo = formatTableWithDefaults({
  defaults: {
    blockWidth: 4,
    blockHeight: 4,
    multisample: false,
    feature: 'texture-compression-etc2'
  },
  table: {
    'etc2-rgb8unorm': {
      color: {
        type: 'float',
        componentCount: 3,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      baseFormat: 'etc2-rgb8unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'etc2-rgb8unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 3,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      baseFormat: 'etc2-rgb8unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'etc2-rgb8a1unorm': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      baseFormat: 'etc2-rgb8a1unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'etc2-rgb8a1unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      baseFormat: 'etc2-rgb8a1unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'etc2-rgba8unorm': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'etc2-rgba8unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'etc2-rgba8unorm-srgb': {
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'etc2-rgba8unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'eac-r11unorm': {
      color: {
        type: 'float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      get bytesPerBlock() {return this.color.bytes;}
    },
    'eac-r11snorm': {
      color: {
        type: 'float',
        componentCount: 1,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 8
      },
      get bytesPerBlock() {return this.color.bytes;}
    },

    'eac-rg11unorm': {
      color: {
        type: 'float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      get bytesPerBlock() {return this.color.bytes;}
    },
    'eac-rg11snorm': {
      color: {
        type: 'float',
        componentCount: 2,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      get bytesPerBlock() {return this.color.bytes;}
    }
  }
});

const kASTCTextureFormatInfo = formatTableWithDefaults({
  defaults: {
    multisample: false,
    feature: 'texture-compression-astc'
  },
  table: {
    'astc-4x4-unorm': {
      blockWidth: 4,
      blockHeight: 4,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-4x4-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-4x4-unorm-srgb': {
      blockWidth: 4,
      blockHeight: 4,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-4x4-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-5x4-unorm': {
      blockWidth: 5,
      blockHeight: 4,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-5x4-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-5x4-unorm-srgb': {
      blockWidth: 5,
      blockHeight: 4,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-5x4-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-5x5-unorm': {
      blockWidth: 5,
      blockHeight: 5,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-5x5-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-5x5-unorm-srgb': {
      blockWidth: 5,
      blockHeight: 5,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-5x5-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-6x5-unorm': {
      blockWidth: 6,
      blockHeight: 5,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-6x5-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-6x5-unorm-srgb': {
      blockWidth: 6,
      blockHeight: 5,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-6x5-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-6x6-unorm': {
      blockWidth: 6,
      blockHeight: 6,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-6x6-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-6x6-unorm-srgb': {
      blockWidth: 6,
      blockHeight: 6,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-6x6-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-8x5-unorm': {
      blockWidth: 8,
      blockHeight: 5,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-8x5-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-8x5-unorm-srgb': {
      blockWidth: 8,
      blockHeight: 5,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-8x5-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-8x6-unorm': {
      blockWidth: 8,
      blockHeight: 6,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-8x6-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-8x6-unorm-srgb': {
      blockWidth: 8,
      blockHeight: 6,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-8x6-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-8x8-unorm': {
      blockWidth: 8,
      blockHeight: 8,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-8x8-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-8x8-unorm-srgb': {
      blockWidth: 8,
      blockHeight: 8,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-8x8-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-10x5-unorm': {
      blockWidth: 10,
      blockHeight: 5,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-10x5-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-10x5-unorm-srgb': {
      blockWidth: 10,
      blockHeight: 5,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-10x5-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-10x6-unorm': {
      blockWidth: 10,
      blockHeight: 6,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-10x6-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-10x6-unorm-srgb': {
      blockWidth: 10,
      blockHeight: 6,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-10x6-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-10x8-unorm': {
      blockWidth: 10,
      blockHeight: 8,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-10x8-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-10x8-unorm-srgb': {
      blockWidth: 10,
      blockHeight: 8,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-10x8-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-10x10-unorm': {
      blockWidth: 10,
      blockHeight: 10,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-10x10-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-10x10-unorm-srgb': {
      blockWidth: 10,
      blockHeight: 10,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-10x10-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-12x10-unorm': {
      blockWidth: 12,
      blockHeight: 10,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-12x10-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-12x10-unorm-srgb': {
      blockWidth: 12,
      blockHeight: 10,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-12x10-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },

    'astc-12x12-unorm': {
      blockWidth: 12,
      blockHeight: 12,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-12x12-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    },
    'astc-12x12-unorm-srgb': {
      blockWidth: 12,
      blockHeight: 12,
      color: {
        type: 'float',
        componentCount: 4,
        copySrc: true,
        copyDst: true,
        storage: false,
        readWriteStorage: false,
        bytes: 16
      },
      baseFormat: 'astc-12x12-unorm',
      get bytesPerBlock() {return this.color.bytes;}
    }
  }
});

// Definitions for use locally.

// MAINTENANCE_TODO: Consider generating the exports below programmatically by filtering the big list, instead
// of using these local constants? Requires some type magic though.
const kCompressedTextureFormatInfo = { ...kBCTextureFormatInfo, ...kETC2TextureFormatInfo, ...kASTCTextureFormatInfo };
const kColorTextureFormatInfo = { ...kRegularTextureFormatInfo, ...kCompressedTextureFormatInfo };
const kEncodableTextureFormatInfo = { ...kRegularTextureFormatInfo, ...kSizedDepthStencilFormatInfo };
const kSizedTextureFormatInfo = { ...kRegularTextureFormatInfo, ...kSizedDepthStencilFormatInfo, ...kCompressedTextureFormatInfo };
const kDepthStencilFormatInfo = { ...kSizedDepthStencilFormatInfo, ...kUnsizedDepthStencilFormatInfo };
const kUncompressedTextureFormatInfo = { ...kRegularTextureFormatInfo, ...kSizedDepthStencilFormatInfo, ...kUnsizedDepthStencilFormatInfo };
const kAllTextureFormatInfo = { ...kUncompressedTextureFormatInfo, ...kCompressedTextureFormatInfo };

/** A "regular" texture format (uncompressed, sized, single-plane color formats). */

/** A sized depth/stencil texture format. */

/** An unsized depth/stencil texture format. */

/** A compressed (block) texture format. */


/** A color texture format (regular | compressed). */

/** An encodable texture format (regular | sized depth/stencil). */

/** A sized texture format (regular | sized depth/stencil | compressed). */

/** A depth/stencil format (sized | unsized). */

/** An uncompressed (block size 1x1) format (regular | depth/stencil). */


const kRegularTextureFormats = keysOf(kRegularTextureFormatInfo);
const kSizedDepthStencilFormats = keysOf(kSizedDepthStencilFormatInfo);
const kUnsizedDepthStencilFormats = keysOf(kUnsizedDepthStencilFormatInfo);
const kCompressedTextureFormats = keysOf(kCompressedTextureFormatInfo);
const kBCCompressedTextureFormats = keysOf(kBCTextureFormatInfo);
const kASTCCompressedTextureFormats = keysOf(kASTCTextureFormatInfo);

const kColorTextureFormats = keysOf(kColorTextureFormatInfo);
const kEncodableTextureFormats = keysOf(kEncodableTextureFormatInfo);
const kSizedTextureFormats = keysOf(kSizedTextureFormatInfo);
const kDepthStencilFormats = keysOf(kDepthStencilFormatInfo);
const kUncompressedTextureFormats = keysOf(kUncompressedTextureFormatInfo);
const kAllTextureFormats = keysOf(kAllTextureFormatInfo);

/** Per-GPUTextureFormat-per-aspect info. */












/** Per GPUTextureFormat-per-aspect info for color aspects. */







/** Per GPUTextureFormat-per-aspect info for depth aspects. */




/** Per GPUTextureFormat-per-aspect info for stencil aspects. */





/**
 * Per-GPUTextureFormat info.
 * This is not actually the type of values in kTextureFormatInfo; that type is fully const
 * so that it can be narrowed very precisely at usage sites by the compiler.
 * This type exists only as a type check on the inferred type of kTextureFormatInfo.
 */













































/**
 * DO NOT EXPORT THIS - functions that need info from this table should use the appropriate
 * method for their needs.
 *
 * For a list of textures formats for test parameters there are:
 *
 * Lists of formats that might require features to be enabled
 * * kPossibleColorRenderableTextureFormats
 * * kPossibleStorageTextureFormats
 * * kPossibleReadWriteStorageTextureFormats
 * * kPossibleMultisampledTextureFormats
 *
 * Lists of formats that end in -srgb
 * * kDifferentBaseFormatTextureFormats  (includes compressed textures)
 * * kDifferentBaseFormatRegularTextureFormats (does not include compressed textures)
 *
 * Formats that require a feature to use at all (mostly compressed formats)
 * * kOptionalTextureFormats
 *
 * Misc
 * * kRegularTextureFormats
 * * kSizedDepthStencilFormats
 * * kUnsizedDepthStencilFormats
 * * kCompressedTextureFormats
 * * kUncompressedTextureFormats
 * * kColorTextureFormats - color formats including compressed and sint/uint
 * * kEncodableTextureFormats - formats that TexelView supports.
 * * kSizedTextureFormats - formats that have a known size (so not depth24plus ...)
 * * kDepthStencilFormats - depth, stencil, depth-stencil
 * * kDepthTextureFormats - depth and depth-stencil
 * * kStencilTextureFormats - stencil and depth-stencil
 * * kAllTextureFormats
 *
 * If one of the list above does not work, add a new one or to filter in beforeAllSubcases you generally want to use
 * You will not know if you can actually use a texture for the given use case until the test runs and has a device.
 *
 * * isTextureFormatPossiblyUsableAsRenderAttachment
 * * isTextureFormatPossiblyUsableAsColorRenderAttachment
 * * isTextureFormatPossiblyMultisampled
 * * isTextureFormatPossiblyStorageReadable
 * * isTextureFormatPossiblyStorageReadWritable
 * * isTextureFormatPossiblyFilterableAsTextureF32
 * * isTextureFormatPossiblyUsableWithCopyExternalImageToTexture
 *
 * These are also usable before or during a test
 *
 * * isColorTextureFormat
 * * isColorTextureFormatWithAlpha
 * * isDepthTextureFormat
 * * isStencilTextureFormat
 * * isDepthOrStencilTextureFormat
 * * isEncodableTextureFormat
 * * isRegularTextureFormat
 * * isCompressedFloatTextureFormat
 * * isSintOrUintFormat
 *
 * To skip a test use the `skipIfXXX` tests in `GPUTest` if possible. Otherwise these functions
 * require a device to give a correct answer.
 *
 * * isTextureFormatUsableAsRenderAttachment
 * * isTextureFormatColorRenderable
 * * isTextureFormatResolvable
 * * isTextureFormatBlendable
 * * isTextureFormatMultisampled
 * * isTextureFormatUsableAsStorageTexture
 * * isTextureFormatUsableAsReadWriteStorageTexture
 * * isTextureFormatUsableAsStorageFormatInCreateShaderModule
 * * isTextureFormatUsableWithCopyExternalImageToTexture
 *
 * Per-GPUTextureFormat info.
 */
const kTextureFormatInfo = {
  ...kRegularTextureFormatInfo,
  ...kSizedDepthStencilFormatInfo,
  ...kUnsizedDepthStencilFormatInfo,
  ...kBCTextureFormatInfo,
  ...kETC2TextureFormatInfo,
  ...kASTCTextureFormatInfo
};

/** Defining this variable verifies the type of kTextureFormatInfo2. It is not used. */

const kTextureFormatInfo_TypeCheck =

kTextureFormatInfo;

// Depth texture formats including formats that also support stencil
const kDepthTextureFormats = [
...kDepthStencilFormats.filter((v) => kTextureFormatInfo[v].depth)];

// Stencil texture formats including formats that also support depth
const kStencilTextureFormats = kDepthStencilFormats.filter(
  (v) => kTextureFormatInfo[v].stencil
);

const kTextureFormatTier1AllowsResolve = [
'r8snorm',
'rg8snorm',
'rgba8snorm',
'rg11b10ufloat'];


const kTextureFormatTier1ThrowsWhenNotEnabled = [
'r16unorm',
'r16snorm',
'rg16unorm',
'rg16snorm',
'rgba16unorm',
'rgba16snorm'];


const kTextureFormatTier1AllowsRenderAttachmentBlendableMultisample =
[
'r16unorm',
'r16snorm',
'rg16unorm',
'rg16snorm',
'rgba16unorm',
'rgba16snorm',
'r8snorm',
'rg8snorm',
'rgba8snorm',
'rg11b10ufloat'];


const kTextureFormatsTier1EnablesStorageReadOnlyWriteOnly = [
'r8unorm',
'r8snorm',
'r8uint',
'r8sint',
'rg8unorm',
'rg8snorm',
'rg8uint',
'rg8sint',
'r16uint',
'r16sint',
'r16float',
'rg16uint',
'rg16sint',
'rg16float',
'rgb10a2uint',
'rgb10a2unorm',
'rg11b10ufloat'];


const kTextureFormatsTier2EnablesStorageReadWrite = [
'r8unorm',
'r8uint',
'r8sint',
'rgba8unorm',
'rgba8uint',
'rgba8sint',
'r16uint',
'r16sint',
'r16float',
'rgba16uint',
'rgba16sint',
'rgba16float',
'rgba32uint',
'rgba32sint',
'rgba32float'];


// Texture formats that may possibly be used as a storage texture.
// Some may require certain features to be enabled.
const kPossibleStorageTextureFormats = [
...kRegularTextureFormats.filter((f) => kTextureFormatInfo[f].color?.storage),
'bgra8unorm',
// these can be used as storage when texture-formats-tier1 is enabled
...kTextureFormatsTier1EnablesStorageReadOnlyWriteOnly];


// Texture formats that may possibly be used as a storage texture.
// Some may require certain features to be enabled.
const kPossibleReadWriteStorageTextureFormats = [
...kPossibleStorageTextureFormats.filter((f) => kTextureFormatInfo[f].color?.readWriteStorage),
// these can be used as storage when texture-formats-tier2 is enabled
...kTextureFormatsTier2EnablesStorageReadWrite];


// Texture formats that may possibly be multisampled.
// Some may require certain features to be enabled.
const kPossibleMultisampledTextureFormats = [
...kRegularTextureFormats.filter((f) => kTextureFormatInfo[f].multisample),
...kDepthStencilFormats.filter((f) => kTextureFormatInfo[f].multisample)];


// Texture formats that may possibly be color renderable.
// Some may require certain features to be enabled.
const kPossibleColorRenderableTextureFormats = [
...kRegularTextureFormats.filter((f) => kTextureFormatInfo[f].colorRender)];




// Texture formats that have a different base format. This is effectively all -srgb formats
// including compressed formats.
const kDifferentBaseFormatTextureFormats = kColorTextureFormats.filter(
  (f) => kTextureFormatInfo[f].baseFormat && kTextureFormatInfo[f].baseFormat !== f
);

// "Regular" texture formats that have a different base format. This is effectively all -srgb formats
// except compressed formats.
const kDifferentBaseFormatRegularTextureFormats = kRegularTextureFormats.filter(
  (f) => kTextureFormatInfo[f].baseFormat && kTextureFormatInfo[f].baseFormat !== f
);

// Textures formats that are optional
const kOptionalTextureFormats = kAllTextureFormats.filter(
  (t) => kTextureFormatInfo[t].feature !== undefined
);

function isSnormTextureFormat(format) {
  return format.endsWith('snorm');
}

/**
 * Returns true if a texture can be possibly used with copyExternalImageToTexture.
 * The texture may require certain features to be enabled.
 */
function isTextureFormatPossiblyUsableWithCopyExternalImageToTexture(
format)
{
  return (
    isColorTextureFormat(format) &&
    !isSintOrUintFormat(format) &&
    !isCompressedTextureFormat(format) &&
    !isSnormTextureFormat(format) &&
    isTextureFormatPossiblyUsableAsColorRenderAttachment(format));

}

/**
 * Returns true if a texture can be used with copyExternalImageToTexture.
 */
function isTextureFormatUsableWithCopyExternalImageToTexture(
features,
format)
{
  return (
    isColorTextureFormat(format) &&
    !isSintOrUintFormat(format) &&
    !isCompressedTextureFormat(format) &&
    !isSnormTextureFormat(format) &&
    isTextureFormatColorRenderable(features, format));

}

//
// Other related stuff
//

const kDepthStencilFormatCapabilityInBufferTextureCopy = {
  // kUnsizedDepthStencilFormats
  depth24plus: {
    CopyB2T: [],
    CopyT2B: [],
    texelAspectSize: { 'depth-only': -1, 'stencil-only': -1 }
  },
  'depth24plus-stencil8': {
    CopyB2T: ['stencil-only'],
    CopyT2B: ['stencil-only'],
    texelAspectSize: { 'depth-only': -1, 'stencil-only': 1 }
  },

  // kSizedDepthStencilFormats
  depth16unorm: {
    CopyB2T: ['all', 'depth-only'],
    CopyT2B: ['all', 'depth-only'],
    texelAspectSize: { 'depth-only': 2, 'stencil-only': -1 }
  },
  depth32float: {
    CopyB2T: [],
    CopyT2B: ['all', 'depth-only'],
    texelAspectSize: { 'depth-only': 4, 'stencil-only': -1 }
  },
  'depth32float-stencil8': {
    CopyB2T: ['stencil-only'],
    CopyT2B: ['depth-only', 'stencil-only'],
    texelAspectSize: { 'depth-only': 4, 'stencil-only': 1 }
  },
  stencil8: {
    CopyB2T: ['all', 'stencil-only'],
    CopyT2B: ['all', 'stencil-only'],
    texelAspectSize: { 'depth-only': -1, 'stencil-only': 1 }
  }
};

/** `kDepthStencilFormatResolvedAspect[format][aspect]` returns the aspect-specific format for a
 *  depth-stencil format, or `undefined` if the format doesn't have the aspect.
 */
const kDepthStencilFormatResolvedAspect =



{
  // kUnsizedDepthStencilFormats
  depth24plus: {
    all: 'depth24plus',
    'depth-only': 'depth24plus',
    'stencil-only': undefined
  },
  'depth24plus-stencil8': {
    all: 'depth24plus-stencil8',
    'depth-only': 'depth24plus',
    'stencil-only': 'stencil8'
  },

  // kSizedDepthStencilFormats
  depth16unorm: {
    all: 'depth16unorm',
    'depth-only': 'depth16unorm',
    'stencil-only': undefined
  },
  depth32float: {
    all: 'depth32float',
    'depth-only': 'depth32float',
    'stencil-only': undefined
  },
  'depth32float-stencil8': {
    all: 'depth32float-stencil8',
    'depth-only': 'depth32float',
    'stencil-only': 'stencil8'
  },
  stencil8: {
    all: 'stencil8',
    'depth-only': undefined,
    'stencil-only': 'stencil8'
  }
};

/**
 * @returns the GPUTextureFormat corresponding to the @param aspect of @param format.
 * This allows choosing the correct format for depth-stencil aspects when creating pipelines that
 * will have to match the resolved format of views, or to get per-aspect information like the
 * `blockByteSize`.
 *
 * Many helpers use an `undefined` `aspect` to means `'all'` so this is also the default for this
 * function.
 */
function resolvePerAspectFormat(
format,
aspect)
{
  if (aspect === 'all' || aspect === undefined) {
    return format;
  }
  assert(!!kTextureFormatInfo[format].depth || !!kTextureFormatInfo[format].stencil);
  const resolved = kDepthStencilFormatResolvedAspect[format][aspect ?? 'all'];
  assert(resolved !== undefined);
  return resolved;
}

/**
 * @returns the sample type of the specified aspect of the specified format.
 */
function sampleTypeForFormatAndAspect(
format,
aspect)
{
  const info = kTextureFormatInfo[format];
  if (info.color) {
    assert(aspect === 'all', `color format ${format} used with aspect ${aspect}`);
    return info.color.type;
  } else if (info.depth && info.stencil) {
    if (aspect === 'depth-only') {
      return info.depth.type;
    } else if (aspect === 'stencil-only') {
      return info.stencil.type;
    } else {
      unreachable(`depth-stencil format ${format} used with aspect ${aspect}`);
    }
  } else if (info.depth) {
    assert(aspect !== 'stencil-only', `depth-only format ${format} used with aspect ${aspect}`);
    return info.depth.type;
  } else if (info.stencil) {
    assert(aspect !== 'depth-only', `stencil-only format ${format} used with aspect ${aspect}`);
    return info.stencil.type;
  }
  unreachable();
}

/**
 * Gets all copyable aspects for copies between texture and buffer for specified depth/stencil format and copy type, by spec.
 */
function depthStencilFormatCopyableAspects(
type,
format)
{
  const appliedType = type === 'WriteTexture' ? 'CopyB2T' : type;
  return kDepthStencilFormatCapabilityInBufferTextureCopy[format][appliedType];
}

/**
 * Computes whether a copy between a depth/stencil texture aspect and a buffer is supported, by spec.
 */
function depthStencilBufferTextureCopySupported(
type,
format,
aspect)
{
  const supportedAspects = depthStencilFormatCopyableAspects(
    type,
    format
  );
  return supportedAspects.includes(aspect);
}

/**
 * Returns the byte size of the depth or stencil aspect of the specified depth/stencil format,
 * or -1 if none.
 */
function depthStencilFormatAspectSize(
format,
aspect)
{
  const texelAspectSize =
  kDepthStencilFormatCapabilityInBufferTextureCopy[format].texelAspectSize[aspect];
  assert(texelAspectSize > 0);
  return texelAspectSize;
}

/**
 * Returns true iff a texture can be created with the provided GPUTextureDimension
 * (defaulting to 2d) and GPUTextureFormat, by spec.
 */
function textureFormatAndDimensionPossiblyCompatible(
dimension,
format)
{
  if (dimension === '3d' && (isBCTextureFormat(format) || isASTCTextureFormat(format))) {
    return true;
  }
  const info = kAllTextureFormatInfo[format];
  return !(
  (dimension === '1d' || dimension === '3d') && (
  info.blockWidth > 1 || info.depth || info.stencil));

}

/**
 * Returns true iff a texture can be created with the provided GPUTextureDimension
 * (defaulting to 2d) and GPUTextureFormat for a GPU device, by spec.
 */
function textureDimensionAndFormatCompatibleForDevice(
features,
dimension,
format)
{
  if (
  dimension === '3d' && (
  isBCTextureFormat(format) && hasFeature(features, 'texture-compression-bc-sliced-3d') ||
  isASTCTextureFormat(format) && hasFeature(features, 'texture-compression-astc-sliced-3d')))
  {
    return true;
  }
  const info = kAllTextureFormatInfo[format];
  return !(
  (dimension === '1d' || dimension === '3d') && (
  info.blockWidth > 1 || info.depth || info.stencil));

}

/**
 * Returns true iff a texture can be used with the provided GPUTextureViewDimension
 */
function textureViewDimensionAndFormatCompatibleForDevice(
features,
dimension,
format)
{
  return textureDimensionAndFormatCompatibleForDevice(
    features,
    getTextureDimensionFromView(dimension),
    format
  );
}

/**
 * Check if two formats are view format compatible.
 */
function textureFormatsAreViewCompatible(
features,
a,
b)
{
  return isCompatibilityMode(features) ?
  a === b :
  a === b || a + '-srgb' === b || b + '-srgb' === a;
}

/**
 * Gets the block width, height, and bytes per block for a color texture format.
 * This is for color textures only. For all texture formats @see {@link getBlockInfoForTextureFormat}
 * The point of this function is bytesPerBlock is always defined so no need to check that it's not
 * vs getBlockInfoForTextureFormat where it may not be defined.
 */
function getBlockInfoForColorTextureFormat(format) {
  const info = kTextureFormatInfo[format];
  return {
    blockWidth: info.blockWidth,
    blockHeight: info.blockHeight,
    bytesPerBlock: info.color?.bytes
  };
}

/**
 * Gets the block width, height, and bytes per block for a sized texture format.
 * This is for sized textures only. For all texture formats @see {@link getBlockInfoForTextureFormat}
 * The point of this function is bytesPerBlock is always defined so no need to check that it's not
 * vs getBlockInfoForTextureFormat where it may not be defined.
 */
function getBlockInfoForSizedTextureFormat(format) {
  const info = kTextureFormatInfo[format];
  const bytesPerBlock = info.color?.bytes || info.depth?.bytes || info.stencil?.bytes;
  assert(!!bytesPerBlock);
  return {
    blockWidth: info.blockWidth,
    blockHeight: info.blockHeight,
    bytesPerBlock
  };
}

/**
 * Gets the block width, height, and bytes per block for an encodable texture format.
 * This is for encodable textures only. For all texture formats @see {@link getBlockInfoForTextureFormat}
 * The point of this function is bytesPerBlock is always defined so no need to check that it's not
 * vs getBlockInfoForTextureFormat where it may not be defined.
 */
function getBlockInfoForEncodableTextureFormat(format) {
  const info = kTextureFormatInfo[format];
  const bytesPerBlock = info.color?.bytes || info.depth?.bytes || info.stencil?.bytes;
  assert(!!bytesPerBlock);
  return {
    blockWidth: info.blockWidth,
    blockHeight: info.blockHeight,
    bytesPerBlock
  };
}

/**
 * Gets the block width, height, and bytes per block for a color texture format.
 * Note that bytesPerBlock will be undefined if format's size is undefined.
 * If you are only using color or encodable formats, @see {@link getBlockInfoForColorTextureFormat}
 * or {@link getBlockInfoForEncodableTextureFormat}
 */
function getBlockInfoForTextureFormat(format) {
  const info = kTextureFormatInfo[format];
  return {
    blockWidth: info.blockWidth,
    blockHeight: info.blockHeight,
    bytesPerBlock: info.color?.bytes ?? info.depth?.bytes ?? info.stencil?.bytes
  };
}

/**
 * Returns the "byteCost" of rendering to a color texture format.
 */
function getColorRenderByteCost(format) {
  const byteCost = kTextureFormatInfo[format].colorRender?.byteCost;
  // MAINTENANCE_TODO: remove this assert. The issue is typescript thinks
  // PossibleColorRenderTextureFormat contains all texture formats and not just
  // a filtered list.
  assert(byteCost !== undefined);
  return byteCost;
}

/**
 * Returns the "alignment" of rendering to a color texture format.
 */
function getColorRenderAlignment(format) {
  const alignment = kTextureFormatInfo[format].colorRender?.alignment;
  // MAINTENANCE_TODO: remove this assert. The issue is typescript thinks
  // PossibleColorRenderTextureFormat contains all texture formats and not just
  // a filtered list.
  assert(alignment !== undefined);
  return alignment;
}

/**
 * Gets the baseFormat for a texture format.
 */
function getBaseFormatForTextureFormat(
format)
{
  return kTextureFormatInfo[format].baseFormat;
}

function getBaseFormatForRegularTextureFormat(
format)
{
  return kTextureFormatInfo[format].baseFormat;
}

/**
 * Gets the feature needed for a give texture format or undefined if none.
 */
function getRequiredFeatureForTextureFormat(format) {
  return kTextureFormatInfo[format].feature;
}

function getFeaturesForFormats(
formats)
{
  return Array.from(new Set(formats.map((f) => f ? kTextureFormatInfo[f].feature : undefined)));
}

function filterFormatsByFeature(
feature,
formats)
{
  return formats.filter((f) => f === undefined || kTextureFormatInfo[f].feature === feature);
}

function isTextureFormatTier1EnablesRenderAttachmentBlendableMultisample(format) {
  return kTextureFormatTier1AllowsRenderAttachmentBlendableMultisample.includes(
    format
  );
}

function isTextureFormatTier1EnablesResolve(format) {
  return kTextureFormatTier1AllowsResolve.includes(format);
}

function isTextureFormatTier1EnablesStorageReadOnlyWriteOnly(format) {
  return kTextureFormatsTier1EnablesStorageReadOnlyWriteOnly.includes(format);
}

function isTextureFormatTier2EnablesStorageReadWrite(format) {
  return kTextureFormatsTier2EnablesStorageReadWrite.includes(format);
}

function canCopyToAspectOfTextureFormat(format, aspect) {
  const info = kTextureFormatInfo[format];
  switch (aspect) {
    case 'depth-only':
      assert(isDepthTextureFormat(format));
      return info.depth && info.depth.copyDst;
    case 'stencil-only':
      assert(isStencilTextureFormat(format));
      return info.stencil && info.stencil.copyDst;
    case 'all':
      return (
        (!isDepthTextureFormat(format) || info.depth?.copyDst) && (
        !isStencilTextureFormat(format) || info.stencil?.copyDst) && (
        !isColorTextureFormat(format) || !info.color?.copyDst));

  }
}

function canCopyFromAspectOfTextureFormat(
format,
aspect)
{
  const info = kTextureFormatInfo[format];
  switch (aspect) {
    case 'depth-only':
      assert(isDepthTextureFormat(format));
      return info.depth && info.depth.copySrc;
    case 'stencil-only':
      assert(isStencilTextureFormat(format));
      return info.stencil && info.stencil.copySrc;
    case 'all':
      return (
        (!isDepthTextureFormat(format) || info.depth?.copySrc) && (
        !isStencilTextureFormat(format) || info.stencil?.copySrc) && (
        !isColorTextureFormat(format) || !info.color?.copySrc));

  }
}

/**
 * Returns true if all aspects of texture can be copied to (used with COPY_DST)
 */
function canCopyToAllAspectsOfTextureFormat(format) {
  const info = kTextureFormatInfo[format];
  return (
    (!info.color || info.color.copyDst) && (
    !info.depth || info.depth.copyDst) && (
    !info.stencil || info.stencil.copyDst));

}

/**
 * Returns true if all aspects of texture can be copied from (used with COPY_SRC)
 */
function canCopyFromAllAspectsOfTextureFormat(format) {
  const info = kTextureFormatInfo[format];
  return (
    (!info.color || info.color.copySrc) && (
    !info.depth || info.depth.copySrc) && (
    !info.stencil || info.stencil.copySrc));

}

function isCompressedTextureFormat(format) {
  return format in kCompressedTextureFormatInfo;
}

function isBCTextureFormat(format) {
  return format in kBCTextureFormatInfo;
}

function isASTCTextureFormat(format) {
  return format in kASTCTextureFormatInfo;
}

function isColorTextureFormat(format) {
  return !!kTextureFormatInfo[format].color;
}

function isColorTextureFormatWithAlpha(format) {
  return isColorTextureFormat(format) && kTextureFormatInfo[format].color?.componentCount === 4;
}

function isDepthTextureFormat(format) {
  return !!kTextureFormatInfo[format].depth;
}

function isStencilTextureFormat(format) {
  return !!kTextureFormatInfo[format].stencil;
}

function isDepthStencilTextureFormat(format) {
  return isDepthTextureFormat(format) && isStencilTextureFormat(format);
}

function isDepthOrStencilTextureFormat(format) {
  return isDepthTextureFormat(format) || isStencilTextureFormat(format);
}

function isEncodableTextureFormat(format) {
  return kEncodableTextureFormats.includes(format);
}

/**
 * Returns if a texture can be used as a render attachment. some color formats and all
 * depth textures and stencil textures are usable with usage RENDER_ATTACHMENT.
 */
function isTextureFormatUsableAsRenderAttachment(
features,
format)
{
  if (format === 'rg11b10ufloat') {
    return hasFeature(features, 'rg11b10ufloat-renderable');
  }
  if (isTextureFormatTier1EnablesRenderAttachmentBlendableMultisample(format)) {
    return hasFeature(features, 'texture-formats-tier1');
  }
  return !!(kTextureFormatInfo[format].colorRender ?? isDepthOrStencilTextureFormat(format));
}

/**
 * Returns if a texture can be used as a "colorAttachment".
 */
function isTextureFormatColorRenderable(
features,
format)
{
  if (format === 'rg11b10ufloat') {
    return hasFeature(features, 'rg11b10ufloat-renderable');
  }
  if (isTextureFormatTier1EnablesRenderAttachmentBlendableMultisample(format)) {
    return hasFeature(features, 'texture-formats-tier1');
  }
  return !!kAllTextureFormatInfo[format].colorRender;
}

/**
 * Returns if a texture can be blended.
 */
function isTextureFormatBlendable(
features,
format)
{
  if (!isTextureFormatColorRenderable(features, format)) {
    return false;
  }
  if (format === 'rg11b10ufloat') {
    return hasFeature(features, 'rg11b10ufloat-renderable');
  }
  if (is32Float(format)) {
    return hasFeature(features, 'float32-blendable');
  }
  if (isTextureFormatTier1EnablesRenderAttachmentBlendableMultisample(format)) {
    return hasFeature(features, 'texture-formats-tier1');
  }
  return !!kAllTextureFormatInfo[format].colorRender?.blend;
}

/**
 * Returns true if a texture can be filtered.
 */
function isTextureFormatFilterable(
features,
format)
{
  const type = getTextureFormatType(format);
  switch (type) {
    case 'float':
      return true;
    case 'unfilterable-float':
      if (is32Float(format)) {
        return hasFeature(features, 'float32-filterable');
      } else {
        return false;
      }
    default:
      return false;
  }
}

/**
 * Returns the texture's type (float, unsigned-float, sint, uint, depth)
 */
function getTextureFormatType(format, aspect = 'all') {
  const info = kTextureFormatInfo[format];
  let type;
  switch (aspect) {
    case 'all':
      type = info.color?.type ?? info.depth?.type ?? info.stencil?.type;
      break;
    case 'depth-only':
      type = info.depth?.type;
      break;
    case 'stencil-only':
      type = info.stencil?.type;
      break;
  }
  assert(!!type);
  return type;
}

/**
 * Returns the regular texture's type (float, unsigned-float, sint, uint)
 */
function getTextureFormatColorType(format) {
  const info = kTextureFormatInfo[format];
  const type = info.color?.type;
  assert(!!type);
  return type;
}

/**
 * Returns true if a texture can possibly be used as a render attachment.
 * The texture may require certain features to be enabled.
 */
function isTextureFormatPossiblyUsableAsRenderAttachment(format) {
  const info = kTextureFormatInfo[format];
  return (
    isDepthOrStencilTextureFormat(format) ||
    !!info.colorRender ||
    isTextureFormatTier1EnablesRenderAttachmentBlendableMultisample(format));

}

/**
 * Returns true if a texture can possibly be used as a color render attachment.
 * The texture may require certain features to be enabled.
 */
function isTextureFormatPossiblyUsableAsColorRenderAttachment(format) {
  const info = kTextureFormatInfo[format];
  return (
    !!info.colorRender || isTextureFormatTier1EnablesRenderAttachmentBlendableMultisample(format));

}

/**
 * Returns true if a texture can possibly be used multisampled.
 * The texture may require certain features to be enabled.
 */
function isTextureFormatPossiblyMultisampled(format) {
  const info = kTextureFormatInfo[format];
  return (
    info.multisample || isTextureFormatTier1EnablesRenderAttachmentBlendableMultisample(format));

}

/**
 * Returns true if a texture can possibly be resolved.
 * The texture may require certain features to be enabled.
 */
function isTextureFormatPossiblyResolvable(format) {
  if (format === 'rg11b10ufloat') {
    return true;
  }
  if (isTextureFormatTier1EnablesResolve(format)) {
    return true;
  }
  const info = kTextureFormatInfo[format];
  return !!info.colorRender?.resolve;
}

/**
 * Returns true if a texture can possibly be used as a storage texture.
 * The texture may require certain features to be enabled.
 */
function isTextureFormatPossiblyStorageReadable(format) {
  return (
    !!kTextureFormatInfo[format].color?.storage ||
    isTextureFormatTier1EnablesStorageReadOnlyWriteOnly(format));

}

/**
 * Returns true if a texture can possibly be used as a read-write storage texture.
 * The texture may require certain features to be enabled.
 */
function isTextureFormatPossiblyStorageReadWritable(format) {
  return (
    !!kTextureFormatInfo[format].color?.readWriteStorage ||
    isTextureFormatTier2EnablesStorageReadWrite(format));

}

function is16Float(format) {
  return format === 'r16float' || format === 'rg16float' || format === 'rgba16float';
}

function is32Float(format) {
  return format === 'r32float' || format === 'rg32float' || format === 'rgba32float';
}

/**
 * Returns true if texture is filterable as `texture_xxx<f32>`
 *
 * examples:
 * * 'rgba8unorm' -> true
 * * 'depth16unorm' -> false
 * * 'rgba32float' -> true (you need to enable feature 'float32-filterable')
 */
function isTextureFormatPossiblyFilterableAsTextureF32(format) {
  const info = kTextureFormatInfo[format];
  return info.color?.type === 'float' || is32Float(format);
}

const kCompatModeUnsupportedStorageTextureFormats = [
'rg32float',
'rg32sint',
'rg32uint'];


/**
 * Return true if the format can be used as a write only storage texture.
 * Note: Some formats can be compiled in a shader but can not be used
 * in a pipeline or elsewhere. This function returns whether or not the format
 * can be used in general. If you want to know if the format can used when compiling
 * a shader @see {@link isTextureFormatUsableAsStorageFormatInCreateShaderModule}
 */
function isTextureFormatUsableAsWriteOnlyStorageTexture(
features,
format)
{
  if (isCompatibilityMode(features)) {
    if (kCompatModeUnsupportedStorageTextureFormats.indexOf(format) >= 0) {
      return false;
    }
  }
  if (format === 'bgra8unorm' && hasFeature(features, 'bgra8unorm-storage')) {
    return true;
  }
  if (
  isTextureFormatTier1EnablesStorageReadOnlyWriteOnly(format) &&
  hasFeature(features, 'texture-formats-tier1'))
  {
    return true;
  }
  const info = kTextureFormatInfo[format];
  return !!(info.color?.storage || info.depth?.storage || info.stencil?.storage);
}

/**
 * Return true if the format can be used with the given access mode
 * access can be either GPUStorageTextureAccess or WGSL access
 * Note: Some formats can be compiled in a shader but can not be used
 * in a pipeline or elsewhere. This function returns whether or not the format
 * can be used in general. If you want to know if the format can used when compiling
 * a shader @see {@link isTextureFormatUsableAsStorageFormatInCreateShaderModule}
 */
function isTextureFormatUsableWithStorageAccessMode(
features,
format,
access)
{
  switch (access) {
    case 'read':
    case 'read-only':
      return isTextureFormatUsableAsReadOnlyStorageTexture(features, format);
    case 'write':
    case 'write-only':
      return isTextureFormatUsableAsWriteOnlyStorageTexture(features, format);
    case 'read_write':
    case 'read-write':
      return isTextureFormatUsableAsReadWriteStorageTexture(features, format);
  }
}

/**
 * Return true if the format can be used as a read only storage texture.
 * Note: Some formats can be compiled in a shader but can not be used
 * in a pipeline or elsewhere. This function returns whether or not the format
 * can be used in general. If you want to know if the format can used when compiling
 * a shader @see {@link isTextureFormatUsableAsStorageFormatInCreateShaderModule}
 */
function isTextureFormatUsableAsReadOnlyStorageTexture(
features,
format)
{
  // This is the only storage texture format that isn't readable as a storage format.
  if (format === 'bgra8unorm') {
    return false;
  }
  // All other formats that can be used as a storage texture can be used as
  // both read-only and write-only.
  return isTextureFormatUsableAsWriteOnlyStorageTexture(features, format);
}

/**
 * Returns true if format can be used with createShaderModule on the device.
 * Some formats may require a feature to be enabled before they can be used
 * as a storage texture. Others, can't be used in a pipeline but can be compiled
 * in a shader. Examples are rg32float, rg32uint, rg32sint which are not usable
 * in compat mode but shaders can be compiled. Similarly, bgra8unorm can be
 * compiled but can't be used in a pipeline unless feature 'bgra8unorm-storage'
 * is available.
 */
function isTextureFormatUsableAsStorageFormatInCreateShaderModule(
features,
format)
{
  return kPossibleStorageTextureFormats.includes(
    format
  );
}

function isTextureFormatUsableAsReadWriteStorageTexture(
features,
format)
{
  if (isTextureFormatTier2EnablesStorageReadWrite(format)) {
    return hasFeature(features, 'texture-formats-tier2');
  }
  return !!kTextureFormatInfo[format].color?.readWriteStorage;
}

function isRegularTextureFormat(format) {
  return format in kRegularTextureFormatInfo;
}

/**
 * Returns true if format is both compressed and a float format, for example 'bc6h-rgb-ufloat'.
 */
function isCompressedFloatTextureFormat(format) {
  return isCompressedTextureFormat(format) && format.includes('float');
}

/**
 * Returns true if format is sint or uint
 */
function isSintOrUintFormat(format) {
  const info = kTextureFormatInfo[format];
  const type = info.color?.type ?? info.depth?.type ?? info.stencil?.type;
  return type === 'sint' || type === 'uint';
}

/**
 * Returns true if format can be multisampled.
 */
const kCompatModeUnsupportedMultisampledTextureFormats = [
'r8uint',
'r8sint',
'rg8uint',
'rg8sint',
'rgba8uint',
'rgba8sint',
'r16uint',
'r16sint',
'rg16uint',
'rg16sint',
'rgba16uint',
'rgba16sint',
'rgb10a2uint',
'rgba16float',
'r32float'];


/**
 * Returns true if you can make a multisampled texture from the given format.
 */
function isTextureFormatMultisampled(
features,
format)
{
  if (isCompatibilityMode(features)) {
    if (kCompatModeUnsupportedMultisampledTextureFormats.indexOf(format) >= 0) {
      return false;
    }
  }
  if (format === 'rg11b10ufloat') {
    return hasFeature(features, 'rg11b10ufloat-renderable');
  }
  if (isTextureFormatTier1EnablesRenderAttachmentBlendableMultisample(format)) {
    return hasFeature(features, 'texture-formats-tier1');
  }
  return kAllTextureFormatInfo[format].multisample;
}

/**
 * Returns true if a texture can be "resolved". uint/sint formats can be multisampled but
 * can not be resolved.
 */
function isTextureFormatResolvable(
features,
format)
{
  if (format === 'rg11b10ufloat') {
    return hasFeature(features, 'rg11b10ufloat-renderable');
  }
  if (isTextureFormatTier1EnablesResolve(format)) {
    return hasFeature(features, 'texture-formats-tier1');
  }
  // You can't resolve a non-multisampled format.
  if (!isTextureFormatMultisampled(features, format)) {
    return false;
  }
  const info = kAllTextureFormatInfo[format];
  return !!info.colorRender?.resolve;
}

// MAINTENANCE_TODD: See if we can remove this. This doesn't seem useful since
// formats are not on/off by feature. Some are on but a feature allows them to be
// used in more cases, like going from un-renderable to renderable, etc...
const kFeaturesForFormats = getFeaturesForFormats(kAllTextureFormats);

/**
 * Given an array of texture formats return the number of bytes per sample.
 */
function computeBytesPerSampleFromFormats(formats) {
  let bytesPerSample = 0;
  for (const format of formats) {
    // MAINTENANCE_TODO: Add colorRender to rg11b10ufloat format in kTextureFormatInfo
    // The issue is if we add it now lots of tests will break as they'll think they can
    // render to the format but are not enabling 'rg11b10ufloat-renderable'. Once we
    // get the CTS refactored (see issue 4181), then fix this.
    const info = kTextureFormatInfo[format];
    const alignedBytesPerSample = align(bytesPerSample, info.colorRender.alignment);
    bytesPerSample = alignedBytesPerSample + info.colorRender.byteCost;
  }
  return bytesPerSample;
}

/**
 * Given an array of GPUColorTargetState return the number of bytes per sample
 */
function computeBytesPerSample(targets) {
  return computeBytesPerSampleFromFormats(targets.map(({ format }) => format));
}

/**
 * Returns the maximum valid size in each dimension for a given texture format.
 * This is useful because compressed formats must be a multiple of blocks in size
 * so, for example, the largest valid width of a 2d texture
 * roundDown(device.limits.maxTextureDimension2D, blockWidth)
 */
function getMaxValidTextureSizeForFormatAndDimension(
device,
format,
dimension)
{
  const info = getBlockInfoForTextureFormat(format);
  switch (dimension) {
    case '1d':
      return [device.limits.maxTextureDimension1D, 1, 1];
    case '2d':
      return [
      roundDown(device.limits.maxTextureDimension2D, info.blockWidth),
      roundDown(device.limits.maxTextureDimension2D, info.blockHeight),
      device.limits.maxTextureArrayLayers];

    case '3d':
      return [
      roundDown(device.limits.maxTextureDimension3D, info.blockWidth),
      roundDown(device.limits.maxTextureDimension3D, info.blockHeight),
      device.limits.maxTextureDimension3D];

  }
}
});
__define('webgpu/util/texture/base.js', function (__exports) {
'use strict';
__get(__exports, 'maxMipLevelCount', () => maxMipLevelCount);
__get(__exports, 'physicalMipSize', () => physicalMipSize);
__get(__exports, 'physicalMipSizeFromTexture', () => physicalMipSizeFromTexture);
__get(__exports, 'virtualMipSize', () => virtualMipSize);
__get(__exports, 'getTextureDimensionFromView', () => getTextureDimensionFromView);
__get(__exports, 'viewDimensionsForTextureDimension', () => viewDimensionsForTextureDimension);
__get(__exports, 'effectiveViewDimensionForDimension', () => effectiveViewDimensionForDimension);
__get(__exports, 'effectiveViewDimensionForTexture', () => effectiveViewDimensionForTexture);
__get(__exports, 'defaultViewDimensionsForTexture', () => defaultViewDimensionsForTexture);
__get(__exports, 'reifyTextureDescriptor', () => reifyTextureDescriptor);
__get(__exports, 'reifyTextureViewDescriptor', () => reifyTextureViewDescriptor);
__get(__exports, 'fullSubrectCoordinates', () => fullSubrectCoordinates);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert, unreachable; __bind('common/util/util.js', (__m) => { ({ assert, unreachable } = __m); });let getBlockInfoForTextureFormat; __bind('webgpu/format_info.js', (__m) => { ({ getBlockInfoForTextureFormat } = __m); });let align; __bind('webgpu/util/math.js', (__m) => { ({ align } = __m); });
let reifyExtent3D; __bind('webgpu/util/unions.js', (__m) => { ({ reifyExtent3D } = __m); });



/**
 * Compute the maximum mip level count allowed for a given texture size and texture dimension.
 */
function maxMipLevelCount({
  size,
  dimension = '2d'



}) {
  const sizeDict = reifyExtent3D(size);

  let maxMippedDimension = 0;
  switch (dimension) {
    case '1d':
      maxMippedDimension = 1; // No mipmaps allowed.
      break;
    case '2d':
      maxMippedDimension = Math.max(sizeDict.width, sizeDict.height);
      break;
    case '3d':
      maxMippedDimension = Math.max(sizeDict.width, sizeDict.height, sizeDict.depthOrArrayLayers);
      break;
  }

  return Math.floor(Math.log2(maxMippedDimension)) + 1;
}

/**
 * Compute the "physical size" of a mip level: the size of the level, rounded up to a
 * multiple of the texel block size.
 */
function physicalMipSize(
baseSize,
format,
dimension,
level)
{
  switch (dimension) {
    case '1d':
      assert(level === 0, '1d textures cannot be mipmapped');
      assert(baseSize.height === 1 && baseSize.depthOrArrayLayers === 1, '1d texture not Wx1x1');
      return { width: baseSize.width, height: 1, depthOrArrayLayers: 1 };

    case '2d':{
        assert(
          Math.max(baseSize.width, baseSize.height) >> level > 0,
          () => `level (${level}) too large for base size (${baseSize.width}x${baseSize.height})`
        );

        const { blockWidth, blockHeight } = getBlockInfoForTextureFormat(format);
        const virtualWidthAtLevel = Math.max(baseSize.width >> level, 1);
        const virtualHeightAtLevel = Math.max(baseSize.height >> level, 1);
        const physicalWidthAtLevel = align(virtualWidthAtLevel, blockWidth);
        const physicalHeightAtLevel = align(virtualHeightAtLevel, blockHeight);
        return {
          width: physicalWidthAtLevel,
          height: physicalHeightAtLevel,
          depthOrArrayLayers: baseSize.depthOrArrayLayers
        };
      }

    case '3d':{
        assert(
          Math.max(baseSize.width, baseSize.height, baseSize.depthOrArrayLayers) >> level > 0,
          () =>
          `level (${level}) too large for base size (${baseSize.width}x${baseSize.height}x${baseSize.depthOrArrayLayers})`
        );
        const virtualWidthAtLevel = Math.max(baseSize.width >> level, 1);
        const virtualHeightAtLevel = Math.max(baseSize.height >> level, 1);
        const { blockWidth, blockHeight } = getBlockInfoForTextureFormat(format);
        const physicalWidthAtLevel = align(virtualWidthAtLevel, blockWidth);
        const physicalHeightAtLevel = align(virtualHeightAtLevel, blockHeight);
        return {
          width: physicalWidthAtLevel,
          height: physicalHeightAtLevel,
          depthOrArrayLayers: Math.max(baseSize.depthOrArrayLayers >> level, 1)
        };
      }
  }
}

/**
 * Compute the "physical size" of a mip level: the size of the level, rounded up to a
 * multiple of the texel block size.
 */
function physicalMipSizeFromTexture(
texture,
mipLevel)
{
  const size = physicalMipSize(texture, texture.format, texture.dimension, mipLevel);
  return [size.width, size.height, size.depthOrArrayLayers];
}

/**
 * Compute the "virtual size" of a mip level of a texture (not accounting for texel block rounding).
 *
 * MAINTENANCE_TODO: Change output to Required<GPUExtent3DDict> for consistency.
 */
function virtualMipSize(
dimension,
size,
mipLevel)
{
  const { width, height, depthOrArrayLayers } = reifyExtent3D(size);
  const shiftMinOne = (n) => Math.max(1, n >> mipLevel);
  switch (dimension) {
    case '1d':
      return [shiftMinOne(width), height, depthOrArrayLayers];
    case '2d':
      return [shiftMinOne(width), shiftMinOne(height), depthOrArrayLayers];
    case '3d':
      return [shiftMinOne(width), shiftMinOne(height), shiftMinOne(depthOrArrayLayers)];
    default:
      unreachable();
  }
}

/**
 * Get texture dimension from view dimension in order to create an compatible texture for a given
 * view dimension.
 */
function getTextureDimensionFromView(viewDimension) {
  switch (viewDimension) {
    case '1d':
      return '1d';
    case '2d':
    case '2d-array':
    case 'cube':
    case 'cube-array':
      return '2d';
    case '3d':
      return '3d';
    default:
      unreachable();
  }
}

/** Returns the possible valid view dimensions for a given texture dimension. */
function viewDimensionsForTextureDimension(textureDimension) {
  switch (textureDimension) {
    case '1d':
      return ['1d'];
    case '2d':
      return ['2d', '2d-array', 'cube', 'cube-array'];
    case '3d':
      return ['3d'];
  }
}

/** Returns the effective view dimension for a given texture dimension and depthOrArrayLayers */
function effectiveViewDimensionForDimension(
viewDimension,
dimension,
depthOrArrayLayers)
{
  if (viewDimension) {
    return viewDimension;
  }

  switch (dimension || '2d') {
    case '1d':
      return '1d';
    case '2d':
    case undefined:
      return depthOrArrayLayers > 1 ? '2d-array' : '2d';
      break;
    case '3d':
      return '3d';
    default:
      unreachable();
  }
}

/** Returns the effective view dimension for a given texture */
function effectiveViewDimensionForTexture(
texture,
viewDimension)
{
  return effectiveViewDimensionForDimension(
    viewDimension,
    texture.dimension,
    texture.depthOrArrayLayers
  );
}

/** Returns the default view dimension for a given texture descriptor. */
function defaultViewDimensionsForTexture(textureDescriptor) {
  const sizeDict = reifyExtent3D(textureDescriptor.size);
  return effectiveViewDimensionForDimension(
    undefined,
    textureDescriptor.dimension,
    sizeDict.depthOrArrayLayers
  );
}

/** Reifies the optional fields of `GPUTextureDescriptor`.
 * MAINTENANCE_TODO: viewFormats should not be omitted here, but it seems likely that the
 * @webgpu/types definition will have to change before we can include it again.
 */
function reifyTextureDescriptor(
desc)

{
  return { dimension: '2d', mipLevelCount: 1, sampleCount: 1, ...desc };
}

/** Reifies the optional fields of `GPUTextureViewDescriptor` (given a `GPUTextureDescriptor`). */
function reifyTextureViewDescriptor(
textureDescriptor,
view)
{
  const texture = reifyTextureDescriptor(textureDescriptor);

  // IDL defaulting

  const baseMipLevel = view.baseMipLevel ?? 0;
  const baseArrayLayer = view.baseArrayLayer ?? 0;
  const aspect = view.aspect ?? 'all';
  const swizzle = view.swizzle ?? 'rgba';

  // Spec defaulting

  const format = view.format ?? texture.format;
  const mipLevelCount = view.mipLevelCount ?? texture.mipLevelCount - baseMipLevel;
  const dimension = view.dimension ?? defaultViewDimensionsForTexture(texture);
  const usage = (view.usage ?? 0) === 0 ? texture.usage : view.usage;

  let arrayLayerCount = view.arrayLayerCount;
  if (arrayLayerCount === undefined) {
    if (dimension === '2d-array' || dimension === 'cube-array') {
      arrayLayerCount = reifyExtent3D(texture.size).depthOrArrayLayers - baseArrayLayer;
    } else if (dimension === 'cube') {
      arrayLayerCount = 6;
    } else {
      arrayLayerCount = 1;
    }
  }

  return {
    format,
    dimension,
    aspect,
    usage,
    baseMipLevel,
    mipLevelCount,
    baseArrayLayer,
    arrayLayerCount,
    swizzle
  };
}

/**
 * Get generator of all the coordinates in a subrect.
 * @param subrectOrigin - Subrect origin
 * @param subrectSize - Subrect size
 */
function* fullSubrectCoordinates(
subrectOrigin,
subrectSize,
sampleCount = 1)
{
  for (let z = subrectOrigin.z; z < subrectOrigin.z + subrectSize.depthOrArrayLayers; ++z) {
    for (let y = subrectOrigin.y; y < subrectOrigin.y + subrectSize.height; ++y) {
      for (let x = subrectOrigin.x; x < subrectOrigin.x + subrectSize.width; ++x) {
        for (let sampleIndex = 0; sampleIndex < sampleCount; ++sampleIndex) {
          yield { x, y, z, sampleIndex };
        }
      }
    }
  }
}
});
__define('webgpu/util/unions.js', function (__exports) {
'use strict';
__get(__exports, 'reifyOrigin3D', () => reifyOrigin3D);
__get(__exports, 'reifyExtent3D', () => reifyExtent3D);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /**
 * Reifies a `GPUOrigin3D` into a `Required<GPUOrigin3DDict>`.
 */function reifyOrigin3D(val)
{
  if (Symbol.iterator in val) {
    const v = Array.from(val);
    return {
      x: (v[0] ?? 0) | 0,
      y: (v[1] ?? 0) | 0,
      z: (v[2] ?? 0) | 0
    };
  } else {
    const v = val;
    return {
      x: (v.x ?? 0) | 0,
      y: (v.y ?? 0) | 0,
      z: (v.z ?? 0) | 0
    };
  }
}

/**
 * Reifies a `GPUExtent3D` into a `Required<GPUExtent3DDict>`.
 */
function reifyExtent3D(
val)
{
  if (Symbol.iterator in val) {
    const v = Array.from(val);
    return {
      width: (v[0] ?? 1) | 0,
      height: (v[1] ?? 1) | 0,
      depthOrArrayLayers: (v[2] ?? 1) | 0
    };
  } else {
    const v = val;
    return {
      width: (v.width ?? 1) | 0,
      height: (v.height ?? 1) | 0,
      depthOrArrayLayers: (v.depthOrArrayLayers ?? 1) | 0
    };
  }
}
});
__define('webgpu/util/command_buffer_maker.js', function (__exports) {
'use strict';
__get(__exports, 'kRenderEncodeTypes', () => kRenderEncodeTypes);
__get(__exports, 'kProgrammableEncoderTypes', () => kProgrammableEncoderTypes);
__get(__exports, 'kEncoderTypes', () => kEncoderTypes);
__get(__exports, 'CommandBufferMaker', () => CommandBufferMaker);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const kRenderEncodeTypes = ['render pass', 'render bundle'];

const kProgrammableEncoderTypes = ['compute pass', ...kRenderEncodeTypes];

const kEncoderTypes = ['non-pass', ...kProgrammableEncoderTypes];


// Look up the type of the encoder based on `T`. If `T` is a union, this will be too!







/** See {@link GPUTestBase.createEncoder}. */
class CommandBufferMaker {
  /** `GPU___Encoder` for recording commands into. */
  // Look up the type of the encoder based on `T`. If `T` is a union, this will be too!


  /**
   * Finish any passes, finish and record any bundles, and finish/return the command buffer. Any
   * errors are ignored and the GPUCommandBuffer (which may be an error buffer) is returned.
   */


  /**
   * Finish any passes, finish and record any bundles, and finish/return the command buffer.
   * Checks for validation errors in (only) the appropriate finish call.
   */


  /**
   * Finish the command buffer and submit it. Checks for validation errors in either the submit or
   * the appropriate finish call, depending on the state of a resource used in the encoding.
   */





  /**
   * `validateFinishAndSubmit()` based on the state of a resource in the command encoder.
   * - `finish()` should fail if the resource is 'invalid'.
   * - Only `submit()` should fail if the resource is 'destroyed'.
   */


  constructor(
  t,
  encoder,
  finish)
  {
    // TypeScript introduces an intersection type here where we don't want one.
    this.encoder = encoder;
    this.finish = finish;

    // Define extra methods like this, otherwise they get unbound when destructured, e.g.:
    //   const { encoder, validateFinishAndSubmit } = t.createEncoder(type);
    // Alternatively, do not destructure, and call member functions, e.g.:
    //   const encoder = t.createEncoder(type);
    //   encoder.validateFinish(true);
    this.validateFinish = (shouldSucceed) => {
      return t.expectGPUError('validation', this.finish, !shouldSucceed);
    };

    this.validateFinishAndSubmit = (
    shouldBeValid,
    submitShouldSucceedIfValid) =>
    {
      const commandBuffer = this.validateFinish(shouldBeValid);
      if (shouldBeValid) {
        t.expectValidationError(() => t.queue.submit([commandBuffer]), !submitShouldSucceedIfValid);
      }
    };

    this.validateFinishAndSubmitGivenState = (resourceState) => {
      this.validateFinishAndSubmit(resourceState !== 'invalid', resourceState !== 'destroyed');
    };
  }
}
});
__define('webgpu/util/device_pool.js', function (__exports) {
'use strict';
__get(__exports, 'TestOOMedShouldAttemptGC', () => TestOOMedShouldAttemptGC);
__get(__exports, 'DevicePool', () => DevicePool);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let SkipTestCase; __bind('common/framework/fixture.js', (__m) => { ({ SkipTestCase } = __m); });let globalTestConfig; __bind('common/framework/test_config.js', (__m) => { ({ globalTestConfig } = __m); });let attemptGarbageCollection; __bind('common/util/collect_garbage.js', (__m) => { ({ attemptGarbageCollection } = __m); });
let getGPU, getDefaultRequestAdapterOptions; __bind('common/util/navigator_gpu.js', (__m) => { ({ getGPU, getDefaultRequestAdapterOptions } = __m); });
let assert, raceWithRejectOnTimeout, assertReject, unreachable, hasFeature; __bind('common/util/util.js', (__m) => { ({ assert, raceWithRejectOnTimeout, assertReject, unreachable, hasFeature } = __m); });
let getDefaultLimits, kPossibleLimits; __bind('webgpu/capability_info.js', (__m) => { ({ getDefaultLimits, kPossibleLimits } = __m); });

// MUST_NOT_BE_IMPORTED_BY_DATA_CACHE
// This file should not be transitively imported by .cache.ts files








class TestFailedButDeviceReusable extends Error {}
class FeaturesNotSupported extends Error {}
class TestOOMedShouldAttemptGC extends Error {}

/**
 * DescriptorModifier lets you supply a function to select a device
 * based on the limits/features available from the adapter.
 * Devices pooled based on a key and that key is derived before
 * an adapter is requested. That means you select key without
 * knowledge of what the adapter will provide. You do this by
 * providing a keyModifier function that appends a suffix.
 *
 * For example: If your modifier adds all the limits you might
 * choose 'maxLimits' are your suffix
 *
 * ```js
 *   keyModifier(s: string) { return `${s}:maxLimits`; },
 * ```
 *
 * If your modifier selects only `maxBindGroups` and `maxColorAttachments`
 * then your suffix might be `maxBindGroups&maxColorAttachments`
 *
 * ```js
 *   keyModifier(s: string) { return `${s}:maxBindGroups&maxColorAttachments`; },
 * ```
 */








class DevicePool {
  holders = 'uninitialized';

  /** Acquire a device from the pool and begin the error scopes. */
  async acquire(
  recorder,
  descriptor,
  descriptorModifier)
  {
    let errorMessage = '';
    if (this.holders === 'uninitialized') {
      this.holders = new DescriptorToHolderMap();
      try {
        await this.holders.getOrCreate(recorder, undefined, descriptorModifier);
      } catch (ex) {
        this.holders = 'failed';
        if (ex instanceof Error) {
          errorMessage = ` with ${ex.name} "${ex.message}"`;
        }
      }
    }

    assert(
      this.holders !== 'failed',
      `WebGPU device failed to initialize${errorMessage}; not retrying`
    );

    const holder = await this.holders.getOrCreate(recorder, descriptor, descriptorModifier);

    assert(holder.state === 'free', 'Device was in use on DevicePool.acquire');
    holder.state = 'acquired';
    holder.beginTestScope();
    return holder;
  }

  /**
   * End the error scopes and check for errors.
   * Then, if the device seems reusable, release it back into the pool. Otherwise, drop it.
   */
  async release(holder) {
    assert(this.holders instanceof DescriptorToHolderMap, 'DevicePool got into a bad state');
    assert(holder instanceof DeviceHolder, 'DeviceProvider should always be a DeviceHolder');

    assert(holder.state === 'acquired', 'trying to release a device while already released');
    let deviceNeedsReplacement = false;
    try {
      await holder.endTestScope();

      if (holder.expectedLostReason) {
        deviceNeedsReplacement = true;
        assert(holder.lostInfo !== undefined, 'Device expected to be lost, but was not lost');
        assert(
          holder.lostInfo.reason === holder.expectedLostReason,
          `Expected device loss reason "${holder.expectedLostReason}", got "${holder.lostInfo?.reason}"`
        );
      } else {
        // Hopefully if the device was lost, it has been reported by the time endErrorScopes()
        // has finished (or timed out). If not, it could cause a finite number of extra test
        // failures following this test. (It should recover after one test in most cases.)
        assert(holder.lostInfo === undefined, 'Device lost unexpectedly during test');
      }
    } catch (ex) {
      // Any error that isn't explicitly TestFailedButDeviceReusable forces a new device to be
      // created for the next test.
      if (!(ex instanceof TestFailedButDeviceReusable)) {
        deviceNeedsReplacement = true;

        // Try to clean up, in case there are stray resources in need of collection that won't be
        // cleaned up by destroying the device - either outside the device (related to
        // interop/video/canvas/ImageBitmap/etc.) or not destroyed along with the device for some
        // other reason.
        if (ex instanceof TestOOMedShouldAttemptGC) {
          await attemptGarbageCollection();
        }
      }

      throw ex;
    } finally {
      const deviceDueForReplacement =
      holder.testCaseUseCounter >= globalTestConfig.casesBetweenReplacingDevice;
      if (deviceNeedsReplacement || deviceDueForReplacement) {
        this.holders.delete(holder);
        holder.device.destroy();
        await holder.device.lost;
      }

      // Mark the holder as free so the device can be reused (if it's still in this.devices).
      holder.state = 'free';
    }
  }

  /**
   * Destroy the pool, moving it to the persistent 'failed' state and destroy()ing any devices
   * in the pool, regardless of whether they're in use by a test.
   */
  destroy() {
    if (this.holders instanceof DescriptorToHolderMap) {
      this.holders.clear();
    }
    this.holders = 'failed';
  }
}

/**
 * Map from GPUDeviceDescriptor to DeviceHolder.
 */
class DescriptorToHolderMap {
  /** Map keys that are known to be unsupported and can be rejected quickly. */
  unsupported = new Set();
  holders = new Map();

  /** Deletes an item from the map by DeviceHolder value. */
  delete(holder) {
    for (const [k, v] of this.holders) {
      if (v === holder) {
        holder.device.destroy();
        this.holders.delete(k);
        return;
      }
    }
    unreachable("internal error: couldn't find DeviceHolder to delete");
  }

  /**
   * Gets a DeviceHolder from the map if it exists; otherwise, calls create() to create one,
   * inserts it, and returns it.
   *
   * If an `uncanonicalizedDescriptor` is provided, it is canonicalized and used as the map key.
   * If one is not provided, the map key is `""` (empty string).
   *
   * Throws SkipTestCase if devices with this descriptor are unsupported.
   */
  async getOrCreate(
  recorder,
  uncanonicalizedDescriptor,
  descriptorModifier)
  {
    const [descriptor, baseKey] = canonicalizeDescriptor(uncanonicalizedDescriptor);
    const key = descriptorModifier?.keyModifier(baseKey) || baseKey;
    // Quick-reject descriptors that are known to be unsupported already.
    if (this.unsupported.has(key)) {
      throw new SkipTestCase(
        `GPUDeviceDescriptor previously failed: ${JSON.stringify(descriptor)}`
      );
    }

    // Search for an existing device with the same descriptor.
    {
      const value = this.holders.get(key);
      if (value) {
        // Move it to the end of the Map (most-recently-used).
        this.holders.delete(key);
        this.holders.set(key, value);
        return value;
      }
    }

    // No existing item was found; add a new one.
    let value;
    try {
      value = await DeviceHolder.create(recorder, descriptor, descriptorModifier);
    } catch (ex) {
      if (ex instanceof FeaturesNotSupported) {
        this.unsupported.add(key);
        throw new SkipTestCase(
          `GPUDeviceDescriptor not supported: ${JSON.stringify(descriptor)}\n${ex?.message ?? ''}`
        );
      }

      throw ex;
    }
    this.insertAndCleanUp(key, value);
    return value;
  }

  /**
   * Insert an entry, then remove and destroy() the least-recently-used devices
   * if there are too many.
   */
  insertAndCleanUp(key, value) {
    this.holders.set(key, value);

    const kMaxEntries = 5;
    if (this.holders.size > kMaxEntries) {
      // Delete the first (least recently used) item in the set.
      for (const [key, value] of this.holders) {
        value.device.destroy();
        this.holders.delete(key);
        break;
      }
    }
  }

  /** Destroy all the devices and clear the map. This destroys devices even if they're in use. */
  clear() {
    for (const [, value] of this.holders) {
      value.device.destroy();
    }
    this.holders.clear();
  }
}









/**
 * Make a stringified map-key from a GPUDeviceDescriptor.
 * Tries to make sure all defaults are resolved, first - but it's okay if some are missed
 * (it just means some GPUDevice objects won't get deduplicated).
 *
 * This does **not** canonicalize `undefined` (the "default" descriptor) into a fully-qualified
 * GPUDeviceDescriptor. This is just because `undefined` is a common case and we want to use it
 * as a sanity check that WebGPU is working.
 */
function canonicalizeDescriptor(
desc)
{
  if (desc === undefined) {
    return [undefined, ''];
  }

  const featuresCanonicalized = desc.requiredFeatures ?
  Array.from(new Set(desc.requiredFeatures)).sort() :
  [];

  /** Canonicalized version of the requested limits: in canonical order, with only values which are
   * specified _and_ non-default. */
  const limitsCanonicalized = {};
  const featureLevel = getDefaultRequestAdapterOptions()?.featureLevel ?? 'core';
  assert(featureLevel === 'compatibility' || featureLevel === 'core');
  const defaultLimits = getDefaultLimits(featureLevel);
  if (desc.requiredLimits) {
    for (const limit of kPossibleLimits) {
      const requestedValue = desc.requiredLimits[limit];
      const defaultValue = defaultLimits[limit]?.default;
      // Skip adding a limit to limitsCanonicalized if it is the same as the default.
      if (requestedValue !== undefined && requestedValue !== defaultValue) {
        limitsCanonicalized[limit] = requestedValue;
      }
    }
  }

  // Type ensures every field is carried through.
  const descriptorCanonicalized = {
    requiredFeatures: featuresCanonicalized,
    requiredLimits: limitsCanonicalized,
    defaultQueue: {}
  };
  return [descriptorCanonicalized, JSON.stringify(descriptorCanonicalized)];
}

function supportsFeature(
adapter,
descriptor)
{
  if (descriptor === undefined) {
    return true;
  }

  for (const feature of descriptor.requiredFeatures) {
    if (!hasFeature(adapter.features, feature)) {
      return false;
    }
  }

  return true;
}

/**
 * DeviceHolder has three states:
 * - 'free': Free to be used for a new test.
 * - 'acquired': In use by a running test.
 */


/**
 * Holds a GPUDevice and tracks its state (free/acquired) and handles device loss.
 */
class DeviceHolder {
  /** Adapter the device was created from. Cannot be reused; just for adapter info. */

  /** The device. Will be cleared during cleanup if there were unexpected errors. */

  /** Whether the device is in use by a test or not. */
  state = 'free';
  /** initially undefined; becomes set when the device is lost */

  /** Set if the device is expected to be lost. */

  /** Number of test cases the device has been used for. */
  testCaseUseCounter = 0;

  // Gets a device and creates a DeviceHolder.
  // If the device is lost, DeviceHolder.lost gets set.
  static async create(
  recorder,
  descriptor,
  descriptorModifier)
  {
    const gpu = getGPU(recorder);
    const adapter = await gpu.requestAdapter();
    assert(adapter !== null, 'requestAdapter returned null');
    if (descriptorModifier) {
      descriptor = descriptorModifier.descriptorModifier(adapter, descriptor);
    }
    if (!supportsFeature(adapter, descriptor)) {
      throw new FeaturesNotSupported('One or more features are not supported');
    }
    // No trackForCleanup because we plan to reuse the device for the next test.

    const device = await adapter.requestDevice(descriptor);
    assert(device !== null, 'requestDevice returned null');

    return new DeviceHolder(adapter, device);
  }

  constructor(adapter, device) {
    this.adapter = adapter;
    this._device = device;
    void this._device.lost.then((ev) => {
      this.lostInfo = ev;
    });
  }

  get device() {
    return this._device;
  }

  /** Push error scopes that surround test execution. */
  beginTestScope() {
    assert(this.state === 'acquired');
    this.testCaseUseCounter++;
    this.device.pushErrorScope('validation');
    this.device.pushErrorScope('internal');
    this.device.pushErrorScope('out-of-memory');
  }

  /** Mark the DeviceHolder as expecting a device loss when the test scope ends. */
  expectDeviceLost(reason) {
    assert(this.state === 'acquired');
    this.expectedLostReason = reason;
  }

  /**
   * Attempt to end test scopes: Check that there are no extra error scopes, and that no
   * otherwise-uncaptured errors occurred during the test. Time out if it takes too long.
   */
  endTestScope() {
    assert(this.state === 'acquired');
    const kTimeout = 5000;

    // Time out if attemptEndTestScope (popErrorScope or onSubmittedWorkDone) never completes. If
    // this rejects, the device won't be reused, so it's OK that popErrorScope calls may not have
    // finished.
    //
    // This could happen due to a browser bug - e.g.,
    // as of this writing, on Chrome GPU process crash, popErrorScope just hangs.
    return raceWithRejectOnTimeout(this.attemptEndTestScope(), kTimeout, 'endTestScope timed out');
  }

  async attemptEndTestScope() {
    let gpuValidationError;
    let gpuInternalError;
    let gpuOutOfMemoryError;

    // Wait for queue to be idle just in case there are any implementation bugs where errors are not
    // reported promptly. (This won't catch everything, e.g. deferred pipeline creations, but is
    // still slightly more likely to catch things.)
    await this.device.queue.onSubmittedWorkDone();

    try {
      // If the device is lost, all of these should return null.
      [gpuOutOfMemoryError, gpuInternalError, gpuValidationError] = await Promise.all([
      this.device.popErrorScope(),
      this.device.popErrorScope(),
      this.device.popErrorScope()]
      );
    } catch (ex) {
      unreachable('popErrorScope failed. Did the test body pop too many scopes?');
    }

    if (!this.expectedLostReason) {
      await assertReject('OperationError', this.device.popErrorScope(), {
        allowMissingStack: true,
        message: 'There was an extra error scope on the stack after a test'
      });
    }

    if (gpuOutOfMemoryError !== null) {
      assert(gpuOutOfMemoryError instanceof GPUOutOfMemoryError);
      // Don't allow the device to be reused; unexpected OOM could break the device.
      throw new TestOOMedShouldAttemptGC('Unexpected out-of-memory error occurred');
    }
    if (gpuInternalError !== null) {
      assert(gpuInternalError instanceof GPUInternalError);
      // Allow the device to be reused.
      throw new TestFailedButDeviceReusable(
        `Unexpected internal error occurred: ${gpuInternalError.message}`
      );
    }
    if (gpuValidationError !== null) {
      assert(gpuValidationError instanceof GPUValidationError);
      // Allow the device to be reused.
      throw new TestFailedButDeviceReusable(
        `Unexpected validation error occurred: ${gpuValidationError.message}`
      );
    }
  }
}
});
__define('webgpu/util/texture/layout.js', function (__exports) {
'use strict';
__get(__exports, 'kBytesPerRowAlignment', () => kBytesPerRowAlignment);
__get(__exports, 'kBufferCopyAlignment', () => kBufferCopyAlignment);
__get(__exports, 'getTextureCopyLayout', () => getTextureCopyLayout);
__get(__exports, 'getTextureSubCopyLayout', () => getTextureSubCopyLayout);
__get(__exports, 'fillTextureDataWithTexelValue', () => fillTextureDataWithTexelValue);
__get(__exports, 'createTextureUploadBuffer', () => createTextureUploadBuffer);
__get(__exports, 'kImageCopyTypes', () => kImageCopyTypes);
__get(__exports, 'bytesInACompleteRow', () => bytesInACompleteRow);
__get(__exports, 'dataBytesForCopyOrFail', () => dataBytesForCopyOrFail);
__get(__exports, 'dataBytesForCopyOrOverestimate', () => dataBytesForCopyOrOverestimate);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert, memcpy; __bind('common/util/util.js', (__m) => { ({ assert, memcpy } = __m); });let resolvePerAspectFormat, getBlockInfoForTextureFormat, getBlockInfoForEncodableTextureFormat; __bind('webgpu/format_info.js', (__m) => { ({ resolvePerAspectFormat, getBlockInfoForTextureFormat, getBlockInfoForEncodableTextureFormat } = __m); });

let align; __bind('webgpu/util/math.js', (__m) => { ({ align } = __m); });
let reifyExtent3D; __bind('webgpu/util/unions.js', (__m) => { ({ reifyExtent3D } = __m); });

let physicalMipSize, virtualMipSize; __bind('webgpu/util/texture/base.js', (__m) => { ({ physicalMipSize, virtualMipSize } = __m); });

/** The minimum `bytesPerRow` alignment, per spec. */
const kBytesPerRowAlignment = 256;
/** The minimum buffer copy alignment, per spec. */
const kBufferCopyAlignment = 4;

/**
 * Overridable layout options for {@link getTextureCopyLayout}.
 */







const kDefaultLayoutOptions = {
  mipLevel: 0,
  bytesPerRow: undefined,
  rowsPerImage: undefined,
  aspect: 'all'
};

/** The info returned by {@link getTextureSubCopyLayout}. */














/** The info returned by {@link getTextureCopyLayout}. */




/**
 * Computes layout information for a copy of the whole subresource at `mipLevel` of a GPUTexture
 * of size `baseSize` with the provided `format` and `dimension`.
 *
 * Computes default values for `bytesPerRow` and `rowsPerImage` if not specified.
 *
 * MAINTENANCE_TODO: Change input/output to Required<GPUExtent3DDict> for consistency.
 */
function getTextureCopyLayout(
format,
dimension,
baseSize,
{ mipLevel, bytesPerRow, rowsPerImage, aspect } = kDefaultLayoutOptions)
{
  const mipSize = physicalMipSize(
    { width: baseSize[0], height: baseSize[1], depthOrArrayLayers: baseSize[2] },
    format,
    dimension,
    mipLevel
  );

  const layout = getTextureSubCopyLayout(format, mipSize, { bytesPerRow, rowsPerImage, aspect });
  return { ...layout, mipSize: [mipSize.width, mipSize.height, mipSize.depthOrArrayLayers] };
}

/**
 * Computes layout information for a copy of size `copySize` to/from a GPUTexture with the provided
 * `format`.
 *
 * Computes default values for `bytesPerRow` and `rowsPerImage` if not specified.
 */
function getTextureSubCopyLayout(
format,
copySize,
{
  bytesPerRow,
  rowsPerImage,
  aspect = 'all'




} = {})
{
  format = resolvePerAspectFormat(format, aspect);
  const { blockWidth, blockHeight, bytesPerBlock } = getBlockInfoForTextureFormat(format);
  assert(bytesPerBlock !== undefined);

  const copySize_ = reifyExtent3D(copySize);
  assert(
    copySize_.width > 0 && copySize_.height > 0 && copySize_.depthOrArrayLayers > 0,
    'not implemented for empty copySize'
  );
  assert(
    copySize_.width % blockWidth === 0 && copySize_.height % blockHeight === 0,
    () =>
    `copySize (${copySize_.width},${copySize_.height}) must be a multiple of the block size (${blockWidth},${blockHeight})`
  );
  const copySizeBlocks = {
    width: copySize_.width / blockWidth,
    height: copySize_.height / blockHeight,
    depthOrArrayLayers: copySize_.depthOrArrayLayers
  };

  const minBytesPerRow = copySizeBlocks.width * bytesPerBlock;
  const alignedMinBytesPerRow = align(minBytesPerRow, kBytesPerRowAlignment);
  if (bytesPerRow !== undefined) {
    assert(bytesPerRow >= alignedMinBytesPerRow);
    assert(bytesPerRow % kBytesPerRowAlignment === 0);
  } else {
    bytesPerRow = alignedMinBytesPerRow;
  }

  if (rowsPerImage !== undefined) {
    assert(rowsPerImage >= copySizeBlocks.height);
  } else {
    rowsPerImage = copySizeBlocks.height;
  }

  const bytesPerSlice = bytesPerRow * rowsPerImage;
  const sliceSize =
  bytesPerRow * (copySizeBlocks.height - 1) + bytesPerBlock * copySizeBlocks.width;
  const byteLength = bytesPerSlice * (copySizeBlocks.depthOrArrayLayers - 1) + sliceSize;

  return {
    bytesPerBlock,
    byteLength: align(byteLength, kBufferCopyAlignment),
    minBytesPerRow,
    bytesPerRow,
    rowsPerImage
  };
}

/**
 * Fill an ArrayBuffer with the linear-memory representation of a solid-color
 * texture where every texel has the byte value `texelValue`.
 * Preserves the contents of `outputBuffer` which are in "padding" space between image rows.
 *
 * Effectively emulates a copyTextureToBuffer from a solid-color texture to a buffer.
 */
function fillTextureDataWithTexelValue(
texelValue,
format,
dimension,
outputBuffer,
size,
options = kDefaultLayoutOptions)
{
  const { blockWidth, blockHeight, bytesPerBlock } = getBlockInfoForEncodableTextureFormat(format);
  // Block formats are not handled correctly below.
  assert(blockWidth === 1);
  assert(blockHeight === 1);

  assert(bytesPerBlock === texelValue.byteLength, 'texelValue must be of size bytesPerBlock');

  const { byteLength, rowsPerImage, bytesPerRow } = getTextureCopyLayout(
    format,
    dimension,
    size,
    options
  );

  assert(byteLength <= outputBuffer.byteLength);

  const mipSize = virtualMipSize(dimension, size, options.mipLevel);

  const outputTexelValueBytes = new Uint8Array(outputBuffer);
  for (let slice = 0; slice < mipSize[2]; ++slice) {
    for (let row = 0; row < mipSize[1]; row += blockHeight) {
      for (let col = 0; col < mipSize[0]; col += blockWidth) {
        const byteOffset =
        slice * rowsPerImage * bytesPerRow + row * bytesPerRow + col * texelValue.byteLength;
        memcpy({ src: texelValue }, { dst: outputTexelValueBytes, start: byteOffset });
      }
    }
  }
}

/**
 * Create a `COPY_SRC` GPUBuffer containing the linear-memory representation of a solid-color
 * texture where every texel has the byte value `texelValue`.
 */
function createTextureUploadBuffer(
t,
texelValue,
format,
dimension,
size,
options = kDefaultLayoutOptions)




{
  const { byteLength, bytesPerRow, rowsPerImage, bytesPerBlock } = getTextureCopyLayout(
    format,
    dimension,
    size,
    options
  );

  const buffer = t.createBufferTracked({
    mappedAtCreation: true,
    size: byteLength,
    usage: GPUBufferUsage.COPY_SRC
  });
  const mapping = buffer.getMappedRange();

  assert(texelValue.byteLength === bytesPerBlock);
  fillTextureDataWithTexelValue(texelValue, format, dimension, mapping, size, options);
  buffer.unmap();

  return {
    buffer,
    bytesPerRow,
    rowsPerImage
  };
}


const kImageCopyTypes = [
'WriteTexture',
'CopyB2T',
'CopyT2B'];


/**
 * Computes `bytesInACompleteRow` (as defined by the WebGPU spec) for image copies (B2T/T2B/writeTexture).
 */
function bytesInACompleteRow(copyWidth, format) {
  const info = getBlockInfoForTextureFormat(format);
  assert(!!info.bytesPerBlock);
  assert(copyWidth % info.blockWidth === 0);
  return info.bytesPerBlock * copyWidth / info.blockWidth;
}

function validateBytesPerRow({
  bytesPerRow,
  bytesInLastRow,
  sizeInBlocks




}) {
  // If specified, layout.bytesPerRow must be greater than or equal to bytesInLastRow.
  if (bytesPerRow !== undefined && bytesPerRow < bytesInLastRow) {
    return false;
  }
  // If heightInBlocks > 1, layout.bytesPerRow must be specified.
  // If copyExtent.depthOrArrayLayers > 1, layout.bytesPerRow and layout.rowsPerImage must be specified.
  if (
  bytesPerRow === undefined && (
  sizeInBlocks.height > 1 || sizeInBlocks.depthOrArrayLayers > 1))
  {
    return false;
  }
  return true;
}

function validateRowsPerImage({
  rowsPerImage,
  sizeInBlocks



}) {
  // If specified, layout.rowsPerImage must be greater than or equal to heightInBlocks.
  if (rowsPerImage !== undefined && rowsPerImage < sizeInBlocks.height) {
    return false;
  }
  // If copyExtent.depthOrArrayLayers > 1, layout.bytesPerRow and layout.rowsPerImage must be specified.
  if (rowsPerImage === undefined && sizeInBlocks.depthOrArrayLayers > 1) {
    return false;
  }
  return true;
}








/**
 * Validate a copy and compute the number of bytes it needs. Throws if the copy is invalid.
 */
function dataBytesForCopyOrFail(args) {
  const { minDataSizeOrOverestimate, copyValid } = dataBytesForCopyOrOverestimate(args);
  assert(copyValid, 'copy was invalid');
  return minDataSizeOrOverestimate;
}

/**
 * Validate a copy and compute the number of bytes it needs. If the copy is invalid, attempts to
 * "conservatively guess" (overestimate) the number of bytes that could be needed for a copy, even
 * if the copy parameters turn out to be invalid. This hopes to avoid "buffer too small" validation
 * errors when attempting to test other validation errors.
 */
function dataBytesForCopyOrOverestimate({
  layout,
  format,
  copySize: copySize_,
  method
}) {
  const copyExtent = reifyExtent3D(copySize_);

  const info = getBlockInfoForTextureFormat(format);
  assert(!!info.bytesPerBlock);
  assert(copyExtent.width % info.blockWidth === 0);
  assert(copyExtent.height % info.blockHeight === 0);
  const sizeInBlocks = {
    width: copyExtent.width / info.blockWidth,
    height: copyExtent.height / info.blockHeight,
    depthOrArrayLayers: copyExtent.depthOrArrayLayers
  };
  const bytesInLastRow = sizeInBlocks.width * info.bytesPerBlock;

  let valid = true;
  const offset = layout.offset ?? 0;
  if (method !== 'WriteTexture') {
    if (offset % info.bytesPerBlock !== 0) valid = false;
    if (layout.bytesPerRow && layout.bytesPerRow % 256 !== 0) valid = false;
  }

  let requiredBytesInCopy = 0;
  {
    let { bytesPerRow, rowsPerImage } = layout;

    // If bytesPerRow or rowsPerImage is invalid, guess a value for the sake of various tests that
    // don't actually care about the exact value.
    // (In particular for validation tests that want to test invalid bytesPerRow or rowsPerImage but
    // need to make sure the total buffer size is still big enough.)
    if (!validateBytesPerRow({ bytesPerRow, bytesInLastRow, sizeInBlocks })) {
      bytesPerRow = undefined;
      valid = false;
    }
    if (!validateRowsPerImage({ rowsPerImage, sizeInBlocks })) {
      rowsPerImage = undefined;
      valid = false;
    }
    // Pick values for cases when (a) bpr/rpi was invalid or (b) they're validly undefined.
    bytesPerRow ??= align(info.bytesPerBlock * sizeInBlocks.width, 256);
    rowsPerImage ??= sizeInBlocks.height;

    if (copyExtent.depthOrArrayLayers > 1) {
      const bytesPerImage = bytesPerRow * rowsPerImage;
      const bytesBeforeLastImage = bytesPerImage * (copyExtent.depthOrArrayLayers - 1);
      requiredBytesInCopy += bytesBeforeLastImage;
    }
    if (copyExtent.depthOrArrayLayers > 0) {
      if (sizeInBlocks.height > 1) requiredBytesInCopy += bytesPerRow * (sizeInBlocks.height - 1);
      if (sizeInBlocks.height > 0) requiredBytesInCopy += bytesInLastRow;
    }
  }

  return { minDataSizeOrOverestimate: offset + requiredBytesInCopy, copyValid: valid };
}
});
__define('webgpu/util/texture/texel_data.js', function (__exports) {
'use strict';
__get(__exports, 'TexelComponent', () => TexelComponent);
__get(__exports, 'makeClampToRange', () => makeClampToRange);
__get(__exports, 'kTexelRepresentationInfo', () => kTexelRepresentationInfo);
__get(__exports, 'getSingleDataType', () => getSingleDataType);
__get(__exports, 'getComponentReadbackTraits', () => getComponentReadbackTraits);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let assert, unreachable; __bind('common/util/util.js', (__m) => { ({ assert, unreachable } = __m); });let kValue; __bind('webgpu/util/constants.js', (__m) => { ({ kValue } = __m); });
let assertInIntegerRange, float32ToFloatBits, float32ToFloat16Bits, floatAsNormalizedInteger, gammaCompress, gammaDecompress, normalizedIntegerAsFloat, packRGB9E5UFloat, floatBitsToNumber, float16BitsToFloat32, floatBitsToNormalULPFromZero, kFloat32Format, kFloat16Format, kUFloat9e5Format, numberToFloat32Bits, float32BitsToNumber, numberToFloatBits, ufloatM9E5BitsToNumber; __bind('webgpu/util/conversion.js', (__m) => { ({ assertInIntegerRange, float32ToFloatBits, float32ToFloat16Bits, floatAsNormalizedInteger, gammaCompress, gammaDecompress, normalizedIntegerAsFloat, packRGB9E5UFloat, floatBitsToNumber, float16BitsToFloat32, floatBitsToNormalULPFromZero, kFloat32Format, kFloat16Format, kUFloat9e5Format, numberToFloat32Bits, float32BitsToNumber, numberToFloatBits, ufloatM9E5BitsToNumber } = __m); });
let clamp, signExtend; __bind('webgpu/util/math.js', (__m) => { ({ clamp, signExtend } = __m); });

/** A component of a texture format: R, G, B, A, Depth, or Stencil. */
let TexelComponent = /*#__PURE__*/function (TexelComponent) {TexelComponent["R"] = "R";TexelComponent["G"] = "G";TexelComponent["B"] = "B";TexelComponent["A"] = "A";TexelComponent["Depth"] = "Depth";TexelComponent["Stencil"] = "Stencil";return TexelComponent;}({});








/** Arbitrary data, per component of a texel format. */


/** How a component is encoded in its bit range of a texel format. */


/**
 * Maps component values to component values
 * @param {PerTexelComponent<number>} components - The input components.
 * @returns {PerTexelComponent<number>} The new output components.
 */


/**
 * Packs component values as an ArrayBuffer
 * @param {PerTexelComponent<number>} components - The input components.
 * @returns {ArrayBuffer} The packed data.
 */


/** Unpacks component values from a Uint8Array */


/**
 * Create a PerTexelComponent object filled with the same value for all components.
 * @param {TexelComponent[]} components - The component names.
 * @param {T} value - The value to assign to each component.
 * @returns {PerTexelComponent<T>}
 */
function makePerTexelComponent(components, value) {
  const values = {};
  for (const c of components) {
    values[c] = value;
  }
  return values;
}

/**
 * Create a function which applies clones a `PerTexelComponent<number>` and then applies the
 * function `fn` to each component of `components`.
 * @param {(value: number) => number} fn - The mapping function to apply to component values.
 * @param {TexelComponent[]} components - The component names.
 * @returns {ComponentMapFn} The map function which clones the input component values, and applies
 *                           `fn` to each of component of `components`.
 */
function applyEach(
fn,
components)
{
  return (values) => {
    values = Object.assign({}, values);
    for (const c of components) {
      assert(values[c] !== undefined);
      values[c] = fn(values[c], c);
    }
    return values;
  };
}

/**
 * A `ComponentMapFn` for encoding sRGB.
 * @param {PerTexelComponent<number>} components - The input component values.
 * @returns {TexelComponent<number>} Gamma-compressed copy of `components`.
 */
const encodeSRGB = (components) => {
  assert(
    components.R !== undefined && components.G !== undefined && components.B !== undefined,
    'sRGB requires all of R, G, and B components'
  );
  return applyEach(gammaCompress, kRGB)(components);
};

/**
 * A `ComponentMapFn` for decoding sRGB.
 * @param {PerTexelComponent<number>} components - The input component values.
 * @returns {TexelComponent<number>} Gamma-decompressed copy of `components`.
 */
const decodeSRGB = (components) => {
  components = Object.assign({}, components);
  assert(
    components.R !== undefined && components.G !== undefined && components.B !== undefined,
    'sRGB requires all of R, G, and B components'
  );
  return applyEach(gammaDecompress, kRGB)(components);
};

/**
 * Makes a `ComponentMapFn` for clamping values to the specified range.
 */
function makeClampToRange(format) {
  const repr = kTexelRepresentationInfo[format];
  assert(repr.numericRange !== null, 'Format has unknown numericRange');
  const perComponentRanges = repr.numericRange;
  const range = repr.numericRange;

  return applyEach((x, component) => {
    const perComponentRange = perComponentRanges[component];
    return clamp(x, perComponentRange ? perComponentRange : range);
  }, repr.componentOrder);
}

// MAINTENANCE_TODO: Look into exposing this map to the test fixture so that it can be GCed at the
// end of each test group. That would allow for caching of larger buffers (though it's unclear how
// ofter larger buffers are used by packComponents.)
const smallComponentDataViews = new Map();
function getComponentDataView(byteLength) {
  if (byteLength > 32) {
    const buffer = new ArrayBuffer(byteLength);
    return new DataView(buffer);
  }
  let dataView = smallComponentDataViews.get(byteLength);
  if (!dataView) {
    const buffer = new ArrayBuffer(byteLength);
    dataView = new DataView(buffer);
    smallComponentDataViews.set(byteLength, dataView);
  }
  return dataView;
}

/**
 * Helper function to pack components as an ArrayBuffer.
 * @param {TexelComponent[]} componentOrder - The order of the component data.
 * @param {PerTexelComponent<number>} components - The input component values.
 * @param {number | PerTexelComponent<number>} bitLengths - The length in bits of each component.
 *   If a single number, all components are the same length, otherwise this is a dictionary of
 *   per-component bit lengths.
 * @param {ComponentDataType | PerTexelComponent<ComponentDataType>} componentDataTypes -
 *   The type of the data in `components`. If a single value, all components have the same value.
 *   Otherwise, this is a dictionary of per-component data types.
 * @returns {ArrayBuffer} The packed component data.
 */
function packComponents(
componentOrder,
components,
bitLengths,
componentDataTypes)
{
  let bitLengthMap;
  let totalBitLength;
  if (typeof bitLengths === 'number') {
    bitLengthMap = makePerTexelComponent(componentOrder, bitLengths);
    totalBitLength = bitLengths * componentOrder.length;
  } else {
    bitLengthMap = bitLengths;
    totalBitLength = Object.entries(bitLengthMap).reduce((acc, [, value]) => {
      assert(value !== undefined);
      return acc + value;
    }, 0);
  }
  assert(totalBitLength % 8 === 0);

  const componentDataTypeMap =
  typeof componentDataTypes === 'string' || componentDataTypes === null ?
  makePerTexelComponent(componentOrder, componentDataTypes) :
  componentDataTypes;

  const dataView = getComponentDataView(totalBitLength / 8);
  let bitOffset = 0;
  for (const c of componentOrder) {
    const value = components[c];
    const type = componentDataTypeMap[c];
    const bitLength = bitLengthMap[c];
    assert(value !== undefined);
    assert(type !== undefined);
    assert(bitLength !== undefined);

    const byteOffset = Math.floor(bitOffset / 8);
    const byteLength = Math.ceil(bitLength / 8);
    switch (type) {
      case 'uint':
      case 'unorm':
        if (byteOffset === bitOffset / 8 && byteLength === bitLength / 8) {
          switch (byteLength) {
            case 1:
              dataView.setUint8(byteOffset, value);
              break;
            case 2:
              dataView.setUint16(byteOffset, value, true);
              break;
            case 4:
              dataView.setUint32(byteOffset, value, true);
              break;
            default:
              unreachable();
          }
        } else {
          // Packed representations are all 32-bit and use Uint as the data type.
          // ex.) rg10b11float, rgb10a2unorm
          switch (dataView.byteLength) {
            case 4:{
                const currentValue = dataView.getUint32(0, true);

                let mask = 0xffffffff;
                const bitsToClearRight = bitOffset;
                const bitsToClearLeft = 32 - (bitLength + bitOffset);

                mask = mask >>> bitsToClearRight << bitsToClearRight;
                mask = mask << bitsToClearLeft >>> bitsToClearLeft;

                const newValue = currentValue & ~mask | value << bitOffset;

                dataView.setUint32(0, newValue, true);
                break;
              }
            default:
              unreachable();
          }
        }
        break;
      case 'sint':
      case 'snorm':
        assert(byteOffset === bitOffset / 8 && byteLength === bitLength / 8);
        switch (byteLength) {
          case 1:
            dataView.setInt8(byteOffset, value);
            break;
          case 2:
            dataView.setInt16(byteOffset, value, true);
            break;
          case 4:
            dataView.setInt32(byteOffset, value, true);
            break;
          default:
            unreachable();
        }
        break;
      case 'float':
        assert(byteOffset === bitOffset / 8 && byteLength === bitLength / 8);
        switch (byteLength) {
          case 4:
            dataView.setFloat32(byteOffset, value, true);
            break;
          default:
            unreachable();
        }
        break;
      case 'ufloat':
      case null:
        unreachable();
    }

    bitOffset += bitLength;
  }

  return dataView.buffer;
}

/**
 * Unpack substrings of bits from a Uint8Array, e.g. [8,8,8,8] or [9,9,9,5].
 */
function unpackComponentsBits(
componentOrder,
byteView,
bitLengths)
{
  const components = makePerTexelComponent(componentOrder, 0);

  let bitLengthMap;
  let totalBitLength;
  if (typeof bitLengths === 'number') {
    let index = 0;
    // Optimized cases for when the bit lengths are all a well aligned value.
    switch (bitLengths) {
      case 8:
        for (const c of componentOrder) {
          components[c] = byteView[index++];
        }
        return components;
      case 16:{
          const shortView = new Uint16Array(byteView.buffer, byteView.byteOffset);
          for (const c of componentOrder) {
            components[c] = shortView[index++];
          }
          return components;
        }
      case 32:{
          const longView = new Uint32Array(byteView.buffer, byteView.byteOffset);
          for (const c of componentOrder) {
            components[c] = longView[index++];
          }
          return components;
        }
    }

    bitLengthMap = makePerTexelComponent(componentOrder, bitLengths);
    totalBitLength = bitLengths * componentOrder.length;
  } else {
    bitLengthMap = bitLengths;
    totalBitLength = Object.entries(bitLengthMap).reduce((acc, [, value]) => {
      assert(value !== undefined);
      return acc + value;
    }, 0);
  }

  assert(totalBitLength % 8 === 0);

  const dataView = new DataView(byteView.buffer, byteView.byteOffset, byteView.byteLength);
  let bitOffset = 0;
  for (const c of componentOrder) {
    const bitLength = bitLengthMap[c];
    assert(bitLength !== undefined);

    let value;

    const byteOffset = Math.floor(bitOffset / 8);
    const byteLength = Math.ceil(bitLength / 8);
    if (byteOffset === bitOffset / 8 && byteLength === bitLength / 8) {
      switch (byteLength) {
        case 1:
          value = dataView.getUint8(byteOffset);
          break;
        case 2:
          value = dataView.getUint16(byteOffset, true);
          break;
        case 4:
          value = dataView.getUint32(byteOffset, true);
          break;
        default:
          unreachable();
      }
    } else {
      // Packed representations are all 32-bit and use Uint as the data type.
      // ex.) rg10b11float, rgb10a2unorm
      assert(dataView.byteLength === 4);
      const word = dataView.getUint32(0, true);
      value = word >>> bitOffset & (1 << bitLength) - 1;
    }

    bitOffset += bitLength;
    components[c] = value;
  }

  return components;
}

/**
 * Create an entry in `kTexelRepresentationInfo` for normalized integer texel data with constant
 * bitlength.
 * @param {TexelComponent[]} componentOrder - The order of the component data.
 * @param {number} bitLength - The number of bits in each component.
 * @param {{signed: boolean; sRGB: boolean}} opt - Boolean flags for `signed` and `sRGB`.
 */
function makeNormalizedInfo(
componentOrder,
bitLength,
opt)
{
  const encodeNonSRGB = applyEach(
    (n) => floatAsNormalizedInteger(n, bitLength, opt.signed),
    componentOrder
  );
  const decodeNonSRGB = applyEach(
    (n) => normalizedIntegerAsFloat(n, bitLength, opt.signed),
    componentOrder
  );

  const numberToBitsNonSRGB = applyEach(
    (n) => floatAsNormalizedInteger(n, bitLength, opt.signed),
    componentOrder
  );
  let bitsToNumberNonSRGB;
  if (opt.signed) {
    bitsToNumberNonSRGB = applyEach(
      (n) => normalizedIntegerAsFloat(signExtend(n, bitLength), bitLength, opt.signed),
      componentOrder
    );
  } else {
    bitsToNumberNonSRGB = applyEach(
      (n) => normalizedIntegerAsFloat(n, bitLength, opt.signed),
      componentOrder
    );
  }

  let encode;
  let decode;
  let numberToBits;
  let bitsToNumber;
  if (opt.sRGB) {
    encode = (components) => encodeNonSRGB(encodeSRGB(components));
    decode = (components) => decodeSRGB(decodeNonSRGB(components));
    numberToBits = (components) => numberToBitsNonSRGB(encodeSRGB(components));
    bitsToNumber = (components) => decodeSRGB(bitsToNumberNonSRGB(components));
  } else {
    encode = encodeNonSRGB;
    decode = decodeNonSRGB;
    numberToBits = numberToBitsNonSRGB;
    bitsToNumber = bitsToNumberNonSRGB;
  }

  let bitsToULPFromZero;
  if (opt.signed) {
    const maxValue = (1 << bitLength - 1) - 1; // e.g. 127 for snorm8
    bitsToULPFromZero = applyEach(
      (n) => Math.max(-maxValue, signExtend(n, bitLength)),
      componentOrder
    );
  } else {
    bitsToULPFromZero = (components) => components;
  }

  const dataType = opt.signed ? 'snorm' : 'unorm';
  const min = opt.signed ? -1 : 0;
  const max = 1;
  return {
    componentOrder,
    componentInfo: makePerTexelComponent(componentOrder, {
      dataType,
      bitLength
    }),
    encode,
    decode,
    pack: (components) =>
    packComponents(componentOrder, components, bitLength, dataType),
    unpackBits: (data) => unpackComponentsBits(componentOrder, data, bitLength),
    numberToBits,
    bitsToNumber,
    bitsToULPFromZero,
    numericRange: { min, max, finiteMin: min, finiteMax: max }
  };
}

/**
 * Create an entry in `kTexelRepresentationInfo` for integer texel data with constant bitlength.
 * @param {TexelComponent[]} componentOrder - The order of the component data.
 * @param {number} bitLength - The number of bits in each component.
 * @param {{signed: boolean}} opt - Boolean flag for `signed`.
 */
function makeIntegerInfo(
componentOrder,
bitLength,
opt)
{
  assert(bitLength <= 32);
  const min = opt.signed ? -(2 ** (bitLength - 1)) : 0;
  const max = opt.signed ? 2 ** (bitLength - 1) - 1 : 2 ** bitLength - 1;
  const numericRange = { min, max, finiteMin: min, finiteMax: max };
  const maxUnsignedValue = 2 ** bitLength;
  const encode = applyEach(
    (n) => (assertInIntegerRange(n, bitLength, opt.signed), n),
    componentOrder
  );
  const decode = applyEach(
    (n) => (assertInIntegerRange(n, bitLength, opt.signed), n),
    componentOrder
  );
  const bitsToNumber = applyEach((n) => {
    const decodedN = opt.signed ? n > numericRange.max ? n - maxUnsignedValue : n : n;
    assertInIntegerRange(decodedN, bitLength, opt.signed);
    return decodedN;
  }, componentOrder);

  let bitsToULPFromZero;
  if (opt.signed) {
    bitsToULPFromZero = applyEach((n) => signExtend(n, bitLength), componentOrder);
  } else {
    bitsToULPFromZero = (components) => components;
  }

  const dataType = opt.signed ? 'sint' : 'uint';
  const bitMask = (1 << bitLength) - 1;
  return {
    componentOrder,
    componentInfo: makePerTexelComponent(componentOrder, {
      dataType,
      bitLength
    }),
    encode,
    decode,
    pack: (components) =>
    packComponents(componentOrder, components, bitLength, dataType),
    unpackBits: (data) => unpackComponentsBits(componentOrder, data, bitLength),
    numberToBits: applyEach((v) => v & bitMask, componentOrder),
    bitsToNumber,
    bitsToULPFromZero,
    numericRange
  };
}

/**
 * Create an entry in `kTexelRepresentationInfo` for floating point texel data with constant
 * bitlength.
 * @param {TexelComponent[]} componentOrder - The order of the component data.
 * @param {number} bitLength - The number of bits in each component.
 */
function makeFloatInfo(
componentOrder,
bitLength,
{ restrictedDepth = false } = {})
{
  let encode;
  let numberToBits;
  let bitsToNumber;
  let bitsToULPFromZero;
  switch (bitLength) {
    case 32:
      if (restrictedDepth) {
        encode = applyEach((v) => {
          assert(v >= 0.0 && v <= 1.0, 'depth out of range');
          return new Float32Array([v])[0];
        }, componentOrder);
      } else {
        encode = applyEach((v) => new Float32Array([v])[0], componentOrder);
      }
      numberToBits = applyEach(numberToFloat32Bits, componentOrder);
      bitsToNumber = applyEach(float32BitsToNumber, componentOrder);
      bitsToULPFromZero = applyEach(
        (v) => floatBitsToNormalULPFromZero(v, kFloat32Format),
        componentOrder
      );
      break;
    case 16:
      if (restrictedDepth) {
        encode = applyEach((v) => {
          assert(v >= 0.0 && v <= 1.0, 'depth out of range');
          return float16BitsToFloat32(float32ToFloat16Bits(v));
        }, componentOrder);
      } else {
        encode = applyEach((v) => float16BitsToFloat32(float32ToFloat16Bits(v)), componentOrder);
      }
      numberToBits = applyEach(float32ToFloat16Bits, componentOrder);
      bitsToNumber = applyEach(float16BitsToFloat32, componentOrder);
      bitsToULPFromZero = applyEach(
        (v) => floatBitsToNormalULPFromZero(v, kFloat16Format),
        componentOrder
      );
      break;
    default:
      unreachable();
  }
  const decode = applyEach(identity, componentOrder);

  return {
    componentOrder,
    componentInfo: makePerTexelComponent(componentOrder, {
      dataType: 'float',
      bitLength
    }),
    encode,
    decode,
    pack: (components) => {
      switch (bitLength) {
        case 16:
          components = applyEach(float32ToFloat16Bits, componentOrder)(components);
          return packComponents(componentOrder, components, 16, 'uint');
        case 32:
          return packComponents(componentOrder, components, bitLength, 'float');
        default:
          unreachable();
      }
    },
    unpackBits: (data) => unpackComponentsBits(componentOrder, data, bitLength),
    numberToBits,
    bitsToNumber,
    bitsToULPFromZero,
    numericRange: restrictedDepth ?
    { min: 0, max: 1, finiteMin: 0, finiteMax: 1 } :
    {
      min: Number.NEGATIVE_INFINITY,
      max: Number.POSITIVE_INFINITY,
      finiteMin: bitLength === 32 ? kValue.f32.negative.min : kValue.f16.negative.min,
      finiteMax: bitLength === 32 ? kValue.f32.positive.max : kValue.f16.positive.max
    }
  };
}

const kR = [TexelComponent.R];
const kRG = [TexelComponent.R, TexelComponent.G];
const kRGB = [TexelComponent.R, TexelComponent.G, TexelComponent.B];
const kRGBA = [TexelComponent.R, TexelComponent.G, TexelComponent.B, TexelComponent.A];
const kBGRA = [TexelComponent.B, TexelComponent.G, TexelComponent.R, TexelComponent.A];

const identity = (n) => n;

const kFloat11Format = { signed: 0, exponentBits: 5, mantissaBits: 6, bias: 15 };
const kFloat10Format = { signed: 0, exponentBits: 5, mantissaBits: 5, bias: 15 };



















































const kTexelRepresentationInfo =

{
  ...{
    'r8unorm': makeNormalizedInfo(kR, 8, { signed: false, sRGB: false }),
    'r8snorm': makeNormalizedInfo(kR, 8, { signed: true, sRGB: false }),
    'r8uint': makeIntegerInfo(kR, 8, { signed: false }),
    'r8sint': makeIntegerInfo(kR, 8, { signed: true }),
    'r16unorm': makeNormalizedInfo(kR, 16, { signed: false, sRGB: false }),
    'r16snorm': makeNormalizedInfo(kR, 16, { signed: true, sRGB: false }),
    'r16uint': makeIntegerInfo(kR, 16, { signed: false }),
    'r16sint': makeIntegerInfo(kR, 16, { signed: true }),
    'r16float': makeFloatInfo(kR, 16),
    'rg8unorm': makeNormalizedInfo(kRG, 8, { signed: false, sRGB: false }),
    'rg8snorm': makeNormalizedInfo(kRG, 8, { signed: true, sRGB: false }),
    'rg8uint': makeIntegerInfo(kRG, 8, { signed: false }),
    'rg8sint': makeIntegerInfo(kRG, 8, { signed: true }),
    'r32uint': makeIntegerInfo(kR, 32, { signed: false }),
    'r32sint': makeIntegerInfo(kR, 32, { signed: true }),
    'r32float': makeFloatInfo(kR, 32),
    'rg16unorm': makeNormalizedInfo(kRG, 16, { signed: false, sRGB: false }),
    'rg16snorm': makeNormalizedInfo(kRG, 16, { signed: true, sRGB: false }),
    'rg16uint': makeIntegerInfo(kRG, 16, { signed: false }),
    'rg16sint': makeIntegerInfo(kRG, 16, { signed: true }),
    'rg16float': makeFloatInfo(kRG, 16),
    'rgba8unorm': makeNormalizedInfo(kRGBA, 8, { signed: false, sRGB: false }),
    'rgba8unorm-srgb': makeNormalizedInfo(kRGBA, 8, { signed: false, sRGB: true }),
    'rgba8snorm': makeNormalizedInfo(kRGBA, 8, { signed: true, sRGB: false }),
    'rgba8uint': makeIntegerInfo(kRGBA, 8, { signed: false }),
    'rgba8sint': makeIntegerInfo(kRGBA, 8, { signed: true }),
    'bgra8unorm': makeNormalizedInfo(kBGRA, 8, { signed: false, sRGB: false }),
    'bgra8unorm-srgb': makeNormalizedInfo(kBGRA, 8, { signed: false, sRGB: true }),
    'rg32uint': makeIntegerInfo(kRG, 32, { signed: false }),
    'rg32sint': makeIntegerInfo(kRG, 32, { signed: true }),
    'rg32float': makeFloatInfo(kRG, 32),
    'rgba16unorm': makeNormalizedInfo(kRGBA, 16, { signed: false, sRGB: false }),
    'rgba16snorm': makeNormalizedInfo(kRGBA, 16, { signed: true, sRGB: false }),
    'rgba16uint': makeIntegerInfo(kRGBA, 16, { signed: false }),
    'rgba16sint': makeIntegerInfo(kRGBA, 16, { signed: true }),
    'rgba16float': makeFloatInfo(kRGBA, 16),
    'rgba32uint': makeIntegerInfo(kRGBA, 32, { signed: false }),
    'rgba32sint': makeIntegerInfo(kRGBA, 32, { signed: true }),
    'rgba32float': makeFloatInfo(kRGBA, 32)
  },
  ...{
    rgb10a2uint: {
      componentOrder: kRGBA,
      componentInfo: {
        R: { dataType: 'uint', bitLength: 10 },
        G: { dataType: 'uint', bitLength: 10 },
        B: { dataType: 'uint', bitLength: 10 },
        A: { dataType: 'uint', bitLength: 2 }
      },
      encode: (components) => {
        assertInIntegerRange(components.R, 10, false);
        assertInIntegerRange(components.G, 10, false);
        assertInIntegerRange(components.B, 10, false);
        assertInIntegerRange(components.A, 2, false);
        return components;
      },
      decode: (components) => {
        assertInIntegerRange(components.R, 10, false);
        assertInIntegerRange(components.G, 10, false);
        assertInIntegerRange(components.B, 10, false);
        assertInIntegerRange(components.A, 2, false);
        return components;
      },
      pack: (components) =>
      packComponents(
        kRGBA,
        components,
        {
          R: 10,
          G: 10,
          B: 10,
          A: 2
        },
        'uint'
      ),
      unpackBits: (data) =>
      unpackComponentsBits(kRGBA, data, { R: 10, G: 10, B: 10, A: 2 }),
      numberToBits: (components) => ({
        R: components.R & 0x3ff,
        G: components.G & 0x3ff,
        B: components.B & 0x3ff,
        A: components.A & 0x3
      }),
      bitsToNumber: (components) => {
        assertInIntegerRange(components.R, 10, false);
        assertInIntegerRange(components.G, 10, false);
        assertInIntegerRange(components.B, 10, false);
        assertInIntegerRange(components.A, 2, false);
        return components;
      },
      bitsToULPFromZero: (components) => components,
      numericRange: {
        R: { min: 0, max: 0x3ff, finiteMin: 0, finiteMax: 0x3ff },
        G: { min: 0, max: 0x3ff, finiteMin: 0, finiteMax: 0x3ff },
        B: { min: 0, max: 0x3ff, finiteMin: 0, finiteMax: 0x3ff },
        A: { min: 0, max: 0x3, finiteMin: 0, finiteMax: 0x3 }
      }
    },
    rgb10a2unorm: {
      componentOrder: kRGBA,
      componentInfo: {
        R: { dataType: 'unorm', bitLength: 10 },
        G: { dataType: 'unorm', bitLength: 10 },
        B: { dataType: 'unorm', bitLength: 10 },
        A: { dataType: 'unorm', bitLength: 2 }
      },
      encode: (components) => {
        return {
          R: floatAsNormalizedInteger(components.R ?? unreachable(), 10, false),
          G: floatAsNormalizedInteger(components.G ?? unreachable(), 10, false),
          B: floatAsNormalizedInteger(components.B ?? unreachable(), 10, false),
          A: floatAsNormalizedInteger(components.A ?? unreachable(), 2, false)
        };
      },
      decode: (components) => {
        return {
          R: normalizedIntegerAsFloat(components.R ?? unreachable(), 10, false),
          G: normalizedIntegerAsFloat(components.G ?? unreachable(), 10, false),
          B: normalizedIntegerAsFloat(components.B ?? unreachable(), 10, false),
          A: normalizedIntegerAsFloat(components.A ?? unreachable(), 2, false)
        };
      },
      pack: (components) =>
      packComponents(
        kRGBA,
        components,
        {
          R: 10,
          G: 10,
          B: 10,
          A: 2
        },
        'uint'
      ),
      unpackBits: (data) =>
      unpackComponentsBits(kRGBA, data, { R: 10, G: 10, B: 10, A: 2 }),
      numberToBits: (components) => ({
        R: floatAsNormalizedInteger(components.R ?? unreachable(), 10, false),
        G: floatAsNormalizedInteger(components.G ?? unreachable(), 10, false),
        B: floatAsNormalizedInteger(components.B ?? unreachable(), 10, false),
        A: floatAsNormalizedInteger(components.A ?? unreachable(), 2, false)
      }),
      bitsToNumber: (components) => ({
        R: normalizedIntegerAsFloat(components.R, 10, false),
        G: normalizedIntegerAsFloat(components.G, 10, false),
        B: normalizedIntegerAsFloat(components.B, 10, false),
        A: normalizedIntegerAsFloat(components.A, 2, false)
      }),
      bitsToULPFromZero: (components) => components,
      numericRange: { min: 0, max: 1, finiteMin: 0, finiteMax: 1 }
    },
    rg11b10ufloat: {
      componentOrder: kRGB,
      encode: applyEach(identity, kRGB),
      decode: applyEach(identity, kRGB),
      componentInfo: {
        R: { dataType: 'ufloat', bitLength: 11 },
        G: { dataType: 'ufloat', bitLength: 11 },
        B: { dataType: 'ufloat', bitLength: 10 }
      },
      pack: (components) => {
        const componentsBits = {
          R: float32ToFloatBits(components.R ?? unreachable(), 0, 5, 6, 15),
          G: float32ToFloatBits(components.G ?? unreachable(), 0, 5, 6, 15),
          B: float32ToFloatBits(components.B ?? unreachable(), 0, 5, 5, 15)
        };
        return packComponents(
          kRGB,
          componentsBits,
          {
            R: 11,
            G: 11,
            B: 10
          },
          'uint'
        );
      },
      unpackBits: (data) => unpackComponentsBits(kRGB, data, { R: 11, G: 11, B: 10 }),
      numberToBits: (components) => ({
        R: numberToFloatBits(components.R ?? unreachable(), kFloat11Format),
        G: numberToFloatBits(components.G ?? unreachable(), kFloat11Format),
        B: numberToFloatBits(components.B ?? unreachable(), kFloat10Format)
      }),
      bitsToNumber: (components) => ({
        R: floatBitsToNumber(components.R, kFloat11Format),
        G: floatBitsToNumber(components.G, kFloat11Format),
        B: floatBitsToNumber(components.B, kFloat10Format)
      }),
      bitsToULPFromZero: (components) => ({
        R: floatBitsToNormalULPFromZero(components.R, kFloat11Format),
        G: floatBitsToNormalULPFromZero(components.G, kFloat11Format),
        B: floatBitsToNormalULPFromZero(components.B, kFloat10Format)
      }),
      numericRange: {
        min: 0,
        max: Number.POSITIVE_INFINITY,
        finiteMin: 0,
        finiteMax: {
          R: floatBitsToNumber(0b111_1011_1111, kFloat11Format),
          G: floatBitsToNumber(0b111_1011_1111, kFloat11Format),
          B: floatBitsToNumber(0b11_1101_1111, kFloat10Format)
        }
      }
    },
    rgb9e5ufloat: {
      componentOrder: kRGB,
      componentInfo: makePerTexelComponent(kRGB, {
        dataType: 'ufloat',
        bitLength: -1 // Components don't really have a bitLength since the format is packed.
      }),
      encode: applyEach(identity, kRGB),
      decode: applyEach(identity, kRGB),
      pack: (components) =>
      new Uint32Array([
      packRGB9E5UFloat(
        components.R ?? unreachable(),
        components.G ?? unreachable(),
        components.B ?? unreachable()
      )]
      ).buffer,
      unpackBits: (data) => {
        const encoded = data[3] << 24 | data[2] << 16 | data[1] << 8 | data[0];
        const redMantissa = encoded >>> 0 & 0b111111111;
        const greenMantissa = encoded >>> 9 & 0b111111111;
        const blueMantissa = encoded >>> 18 & 0b111111111;
        const exponentSharedBits = (encoded >>> 27 & 0b11111) << 9;
        return {
          R: exponentSharedBits | redMantissa,
          G: exponentSharedBits | greenMantissa,
          B: exponentSharedBits | blueMantissa
        };
      },
      numberToBits: (components) => ({
        R: float32ToFloatBits(components.R ?? unreachable(), 0, 5, 9, 15),
        G: float32ToFloatBits(components.G ?? unreachable(), 0, 5, 9, 15),
        B: float32ToFloatBits(components.B ?? unreachable(), 0, 5, 9, 15)
      }),
      bitsToNumber: (components) => ({
        R: ufloatM9E5BitsToNumber(components.R, kUFloat9e5Format),
        G: ufloatM9E5BitsToNumber(components.G, kUFloat9e5Format),
        B: ufloatM9E5BitsToNumber(components.B, kUFloat9e5Format)
      }),
      bitsToULPFromZero: (components) => ({
        R: floatBitsToNormalULPFromZero(components.R, kUFloat9e5Format),
        G: floatBitsToNormalULPFromZero(components.G, kUFloat9e5Format),
        B: floatBitsToNormalULPFromZero(components.B, kUFloat9e5Format)
      }),
      numericRange: {
        min: 0,
        max: Number.POSITIVE_INFINITY,
        finiteMin: 0,
        finiteMax: ufloatM9E5BitsToNumber(0b11_1111_1111_1111, kUFloat9e5Format)
      }
    },
    depth32float: makeFloatInfo([TexelComponent.Depth], 32, { restrictedDepth: true }),
    depth16unorm: makeNormalizedInfo([TexelComponent.Depth], 16, { signed: false, sRGB: false }),
    depth24plus: {
      componentOrder: [TexelComponent.Depth],
      componentInfo: { Depth: { dataType: null, bitLength: 24 } },
      encode: applyEach(() => unreachable('depth24plus cannot be encoded'), [TexelComponent.Depth]),
      decode: applyEach(() => unreachable('depth24plus cannot be decoded'), [TexelComponent.Depth]),
      pack: () => unreachable('depth24plus data cannot be packed'),
      unpackBits: () => unreachable('depth24plus data cannot be unpacked'),
      numberToBits: () => unreachable('depth24plus has no representation'),
      bitsToNumber: () => unreachable('depth24plus has no representation'),
      bitsToULPFromZero: () => unreachable('depth24plus has no representation'),
      numericRange: { min: 0, max: 1, finiteMin: 0, finiteMax: 1 }
    },
    stencil8: makeIntegerInfo([TexelComponent.Stencil], 8, { signed: false }),
    'depth32float-stencil8': {
      componentOrder: [TexelComponent.Depth, TexelComponent.Stencil],
      componentInfo: {
        Depth: {
          dataType: 'float',
          bitLength: 32
        },
        Stencil: {
          dataType: 'uint',
          bitLength: 8
        }
      },
      encode: (components) => {
        assert(components.Stencil !== undefined);
        assertInIntegerRange(components.Stencil, 8, false);
        return components;
      },
      decode: (components) => {
        assert(components.Stencil !== undefined);
        assertInIntegerRange(components.Stencil, 8, false);
        return components;
      },
      pack: () => unreachable('depth32float-stencil8 data cannot be packed'),
      unpackBits: () => unreachable('depth32float-stencil8 data cannot be unpacked'),
      numberToBits: () => unreachable('not implemented'),
      bitsToNumber: () => unreachable('not implemented'),
      bitsToULPFromZero: () => unreachable('not implemented'),
      numericRange: null
    },
    'depth24plus-stencil8': {
      componentOrder: [TexelComponent.Depth, TexelComponent.Stencil],
      componentInfo: {
        Depth: {
          dataType: null,
          bitLength: 24
        },
        Stencil: {
          dataType: 'uint',
          bitLength: 8
        }
      },
      encode: (components) => {
        assert(components.Depth === undefined, 'depth24plus cannot be encoded');
        assert(components.Stencil !== undefined);
        assertInIntegerRange(components.Stencil, 8, false);
        return components;
      },
      decode: (components) => {
        assert(components.Depth === undefined, 'depth24plus cannot be decoded');
        assert(components.Stencil !== undefined);
        assertInIntegerRange(components.Stencil, 8, false);
        return components;
      },
      pack: () => unreachable('depth24plus-stencil8 data cannot be packed'),
      unpackBits: () => unreachable('depth24plus-stencil8 data cannot be unpacked'),
      numberToBits: () => unreachable('depth24plus-stencil8 has no representation'),
      bitsToNumber: () => unreachable('depth24plus-stencil8 has no representation'),
      bitsToULPFromZero: () => unreachable('depth24plus-stencil8 has no representation'),
      numericRange: null
    }
  }
};

/**
 * Get the `ComponentDataType` for a format. All components must have the same type.
 * @param {UncompressedTextureFormat} format - The input format.
 * @returns {ComponentDataType} The data of the components.
 */
function getSingleDataType(format) {
  const infos = Object.values(kTexelRepresentationInfo[format].componentInfo);
  assert(infos.length > 0);
  return infos.reduce((acc, cur) => {
    assert(cur !== undefined);
    assert(acc === undefined || acc === cur.dataType);
    return cur.dataType;
  }, infos[0].dataType);
}

/**
 * Get traits for generating code to readback data from a component.
 * @param {ComponentDataType} dataType - The input component data type.
 * @returns A dictionary containing the respective `ReadbackTypedArray` and `shaderType`.
 */
function getComponentReadbackTraits(dataType) {
  switch (dataType) {
    case 'ufloat':
    case 'float':
    case 'unorm':
    case 'snorm':
      return {
        ReadbackTypedArray: Float32Array,
        shaderType: 'f32'
      };
    case 'uint':
      return {
        ReadbackTypedArray: Uint32Array,
        shaderType: 'u32'
      };
    case 'sint':
      return {
        ReadbackTypedArray: Int32Array,
        shaderType: 'i32'
      };
    default:
      unreachable();
  }
}
});
__define('webgpu/util/conversion.js', function (__exports) {
'use strict';
__get(__exports, 'floatAsNormalizedIntegerUnquantized', () => floatAsNormalizedIntegerUnquantized);
__get(__exports, 'floatAsNormalizedInteger', () => floatAsNormalizedInteger);
__get(__exports, 'normalizedIntegerAsFloat', () => normalizedIntegerAsFloat);
__get(__exports, 'numbersApproximatelyEqual', () => numbersApproximatelyEqual);
__get(__exports, 'float32ToFloatBits', () => float32ToFloatBits);
__get(__exports, 'float32ToFloat16Bits', () => float32ToFloat16Bits);
__get(__exports, 'float16BitsToFloat32', () => float16BitsToFloat32);
__get(__exports, 'kFloat32Format', () => kFloat32Format);
__get(__exports, 'kFloat16Format', () => kFloat16Format);
__get(__exports, 'kUFloat9e5Format', () => kUFloat9e5Format);
__get(__exports, 'float32BitsToNumber', () => float32BitsToNumber);
__get(__exports, 'numberToFloat32Bits', () => numberToFloat32Bits);
__get(__exports, 'floatBitsToNumber', () => floatBitsToNumber);
__get(__exports, 'ufloatM9E5BitsToNumber', () => ufloatM9E5BitsToNumber);
__get(__exports, 'numberToFloatBits', () => numberToFloatBits);
__get(__exports, 'floatBitsToNormalULPFromZero', () => floatBitsToNormalULPFromZero);
__get(__exports, 'packRGB9E5UFloat', () => packRGB9E5UFloat);
__get(__exports, 'unpackRGB9E5UFloat', () => unpackRGB9E5UFloat);
__get(__exports, 'pack2x16float', () => pack2x16float);
__get(__exports, 'pack2x16snorm', () => pack2x16snorm);
__get(__exports, 'pack2x16unorm', () => pack2x16unorm);
__get(__exports, 'pack4x8snorm', () => pack4x8snorm);
__get(__exports, 'pack4x8unorm', () => pack4x8unorm);
__get(__exports, 'assertInIntegerRange', () => assertInIntegerRange);
__get(__exports, 'gammaCompress', () => gammaCompress);
__get(__exports, 'gammaDecompress', () => gammaDecompress);
__get(__exports, 'float32ToUint32', () => float32ToUint32);
__get(__exports, 'uint32ToFloat32', () => uint32ToFloat32);
__get(__exports, 'float32ToInt32', () => float32ToInt32);
__get(__exports, 'uint32ToInt32', () => uint32ToInt32);
__get(__exports, 'float16ToUint16', () => float16ToUint16);
__get(__exports, 'uint16ToFloat16', () => uint16ToFloat16);
__get(__exports, 'float16ToInt16', () => float16ToInt16);
__get(__exports, 'ScalarType', () => ScalarType);
__get(__exports, 'VectorType', () => VectorType);
__get(__exports, 'MatrixType', () => MatrixType);
__get(__exports, 'ArrayType', () => ArrayType);
__get(__exports, 'Type', () => Type);
__get(__exports, 'stringToType', () => stringToType);
__get(__exports, 'scalarType', () => scalarType);
__get(__exports, 'numElementsOf', () => numElementsOf);
__get(__exports, 'elementsOf', () => elementsOf);
__get(__exports, 'scalarElementsOf', () => scalarElementsOf);
__get(__exports, 'elementTypeOf', () => elementTypeOf);
__get(__exports, 'scalarTypeOf', () => scalarTypeOf);
__get(__exports, 'concreteTypeOf', () => concreteTypeOf);
__get(__exports, 'AbstractIntValue', () => AbstractIntValue);
__get(__exports, 'AbstractFloatValue', () => AbstractFloatValue);
__get(__exports, 'I32Value', () => I32Value);
__get(__exports, 'U32Value', () => U32Value);
__get(__exports, 'I16Value', () => I16Value);
__get(__exports, 'U16Value', () => U16Value);
__get(__exports, 'I8Value', () => I8Value);
__get(__exports, 'U8Value', () => U8Value);
__get(__exports, 'F64Value', () => F64Value);
__get(__exports, 'F32Value', () => F32Value);
__get(__exports, 'F16Value', () => F16Value);
__get(__exports, 'BoolValue', () => BoolValue);
__get(__exports, 'isScalarValue', () => isScalarValue);
__get(__exports, 'abstractInt', () => abstractInt);
__get(__exports, 'abstractIntBits', () => abstractIntBits);
__get(__exports, 'abstractFloat', () => abstractFloat);
__get(__exports, 'i32', () => i32);
__get(__exports, 'i32Bits', () => i32Bits);
__get(__exports, 'u32', () => u32);
__get(__exports, 'u32Bits', () => u32Bits);
__get(__exports, 'i16', () => i16);
__get(__exports, 'u16', () => u16);
__get(__exports, 'i8', () => i8);
__get(__exports, 'u8', () => u8);
__get(__exports, 'f64', () => f64);
__get(__exports, 'f32', () => f32);
__get(__exports, 'f32Bits', () => f32Bits);
__get(__exports, 'f16', () => f16);
__get(__exports, 'f16Bits', () => f16Bits);
__get(__exports, 'bool', () => bool);
__get(__exports, 'True', () => True);
__get(__exports, 'False', () => False);
__get(__exports, 'VectorValue', () => VectorValue);
__get(__exports, 'vec', () => vec);
__get(__exports, 'vec2', () => vec2);
__get(__exports, 'vec3', () => vec3);
__get(__exports, 'vec4', () => vec4);
__get(__exports, 'toVector', () => toVector);
__get(__exports, 'MatrixValue', () => MatrixValue);
__get(__exports, 'ArrayValue', () => ArrayValue);
__get(__exports, 'array', () => array);
__get(__exports, 'toMatrix', () => toMatrix);
__get(__exports, 'serializeValue', () => serializeValue);
__get(__exports, 'deserializeValue', () => deserializeValue);
__get(__exports, 'isFloatValue', () => isFloatValue);
__get(__exports, 'isAbstractType', () => isAbstractType);
__get(__exports, 'isFloatType', () => isFloatType);
__get(__exports, 'isIntegerType', () => isIntegerType);
__get(__exports, 'isConvertibleToFloatType', () => isConvertibleToFloatType);
__get(__exports, 'isUnsignedType', () => isUnsignedType);
__get(__exports, 'isConvertible', () => isConvertible);
__get(__exports, 'kFloatScalars', () => kFloatScalars);
__get(__exports, 'kConcreteFloatScalars', () => kConcreteFloatScalars);
__get(__exports, 'kConcreteF32ScalarsAndVectors', () => kConcreteF32ScalarsAndVectors);
__get(__exports, 'kConcreteF16ScalarsAndVectors', () => kConcreteF16ScalarsAndVectors);
__get(__exports, 'kFloatVectors', () => kFloatVectors);
__get(__exports, 'kFloatScalarsAndVectors', () => kFloatScalarsAndVectors);
__get(__exports, 'kConcreteSignedIntegerVectors', () => kConcreteSignedIntegerVectors);
__get(__exports, 'kConcreteUnsignedIntegerVectors', () => kConcreteUnsignedIntegerVectors);
__get(__exports, 'kConcreteIntegerVectors', () => kConcreteIntegerVectors);
__get(__exports, 'kConcreteSignedIntegerScalarsAndVectors', () => kConcreteSignedIntegerScalarsAndVectors);
__get(__exports, 'kConcreteUnsignedIntegerScalarsAndVectors', () => kConcreteUnsignedIntegerScalarsAndVectors);
__get(__exports, 'kConcreteIntegerScalarsAndVectors', () => kConcreteIntegerScalarsAndVectors);
__get(__exports, 'kConvertableToFloatScalar', () => kConvertableToFloatScalar);
__get(__exports, 'kConvertableToFloatVec2', () => kConvertableToFloatVec2);
__get(__exports, 'kConvertableToFloatVec3', () => kConvertableToFloatVec3);
__get(__exports, 'kConvertableToFloatVec4', () => kConvertableToFloatVec4);
__get(__exports, 'kConvertableToFloatVectors', () => kConvertableToFloatVectors);
__get(__exports, 'kConvertableToFloatScalarsAndVectors', () => kConvertableToFloatScalarsAndVectors);
__get(__exports, 'kAllNumericScalarsAndVectors', () => kAllNumericScalarsAndVectors);
__get(__exports, 'kConcreteNumericScalarsAndVectors', () => kConcreteNumericScalarsAndVectors);
__get(__exports, 'kAllBoolScalarsAndVectors', () => kAllBoolScalarsAndVectors);
__get(__exports, 'kAllScalarsAndVectors', () => kAllScalarsAndVectors);
__get(__exports, 'kAllVecTypes', () => kAllVecTypes);
__get(__exports, 'kAllMatrices', () => kAllMatrices);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let Colors; __bind('common/util/colors.js', (__m) => { ({ Colors } = __m); });let objectsToRecord; __bind('common/util/data_tables.js', (__m) => { ({ objectsToRecord } = __m); });
let assert, objectEquals, unreachable; __bind('common/util/util.js', (__m) => { ({ assert, objectEquals, unreachable } = __m); });
let Float16Array; __bind('external/petamoriken/float16/float16.js', (__m) => { ({ Float16Array } = __m); });


let kBit; __bind('webgpu/util/constants.js', (__m) => { ({ kBit } = __m); });
let align, cartesianProduct, clamp, correctlyRoundedF16, isFiniteF16, isSubnormalNumberF16, isSubnormalNumberF32, isSubnormalNumberF64; __bind('webgpu/util/math.js', (__m) => { ({ align, cartesianProduct, clamp, correctlyRoundedF16, isFiniteF16, isSubnormalNumberF16, isSubnormalNumberF32, isSubnormalNumberF64 } = __m); });

/**
 * Encode a JS `number` into the "normalized" (unorm/snorm) integer scale with `bits` bits but
 * remain unquantized. Input must be between -1 and 1 if signed, or 0 and 1 if unsigned.
 * e.g. float 0.5 -> "unorm8" 127.5
 *
 * MAINTENANCE_TODO: See if performance of texel_data improves if this function is pre-specialized
 * for a particular `bits`/`signed`.
 */
function floatAsNormalizedIntegerUnquantized(
float,
bits,
signed)
{
  if (signed) {
    assert(float >= -1 && float <= 1, () => `${float} out of bounds of snorm`);
    const max = Math.pow(2, bits - 1) - 1;
    return float * max;
  } else {
    assert(float >= 0 && float <= 1, () => `${float} out of bounds of unorm`);
    const max = Math.pow(2, bits) - 1;
    return float * max;
  }
}

/**
 * Encodes a JS `number` into a "normalized" (unorm/snorm) integer representation with `bits` bits.
 * Input must be between -1 and 1 if signed, or 0 and 1 if unsigned.
 *
 * MAINTENANCE_TODO: See if performance of texel_data improves if this function is pre-specialized
 * for a particular `bits`/`signed`.
 */
function floatAsNormalizedInteger(float, bits, signed) {
  return Math.round(floatAsNormalizedIntegerUnquantized(float, bits, signed));
}

/**
 * Decodes a JS `number` from a "normalized" (unorm/snorm) integer representation with `bits` bits.
 * Input must be an integer in the range of the specified unorm/snorm type.
 */
function normalizedIntegerAsFloat(integer, bits, signed) {
  assert(Number.isInteger(integer));
  if (signed) {
    const max = Math.pow(2, bits - 1) - 1;
    assert(integer >= -max - 1 && integer <= max);
    if (integer === -max - 1) {
      integer = -max;
    }
    return integer / max;
  } else {
    const max = Math.pow(2, bits) - 1;
    assert(integer >= 0 && integer <= max);
    return integer / max;
  }
}

/**
 * Compares 2 numbers. Returns true if their absolute value is
 * less than or equal to maxDiff or if they are both NaN or the
 * same sign infinity.
 */
function numbersApproximatelyEqual(a, b, maxDiff = 0) {
  return (
    Number.isNaN(a) && Number.isNaN(b) ||
    a === Number.POSITIVE_INFINITY && b === Number.POSITIVE_INFINITY ||
    a === Number.NEGATIVE_INFINITY && b === Number.NEGATIVE_INFINITY ||
    Math.abs(a - b) <= maxDiff);

}

/**
 * Once-allocated ArrayBuffer/views to avoid overhead of allocation when converting between numeric formats
 *
 * workingData* is shared between multiple functions in this file, so to avoid re-entrancy problems, make sure in
 * functions that use it that they don't call themselves or other functions that use workingData*.
 */
const workingData = new ArrayBuffer(8);
const workingDataU32 = new Uint32Array(workingData);
const workingDataU16 = new Uint16Array(workingData);
const workingDataU8 = new Uint8Array(workingData);
const workingDataF32 = new Float32Array(workingData);
const workingDataF16 = new Float16Array(workingData);
const workingDataI16 = new Int16Array(workingData);
const workingDataI32 = new Int32Array(workingData);
const workingDataI8 = new Int8Array(workingData);
const workingDataF64 = new Float64Array(workingData);
const workingDataI64 = new BigInt64Array(workingData);
const workingDataU64 = new BigUint64Array(workingData);
const workingDataView = new DataView(workingData);

/**
 * Encodes a JS `number` into an IEEE754 floating point number with the specified number of
 * sign, exponent, mantissa bits, and exponent bias.
 * Returns the result as an integer-valued JS `number`.
 *
 * Does not handle clamping, overflow, or denormal inputs.
 * On underflow (result is subnormal), rounds to (signed) zero.
 *
 * MAINTENANCE_TODO: Replace usages of this with numberToFloatBits.
 */
function float32ToFloatBits(
n,
signBits,
exponentBits,
mantissaBits,
bias)
{
  assert(exponentBits <= 8);
  assert(mantissaBits <= 23);

  if (Number.isNaN(n)) {
    // NaN = all exponent bits true, 1 or more mantissa bits true
    return (1 << exponentBits) - 1 << mantissaBits | (1 << mantissaBits) - 1;
  }

  workingDataView.setFloat32(0, n, true);
  const bits = workingDataView.getUint32(0, true);
  // bits (32): seeeeeeeefffffffffffffffffffffff

  // 0 or 1
  const sign = bits >> 31 & signBits;

  if (n === 0) {
    if (sign === 1) {
      // Handle negative zero.
      return 1 << exponentBits + mantissaBits;
    }
    return 0;
  }

  if (signBits === 0) {
    assert(n >= 0);
  }

  if (!Number.isFinite(n)) {
    // Infinity = all exponent bits true, no mantissa bits true
    // plus the sign bit.
    return (
      (1 << exponentBits) - 1 << mantissaBits | (n < 0 ? 2 ** (exponentBits + mantissaBits) : 0));

  }

  const mantissaBitsToDiscard = 23 - mantissaBits;

  // >> to remove mantissa, & to remove sign, - 127 to remove bias.
  const exp = (bits >> 23 & 0xff) - 127;

  // Convert to the new biased exponent.
  const newBiasedExp = bias + exp;
  assert(newBiasedExp < 1 << exponentBits, () => `input number ${n} overflows target type`);

  if (newBiasedExp <= 0) {
    // Result is subnormal or zero. Round to (signed) zero.
    return sign << exponentBits + mantissaBits;
  } else {
    // Mask only the mantissa, and discard the lower bits.
    const newMantissa = (bits & 0x7fffff) >> mantissaBitsToDiscard;
    return sign << exponentBits + mantissaBits | newBiasedExp << mantissaBits | newMantissa;
  }
}

/**
 * Encodes a JS `number` into an IEEE754 16 bit floating point number.
 * Returns the result as an integer-valued JS `number`.
 *
 * Does not handle clamping, overflow, or denormal inputs.
 * On underflow (result is subnormal), rounds to (signed) zero.
 */
function float32ToFloat16Bits(n) {
  return float32ToFloatBits(n, 1, 5, 10, 15);
}

/**
 * Decodes an IEEE754 16 bit floating point number into a JS `number` and returns.
 */
function float16BitsToFloat32(float16Bits) {
  return floatBitsToNumber(float16Bits, kFloat16Format);
}



/** FloatFormat defining IEEE754 32-bit float. */
const kFloat32Format = { signed: 1, exponentBits: 8, mantissaBits: 23, bias: 127 };
/** FloatFormat defining IEEE754 16-bit float. */
const kFloat16Format = { signed: 1, exponentBits: 5, mantissaBits: 10, bias: 15 };
/** FloatFormat for 9 bit mantissa, 5 bit exponent unsigned float */
const kUFloat9e5Format = { signed: 0, exponentBits: 5, mantissaBits: 9, bias: 15 };

/** Bitcast u32 (represented as integer Number) to f32 (represented as floating-point Number). */
function float32BitsToNumber(bits) {
  workingDataU32[0] = bits;
  return workingDataF32[0];
}
/** Bitcast f32 (represented as floating-point Number) to u32 (represented as integer Number). */
function numberToFloat32Bits(number) {
  workingDataF32[0] = number;
  return workingDataU32[0];
}

/**
 * Decodes an IEEE754 float with the supplied format specification into a JS number.
 *
 * The format MUST be no larger than a 32-bit float.
 */
function floatBitsToNumber(bits, fmt) {
  // Pad the provided bits out to f32, then convert to a `number` with the wrong bias.
  // E.g. for f16 to f32:
  // - f16: S    EEEEE MMMMMMMMMM
  //        ^ 000^^^^^ ^^^^^^^^^^0000000000000
  // - f32: S eeeEEEEE MMMMMMMMMMmmmmmmmmmmmmm

  const kNonSignBits = fmt.exponentBits + fmt.mantissaBits;
  const kNonSignBitsMask = (1 << kNonSignBits) - 1;
  const exponentAndMantissaBits = bits & kNonSignBitsMask;
  const exponentMask = (1 << fmt.exponentBits) - 1 << fmt.mantissaBits;
  const infinityOrNaN = (bits & exponentMask) === exponentMask;
  if (infinityOrNaN) {
    const mantissaMask = (1 << fmt.mantissaBits) - 1;
    const signBit = 2 ** kNonSignBits;
    const isNegative = (bits & signBit) !== 0;
    return bits & mantissaMask ?
    Number.NaN :
    isNegative ?
    Number.NEGATIVE_INFINITY :
    Number.POSITIVE_INFINITY;
  }
  let f32BitsWithWrongBias =
  exponentAndMantissaBits << kFloat32Format.mantissaBits - fmt.mantissaBits;
  f32BitsWithWrongBias |= bits << 31 - kNonSignBits & 0x8000_0000;
  const numberWithWrongBias = float32BitsToNumber(f32BitsWithWrongBias);
  return numberWithWrongBias * 2 ** (kFloat32Format.bias - fmt.bias);
}

/**
 * Convert ufloat9e5 bits from rgb9e5ufloat to a JS number
 *
 * The difference between `floatBitsToNumber` and `ufloatBitsToNumber`
 * is that the latter doesn't use an implicit leading bit:
 *
 * floatBitsToNumber      = 2^(exponent - bias) * (1 + mantissa / 2 ^ numMantissaBits)
 * ufloatM9E5BitsToNumber = 2^(exponent - bias) * (mantissa / 2 ^ numMantissaBits)
 *                        = 2^(exponent - bias - numMantissaBits) * mantissa
 */
function ufloatM9E5BitsToNumber(bits, fmt) {
  const exponent = bits >> fmt.mantissaBits;
  const mantissaMask = (1 << fmt.mantissaBits) - 1;
  const mantissa = bits & mantissaMask;
  return mantissa * 2 ** (exponent - fmt.bias - fmt.mantissaBits);
}

/**
 * Encodes a JS `number` into an IEEE754 floating point number with the specified format.
 * Returns the result as an integer-valued JS `number`.
 *
 * Does not handle clamping, overflow, or denormal inputs.
 * On underflow (result is subnormal), rounds to (signed) zero.
 */
function numberToFloatBits(number, fmt) {
  return float32ToFloatBits(number, fmt.signed, fmt.exponentBits, fmt.mantissaBits, fmt.bias);
}

/**
 * Given a floating point number (as an integer representing its bits), computes how many ULPs it is
 * from zero.
 *
 * Subnormal numbers are skipped, so that 0 is one ULP from the minimum normal number.
 * Subnormal values are flushed to 0.
 * Positive and negative 0 are both considered to be 0 ULPs from 0.
 */
function floatBitsToNormalULPFromZero(bits, fmt) {
  const mask_sign = fmt.signed << fmt.exponentBits + fmt.mantissaBits;
  const mask_expt = (1 << fmt.exponentBits) - 1 << fmt.mantissaBits;
  const mask_mant = (1 << fmt.mantissaBits) - 1;
  const mask_rest = mask_expt | mask_mant;

  assert(fmt.exponentBits + fmt.mantissaBits <= 31);

  const sign = bits & mask_sign ? -1 : 1;
  const rest = bits & mask_rest;
  const subnormal_or_zero = (bits & mask_expt) === 0;
  const infinity_or_nan = (bits & mask_expt) === mask_expt;
  assert(!infinity_or_nan, 'no ulp representation for infinity/nan');

  // The first normal number is mask_mant+1, so subtract mask_mant to make min_normal - zero = 1ULP.
  const abs_ulp_from_zero = subnormal_or_zero ? 0 : rest - mask_mant;
  return sign * abs_ulp_from_zero;
}

/**
 * Encodes three JS `number` values into RGB9E5, returned as an integer-valued JS `number`.
 *
 * RGB9E5 represents three partial-precision floating-point numbers encoded into a single 32-bit
 * value all sharing the same 5-bit exponent.
 * There is no sign bit, and there is a shared 5-bit biased (15) exponent and a 9-bit
 * mantissa for each channel. The mantissa does NOT have an implicit leading "1.",
 * and instead has an implicit leading "0.".
 *
 * @see https://registry.khronos.org/OpenGL/extensions/EXT/EXT_texture_shared_exponent.txt
 */
function packRGB9E5UFloat(r, g, b) {
  const N = 9; // number of mantissa bits
  const Emax = 31; // max exponent
  const B = 15; // exponent bias
  const sharedexp_max = ((1 << N) - 1) / (1 << N) * 2 ** (Emax - B);
  const red_c = clamp(r, { min: 0, max: sharedexp_max });
  const green_c = clamp(g, { min: 0, max: sharedexp_max });
  const blue_c = clamp(b, { min: 0, max: sharedexp_max });
  const max_c = Math.max(red_c, green_c, blue_c);
  const exp_shared_p = Math.max(-B - 1, Math.floor(Math.log2(max_c))) + 1 + B;
  const max_s = Math.floor(max_c / 2 ** (exp_shared_p - B - N) + 0.5);
  const exp_shared = max_s === 1 << N ? exp_shared_p + 1 : exp_shared_p;
  const scalar = 1 / 2 ** (exp_shared - B - N);
  const red_s = Math.floor(red_c * scalar + 0.5);
  const green_s = Math.floor(green_c * scalar + 0.5);
  const blue_s = Math.floor(blue_c * scalar + 0.5);
  assert(red_s >= 0 && red_s <= 0b111111111);
  assert(green_s >= 0 && green_s <= 0b111111111);
  assert(blue_s >= 0 && blue_s <= 0b111111111);
  assert(exp_shared >= 0 && exp_shared <= 0b11111);
  return (exp_shared << 27 | blue_s << 18 | green_s << 9 | red_s) >>> 0;
}

/**
 * Decodes a RGB9E5 encoded color.
 * @see packRGB9E5UFloat
 */
function unpackRGB9E5UFloat(encoded) {
  const N = 9; // number of mantissa bits
  const B = 15; // exponent bias
  const red_s = encoded >>> 0 & 0b111111111;
  const green_s = encoded >>> 9 & 0b111111111;
  const blue_s = encoded >>> 18 & 0b111111111;
  const exp_shared = encoded >>> 27 & 0b11111;
  const exp = Math.pow(2, exp_shared - B - N);
  return {
    R: exp * red_s,
    G: exp * green_s,
    B: exp * blue_s
  };
}

/**
 * Quantizes two f32s to f16 and then packs them in a u32
 *
 * This should implement the same behaviour as the builtin `pack2x16float` from
 * WGSL.
 *
 * Caller is responsible to ensuring inputs are f32s
 *
 * @param x first f32 to be packed
 * @param y second f32 to be packed
 * @returns an array of possible results for pack2x16float. Elements are either
 *          a number or undefined.
 *          undefined indicates that any value is valid, since the input went
 *          out of bounds.
 */
function pack2x16float(x, y) {
  // Generates all possible valid u16 bit fields for a given f32 to f16 conversion.
  // Assumes FTZ for both the f32 and f16 value is allowed.
  const generateU16s = (n) => {
    let contains_subnormals = isSubnormalNumberF32(n);
    const n_f16s = correctlyRoundedF16(n);
    contains_subnormals ||= n_f16s.some(isSubnormalNumberF16);

    const n_u16s = n_f16s.map((f16) => {
      workingDataF16[0] = f16;
      return workingDataU16[0];
    });

    const contains_poszero = n_u16s.some((u) => u === kBit.f16.positive.zero);
    const contains_negzero = n_u16s.some((u) => u === kBit.f16.negative.zero);
    if (!contains_negzero && (contains_poszero || contains_subnormals)) {
      n_u16s.push(kBit.f16.negative.zero);
    }

    if (!contains_poszero && (contains_negzero || contains_subnormals)) {
      n_u16s.push(kBit.f16.positive.zero);
    }

    return n_u16s;
  };

  if (!isFiniteF16(x) || !isFiniteF16(y)) {
    // This indicates any value is valid, so it isn't worth bothering
    // calculating the more restrictive possibilities.
    return [undefined];
  }

  const results = new Array();
  for (const p of cartesianProduct(generateU16s(x), generateU16s(y))) {
    assert(p.length === 2, 'cartesianProduct of 2 arrays returned an entry with not 2 elements');
    workingDataU16[0] = p[0];
    workingDataU16[1] = p[1];
    results.push(workingDataU32[0]);
  }

  return results;
}

/**
 * Converts two normalized f32s to i16s and then packs them in a u32
 *
 * This should implement the same behaviour as the builtin `pack2x16snorm` from
 * WGSL.
 *
 * Caller is responsible to ensuring inputs are normalized f32s
 *
 * @param x first f32 to be packed
 * @param y second f32 to be packed
 * @returns a number that is expected result of pack2x16snorm.
 */
function pack2x16snorm(x, y) {
  // Converts f32 to i16 via the pack2x16snorm formula.
  // FTZ is not explicitly handled, because all subnormals will produce a value
  // between 0 and 1, but significantly away from the edges, so floor goes to 0.
  const generateI16 = (n) => {
    return Math.floor(0.5 + 32767 * Math.min(1, Math.max(-1, n)));
  };

  workingDataI16[0] = generateI16(x);
  workingDataI16[1] = generateI16(y);

  return workingDataU32[0];
}

/**
 * Converts two normalized f32s to u16s and then packs them in a u32
 *
 * This should implement the same behaviour as the builtin `pack2x16unorm` from
 * WGSL.
 *
 * Caller is responsible to ensuring inputs are normalized f32s
 *
 * @param x first f32 to be packed
 * @param y second f32 to be packed
 * @returns an number that is expected result of pack2x16unorm.
 */
function pack2x16unorm(x, y) {
  // Converts f32 to u16 via the pack2x16unorm formula.
  // FTZ is not explicitly handled, because all subnormals will produce a value
  // between 0.5 and much less than 1, so floor goes to 0.
  const generateU16 = (n) => {
    return Math.floor(0.5 + 65535 * Math.min(1, Math.max(0, n)));
  };

  workingDataU16[0] = generateU16(x);
  workingDataU16[1] = generateU16(y);

  return workingDataU32[0];
}

/**
 * Converts four normalized f32s to i8s and then packs them in a u32
 *
 * This should implement the same behaviour as the builtin `pack4x8snorm` from
 * WGSL.
 *
 * Caller is responsible to ensuring inputs are normalized f32s
 *
 * @param vals four f32s to be packed
 * @returns a number that is expected result of pack4x8usorm.
 */
function pack4x8snorm(...vals) {
  // Converts f32 to u8 via the pack4x8snorm formula.
  // FTZ is not explicitly handled, because all subnormals will produce a value
  // between 0 and 1, so floor goes to 0.
  const generateI8 = (n) => {
    return Math.floor(0.5 + 127 * Math.min(1, Math.max(-1, n)));
  };

  for (const idx in vals) {
    workingDataI8[idx] = generateI8(vals[idx]);
  }

  return workingDataU32[0];
}

/**
 * Converts four normalized f32s to u8s and then packs them in a u32
 *
 * This should implement the same behaviour as the builtin `pack4x8unorm` from
 * WGSL.
 *
 * Caller is responsible to ensuring inputs are normalized f32s
 *
 * @param vals four f32s to be packed
 * @returns a number that is expected result of pack4x8unorm.
 */
function pack4x8unorm(...vals) {
  // Converts f32 to u8 via the pack4x8unorm formula.
  // FTZ is not explicitly handled, because all subnormals will produce a value
  // between 0.5 and much less than 1, so floor goes to 0.
  const generateU8 = (n) => {
    return Math.floor(0.5 + 255 * Math.min(1, Math.max(0, n)));
  };

  for (const idx in vals) {
    workingDataU8[idx] = generateU8(vals[idx]);
  }

  return workingDataU32[0];
}

/**
 * Asserts that a number is within the representable (inclusive) of the integer type with the
 * specified number of bits and signedness.
 *
 * MAINTENANCE_TODO: Assert isInteger? Then this function "asserts that a number is representable"
 * by the type.
 */
function assertInIntegerRange(n, bits, signed) {
  if (signed) {
    const min = -Math.pow(2, bits - 1);
    const max = Math.pow(2, bits - 1) - 1;
    assert(n >= min && n <= max);
  } else {
    const max = Math.pow(2, bits) - 1;
    assert(n >= 0 && n <= max);
  }
}

/**
 * Converts a linear value into a "gamma"-encoded value using the sRGB-clamped transfer function.
 */
function gammaCompress(n) {
  n = n <= 0.0031308 ? 323 * n / 25 : (211 * Math.pow(n, 5 / 12) - 11) / 200;
  return clamp(n, { min: 0, max: 1 });
}

/**
 * Converts a "gamma"-encoded value into a linear value using the sRGB-clamped transfer function.
 */
function gammaDecompress(n) {
  n = n <= 0.04045 ? n * 25 / 323 : Math.pow((200 * n + 11) / 211, 12 / 5);
  return clamp(n, { min: 0, max: 1 });
}

/** Converts a 32-bit float value to a 32-bit unsigned integer value */
function float32ToUint32(f32) {
  workingDataF32[0] = f32;
  return workingDataU32[0];
}

/** Converts a 32-bit unsigned integer value to a 32-bit float value */
function uint32ToFloat32(u32) {
  workingDataU32[0] = u32;
  return workingDataF32[0];
}

/** Converts a 32-bit float value to a 32-bit signed integer value */
function float32ToInt32(f32) {
  workingDataF32[0] = f32;
  return workingDataI32[0];
}

/** Converts a 32-bit unsigned integer value to a 32-bit signed integer value */
function uint32ToInt32(u32) {
  workingDataU32[0] = u32;
  return workingDataI32[0];
}

/** Converts a 16-bit float value to a 16-bit unsigned integer value */
function float16ToUint16(f16) {
  workingDataF16[0] = f16;
  return workingDataU16[0];
}

/** Converts a 16-bit unsigned integer value to a 16-bit float value */
function uint16ToFloat16(u16) {
  workingDataU16[0] = u16;
  return workingDataF16[0];
}

/** Converts a 16-bit float value to a 16-bit signed integer value */
function float16ToInt16(f16) {
  workingDataF16[0] = f16;
  return workingDataI16[0];
}

/** A type of number representable by Scalar. */














/** ScalarType describes the type of WGSL Scalar. */
class ScalarType {
  // The named type
  // In bytes

  // reads a scalar from a buffer

  constructor(
  kind,
  size,
  signed,
  read)
  {
    this.kind = kind;
    this._size = size;
    this._signed = signed;
    this.read = read;
  }

  toString() {
    return this.kind;
  }

  get size() {
    return this._size;
  }

  get alignment() {
    return this._size;
  }

  get signed() {
    return this._signed;
  }

  // This allows width to be checked in cases where scalar and vector types are mixed.
  get width() {
    return 1;
  }

  requiresF16() {
    return this.kind === 'f16';
  }

  /** Constructs a ScalarValue of this type with `value` */
  create(value) {
    switch (typeof value) {
      case 'number':
        switch (this.kind) {
          case 'abstract-float':
            return abstractFloat(value);
          case 'abstract-int':
            return abstractInt(BigInt(value));
          case 'f64':
            return f64(value);
          case 'f32':
            return f32(value);
          case 'f16':
            return f16(value);
          case 'u32':
            return u32(value);
          case 'u16':
            return u16(value);
          case 'u8':
            return u8(value);
          case 'i32':
            return i32(value);
          case 'i16':
            return i16(value);
          case 'i8':
            return i8(value);
          case 'bool':
            return bool(value !== 0);
        }
        break;
      case 'bigint':
        switch (this.kind) {
          case 'abstract-int':
            return abstractInt(value);
          case 'bool':
            return bool(value !== 0n);
        }
        break;
    }
    unreachable(`Scalar<${this.kind}>.create() does not support ${typeof value}`);
  }
}

/** VectorType describes the type of WGSL Vector. */
class VectorType {
  // Number of elements in the vector
  // Element type

  // Maps a string representation of a vector type to vector type.
  static instances = new Map();

  static create(width, elementType) {
    const key = `${elementType.toString()} ${width}}`;
    let ty = this.instances.get(key);
    if (ty !== undefined) {
      return ty;
    }
    ty = new VectorType(width, elementType);
    this.instances.set(key, ty);
    return ty;
  }

  constructor(width, elementType) {
    this.width = width;
    this.elementType = elementType;
  }

  /**
   * @returns a vector constructed from the values read from the buffer at the
   * given byte offset
   */
  read(buf, offset) {
    const elements = [];
    for (let i = 0; i < this.width; i++) {
      elements[i] = this.elementType.read(buf, offset);
      offset += this.elementType.size;
    }
    return new VectorValue(elements);
  }

  toString() {
    return `vec${this.width}<${this.elementType}>`;
  }

  get size() {
    return this.elementType.size * this.width;
  }

  get alignment() {
    return VectorType.alignmentOf(this.width, this.elementType);
  }

  static alignmentOf(width, elementType) {
    return elementType.size * (width === 3 ? 4 : width);
  }

  /** Constructs a Vector of this type with the given values */
  create(value) {
    if (value instanceof Array) {
      assert(value.length === this.width);
    } else {
      value = Array(this.width).fill(value);
    }
    return new VectorValue(value.map((v) => this.elementType.create(v)));
  }

  requiresF16() {
    return this.elementType.requiresF16();
  }
}

/** MatrixType describes the type of WGSL Matrix. */
class MatrixType {
  // Number of columns in the Matrix
  // Number of elements per column in the Matrix
  // Element type

  // Maps a string representation of a Matrix type to Matrix type.
  static instances = new Map();

  static create(cols, rows, elementType) {
    const key = `${elementType.toString()} ${cols} ${rows}`;
    let ty = this.instances.get(key);
    if (ty !== undefined) {
      return ty;
    }
    ty = new MatrixType(cols, rows, elementType);
    this.instances.set(key, ty);
    return ty;
  }

  constructor(cols, rows, elementType) {
    this.cols = cols;
    this.rows = rows;
    assert(
      elementType.kind === 'f32' ||
      elementType.kind === 'f16' ||
      elementType.kind === 'abstract-float',
      "MatrixType can only have elementType of 'f32' or 'f16' or 'abstract-float'"
    );
    this.elementType = elementType;
  }

  /**
   * @returns a Matrix constructed from the values read from the buffer at the
   * given byte offset
   */
  read(buf, offset) {
    const elements = [...Array(this.cols)].map((_) => [...Array(this.rows)]);
    for (let c = 0; c < this.cols; c++) {
      for (let r = 0; r < this.rows; r++) {
        elements[c][r] = this.elementType.read(buf, offset);
        offset += this.elementType.size;
      }

      // vec3 have one padding element, so need to skip in matrices
      if (this.rows === 3) {
        offset += this.elementType.size;
      }
    }
    return new MatrixValue(elements);
  }

  toString() {
    return `mat${this.cols}x${this.rows}<${this.elementType}>`;
  }

  get size() {
    return VectorType.alignmentOf(this.rows, this.elementType) * this.cols;
  }

  get alignment() {
    return VectorType.alignmentOf(this.rows, this.elementType);
  }

  requiresF16() {
    return this.elementType.requiresF16();
  }

  /** Constructs a Matrix of this type with the given values */
  create(value) {
    if (value instanceof Array) {
      assert(value.length === this.cols * this.rows);
    } else {
      value = Array(this.cols * this.rows).fill(value);
    }
    const columns = [];
    for (let i = 0; i < this.cols; i++) {
      const start = i * this.rows;
      columns.push(value.slice(start, start + this.rows));
    }
    return new MatrixValue(columns.map((c) => c.map((v) => this.elementType.create(v))));
  }
}

/** ArrayType describes the type of WGSL Array. */
class ArrayType {
  // Number of elements in the array. Zero represents a runtime-sized array.
  // Element type

  // Maps a string representation of a array type to array type.
  static instances = new Map();

  static create(count, elementType) {
    const key = `${elementType.toString()} ${count}`;
    let ty = this.instances.get(key);
    if (ty !== undefined) {
      return ty;
    }
    ty = new ArrayType(count, elementType);
    this.instances.set(key, ty);
    return ty;
  }

  constructor(count, elementType) {
    this.count = count;
    this.elementType = elementType;
  }

  /**
   * @returns a array constructed from the values read from the buffer at the
   * given byte offset
   */
  read(buf, offset) {
    const elements = [];

    for (let i = 0; i < this.count; i++) {
      elements[i] = this.elementType.read(buf, offset);
      offset += this.stride;
    }
    return new ArrayValue(elements);
  }

  toString() {
    return this.count !== 0 ?
    `array<${this.elementType}, ${this.count}>` :
    `array<${this.elementType}>`;
  }

  get stride() {
    return align(this.elementType.size, this.elementType.alignment);
  }

  get size() {
    return this.stride * this.count;
  }

  get alignment() {
    return this.elementType.alignment;
  }

  requiresF16() {
    return this.elementType.requiresF16();
  }

  /** Constructs an Array of this type with the given values */
  create(value) {
    if (value instanceof Array) {
      assert(value.length === this.count);
    } else {
      value = Array(this.count).fill(value);
    }
    return new ArrayValue(value.map((v) => this.elementType.create(v)));
  }
}

/** ArrayElementType infers the element type of the indexable type A */


/** Copy bytes from `buf` at `offset` into the working data, then read it out using `workingDataOut` */
function valueFromBytes(
workingDataOut,
buf,
offset)
{
  for (let i = 0; i < workingDataOut.BYTES_PER_ELEMENT; ++i) {
    workingDataU8[i] = buf[offset + i];
  }
  return workingDataOut[0];
}

const abstractIntType = new ScalarType('abstract-int', 8, true, (buf, offset) =>
abstractInt(valueFromBytes(workingDataI64, buf, offset))
);
const i32Type = new ScalarType('i32', 4, true, (buf, offset) =>
i32(valueFromBytes(workingDataI32, buf, offset))
);
const u32Type = new ScalarType('u32', 4, false, (buf, offset) =>
u32(valueFromBytes(workingDataU32, buf, offset))
);
const i16Type = new ScalarType('i16', 2, true, (buf, offset) =>
i16(valueFromBytes(workingDataI16, buf, offset))
);
const u16Type = new ScalarType('u16', 2, false, (buf, offset) =>
u16(valueFromBytes(workingDataU16, buf, offset))
);
const i8Type = new ScalarType('i8', 1, true, (buf, offset) =>
i8(valueFromBytes(workingDataI8, buf, offset))
);
const u8Type = new ScalarType('u8', 1, false, (buf, offset) =>
u8(valueFromBytes(workingDataU8, buf, offset))
);
const abstractFloatType = new ScalarType(
  'abstract-float',
  8,
  true,
  (buf, offset) => abstractFloat(valueFromBytes(workingDataF64, buf, offset))
);
const f64Type = new ScalarType('f64', 8, true, (buf, offset) =>
f64(valueFromBytes(workingDataF64, buf, offset))
);
const f32Type = new ScalarType('f32', 4, true, (buf, offset) =>
f32(valueFromBytes(workingDataF32, buf, offset))
);
const f16Type = new ScalarType('f16', 2, true, (buf, offset) =>
f16Bits(valueFromBytes(workingDataU16, buf, offset))
);
const boolType = new ScalarType('bool', 4, false, (buf, offset) =>
bool(valueFromBytes(workingDataU32, buf, offset) !== 0)
);

/** Type is a ScalarType, VectorType, MatrixType or ArrayType. */


const kVecTypes = {
  vec2ai: VectorType.create(2, abstractIntType),
  vec2i: VectorType.create(2, i32Type),
  vec2u: VectorType.create(2, u32Type),
  vec2af: VectorType.create(2, abstractFloatType),
  vec2f: VectorType.create(2, f32Type),
  vec2h: VectorType.create(2, f16Type),
  vec2b: VectorType.create(2, boolType),
  vec3ai: VectorType.create(3, abstractIntType),
  vec3i: VectorType.create(3, i32Type),
  vec3u: VectorType.create(3, u32Type),
  vec3af: VectorType.create(3, abstractFloatType),
  vec3f: VectorType.create(3, f32Type),
  vec3h: VectorType.create(3, f16Type),
  vec3b: VectorType.create(3, boolType),
  vec4ai: VectorType.create(4, abstractIntType),
  vec4i: VectorType.create(4, i32Type),
  vec4u: VectorType.create(4, u32Type),
  vec4af: VectorType.create(4, abstractFloatType),
  vec4f: VectorType.create(4, f32Type),
  vec4h: VectorType.create(4, f16Type),
  vec4b: VectorType.create(4, boolType)
};

const kMatTypes = {
  mat2x2f: MatrixType.create(2, 2, f32Type),
  mat2x2h: MatrixType.create(2, 2, f16Type),
  mat3x2f: MatrixType.create(3, 2, f32Type),
  mat3x2h: MatrixType.create(3, 2, f16Type),
  mat4x2f: MatrixType.create(4, 2, f32Type),
  mat4x2h: MatrixType.create(4, 2, f16Type),
  mat2x3f: MatrixType.create(2, 3, f32Type),
  mat2x3h: MatrixType.create(2, 3, f16Type),
  mat3x3f: MatrixType.create(3, 3, f32Type),
  mat3x3h: MatrixType.create(3, 3, f16Type),
  mat4x3f: MatrixType.create(4, 3, f32Type),
  mat4x3h: MatrixType.create(4, 3, f16Type),
  mat2x4f: MatrixType.create(2, 4, f32Type),
  mat2x4h: MatrixType.create(2, 4, f16Type),
  mat3x4f: MatrixType.create(3, 4, f32Type),
  mat3x4h: MatrixType.create(3, 4, f16Type),
  mat4x4f: MatrixType.create(4, 4, f32Type),
  mat4x4h: MatrixType.create(4, 4, f16Type)
};

/** Type holds pre-declared Types along with helper constructor functions. */
const Type = {
  abstractInt: abstractIntType,
  'abstract-int': abstractIntType,
  i32: i32Type,
  u32: u32Type,
  i16: i16Type,
  u16: u16Type,
  i8: i8Type,
  u8: u8Type,

  abstractFloat: abstractFloatType,
  'abstract-float': abstractFloatType,
  f64: f64Type,
  f32: f32Type,
  f16: f16Type,

  bool: boolType,

  vec: (width, elementType) => VectorType.create(width, elementType),

  // add vec types like vec2i, vec3f,
  ...kVecTypes,
  // add vec<> types like vec2<i32>, vec3<f32>
  ...objectsToRecord(Object.values(kVecTypes)),

  mat: (cols, rows, elementType) =>
  MatrixType.create(cols, rows, elementType),

  // add mat types like mat2x2f,
  ...kMatTypes,
  // add mat<> types like mat2x2<f32>,
  ...objectsToRecord(Object.values(kVecTypes)),

  array: (count, elementType) => ArrayType.create(count, elementType)
};

/**
 * @returns a type from a string
 * eg:
 *  'f32' -> Type.f32
 *  'vec4<f32>' -> Type.vec4f
 *  'vec3i' -> Type.vec3i
 */
function stringToType(s) {
  const t = Type[s];
  assert(!!t);
  return t;
}

/** @returns the ScalarType from the ScalarKind */
function scalarType(kind) {
  switch (kind) {
    case 'abstract-float':
      return Type.abstractFloat;
    case 'f64':
      return Type.f64;
    case 'f32':
      return Type.f32;
    case 'f16':
      return Type.f16;
    case 'u32':
      return Type.u32;
    case 'u16':
      return Type.u16;
    case 'u8':
      return Type.u8;
    case 'abstract-int':
      return Type.abstractInt;
    case 'i32':
      return Type.i32;
    case 'i16':
      return Type.i16;
    case 'i8':
      return Type.i8;
    case 'bool':
      return Type.bool;
  }
}

/** @returns the number of scalar (element) types of the given Type */
function numElementsOf(ty) {
  if (ty instanceof ScalarType) {
    return 1;
  }
  if (ty instanceof VectorType) {
    return ty.width;
  }
  if (ty instanceof MatrixType) {
    return ty.cols * ty.rows;
  }
  if (ty instanceof ArrayType) {
    return ty.count;
  }
  throw new Error(`unhandled type ${ty}`);
}

/** @returns the scalar elements of the given Value */
function elementsOf(value) {
  if (isScalarValue(value)) {
    return [value];
  }
  if (value instanceof VectorValue) {
    return value.elements;
  }
  if (value instanceof MatrixValue) {
    return value.elements.flat();
  }
  if (value instanceof ArrayValue) {
    return value.elements;
  }
  throw new Error(`unhandled value ${value}`);
}

/** @returns the scalar elements of the given Value */
function scalarElementsOf(value) {
  if (isScalarValue(value)) {
    return [value];
  }
  if (value instanceof VectorValue) {
    return value.elements;
  }
  if (value instanceof MatrixValue) {
    return value.elements.flat();
  }
  if (value instanceof ArrayValue) {
    return value.elements.map((els) => scalarElementsOf(els)).flat();
  }
  throw new Error(`unhandled value ${value}`);
}

/** @returns the inner element type of the given type */
function elementTypeOf(t) {
  if (t instanceof ScalarType) {
    return t;
  }
  return t.elementType;
}

/** @returns the scalar (element) type of the given Type */
function scalarTypeOf(ty) {
  if (ty instanceof ScalarType) {
    return ty;
  }
  if (ty instanceof VectorType) {
    return ty.elementType;
  }
  if (ty instanceof MatrixType) {
    return ty.elementType;
  }
  if (ty instanceof ArrayType) {
    return scalarTypeOf(ty.elementType);
  }
  throw new Error(`unhandled type ${ty}`);
}

/**
 * @returns the implicit concretized type of the given Type.
 * @param abstractIntToF32 if true, returns f32 for abstractInt else i32
 * Example: vec3<abstract-float> -> vec3<float>
 */
function concreteTypeOf(ty, allowedScalarTypes) {
  if (allowedScalarTypes && allowedScalarTypes.length > 0) {
    // https://www.w3.org/TR/WGSL/#conversion-rank
    switch (ty) {
      case Type.abstractInt:
        if (allowedScalarTypes.includes(Type.i32)) {
          return Type.i32;
        }
        if (allowedScalarTypes.includes(Type.u32)) {
          return Type.u32;
        }
      // fallthrough.
      case Type.abstractFloat:
        if (allowedScalarTypes.includes(Type.f32)) {
          return Type.f32;
        }
        if (allowedScalarTypes.includes(Type.f16)) {
          return Type.f16;
        }
        throw new Error(`no ${ty}`);
    }
  } else {
    switch (ty) {
      case Type.abstractInt:
        return Type.i32;
      case Type.abstractFloat:
        return Type.f32;
    }
  }
  if (ty instanceof ScalarType) {
    return ty;
  }
  if (ty instanceof VectorType) {
    return Type.vec(ty.width, concreteTypeOf(ty.elementType, allowedScalarTypes));
  }
  if (ty instanceof MatrixType) {
    return Type.mat(
      ty.cols,
      ty.rows,
      concreteTypeOf(ty.elementType, allowedScalarTypes)
    );
  }
  if (ty instanceof ArrayType) {
    return Type.array(ty.count, concreteTypeOf(ty.elementType, allowedScalarTypes));
  }
  throw new Error(`unhandled type ${ty}`);
}

function hex(sizeInBytes, bitsLow, bitsHigh) {
  let hex = '';
  workingDataU32[0] = bitsLow;
  if (bitsHigh !== undefined) {
    workingDataU32[1] = bitsHigh;
  }
  for (let i = 0; i < sizeInBytes; ++i) {
    hex = workingDataU8[i].toString(16).padStart(2, '0') + hex;
  }
  return `0x${hex}`;
}

function withPoint(x) {
  const str = `${x}`;
  return str.indexOf('.') > 0 || str.indexOf('e') > 0 ? str : `${str}.0`;
}

/** Class that encapsulates a single abstract-int value. */
class AbstractIntValue {
  // The abstract-integer value
  // The low 32 bits of the abstract-integer value.
  // The high 32 bits of the abstract-integer value.
  type = Type.abstractInt; // The type of the value.

  constructor(value, bitsLow, bitsHigh) {
    this.value = value;
    this.bitsLow = bitsLow;
    this.bitsHigh = bitsHigh;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU32[0] = this.bitsLow;
    workingDataU32[1] = this.bitsHigh;
    for (let i = 0; i < 8; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    // WGSL parses negative numbers as a negated positive.
    // This means '-9223372036854775808' parses as `-' & '9223372036854775808', so must be written as
    // '(-9223372036854775807 - 1)' in WGSL, because '9223372036854775808' is not a valid AbstractInt.
    if (this.value === -9223372036854775808n) {
      return `(-9223372036854775807 - 1)`;
    }
    return `${this.value}`;
  }

  toString() {
    return `${Colors.bold(this.value.toString())} (${hex(8, this.bitsLow, this.bitsHigh)})`;
  }
}

/** Class that encapsulates a single abstract-float value. */
class AbstractFloatValue {
  // The f32 value
  // The low 32 bits of the abstract-float value.
  // The high 32 bits of the abstract-float value.
  type = Type.abstractFloat; // The type of the value.

  constructor(value, bitsLow, bitsHigh) {
    this.value = value;
    this.bitsLow = bitsLow;
    this.bitsHigh = bitsHigh;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU32[0] = this.bitsLow;
    workingDataU32[1] = this.bitsHigh;
    for (let i = 0; i < 8; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    return `${withPoint(this.value)}`;
  }

  toString() {
    switch (this.value) {
      case Infinity:
      case -Infinity:
        return Colors.bold(this.value.toString());
      default:{
          let str = this.value.toString();
          str = str.indexOf('.') > 0 || str.indexOf('e') > 0 ? str : `${str}.0`;
          return isSubnormalNumberF64(this.value.valueOf()) ?
          `${Colors.bold(str)} (${hex(8, this.bitsLow, this.bitsHigh)} subnormal)` :
          `${Colors.bold(str)} (${hex(8, this.bitsLow, this.bitsHigh)})`;
        }
    }
  }
}

/** Class that encapsulates a single i32 value. */
class I32Value {
  // The i32 value
  // The i32 value, bitcast to a 32-bit integer.
  type = Type.i32; // The type of the value.

  constructor(value, bits) {
    this.value = value;
    this.bits = bits;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU32[0] = this.bits;
    for (let i = 0; i < 4; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    return `i32(${this.value})`;
  }

  toString() {
    return `${Colors.bold(this.value.toString())} (${hex(4, this.bits)})`;
  }
}

/** Class that encapsulates a single u32 value. */
class U32Value {
  // The u32 value
  type = Type.u32; // The type of the value.

  constructor(value) {
    this.value = value;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU32[0] = this.value;
    for (let i = 0; i < 4; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    return `${this.value}u`;
  }

  toString() {
    return `${Colors.bold(this.value.toString())} (${hex(4, this.value)})`;
  }
}

/**
 * Class that encapsulates a single i16 value.
 * @note type does not exist in WGSL yet
 */
class I16Value {
  // The i16 value
  // The i16 value, bitcast to a 16-bit integer.
  type = Type.i16; // The type of the value.

  constructor(value, bits) {
    this.value = value;
    this.bits = bits;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU16[0] = this.bits;
    for (let i = 0; i < 4; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    return `i16(${this.value})`;
  }

  toString() {
    return `${Colors.bold(this.value.toString())} (${hex(2, this.bits)})`;
  }
}

/**
 * Class that encapsulates a single u16 value.
 * @note type does not exist in WGSL yet
 */
class U16Value {
  // The u16 value
  type = Type.u16; // The type of the value.

  constructor(value) {
    this.value = value;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU16[0] = this.value;
    for (let i = 0; i < 2; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    assert(false, 'u16 is not a WGSL type');
    return `u16(${this.value})`;
  }

  toString() {
    return `${Colors.bold(this.value.toString())} (${hex(2, this.value)})`;
  }
}

/**
 * Class that encapsulates a single i8 value.
 * @note type does not exist in WGSL yet
 */
class I8Value {
  // The i8 value
  // The i8 value, bitcast to a 8-bit integer.
  type = Type.i8; // The type of the value.

  constructor(value, bits) {
    this.value = value;
    this.bits = bits;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU8[0] = this.bits;
    for (let i = 0; i < 4; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    return `i8(${this.value})`;
  }

  toString() {
    return `${Colors.bold(this.value.toString())} (${hex(2, this.bits)})`;
  }
}

/**
 * Class that encapsulates a single u8 value.
 * @note type does not exist in WGSL yet
 */
class U8Value {
  // The u8 value
  type = Type.u8; // The type of the value.

  constructor(value) {
    this.value = value;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU8[0] = this.value;
    for (let i = 0; i < 2; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    assert(false, 'u8 is not a WGSL type');
    return `u8(${this.value})`;
  }

  toString() {
    return `${Colors.bold(this.value.toString())} (${hex(2, this.value)})`;
  }
}

/**
 * Class that encapsulates a single f64 value
 * @note type does not exist in WGSL yet
 */
class F64Value {
  // The f32 value
  // The low 32 bits of the abstract-float value.
  // The high 32 bits of the abstract-float value.
  type = Type.f64; // The type of the value.

  constructor(value, bitsLow, bitsHigh) {
    this.value = value;
    this.bitsLow = bitsLow;
    this.bitsHigh = bitsHigh;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU32[0] = this.bitsLow;
    workingDataU32[1] = this.bitsHigh;
    for (let i = 0; i < 8; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    assert(false, 'f64 is not a WGSL type');
    return `${withPoint(this.value)}`;
  }

  toString() {
    switch (this.value) {
      case Infinity:
      case -Infinity:
        return Colors.bold(this.value.toString());
      default:{
          let str = this.value.toString();
          str = str.indexOf('.') > 0 || str.indexOf('e') > 0 ? str : `${str}.0`;
          return isSubnormalNumberF64(this.value.valueOf()) ?
          `${Colors.bold(str)} (${hex(8, this.bitsLow, this.bitsHigh)} subnormal)` :
          `${Colors.bold(str)} (${hex(8, this.bitsLow, this.bitsHigh)})`;
        }
    }
  }
}

/** Class that encapsulates a single f32 value. */
class F32Value {
  // The f32 value
  // The f32 value, bitcast to a 32-bit integer.
  type = Type.f32; // The type of the value.

  constructor(value, bits) {
    this.value = value;
    this.bits = bits;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU32[0] = this.bits;
    for (let i = 0; i < 4; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    return `${withPoint(this.value)}f`;
  }

  toString() {
    switch (this.value) {
      case Infinity:
      case -Infinity:
        return Colors.bold(this.value.toString());
      default:{
          let str = this.value.toString();
          str = str.indexOf('.') > 0 || str.indexOf('e') > 0 ? str : `${str}.0`;
          return isSubnormalNumberF32(this.value.valueOf()) ?
          `${Colors.bold(str)} (${hex(4, this.bits)} subnormal)` :
          `${Colors.bold(str)} (${hex(4, this.bits)})`;
        }
    }
  }
}

/** Class that encapsulates a single f16 value. */
class F16Value {
  // The f16 value
  // The f16 value, bitcast to a 16-bit integer.
  type = Type.f16; // The type of the value.

  constructor(value, bits) {
    this.value = value;
    this.bits = bits;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    workingDataU16[0] = this.bits;
    for (let i = 0; i < 2; i++) {
      buffer[offset + i] = workingDataU8[i];
    }
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    return `${withPoint(this.value)}h`;
  }

  toString() {
    switch (this.value) {
      case Infinity:
      case -Infinity:
        return Colors.bold(this.value.toString());
      default:{
          let str = this.value.toString();
          str = str.indexOf('.') > 0 || str.indexOf('e') > 0 ? str : `${str}.0`;
          return isSubnormalNumberF16(this.value.valueOf()) ?
          `${Colors.bold(str)} (${hex(2, this.bits)} subnormal)` :
          `${Colors.bold(str)} (${hex(2, this.bits)})`;
        }
    }
  }
}
/** Class that encapsulates a single bool value. */
class BoolValue {
  // The bool value
  type = Type.bool; // The type of the value.

  constructor(value) {
    this.value = value;
  }

  /**
   * Copies the scalar value to the buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the offset in buffer, in units of `buffer`
   */
  copyTo(buffer, offset) {
    buffer[offset] = this.value ? 1 : 0;
  }

  /** @returns the WGSL representation of this scalar value */
  wgsl() {
    return this.value.toString();
  }

  toString() {
    return Colors.bold(this.value.toString());
  }
}

/** Scalar represents all the scalar value types */


















function isScalarValue(value) {
  return (
    value instanceof AbstractIntValue ||
    value instanceof AbstractFloatValue ||
    value instanceof I32Value ||
    value instanceof U32Value ||
    value instanceof I16Value ||
    value instanceof U16Value ||
    value instanceof I8Value ||
    value instanceof U8Value ||
    value instanceof F64Value ||
    value instanceof F32Value ||
    value instanceof F16Value ||
    value instanceof BoolValue);

}

/** Create an AbstractInt from a numeric value, a JS `bigint`. */
function abstractInt(value) {
  workingDataI64[0] = value;
  return new AbstractIntValue(workingDataI64[0], workingDataU32[0], workingDataU32[1]);
}

/** Create an AbstractInt from a bit representation, a uint64 represented as a JS `bigint`. */
function abstractIntBits(value) {
  workingDataU64[0] = value;
  return new AbstractIntValue(workingDataI64[0], workingDataU32[0], workingDataU32[1]);
}

/** Create an AbstractFloat from a numeric value, a JS `number`. */
function abstractFloat(value) {
  workingDataF64[0] = value;
  return new AbstractFloatValue(workingDataF64[0], workingDataU32[0], workingDataU32[1]);
}

/** Create an i32 from a numeric value, a JS `number`. */
function i32(value) {
  workingDataI32[0] = value;
  return new I32Value(workingDataI32[0], workingDataU32[0]);
}

/** Create an i32 from a bit representation, a uint32 represented as a JS `number`. */
function i32Bits(bits) {
  workingDataU32[0] = bits;
  return new I32Value(workingDataI32[0], workingDataU32[0]);
}

/** Create a u32 from a numeric value, a JS `number`. */
function u32(value) {
  workingDataU32[0] = value;
  return new U32Value(workingDataU32[0]);
}

/** Create a u32 from a bit representation, a uint32 represented as a JS `number`. */
function u32Bits(bits) {
  workingDataU32[0] = bits;
  return new U32Value(workingDataU32[0]);
}

/** Create an i16 from a numeric value, a JS `number`. */
function i16(value) {
  workingDataI16[0] = value;
  return new I16Value(workingDataI16[0], workingDataU16[0]);
}

/** Create a u16 from a numeric value, a JS `number`. */
function u16(value) {
  workingDataU16[0] = value;
  return new U16Value(workingDataU16[0]);
}

/** Create an i8 from a numeric value, a JS `number`. */
function i8(value) {
  workingDataI8[0] = value;
  return new I8Value(workingDataI8[0], workingDataU8[0]);
}

/** Create a u8 from a numeric value, a JS `number`. */
function u8(value) {
  workingDataU8[0] = value;
  return new U8Value(workingDataU8[0]);
}

/** Create an f64 from a numeric value, a JS `number`. */
function f64(value) {
  workingDataF64[0] = value;
  return new F64Value(workingDataF64[0], workingDataU32[0], workingDataU32[1]);
}

/** Create an f32 from a numeric value, a JS `number`. */
function f32(value) {
  workingDataF32[0] = value;
  return new F32Value(workingDataF32[0], workingDataU32[0]);
}

/** Create an f32 from a bit representation, a uint32 represented as a JS `number`. */
function f32Bits(bits) {
  workingDataU32[0] = bits;
  return new F32Value(workingDataF32[0], workingDataU32[0]);
}

/** Create an f16 from a numeric value, a JS `number`. */
function f16(value) {
  workingDataF16[0] = value;
  return new F16Value(workingDataF16[0], workingDataU16[0]);
}

/** Create an f16 from a bit representation, a uint16 represented as a JS `number`. */
function f16Bits(bits) {
  workingDataU16[0] = bits;
  return new F16Value(workingDataF16[0], workingDataU16[0]);
}

/** Create a boolean value. */
function bool(value) {
  return new BoolValue(value);
}

/** A 'true' literal value */
const True = bool(true);

/** A 'false' literal value */
const False = bool(false);

/**
 * Class that encapsulates a vector value.
 */
class VectorValue {



  constructor(elements) {
    if (elements.length < 2 || elements.length > 4) {
      throw new Error(`vector element count must be between 2 and 4, got ${elements.length}`);
    }
    for (let i = 1; i < elements.length; i++) {
      const a = elements[0].type;
      const b = elements[i].type;
      if (a !== b) {
        throw new Error(
          `cannot mix vector element types. Found elements with types '${a}' and '${b}'`
        );
      }
    }
    this.elements = elements;
    this.type = VectorType.create(elements.length, elements[0].type);
  }

  /**
   * Copies the vector value to the Uint8Array buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the byte offset within buffer
   */
  copyTo(buffer, offset) {
    for (const element of this.elements) {
      element.copyTo(buffer, offset);
      offset += this.type.elementType.size;
    }
  }

  /**
   * @returns the WGSL representation of this vector value
   */
  wgsl() {
    const els = this.elements.map((v) => v.wgsl()).join(', ');
    return `vec${this.type.width}(${els})`;
  }

  toString() {
    return `${this.type}(${this.elements.map((e) => e.toString()).join(', ')})`;
  }

  get x() {
    assert(0 < this.elements.length);
    return this.elements[0];
  }

  get y() {
    assert(1 < this.elements.length);
    return this.elements[1];
  }

  get z() {
    assert(2 < this.elements.length);
    return this.elements[2];
  }

  get w() {
    assert(3 < this.elements.length);
    return this.elements[3];
  }
}

/** Helper for constructing a new vector with the provided values */
function vec(...elements) {
  return new VectorValue(elements);
}

/** Helper for constructing a new two-element vector with the provided values */
function vec2(x, y) {
  return new VectorValue([x, y]);
}

/** Helper for constructing a new three-element vector with the provided values */
function vec3(x, y, z) {
  return new VectorValue([x, y, z]);
}

/** Helper for constructing a new four-element vector with the provided values */
function vec4(x, y, z, w) {
  return new VectorValue([x, y, z, w]);
}

/**
 * Helper for constructing Vectors from arrays of numbers
 *
 * @param v array of numbers to be converted, must contain 2, 3 or 4 elements
 * @param op function to convert from number to Scalar, e.g. 'f32`
 */
function toVector(v, op) {
  switch (v.length) {
    case 2:
      return vec2(op(v[0]), op(v[1]));
    case 3:
      return vec3(op(v[0]), op(v[1]), op(v[2]));
    case 4:
      return vec4(op(v[0]), op(v[1]), op(v[2]), op(v[3]));
  }
  unreachable(`input to 'toVector' must contain 2, 3, or 4 elements`);
}

/**
 * Class that encapsulates a Matrix value.
 */
class MatrixValue {



  constructor(elements) {
    const num_cols = elements.length;
    if (num_cols < 2 || num_cols > 4) {
      throw new Error(`matrix cols count must be between 2 and 4, got ${num_cols}`);
    }

    const num_rows = elements[0].length;
    if (!elements.every((c) => c.length === num_rows)) {
      throw new Error(`cannot mix matrix column lengths`);
    }

    if (num_rows < 2 || num_rows > 4) {
      throw new Error(`matrix rows count must be between 2 and 4, got ${num_rows}`);
    }

    const elem_type = elements[0][0].type;
    if (!elements.every((c) => c.every((r) => objectEquals(r.type, elem_type)))) {
      throw new Error(`cannot mix matrix element types`);
    }

    this.elements = elements;
    this.type = MatrixType.create(num_cols, num_rows, elem_type);
  }

  /**
   * Copies the matrix value to the Uint8Array buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the byte offset within buffer
   */
  copyTo(buffer, offset) {
    for (let i = 0; i < this.type.cols; i++) {
      for (let j = 0; j < this.type.rows; j++) {
        this.elements[i][j].copyTo(buffer, offset);
        offset += this.type.elementType.size;
      }

      // vec3 have one padding element, so need to skip in matrices
      if (this.type.rows === 3) {
        offset += this.type.elementType.size;
      }
    }
  }

  /**
   * @returns the WGSL representation of this matrix value
   */
  wgsl() {
    const els = this.elements.flatMap((c) => c.map((r) => r.wgsl())).join(', ');
    return `mat${this.type.cols}x${this.type.rows}(${els})`;
  }

  toString() {
    return `${this.type}(${this.elements.map((c) => c.join(', ')).join(', ')})`;
  }
}

/**
 * Class that encapsulates an Array value.
 */
class ArrayValue {



  constructor(elements) {
    const elem_type = elements[0].type;
    if (!elements.every((c) => elements.every((r) => objectEquals(r.type, elem_type)))) {
      throw new Error(`cannot mix array element types`);
    }

    this.elements = elements;
    this.type = ArrayType.create(elements.length, elem_type);
  }

  /**
   * Copies the array value to the Uint8Array buffer at the provided byte offset.
   * @param buffer the destination buffer
   * @param offset the byte offset within buffer
   */
  copyTo(buffer, offset) {
    for (const element of this.elements) {
      element.copyTo(buffer, offset);
      offset += this.type.elementType.size;
    }
  }

  /**
   * @returns the WGSL representation of this array value
   */
  wgsl() {
    const els = this.elements.map((r) => r.wgsl()).join(', ');
    return isAbstractType(this.type.elementType) ? `array(${els})` : `${this.type}(${els})`;
  }

  toString() {
    return this.wgsl();
  }
}

/** Helper for constructing an ArrayValue with the provided values */
function array(...elements) {
  return new ArrayValue(elements);
}

/**
 * Helper for constructing Matrices from arrays of numbers
 *
 * @param m array of array of numbers to be converted, all Array of number must
 *          be of the same length. All Arrays must have 2, 3, or 4 elements.
 * @param op function to convert from number to Scalar, e.g. 'f32`
 */
function toMatrix(m, op) {
  const cols = m.length;
  const rows = m[0].length;
  const elements = [...Array(cols)].map((_) => [
  ...Array(rows)]
  );
  for (let i = 0; i < cols; i++) {
    for (let j = 0; j < rows; j++) {
      elements[i][j] = op(m[i][j]);
    }
  }

  return new MatrixValue(elements);
}

/** Value is a Scalar, Vector, Matrix or Array value. */var




















SerializedScalarKind = /*#__PURE__*/function (SerializedScalarKind) {SerializedScalarKind[SerializedScalarKind["AbstractFloat"] = 0] = "AbstractFloat";SerializedScalarKind[SerializedScalarKind["F64"] = 1] = "F64";SerializedScalarKind[SerializedScalarKind["F32"] = 2] = "F32";SerializedScalarKind[SerializedScalarKind["F16"] = 3] = "F16";SerializedScalarKind[SerializedScalarKind["U32"] = 4] = "U32";SerializedScalarKind[SerializedScalarKind["U16"] = 5] = "U16";SerializedScalarKind[SerializedScalarKind["U8"] = 6] = "U8";SerializedScalarKind[SerializedScalarKind["I32"] = 7] = "I32";SerializedScalarKind[SerializedScalarKind["I16"] = 8] = "I16";SerializedScalarKind[SerializedScalarKind["I8"] = 9] = "I8";SerializedScalarKind[SerializedScalarKind["Bool"] = 10] = "Bool";SerializedScalarKind[SerializedScalarKind["AbstractInt"] = 11] = "AbstractInt";return SerializedScalarKind;}(SerializedScalarKind || {});














/** serializeScalarKind() serializes a ScalarKind to a BinaryStream */
function serializeScalarKind(s, v) {
  switch (v) {
    case 'abstract-float':
      s.writeU8(SerializedScalarKind.AbstractFloat);
      return;
    case 'f64':
      s.writeU8(SerializedScalarKind.F64);
      return;
    case 'f32':
      s.writeU8(SerializedScalarKind.F32);
      return;
    case 'f16':
      s.writeU8(SerializedScalarKind.F16);
      return;
    case 'u32':
      s.writeU8(SerializedScalarKind.U32);
      return;
    case 'u16':
      s.writeU8(SerializedScalarKind.U16);
      return;
    case 'u8':
      s.writeU8(SerializedScalarKind.U8);
      return;
    case 'abstract-int':
      s.writeU8(SerializedScalarKind.AbstractInt);
      return;
    case 'i32':
      s.writeU8(SerializedScalarKind.I32);
      return;
    case 'i16':
      s.writeU8(SerializedScalarKind.I16);
      return;
    case 'i8':
      s.writeU8(SerializedScalarKind.I8);
      return;
    case 'bool':
      s.writeU8(SerializedScalarKind.Bool);
      return;
  }
  unreachable(`Do not know what to write scalar kind = ${v}`);
}

/** deserializeScalarKind() deserializes a ScalarKind from a BinaryStream */
function deserializeScalarKind(s) {
  const kind = s.readU8();
  switch (kind) {
    case SerializedScalarKind.AbstractFloat:
      return 'abstract-float';
    case SerializedScalarKind.F64:
      return 'f64';
    case SerializedScalarKind.F32:
      return 'f32';
    case SerializedScalarKind.F16:
      return 'f16';
    case SerializedScalarKind.U32:
      return 'u32';
    case SerializedScalarKind.U16:
      return 'u16';
    case SerializedScalarKind.U8:
      return 'u8';
    case SerializedScalarKind.AbstractInt:
      return 'abstract-int';
    case SerializedScalarKind.I32:
      return 'i32';
    case SerializedScalarKind.I16:
      return 'i16';
    case SerializedScalarKind.I8:
      return 'i8';
    case SerializedScalarKind.Bool:
      return 'bool';
    default:
      unreachable(`invalid serialized ScalarKind: ${kind}`);
  }
}var

SerializedValueKind = /*#__PURE__*/function (SerializedValueKind) {SerializedValueKind[SerializedValueKind["Scalar"] = 0] = "Scalar";SerializedValueKind[SerializedValueKind["Vector"] = 1] = "Vector";SerializedValueKind[SerializedValueKind["Matrix"] = 2] = "Matrix";return SerializedValueKind;}(SerializedValueKind || {});





/** serializeValue() serializes a Value to a BinaryStream */
function serializeValue(s, v) {
  const serializeScalar = (scalar, kind) => {
    switch (typeof scalar.value) {
      case 'number':
        switch (kind) {
          case 'abstract-float':
            s.writeF64(scalar.value);
            return;
          case 'f64':
            s.writeF64(scalar.value);
            return;
          case 'f32':
            s.writeF32(scalar.value);
            return;
          case 'f16':
            s.writeF16(scalar.value);
            return;
          case 'u32':
            s.writeU32(scalar.value);
            return;
          case 'u16':
            s.writeU16(scalar.value);
            return;
          case 'u8':
            s.writeU8(scalar.value);
            return;
          case 'i32':
            s.writeI32(scalar.value);
            return;
          case 'i16':
            s.writeI16(scalar.value);
            return;
          case 'i8':
            s.writeI8(scalar.value);
            return;
        }
        break;
      case 'bigint':
        switch (kind) {
          case 'abstract-int':
            s.writeI64(scalar.value);
            return;
        }
        break;
      case 'boolean':
        switch (kind) {
          case 'bool':
            s.writeBool(scalar.value);
            return;
        }
        break;
    }
  };

  if (isScalarValue(v)) {
    s.writeU8(SerializedValueKind.Scalar);
    serializeScalarKind(s, v.type.kind);
    serializeScalar(v, v.type.kind);
    return;
  }
  if (v instanceof VectorValue) {
    s.writeU8(SerializedValueKind.Vector);
    serializeScalarKind(s, v.type.elementType.kind);
    s.writeU8(v.type.width);
    for (const element of v.elements) {
      serializeScalar(element, v.type.elementType.kind);
    }
    return;
  }
  if (v instanceof MatrixValue) {
    s.writeU8(SerializedValueKind.Matrix);
    serializeScalarKind(s, v.type.elementType.kind);
    s.writeU8(v.type.cols);
    s.writeU8(v.type.rows);
    for (const column of v.elements) {
      for (const element of column) {
        serializeScalar(element, v.type.elementType.kind);
      }
    }
    return;
  }

  unreachable(`unhandled value type: ${v}`);
}

/** deserializeValue() deserializes a Value from a BinaryStream */
function deserializeValue(s) {
  const deserializeScalar = (kind) => {
    switch (kind) {
      case 'abstract-float':
        return abstractFloat(s.readF64());
      case 'f64':
        return f64(s.readF64());
      case 'f32':
        return f32(s.readF32());
      case 'f16':
        return f16(s.readF16());
      case 'u32':
        return u32(s.readU32());
      case 'u16':
        return u16(s.readU16());
      case 'u8':
        return u8(s.readU8());
      case 'abstract-int':
        return abstractInt(s.readI64());
      case 'i32':
        return i32(s.readI32());
      case 'i16':
        return i16(s.readI16());
      case 'i8':
        return i8(s.readI8());
      case 'bool':
        return bool(s.readBool());
    }
  };
  const valueKind = s.readU8();
  const scalarKind = deserializeScalarKind(s);
  switch (valueKind) {
    case SerializedValueKind.Scalar:
      return deserializeScalar(scalarKind);
    case SerializedValueKind.Vector:{
        const width = s.readU8();
        const scalars = new Array(width);
        for (let i = 0; i < width; i++) {
          scalars[i] = deserializeScalar(scalarKind);
        }
        return new VectorValue(scalars);
      }
    case SerializedValueKind.Matrix:{
        const numCols = s.readU8();
        const numRows = s.readU8();
        const columns = new Array(numCols);
        for (let c = 0; c < numCols; c++) {
          columns[c] = new Array(numRows);
          for (let i = 0; i < numRows; i++) {
            columns[c][i] = deserializeScalar(scalarKind);
          }
        }
        return new MatrixValue(columns);
      }
    default:
      unreachable(`invalid serialized value kind: ${valueKind}`);
  }
}

/** @returns if the Value is a float scalar type */
function isFloatValue(v) {
  return isFloatType(v.type);
}

/**
 * @returns if `ty` is an abstract numeric type.
 * @note this does not consider composite types.
 * Use elementType() if you want to test the element type.
 */
function isAbstractType(ty) {
  if (ty instanceof ScalarType) {
    return ty.kind === 'abstract-float' || ty.kind === 'abstract-int';
  }
  return false;
}

/**
 * @returns if `ty` is a floating point type.
 * @note this does not consider composite types.
 * Use elementType() if you want to test the element type.
 */
function isFloatType(ty) {
  if (ty instanceof ScalarType) {
    return (
      ty.kind === 'abstract-float' || ty.kind === 'f64' || ty.kind === 'f32' || ty.kind === 'f16');

  }
  return false;
}

/**
 * @returns if `ty` is an integer point type.
 * @note this does not consider composite types.
 * Use elementType() if you want to test the element type.
 */
function isIntegerType(ty) {
  if (ty instanceof ScalarType) {
    return (
      ty.kind === 'abstract-int' ||
      ty.kind === 'i32' ||
      ty.kind === 'i16' ||
      ty.kind === 'i8' ||
      ty.kind === 'u32' ||
      ty.kind === 'u16' ||
      ty.kind === 'u8');

  }
  return false;
}

/**
 * @returns if `ty` is a type convertible to floating point type.
 * @note this does not consider composite types.
 * Use elementType() if you want to test the element type.
 */
function isConvertibleToFloatType(ty) {
  if (ty instanceof ScalarType) {
    return (
      ty.kind === 'abstract-int' ||
      ty.kind === 'abstract-float' ||
      ty.kind === 'f64' ||
      ty.kind === 'f32' ||
      ty.kind === 'f16');

  }
  return false;
}

/**
 * @returns if `ty` is an unsigned type.
 */
function isUnsignedType(ty) {
  if (ty instanceof ScalarType) {
    return ty.kind === 'u8' || ty.kind === 'u16' || ty.kind === 'u32';
  } else {
    return isUnsignedType(ty.elementType);
  }
}

/** @returns true if an argument of type 'src' can be used for a parameter of type 'dst' */
function isConvertible(src, dst) {
  if (src === dst) {
    return true;
  }

  const shapeOf = (ty) => {
    if (ty instanceof ScalarType) {
      return `scalar`;
    }
    if (ty instanceof VectorType) {
      return `vec${ty.width}`;
    }
    if (ty instanceof MatrixType) {
      return `mat${ty.cols}x${ty.rows}`;
    }
    if (ty instanceof ArrayType) {
      return `array<${ty.count}>`;
    }
    unreachable(`unhandled type: ${ty}`);
  };

  if (shapeOf(src) !== shapeOf(dst)) {
    return false;
  }

  const elSrc = scalarTypeOf(src);
  const elDst = scalarTypeOf(dst);

  switch (elSrc.kind) {
    case 'abstract-float':
      switch (elDst.kind) {
        case 'abstract-float':
        case 'f16':
        case 'f32':
        case 'f64':
          return true;
        default:
          return false;
      }
    case 'abstract-int':
      switch (elDst.kind) {
        case 'abstract-int':
        case 'abstract-float':
        case 'f16':
        case 'f32':
        case 'f64':
        case 'u16':
        case 'u32':
        case 'u8':
        case 'i16':
        case 'i32':
        case 'i8':
          return true;
        default:
          return false;
      }
    default:
      return false;
  }
}

/// All floating-point scalar types
const kFloatScalars = [Type.abstractFloat, Type.f32, Type.f16];

/// All concrete floating-point scalar types
const kConcreteFloatScalars = [Type.f32, Type.f16];

/// All floating-point vec2 types
const kFloatVec2 = [Type.vec2af, Type.vec2f, Type.vec2h];

/// All floating-point vec3 types
const kFloatVec3 = [Type.vec3af, Type.vec3f, Type.vec3h];

/// All floating-point vec4 types
const kFloatVec4 = [Type.vec4af, Type.vec4f, Type.vec4h];

const kConcreteF32ScalarsAndVectors = [
Type.f32,
Type.vec2f,
Type.vec3f,
Type.vec4f];


/// All f16 floating-point scalar and vector types
const kConcreteF16ScalarsAndVectors = [
Type.f16,
Type.vec2h,
Type.vec3h,
Type.vec4h];


/// All floating-point vector types
const kFloatVectors = [...kFloatVec2, ...kFloatVec3, ...kFloatVec4];

/// All floating-point scalar and vector types
const kFloatScalarsAndVectors = [...kFloatScalars, ...kFloatVectors];

// Abstract and concrete integer types are not grouped into an 'all' type,
// because for many validation tests there is a valid conversion of
// AbstractInt -> AbstractFloat, but not one for the concrete integers. Thus, an
// AbstractInt literal will be a potentially valid input, whereas the concrete
// integers will not be. For many tests the pattern is to have separate fixtures
// for the things that might be valid and those that are never valid.

/// All signed integer vector types
const kConcreteSignedIntegerVectors = [Type.vec2i, Type.vec3i, Type.vec4i];

/// All unsigned integer vector types
const kConcreteUnsignedIntegerVectors = [Type.vec2u, Type.vec3u, Type.vec4u];

/// All concrete integer vector types
const kConcreteIntegerVectors = [
...kConcreteSignedIntegerVectors,
...kConcreteUnsignedIntegerVectors];


/// All signed integer scalar and vector types
const kConcreteSignedIntegerScalarsAndVectors = [
Type.i32,
...kConcreteSignedIntegerVectors];


/// All unsigned integer scalar and vector types
const kConcreteUnsignedIntegerScalarsAndVectors = [
Type.u32,
...kConcreteUnsignedIntegerVectors];


/// All concrete integer scalar and vector types
const kConcreteIntegerScalarsAndVectors = [
...kConcreteSignedIntegerScalarsAndVectors,
...kConcreteUnsignedIntegerScalarsAndVectors];


/// All types which are convertable to floating-point scalar types.
const kConvertableToFloatScalar = [Type.abstractInt, ...kFloatScalars];

/// All types which are convertable to floating-point vector 2 types.
const kConvertableToFloatVec2 = [Type.vec2ai, ...kFloatVec2];

/// All types which are convertable to floating-point vector 3 types.
const kConvertableToFloatVec3 = [Type.vec3ai, ...kFloatVec3];

/// All types which are convertable to floating-point vector 4 types.
const kConvertableToFloatVec4 = [Type.vec4ai, ...kFloatVec4];

/// All the types which are convertable to floating-point vector types.
const kConvertableToFloatVectors = [
Type.vec2ai,
Type.vec3ai,
Type.vec4ai,
...kFloatVectors];


/// All types which are convertable to floating-point scalar or vector types.
const kConvertableToFloatScalarsAndVectors = [
Type.abstractInt,
...kFloatScalars,
...kConvertableToFloatVectors];


/// All the numeric scalar and vector types.
const kAllNumericScalarsAndVectors = [
...kConvertableToFloatScalarsAndVectors,
...kConcreteIntegerScalarsAndVectors];


/// All the concrete integer and floating point scalars and vectors.
const kConcreteNumericScalarsAndVectors = [
...kConcreteIntegerScalarsAndVectors,
...kConcreteF16ScalarsAndVectors,
...kConcreteF32ScalarsAndVectors];


/// All boolean types.
const kAllBoolScalarsAndVectors = [Type.bool, Type.vec2b, Type.vec3b, Type.vec4b];

/// All the scalar and vector types.
const kAllScalarsAndVectors = [
...kAllBoolScalarsAndVectors,
...kAllNumericScalarsAndVectors];


/// All the vector types
const kAllVecTypes = Object.values(kVecTypes);

/// All the matrix types
const kAllMatrices = Object.values(kMatTypes);
});
__define('common/util/colors.js', function (__exports) {
'use strict';
__get(__exports, 'Colors', () => Colors);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/ /**
 * The interface used for formatting strings to contain color metadata.
 *
 * Use the interface properties to construct a style, then use the
 * `(s: string): string` function to format the provided string with the given
 * style.
 */


























































/**
 * The interface used for formatting strings with color metadata.
 *
 * Currently Colors will use the 'ansi-colors' module if it can be loaded.
 * If it cannot be loaded, then the Colors implementation is a straight pass-through.
 *
 * Colors may also be a no-op if the current environment does not support colors.
 */
let Colors;

try {

  Colors = require('ansi-colors');
} catch {
  const passthrough = (s) => s;
  passthrough.enabled = false;
  passthrough.reset = passthrough;
  passthrough.bold = passthrough;
  passthrough.dim = passthrough;
  passthrough.italic = passthrough;
  passthrough.underline = passthrough;
  passthrough.inverse = passthrough;
  passthrough.hidden = passthrough;
  passthrough.strikethrough = passthrough;
  passthrough.black = passthrough;
  passthrough.red = passthrough;
  passthrough.green = passthrough;
  passthrough.yellow = passthrough;
  passthrough.blue = passthrough;
  passthrough.magenta = passthrough;
  passthrough.cyan = passthrough;
  passthrough.white = passthrough;
  passthrough.gray = passthrough;
  passthrough.grey = passthrough;
  passthrough.blackBright = passthrough;
  passthrough.redBright = passthrough;
  passthrough.greenBright = passthrough;
  passthrough.yellowBright = passthrough;
  passthrough.blueBright = passthrough;
  passthrough.magentaBright = passthrough;
  passthrough.cyanBright = passthrough;
  passthrough.whiteBright = passthrough;
  passthrough.bgBlack = passthrough;
  passthrough.bgRed = passthrough;
  passthrough.bgGreen = passthrough;
  passthrough.bgYellow = passthrough;
  passthrough.bgBlue = passthrough;
  passthrough.bgMagenta = passthrough;
  passthrough.bgCyan = passthrough;
  passthrough.bgWhite = passthrough;
  passthrough.bgBlackBright = passthrough;
  passthrough.bgRedBright = passthrough;
  passthrough.bgGreenBright = passthrough;
  passthrough.bgYellowBright = passthrough;
  passthrough.bgBlueBright = passthrough;
  passthrough.bgMagentaBright = passthrough;
  passthrough.bgCyanBright = passthrough;
  passthrough.bgWhiteBright = passthrough;
  Colors = passthrough;
}
});
__define('webgpu/api/operation/command_buffer/copyBufferToBuffer.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = 'copyBufferToBuffer operation tests';let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });
let AllFeaturesMaxLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest } = __m); });

const g = makeTestGroup(AllFeaturesMaxLimitsGPUTest);

g.test('single').
desc(
  `Validate the correctness of the copy by filling the srcBuffer with testable data, doing
  CopyBufferToBuffer() copy, and verifying the content of the whole dstBuffer with MapRead:
  Copy {4 bytes, part of, the whole} srcBuffer to the dstBuffer {with, without} a non-zero valid
  srcOffset that
  - covers the whole dstBuffer
  - covers the beginning of the dstBuffer
  - covers the end of the dstBuffer
  - covers neither the beginning nor the end of the dstBuffer`
).
params((u) =>
u //
// Whether the test case requires the newly-added method signature or not.
.combine('newSig', [false, true]).
beginSubcases().
combine('srcOffset', [0, 4, 8, 16, undefined]).
combine('dstOffset', [0, 4, 8, 16, undefined]).
unless(
  (p) => (p.srcOffset === undefined || p.dstOffset === undefined) && p.srcOffset !== p.dstOffset
).
combine('copySize', [0, 4, 8, 16, undefined]).
expand('srcBufferSize', (p) => [
(p.srcOffset ?? 0) + (p.copySize ?? 0),
(p.srcOffset ?? 0) + (p.copySize ?? 0) + 8]
).
expand('dstBufferSize', (p) => [
(p.dstOffset ?? 0) + (p.copySize ?? 0),
(p.dstOffset ?? 0) + (p.copySize ?? 0) + 8]
)
// Bifurcate the cases between newSig=false and newSig=true based on whether they need it.
.filter((p) => {
  const needsNewSignature = [p.srcOffset, p.dstOffset, p.copySize].includes(undefined);
  return p.newSig === needsNewSignature;
})
).
fn((t) => {
  const { srcOffset, dstOffset, copySize, srcBufferSize, dstBufferSize } = t.params;

  const srcData = new Uint8Array(srcBufferSize);
  for (let i = 0; i < srcBufferSize; ++i) {
    srcData[i] = i + 1;
  }

  const src = t.makeBufferWithContents(srcData, GPUBufferUsage.COPY_SRC);

  const dst = t.createBufferTracked({
    size: dstBufferSize,
    usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST
  });

  const encoder = t.device.createCommandEncoder();
  if (srcOffset === undefined || dstOffset === undefined) {
    assert(srcOffset === undefined && dstOffset === undefined);
    encoder.copyBufferToBuffer(src, dst, copySize);
  } else {
    encoder.copyBufferToBuffer(src, srcOffset, dst, dstOffset, copySize);
  }

  const expectedSrcOffset = srcOffset ?? 0;
  const expectedDstOffset = dstOffset ?? 0;
  const expectedCopySize = copySize ?? srcBufferSize - expectedSrcOffset;

  const isValid = dstBufferSize - expectedDstOffset >= expectedCopySize;
  t.expectValidationError(() => {
    t.device.queue.submit([encoder.finish()]);
  }, !isValid);

  const expectedDstData = new Uint8Array(dstBufferSize);
  for (let i = 0; i < expectedCopySize; ++i) {
    expectedDstData[expectedDstOffset + i] = srcData[expectedSrcOffset + i];
  }

  t.expectGPUBufferValuesEqual(dst, expectedDstData);
});

g.test('state_transitions').
desc(
  `Test proper state transitions/barriers happen between copy commands.
    Copy part of src to dst, then a different part of dst to src, and check contents of both.`
).
fn((t) => {
  const srcData = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
  const dstData = new Uint8Array([10, 20, 30, 40, 50, 60, 70, 80]);

  const src = t.makeBufferWithContents(
    srcData,
    GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST
  );
  const dst = t.makeBufferWithContents(
    dstData,
    GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST
  );

  const encoder = t.device.createCommandEncoder();
  encoder.copyBufferToBuffer(src, 0, dst, 4, 4);
  encoder.copyBufferToBuffer(dst, 0, src, 4, 4);
  t.device.queue.submit([encoder.finish()]);

  const expectedSrcData = new Uint8Array([1, 2, 3, 4, 10, 20, 30, 40]);
  const expectedDstData = new Uint8Array([10, 20, 30, 40, 1, 2, 3, 4]);
  t.expectGPUBufferValuesEqual(src, expectedSrcData);
  t.expectGPUBufferValuesEqual(dst, expectedDstData);
});

g.test('copy_order').
desc(
  `Test copy commands in one command buffer occur in the correct order.
    First copies one region from src to dst, then another region from src to an overlapping region
    of dst, then checks the dst buffer's contents.`
).
fn((t) => {
  const srcData = new Uint32Array([1, 2, 3, 4, 5, 6, 7, 8]);

  const src = t.makeBufferWithContents(srcData, GPUBufferUsage.COPY_SRC);

  const dst = t.createBufferTracked({
    size: srcData.length * 4,
    usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST
  });

  const encoder = t.device.createCommandEncoder();
  encoder.copyBufferToBuffer(src, 0, dst, 0, 16);
  encoder.copyBufferToBuffer(src, 16, dst, 8, 16);
  t.device.queue.submit([encoder.finish()]);

  const expectedDstData = new Uint32Array([1, 2, 5, 6, 7, 8, 0, 0]);
  t.expectGPUBufferValuesEqual(dst, expectedDstData);
});
});
__define('webgpu/api/operation/compute/basic.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Basic command buffer compute tests.
`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let AllFeaturesMaxLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest } = __m); });
let checkElementsEqualGenerated; __bind('webgpu/util/check_contents.js', (__m) => { ({ checkElementsEqualGenerated } = __m); });

const g = makeTestGroup(AllFeaturesMaxLimitsGPUTest);

g.test('memcpy').fn((t) => {
  const data = new Uint32Array([0x01020304]);

  const src = t.makeBufferWithContents(data, GPUBufferUsage.COPY_DST | GPUBufferUsage.STORAGE);

  const dst = t.createBufferTracked({
    size: 4,
    usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.STORAGE
  });

  const pipeline = t.device.createComputePipeline({
    layout: 'auto',
    compute: {
      module: t.device.createShaderModule({
        code: `
          struct Data {
            value : u32
          };

          @group(0) @binding(0) var<storage, read> src : Data;
          @group(0) @binding(1) var<storage, read_write> dst : Data;

          @compute @workgroup_size(1) fn main() {
            dst.value = src.value;
            return;
          }
        `
      }),
      entryPoint: 'main'
    }
  });

  const bg = t.device.createBindGroup({
    entries: [
    { binding: 0, resource: { buffer: src, offset: 0, size: 4 } },
    { binding: 1, resource: { buffer: dst, offset: 0, size: 4 } }],

    layout: pipeline.getBindGroupLayout(0)
  });

  const encoder = t.device.createCommandEncoder();
  const pass = encoder.beginComputePass();
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, bg);
  pass.dispatchWorkgroups(1);
  pass.end();
  t.device.queue.submit([encoder.finish()]);

  t.expectGPUBufferValuesEqual(dst, data);
});

g.test('large_dispatch').
desc(`Test reasonably-sized large dispatches (see also: stress tests).`).
params((u) =>
u
// Reasonably-sized powers of two, and some stranger larger sizes.
.combine('dispatchSize', [256, 2048, 315, 628, 2179, 'maximum'])
// Test some reasonable workgroup sizes.
.beginSubcases()
// 0 == x axis; 1 == y axis; 2 == z axis.
.combine('largeDimension', [0, 1, 2]).
expand('workgroupSize', () => [1, 2, 8, 32, 'maximum'])
).
fn((t) => {
  // The output storage buffer is filled with this value.
  const val = 0x01020304;
  const badVal = 0xbaadf00d;

  const kMaxComputeWorkgroupSize = [
  t.device.limits.maxComputeWorkgroupSizeX,
  t.device.limits.maxComputeWorkgroupSizeY,
  t.device.limits.maxComputeWorkgroupSizeZ];


  const wgSize =
  t.params.workgroupSize === 'maximum' ?
  kMaxComputeWorkgroupSize[t.params.largeDimension] :
  t.params.workgroupSize;
  const dispatchSize =
  t.params.dispatchSize === 'maximum' ?
  t.device.limits.maxComputeWorkgroupsPerDimension :
  t.params.dispatchSize;
  const bufferLength = dispatchSize * wgSize;
  const bufferByteSize = Uint32Array.BYTES_PER_ELEMENT * bufferLength;
  const dst = t.createBufferTracked({
    size: bufferByteSize,
    usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.STORAGE
  });

  // Only use one large dimension and workgroup size in the dispatch
  // call to keep the size of the test reasonable.
  const dims = [1, 1, 1];
  dims[t.params.largeDimension] = dispatchSize;
  const wgSizes = [1, 1, 1];
  wgSizes[t.params.largeDimension] = wgSize;
  const pipeline = t.device.createComputePipeline({
    layout: 'auto',
    compute: {
      module: t.device.createShaderModule({
        code: `
            struct OutputBuffer {
              value : array<u32>
            };

            @group(0) @binding(0) var<storage, read_write> dst : OutputBuffer;

            @compute @workgroup_size(${wgSizes[0]}, ${wgSizes[1]}, ${wgSizes[2]})
            fn main(
              @builtin(global_invocation_id) GlobalInvocationID : vec3<u32>
            ) {
              var xExtent : u32 = ${dims[0]}u * ${wgSizes[0]}u;
              var yExtent : u32 = ${dims[1]}u * ${wgSizes[1]}u;
              var zExtent : u32 = ${dims[2]}u * ${wgSizes[2]}u;
              var index : u32 = (
                GlobalInvocationID.z * xExtent * yExtent +
                GlobalInvocationID.y * xExtent +
                GlobalInvocationID.x);
              var val : u32 = ${val}u;
              // Trivial error checking in the indexing and invocation.
              if (GlobalInvocationID.x > xExtent ||
                  GlobalInvocationID.y > yExtent ||
                  GlobalInvocationID.z > zExtent) {
                val = ${badVal}u;
              }
              dst.value[index] = val;
            }
          `
      }),
      entryPoint: 'main'
    }
  });

  const bg = t.device.createBindGroup({
    entries: [{ binding: 0, resource: { buffer: dst, offset: 0, size: bufferByteSize } }],
    layout: pipeline.getBindGroupLayout(0)
  });

  const encoder = t.device.createCommandEncoder();
  const pass = encoder.beginComputePass();
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, bg);
  pass.dispatchWorkgroups(dims[0], dims[1], dims[2]);
  pass.end();
  t.device.queue.submit([encoder.finish()]);

  t.expectGPUBufferValuesPassCheck(dst, (a) => checkElementsEqualGenerated(a, (_i) => val), {
    type: Uint32Array,
    typedLength: bufferLength
  });

  dst.destroy();
});
});
__define('webgpu/api/operation/labels.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Tests for object labels.
`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let keysOf; __bind('common/util/data_tables.js', (__m) => { ({ keysOf } = __m); });
let getGPU; __bind('common/util/navigator_gpu.js', (__m) => { ({ getGPU } = __m); });
let AllFeaturesMaxLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest } = __m); });

const g = makeTestGroup(AllFeaturesMaxLimitsGPUTest);


const kTestFunctions = {
  createBuffer: (t, label) => {
    const buffer = t.createBufferTracked({ size: 16, usage: GPUBufferUsage.COPY_DST, label });
    t.expect(buffer.label === label);
    buffer.destroy();
    t.expect(buffer.label === label);
  },

  requestDevice: async (t, label) => {
    const gpu = getGPU(t.rec);
    const adapter = await gpu.requestAdapter();
    t.expect(!!adapter);
    const device = await t.requestDeviceTracked(adapter, { label });
    t.expect(!!device);
    t.expect(device.label === label);
    device.destroy();
    t.expect(device.label === label);
  },

  createTexture: (t, label) => {
    const texture = t.createTextureTracked({
      label,
      size: [1, 1, 1],
      format: 'rgba8unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT
    });
    t.expect(texture.label === label);
    texture.destroy();
    t.expect(texture.label === label);
  },

  createSampler: (t, label) => {
    const sampler = t.device.createSampler({ label });
    t.expect(sampler.label === label);
  },

  createBindGroupLayout: (t, label) => {
    const bindGroupLayout = t.device.createBindGroupLayout({ label, entries: [] });
    t.expect(bindGroupLayout.label === label);
  },

  createPipelineLayout: (t, label) => {
    const pipelineLayout = t.device.createPipelineLayout({ label, bindGroupLayouts: [] });
    t.expect(pipelineLayout.label === label);
  },

  createBindGroup: (t, label) => {
    const layout = t.device.createBindGroupLayout({ entries: [] });
    const bindGroup = t.device.createBindGroup({ label, layout, entries: [] });
    t.expect(bindGroup.label === label);
  },

  createShaderModule: (t, label) => {
    const shaderModule = t.device.createShaderModule({
      label,
      code: `
        @vertex fn vs() -> @builtin(position) vec4f {
         return vec4f(0, 0, 0, 1);
        }
      `
    });
    t.expect(shaderModule.label === label);
  },

  createComputePipeline: (t, label) => {
    const module = t.device.createShaderModule({
      code: `
        @compute @workgroup_size(1u) fn foo() {}
      `
    });
    const computePipeline = t.device.createComputePipeline({
      label,
      layout: 'auto',
      compute: {
        module,
        entryPoint: 'foo'
      }
    });
    t.expect(computePipeline.label === label);
  },

  createRenderPipeline: (t, label) => {
    const module = t.device.createShaderModule({
      code: `
        @vertex fn foo() -> @builtin(position) vec4f {
         return vec4f(0, 0, 0, 1);
        }
      `
    });
    const renderPipeline = t.device.createRenderPipeline({
      label,
      layout: 'auto',
      vertex: {
        module,
        entryPoint: 'foo'
      },
      // Specify a color attachment so we have at least one render target. Otherwise, details here
      // are not relevant to this test.
      fragment: {
        targets: [{ format: 'rgba8unorm' }],
        module: t.device.createShaderModule({
          code: `@fragment fn main() -> @location(0) vec4f { return vec4f(0); }`
        })
      }
    });
    t.expect(renderPipeline.label === label);
  },

  createComputePipelineAsync: async (t, label) => {
    const module = t.device.createShaderModule({
      code: `
        @compute @workgroup_size(1u) fn foo() {}
      `
    });
    const computePipeline = await t.device.createComputePipelineAsync({
      label,
      layout: 'auto',
      compute: {
        module,
        entryPoint: 'foo'
      }
    });
    t.expect(computePipeline.label === label);
  },

  createRenderPipelineAsync: async (t, label) => {
    const module = t.device.createShaderModule({
      label,
      code: `
        @vertex fn foo() -> @builtin(position) vec4f {
         return vec4f(0, 0, 0, 1);
        }
      `
    });
    const renderPipeline = await t.device.createRenderPipelineAsync({
      label,
      layout: 'auto',
      vertex: {
        module,
        entryPoint: 'foo'
      },
      // Specify a color attachment so we have at least one render target. Otherwise, details here
      // are not relevant to this test.
      fragment: {
        targets: [{ format: 'rgba8unorm' }],
        module: t.device.createShaderModule({
          code: `@fragment fn main() -> @location(0) vec4f { return vec4f(0); }`
        })
      }
    });
    t.expect(renderPipeline.label === label);
  },

  createCommandEncoder: (t, label) => {
    const encoder = t.device.createCommandEncoder({ label });
    t.expect(encoder.label === label);
  },

  createRenderBundleEncoder: (t, label) => {
    const encoder = t.device.createRenderBundleEncoder({
      label,
      colorFormats: ['rgba8unorm']
    });
    t.expect(encoder.label === label);
  },

  createQuerySet: (t, label) => {
    const querySet = t.createQuerySetTracked({
      label,
      type: 'occlusion',
      count: 1
    });
    t.expect(querySet.label === label);
    querySet.destroy();
    t.expect(querySet.label === label);
  },

  beginRenderPass: (t, label) => {
    const texture = t.createTextureTracked({
      label,
      size: [1, 1, 1],
      format: 'rgba8unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT
    });
    const label2 = `${label}-2`;
    const encoder = t.device.createCommandEncoder();
    encoder.label = label2;
    const renderPass = encoder.beginRenderPass({
      label,
      colorAttachments: [{ view: texture.createView(), loadOp: 'clear', storeOp: 'store' }]
    });
    t.expect(renderPass.label === label);
    renderPass.end();
    t.expect(renderPass.label === label);
    encoder.finish();
    t.expect(renderPass.label === label);
    t.expect(encoder.label === label2);
    texture.destroy();
  },

  beginComputePass: (t, label) => {
    const label2 = `${label}-2`;
    const encoder = t.device.createCommandEncoder();
    encoder.label = label2;
    const computePass = encoder.beginComputePass({ label });
    t.expect(computePass.label === label);
    computePass.end();
    t.expect(computePass.label === label);
    encoder.finish();
    t.expect(computePass.label === label);
    t.expect(encoder.label === label2);
  },

  finish: (t, label) => {
    const encoder = t.device.createCommandEncoder();
    const commandBuffer = encoder.finish({ label });
    t.expect(commandBuffer.label === label);
  },

  createView: (t, label) => {
    const texture = t.createTextureTracked({
      size: [1, 1, 1],
      format: 'rgba8unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT
    });
    const view = texture.createView({ label });
    t.expect(view.label === label);
    texture.destroy();
    t.expect(view.label === label);
  }
};

g.test('object_has_descriptor_label').
desc(
  `
  For every create function, the descriptor.label is carried over to the object.label.

  TODO: test importExternalTexture
  TODO: make a best effort and generating an error that is likely to use label. There's nothing to check for
        but it may surface bugs related to unusual labels.
    `
).
params((u) =>
u.
combine('name', keysOf(kTestFunctions)).
beginSubcases().
combine('label', ['label', '\0', 'null\0in\0label', '\ud83c\udf1e\ud83d\udc46'])
).
fn(async (t) => {
  const { name, label } = t.params;
  const result = kTestFunctions[name](t, label);
  if (result instanceof Promise) {
    await result;
  }
});

g.test('wrappers_do_not_share_labels').
desc('test that different wrapper objects for the same GPU object do not share labels').
fn((t) => {
  const module = t.device.createShaderModule({
    code: `
        @group(0) @binding(0) var<uniform> pos: vec4f;
        @vertex fn main() -> @builtin(position) vec4f {
          return pos;
        }
      `
  });
  const pipeline = t.device.createRenderPipeline({
    layout: 'auto',
    vertex: {
      module,
      entryPoint: 'main'
    },
    // Specify a color attachment so we have at least one render target. Otherwise, details here
    // are not relevant to this test.
    fragment: {
      targets: [{ format: 'rgba8unorm' }],
      module: t.device.createShaderModule({
        code: `@fragment fn main() -> @location(0) vec4f { return vec4f(0); }`
      })
    }
  });
  const layout1 = pipeline.getBindGroupLayout(0);
  const layout2 = pipeline.getBindGroupLayout(0);
  t.expect(layout1 !== layout2);

  layout1.label = 'foo';
  layout2.label = 'bar';

  t.expect(layout1.label === 'foo');
  t.expect(layout2.label === 'bar');
});
});
__define('webgpu/api/operation/render_pass/clear_value.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Tests for render pass clear values.
`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });
let depthStencilFormatAspectSize, kStencilTextureFormats, isDepthTextureFormat; __bind('webgpu/format_info.js', (__m) => { ({ depthStencilFormatAspectSize, kStencilTextureFormats, isDepthTextureFormat } = __m); });
let AllFeaturesMaxLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest } = __m); });

const g = makeTestGroup(AllFeaturesMaxLimitsGPUTest);

g.test('loaded').
desc(
  `Test render pass clear values are visible during the pass by doing some trivial blending
with the attachment (e.g. add [0,0,0,0] to the color and verify the stored result).`
).
unimplemented();

g.test('stencil_clear_value').
desc(
  `Test that when stencilLoadOp is "clear", the stencil aspect should be correctly cleared by
     GPURenderPassDepthStencilAttachment.stencilClearValue, which will be converted to the type of
     the stencil aspect of view by taking the same number of LSBs as the number of bits in the
     stencil aspect of one texel block of view.`
).
params((u) =>
u.
combine('stencilFormat', kStencilTextureFormats).
combine('stencilClearValue', [0, 1, 0xff, 0x100 + 2, 0x10000 + 3]).
combine('applyStencilClearValueAsStencilReferenceValue', [true, false])
).
fn((t) => {
  const { stencilFormat, stencilClearValue, applyStencilClearValueAsStencilReferenceValue } =
  t.params;

  t.skipIfTextureFormatNotSupported(stencilFormat);

  const kSize = [1, 1, 1];
  const colorFormat = 'rgba8unorm';
  const stencilTexture = t.createTextureTracked({
    format: stencilFormat,
    size: kSize,
    usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC
  });
  const colorTexture = t.createTextureTracked({
    format: colorFormat,
    size: kSize,
    usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC
  });
  const renderPipeline = t.device.createRenderPipeline({
    layout: 'auto',
    vertex: {
      module: t.device.createShaderModule({
        code: `
            @vertex
            fn main(@builtin(vertex_index) VertexIndex : u32)-> @builtin(position) vec4<f32> {
              var pos : array<vec2<f32>, 6> = array<vec2<f32>, 6>(
                  vec2<f32>(-1.0,  1.0),
                  vec2<f32>(-1.0, -1.0),
                  vec2<f32>( 1.0,  1.0),
                  vec2<f32>(-1.0, -1.0),
                  vec2<f32>( 1.0,  1.0),
                  vec2<f32>( 1.0, -1.0));
              return vec4<f32>(pos[VertexIndex], 0.0, 1.0);
            }`
      }),
      entryPoint: 'main'
    },
    fragment: {
      module: t.device.createShaderModule({
        code: `
            @fragment
            fn main() -> @location(0) vec4<f32> {
              return vec4<f32>(0.0, 1.0, 0.0, 1.0);
            }`
      }),
      entryPoint: 'main',
      targets: [{ format: colorFormat }]
    },
    depthStencil: {
      format: stencilFormat,
      depthCompare: 'always',
      depthWriteEnabled: false,
      stencilFront: {
        compare: 'equal'
      },
      stencilBack: {
        compare: 'equal'
      }
    },
    primitive: {
      topology: 'triangle-list'
    }
  });

  const stencilAspectSizeInBytes = depthStencilFormatAspectSize(stencilFormat, 'stencil-only');
  assert(stencilAspectSizeInBytes > 0);
  const expectedStencilValue = stencilClearValue & (stencilAspectSizeInBytes << 8) - 1;

  // StencilReference used in setStencilReference will also be masked to the lowest valid bits, so
  // no matter what we set in the rest high bits that will be masked out (different or same
  // between stencilClearValue and stencilReference), the test will pass if and only if the valid
  // lowest bits are the same.
  const stencilReference = applyStencilClearValueAsStencilReferenceValue ?
  stencilClearValue :
  expectedStencilValue;

  const encoder = t.device.createCommandEncoder();

  const depthStencilAttachment = {
    view: stencilTexture.createView(),
    depthClearValue: 0,
    stencilLoadOp: 'clear',
    stencilStoreOp: 'store',
    stencilClearValue
  };
  if (isDepthTextureFormat(stencilFormat)) {
    depthStencilAttachment.depthClearValue = 0;
    depthStencilAttachment.depthLoadOp = 'clear';
    depthStencilAttachment.depthStoreOp = 'store';
  }
  const renderPassEncoder = encoder.beginRenderPass({
    colorAttachments: [
    {
      view: colorTexture.createView(),
      loadOp: 'clear',
      storeOp: 'store',
      clearValue: [1, 0, 0, 1]
    }],

    depthStencilAttachment
  });
  renderPassEncoder.setPipeline(renderPipeline);
  renderPassEncoder.setStencilReference(stencilReference);
  renderPassEncoder.draw(6);
  renderPassEncoder.end();

  const destinationBuffer = t.createBufferTracked({
    usage: GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST,
    size: 4
  });
  encoder.copyTextureToBuffer(
    {
      texture: stencilTexture,
      aspect: 'stencil-only'
    },
    {
      buffer: destinationBuffer
    },
    [1, 1, 1]
  );

  t.queue.submit([encoder.finish()]);

  t.expectSingleColor(colorTexture, colorFormat, {
    size: [1, 1, 1],
    exp: { R: 0, G: 1, B: 0, A: 1 }
  });
  t.expectGPUBufferValuesEqual(destinationBuffer, new Uint8Array([expectedStencilValue]));
});
});
__define('webgpu/api/validation/buffer/create.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Tests for validation in createBuffer.
`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let assert; __bind('common/util/util.js', (__m) => { ({ assert } = __m); });
let kAllBufferUsageBits, kBufferSizeAlignment, kBufferUsages, kSomeBogusBufferUsage; __bind('webgpu/capability_info.js', (__m) => { ({ kAllBufferUsageBits, kBufferSizeAlignment, kBufferUsages, kSomeBogusBufferUsage } = __m); });
let GPUConst; __bind('webgpu/constants.js', (__m) => { ({ GPUConst } = __m); });
let AllFeaturesMaxLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest } = __m); });
let kMaxSafeMultipleOf8; __bind('webgpu/util/math.js', (__m) => { ({ kMaxSafeMultipleOf8 } = __m); });

const g = makeTestGroup(AllFeaturesMaxLimitsGPUTest);

assert(kBufferSizeAlignment === 4);
g.test('size').
desc(
  'Test buffer size alignment is validated to be a multiple of 4 if mappedAtCreation is true.'
).
params((u) =>
u.
combine('mappedAtCreation', [false, true]).
beginSubcases().
combine('size', [
0,
kBufferSizeAlignment * 0.5,
kBufferSizeAlignment,
kBufferSizeAlignment * 1.5,
kBufferSizeAlignment * 2]
)
).
fn((t) => {
  const { mappedAtCreation, size } = t.params;
  const isValid = !mappedAtCreation || size % kBufferSizeAlignment === 0;
  const usage = BufferUsage.COPY_SRC;

  t.shouldThrow(isValid ? false : 'RangeError', () =>
  t.createBufferTracked({ size, usage, mappedAtCreation })
  );
});

g.test('limit').
desc('Test buffer size is validated against maxBufferSize.')
// Note: Avoid using subcases here so we don't allocate the buffers for multiple subcases at once.
.params((u) => u.combine('sizeAddition', [-1, 0, +1])).
fn((t) => {
  const { sizeAddition } = t.params;
  const size = t.makeLimitVariant('maxBufferSize', { mult: 1, add: sizeAddition });
  const isValid = size <= t.device.limits.maxBufferSize;
  const usage = BufferUsage.COPY_SRC;
  t.expectGPUError('validation', () => t.createBufferTracked({ size, usage }), !isValid);
});

g.test('usage').
desc('Test combinations of zero to two usage flags are validated to be valid.').
params((u) =>
u.
combine('usage1', [0, ...kBufferUsages, kSomeBogusBufferUsage]).
combine('usage2', [0, ...kBufferUsages, kSomeBogusBufferUsage]).
filter((p) => p.usage1 <= p.usage2).
beginSubcases().
combine('mappedAtCreation', [false, true])
).
fn((t) => {
  const { mappedAtCreation, usage1, usage2 } = t.params;
  const usage = usage1 | usage2;

  const isValid =
  usage !== 0 &&
  (usage & ~kAllBufferUsageBits) === 0 && (
  (usage & GPUBufferUsage.MAP_READ) === 0 ||
  (usage & ~(GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ)) === 0) && (
  (usage & GPUBufferUsage.MAP_WRITE) === 0 ||
  (usage & ~(GPUBufferUsage.COPY_SRC | GPUBufferUsage.MAP_WRITE)) === 0);

  t.expectGPUError(
    'validation',
    () => t.createBufferTracked({ size: kBufferSizeAlignment * 2, usage, mappedAtCreation }),
    !isValid
  );
});

g.test('new_usages').
desc(`Valid usages not present in GPUBufferUsage shouldn't be accepted by createBuffer().`).
params((u) =>
u //
.beginSubcases().
combine('usage', kBufferUsages)
).
fn((t) => {
  const { usage } = t.params;

  let exposedUsages = 0;
  for (const v of Object.values(GPUBufferUsage)) {
    exposedUsages |= v;
  }

  const success = (usage & exposedUsages) === usage;

  t.expectGPUError(
    'validation',
    () => {
      t.createBufferTracked({ size: 16, usage });
    },
    !success
  );
});

const BufferUsage = GPUConst.BufferUsage;

g.test('createBuffer_invalid_and_oom').
desc(
  `When creating a mappable buffer, it's expected that shmem may be immediately allocated
(in the content process, before validation occurs in the GPU process). If the buffer is really
large, though, it could fail shmem allocation before validation fails. Ensure that OOM error is
hidden behind the "more severe" validation error.`
).
paramsSubcasesOnly((u) =>
u.combineWithParams([
{ _valid: true, usage: BufferUsage.UNIFORM, size: 16 },
{ _valid: true, usage: BufferUsage.STORAGE, size: 16 },
// Invalid because UNIFORM is not allowed with map usages.
{ usage: BufferUsage.MAP_WRITE | BufferUsage.UNIFORM, size: 16 },
{ usage: BufferUsage.MAP_WRITE | BufferUsage.UNIFORM, size: kMaxSafeMultipleOf8 },
{ usage: BufferUsage.MAP_WRITE | BufferUsage.UNIFORM, size: 0x20_0000_0000 }, // 128 GiB
{ usage: BufferUsage.MAP_READ | BufferUsage.UNIFORM, size: 16 },
{ usage: BufferUsage.MAP_READ | BufferUsage.UNIFORM, size: kMaxSafeMultipleOf8 },
{ usage: BufferUsage.MAP_READ | BufferUsage.UNIFORM, size: 0x20_0000_0000 } // 128 GiB
])
).
fn((t) => {
  const { _valid, usage, size } = t.params;

  t.expectGPUError('validation', () => t.createBufferTracked({ size, usage }), !_valid);
});
});
__define('webgpu/api/validation/createBindGroup.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
  createBindGroup validation tests.

  TODO: Ensure sure tests cover all createBindGroup validation rules.
`;let AllFeaturesMaxLimitsGPUTest, kResourceStates; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest, kResourceStates } = __m); });
let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let assert, makeValueTestVariant, unreachable; __bind('common/util/util.js', (__m) => { ({ assert, makeValueTestVariant, unreachable } = __m); });
let allBindingEntries, bindingTypeInfo, bufferBindingEntries, bufferBindingTypeInfo, kBindableResources, kBufferBindingTypes, kBufferUsages, kCompareFunctions, kSamplerBindingTypes, kTextureUsages, kTextureViewDimensions, sampledAndStorageBindingEntries, texBindingTypeInfo, isValidTextureUsageCombination; __bind('webgpu/capability_info.js', (__m) => { ({ allBindingEntries, bindingTypeInfo, bufferBindingEntries, bufferBindingTypeInfo, kBindableResources, kBufferBindingTypes, kBufferUsages, kCompareFunctions, kSamplerBindingTypes, kTextureUsages, kTextureViewDimensions, sampledAndStorageBindingEntries, texBindingTypeInfo, isValidTextureUsageCombination } = __m); });
let GPUConst; __bind('webgpu/constants.js', (__m) => { ({ GPUConst } = __m); });
let kPossibleStorageTextureFormats, kRegularTextureFormats; __bind('webgpu/format_info.js', (__m) => { ({ kPossibleStorageTextureFormats, kRegularTextureFormats } = __m); });
let getTextureDimensionFromView; __bind('webgpu/util/texture/base.js', (__m) => { ({ getTextureDimensionFromView } = __m); });

const vtu = __require('webgpu/api/validation/validation_test_utils.js');

const kTestFormat = 'r32float';

function clone(descriptor) {
  return JSON.parse(JSON.stringify(descriptor));
}

function skipIfResourceNotSupportedInStages(
t,
entry,
visibility)
{
  if (t.isCompatibility) {
    t.skipIf(
      (visibility & GPUShaderStage.FRAGMENT) !== 0 && (
      entry.buffer?.type === 'storage' || entry.buffer?.type === 'read-only-storage') &&
      !(t.device.limits.maxStorageBuffersInFragmentStage >= 2),
      `maxStorageBuffersInFragmentStage(${t.device.limits.maxStorageBuffersInFragmentStage}) < 2`
    );
    t.skipIf(
      (visibility & GPUShaderStage.FRAGMENT) !== 0 &&
      entry.storageTexture !== undefined &&
      !(t.device.limits.maxStorageTexturesInFragmentStage >= 1),
      `maxStorageTexturesInFragmentStage(${t.device.limits.maxStorageTexturesInFragmentStage}) < 1`
    );
    t.skipIf(
      (visibility & GPUShaderStage.VERTEX) !== 0 && (
      entry.buffer?.type === 'storage' || entry.buffer?.type === 'read-only-storage') &&
      !(t.device.limits.maxStorageBuffersInVertexStage >= 2),
      `maxStorageBuffersInVertexStage(${t.device.limits.maxStorageBuffersInVertexStage}) < 2`
    );
    t.skipIf(
      (visibility & GPUShaderStage.VERTEX) !== 0 &&
      entry.storageTexture !== undefined &&
      !(t.device.limits.maxStorageTexturesInVertexStage >= 1),
      `maxStorageTexturesInVertexStage(${t.device.limits.maxStorageTexturesInVertexStage}) < 1`
    );
  }
}

const g = makeTestGroup(AllFeaturesMaxLimitsGPUTest);

g.test('binding_count_mismatch').
desc('Test that the number of entries must match the number of entries in the BindGroupLayout.').
paramsSubcasesOnly((u) =>
u //
.combine('layoutEntryCount', [1, 2, 3]).
combine('bindGroupEntryCount', [1, 2, 3])
).
fn((t) => {
  const { layoutEntryCount, bindGroupEntryCount } = t.params;

  const layoutEntries = [];
  for (let i = 0; i < layoutEntryCount; ++i) {
    layoutEntries.push({
      binding: i,
      visibility: GPUShaderStage.COMPUTE,
      buffer: { type: 'storage' }
    });
  }
  const bindGroupLayout = t.device.createBindGroupLayout({ entries: layoutEntries });

  const entries = [];
  for (let i = 0; i < bindGroupEntryCount; ++i) {
    entries.push({
      binding: i,
      resource: { buffer: vtu.getStorageBuffer(t) }
    });
  }

  const shouldError = layoutEntryCount !== bindGroupEntryCount;
  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries,
      layout: bindGroupLayout
    });
  }, shouldError);
});

g.test('binding_must_be_present_in_layout').
desc(
  'Test that the binding slot for each entry matches a binding slot defined in the BindGroupLayout.'
).
paramsSubcasesOnly((u) =>
u //
.combine('layoutBinding', [0, 1, 2]).
combine('binding', [0, 1, 2])
).
fn((t) => {
  const { layoutBinding, binding } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    { binding: layoutBinding, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'storage' } }]

  });

  const descriptor = {
    entries: [{ binding, resource: { buffer: vtu.getStorageBuffer(t) } }],
    layout: bindGroupLayout
  };

  const shouldError = layoutBinding !== binding;
  t.expectValidationError(() => {
    t.device.createBindGroup(descriptor);
  }, shouldError);
});

g.test('binding_must_contain_resource_defined_in_layout').
desc(
  'Test that only compatible resource types specified in the BindGroupLayout are allowed for each entry.'
).
params((u) =>
u //
.combine('resourceType', kBindableResources).
combine('entry', allBindingEntries(false, kTestFormat))
).
fn((t) => {
  const { resourceType, entry } = t.params;
  const info = bindingTypeInfo(entry);

  const layout = t.device.createBindGroupLayout({
    entries: [{ binding: 0, visibility: GPUShaderStage.COMPUTE, ...entry }]
  });

  const resource = vtu.getBindingResource(t, resourceType);

  const isStorageTextureResourceType = (resourceType) => {
    switch (resourceType) {
      case 'readonlyStorageTex':
      case 'readwriteStorageTex':
      case 'writeonlyStorageTex':
        return true;
      default:
        return false;
    }
  };

  let resourceBindingIsCompatible;
  switch (info.resource) {
    // Either type of sampler may be bound to a filtering sampler binding.
    case 'filtSamp':
      resourceBindingIsCompatible = resourceType === 'filtSamp' || resourceType === 'nonFiltSamp';
      break;
    // But only non-filtering samplers can be used with non-filtering sampler bindings.
    case 'nonFiltSamp':
      resourceBindingIsCompatible = resourceType === 'nonFiltSamp';
      break;
    case 'readonlyStorageTex':
    case 'readwriteStorageTex':
    case 'writeonlyStorageTex':
      resourceBindingIsCompatible = isStorageTextureResourceType(resourceType);
      break;
    default:
      resourceBindingIsCompatible = info.resource === resourceType;
      break;
  }
  t.expectValidationError(() => {
    t.device.createBindGroup({ layout, entries: [{ binding: 0, resource }] });
  }, !resourceBindingIsCompatible);
});

g.test('texture_binding_must_have_correct_usage').
desc('Tests that texture bindings must have the correct usage.').
paramsSubcasesOnly((u) =>
u //
.combine('entry', sampledAndStorageBindingEntries(false, kTestFormat)).
combine('usage', kTextureUsages).
unless(({ entry, usage }) => {
  const info = texBindingTypeInfo(entry);
  // TRANSIENT_ATTACHMENT is only valid when combined with RENDER_ATTACHMENT, so skip.
  if (
  usage === GPUConst.TextureUsage.TRANSIENT_ATTACHMENT &&
  info.resource !== 'sampledTexMS')
  {
    return true;
  }
  // Can't create the texture for this (usage=STORAGE_BINDING and sampleCount=4), so skip.
  return usage === GPUConst.TextureUsage.STORAGE_BINDING && info.resource === 'sampledTexMS';
})
).
fn((t) => {
  const { entry, usage } = t.params;
  const info = texBindingTypeInfo(entry);

  t.skipIf(
    t.isCompatibility && info.resource === 'sampledTexMS',
    "The test requires 'r32float' multisampled support which compat mode doesn't guarantee."
  );

  // MAINTENANCE_TODO(#4509): Remove this after all implementations have TRANSIENT_ATTACHMENT.
  if ((usage & GPUConst.TextureUsage.TRANSIENT_ATTACHMENT) !== 0) {
    t.skipIfTransientAttachmentNotSupported();
  }

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [{ binding: 0, visibility: GPUShaderStage.COMPUTE, ...entry }]
  });

  // The `RENDER_ATTACHMENT` usage must be specified if sampleCount > 1 according to WebGPU SPEC.
  const appliedUsage =
  info.resource === 'sampledTexMS' ? usage | GPUConst.TextureUsage.RENDER_ATTACHMENT : usage;

  const descriptor = {
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: kTestFormat,
    usage: appliedUsage,
    sampleCount: info.resource === 'sampledTexMS' ? 4 : 1
  };
  const resource = t.createTextureTracked(descriptor).createView();

  const shouldError = (usage & info.usage) === 0;
  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource }],
      layout: bindGroupLayout
    });
  }, shouldError);
});

g.test('texture_must_have_correct_component_type').
desc(
  `
    Tests that texture bindings must have a format that matches the sample type specified in the BindGroupLayout.
    - Tests a compatible format for every sample type
    - Tests an incompatible format for every sample type`
).
params((u) => u.combine('sampleType', ['float', 'sint', 'uint'])).
fn((t) => {
  const { sampleType } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.FRAGMENT,
      texture: { sampleType }
    }]

  });

  let format;
  if (sampleType === 'float') {
    format = 'r8unorm';
  } else if (sampleType === 'sint') {
    format = 'r8sint';
  } else if (sampleType === 'uint') {
    format = 'r8uint';
  } else {
    unreachable('Unexpected texture component type');
  }

  const goodDescriptor = {
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format,
    usage: GPUTextureUsage.TEXTURE_BINDING
  };

  // Control case
  t.device.createBindGroup({
    entries: [
    {
      binding: 0,
      resource: t.createTextureTracked(goodDescriptor).createView()
    }],

    layout: bindGroupLayout
  });

  function* mismatchedTextureFormats() {
    if (sampleType !== 'float') {
      yield 'r8unorm';
    }
    if (sampleType !== 'sint') {
      yield 'r8sint';
    }
    if (sampleType !== 'uint') {
      yield 'r8uint';
    }
  }

  // Mismatched texture binding formats are not valid.
  for (const mismatchedTextureFormat of mismatchedTextureFormats()) {
    const badDescriptor = clone(goodDescriptor);
    badDescriptor.format = mismatchedTextureFormat;

    t.expectValidationError(() => {
      t.device.createBindGroup({
        entries: [{ binding: 0, resource: t.createTextureTracked(badDescriptor).createView() }],
        layout: bindGroupLayout
      });
    });
  }
});

g.test('texture_must_have_correct_dimension').
desc(
  `
    Test that bound texture views match the dimensions supplied in the BindGroupLayout
      - Test for every GPUTextureViewDimension
      - Test for both TEXTURE_BINDING and STORAGE_BINDING.
  `
).
params((u) =>
u.
combine('usage', [
GPUConst.TextureUsage.TEXTURE_BINDING,
GPUConst.TextureUsage.STORAGE_BINDING]
).
combine('viewDimension', kTextureViewDimensions).
unless(
  (p) =>
  p.usage === GPUConst.TextureUsage.STORAGE_BINDING && (
  p.viewDimension === 'cube' || p.viewDimension === 'cube-array')
).
beginSubcases().
combine('dimension', kTextureViewDimensions)
).
fn((t) => {
  const { usage, viewDimension, dimension } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    usage === GPUTextureUsage.TEXTURE_BINDING ?
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      texture: { viewDimension }
    } :
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      storageTexture: { access: 'write-only', format: 'rgba8unorm', viewDimension }
    }]

  });

  let height = 16;
  let depthOrArrayLayers = 6;
  if (dimension === '1d') {
    height = 1;
    depthOrArrayLayers = 1;
  }

  const texture = t.createTextureTracked({
    size: { width: 16, height, depthOrArrayLayers },
    format: 'rgba8unorm',
    usage,
    dimension: getTextureDimensionFromView(dimension)
  });

  t.skipIfTextureViewDimensionNotSupported(viewDimension, dimension);
  if (t.isCompatibility && texture.dimension === '2d') {
    if (depthOrArrayLayers === 1) {
      t.skipIf(
        viewDimension !== '2d',
        '1 layer 2d textures default to textureBindingViewDimension: "2d" in compat mode'
      );
    } else {
      t.skipIf(
        viewDimension !== '2d-array',
        '> 1 layer 2d textures default to textureBindingViewDimension "2d-array" in compat mode'
      );
    }
  }

  const shouldError = viewDimension !== dimension;
  const textureView = texture.createView({ dimension });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, shouldError);
});

g.test('multisampled_validation').
desc(
  `
    Test that the sample count of the texture is greater than 1 if the BindGroup entry's
    multisampled is true. Otherwise, the texture's sampleCount should be 1.
  `
).
params((u) =>
u //
.combine('multisampled', [true, false]).
beginSubcases().
combine('sampleCount', [1, 4])
).
fn((t) => {
  const { multisampled, sampleCount } = t.params;
  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      texture: { multisampled, sampleType: multisampled ? 'unfilterable-float' : undefined }
    }]

  });

  const texture = t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.RENDER_ATTACHMENT,
    sampleCount
  });

  const isValid = !multisampled && sampleCount === 1 || multisampled && sampleCount > 1;

  const textureView = texture.createView();
  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('buffer_offset_and_size_for_bind_groups_match').
desc(
  `
    Test that a buffer binding's [offset, offset + size) must be contained in the BindGroup entry's buffer.
    - Test for various offsets and sizes`
).
paramsSubcasesOnly([
{ offset: 0, size: 512, _success: true }, // offset 0 is valid
{ offset: 256, size: 256, _success: true }, // offset 256 (aligned) is valid
{ bindBufferResource: true, _success: true }, // full buffer is valid

// Touching the end of the buffer
{ offset: 0, size: 1024, _success: true },
{ offset: 0, size: undefined, _success: true },
{ offset: 256 * 3, size: 256, _success: true },
{ offset: 256 * 3, size: undefined, _success: true },

// Zero-sized bindings
{ offset: 0, size: 0, _success: false },
{ offset: 256, size: 0, _success: false },
{ offset: 1024, size: 0, _success: false },
{ offset: 1024, size: undefined, _success: false },

// Unaligned buffer offset is invalid
{ offset: 1, size: 256, _success: false },
{ offset: 1, size: undefined, _success: false },
{ offset: 127, size: 256, _success: false },
{ offset: 255, size: 256, _success: false },

// Out-of-bounds
{ offset: 256 * 5, size: 0, _success: false }, // offset is OOB
{ offset: 0, size: 256 * 5, _success: false }, // size is OOB
{ offset: 1024, size: 1, _success: false } // offset+size is OOB
]).
fn((t) => {
  const { bindBufferResource, offset, size, _success } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [{ binding: 0, visibility: GPUShaderStage.COMPUTE, buffer: { type: 'storage' } }]
  });

  const buffer = t.createBufferTracked({
    size: 1024,
    usage: GPUBufferUsage.STORAGE
  });

  const resource = bindBufferResource ? buffer : { buffer, offset, size };
  const descriptor = {
    entries: [
    {
      binding: 0,
      resource
    }],

    layout: bindGroupLayout
  };

  if (_success) {
    // Control case
    t.device.createBindGroup(descriptor);
  } else {
    // Buffer offset and/or size don't match in bind groups.
    t.expectValidationError(() => {
      t.device.createBindGroup(descriptor);
    });
  }
});

g.test('minBindingSize').
desc('Tests that minBindingSize is correctly enforced.').
paramsSubcasesOnly((u) =>
u //
.combine('minBindingSize', [undefined, 4, 8, 256]).
expand('size', ({ minBindingSize }) =>
minBindingSize !== undefined ?
[minBindingSize - 4, minBindingSize, minBindingSize + 4] :
[4, 256]
)
).
fn((t) => {
  const { size, minBindingSize } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      buffer: {
        type: 'storage',
        minBindingSize
      }
    }]

  });

  const storageBuffer = t.createBufferTracked({
    size,
    usage: GPUBufferUsage.STORAGE
  });

  t.expectValidationError(
    () => {
      t.device.createBindGroup({
        layout: bindGroupLayout,
        entries: [
        {
          binding: 0,
          resource: { buffer: storageBuffer }
        }]

      });
    },
    minBindingSize !== undefined && size < minBindingSize
  );
});

const kAllShaderStages =
GPUConst.ShaderStage.COMPUTE | GPUConst.ShaderStage.FRAGMENT | GPUConst.ShaderStage.VERTEX;

g.test('buffer,resource_state').
desc('Test bind group creation with various buffer resource states').
paramsSubcasesOnly((u) =>
u.
combine('state', kResourceStates).
combine('entry', bufferBindingEntries(true)).
combine('visibilityMask', [kAllShaderStages, GPUConst.ShaderStage.COMPUTE]).
combine('bindBufferResource', [false, true])
).
fn((t) => {
  const { state, entry, visibilityMask, bindBufferResource } = t.params;

  assert(entry.buffer !== undefined);
  const info = bufferBindingTypeInfo(entry.buffer);

  const visibility = info.validStages & visibilityMask;
  skipIfResourceNotSupportedInStages(t, entry, visibility);

  const bgl = t.device.createBindGroupLayout({
    entries: [
    {
      ...entry,
      binding: 0,
      visibility
    }]

  });

  const buffer = vtu.createBufferWithState(t, state, {
    usage: info.usage,
    size: 4
  });

  const resource = bindBufferResource ? buffer : { buffer };
  t.expectValidationError(() => {
    t.device.createBindGroup({
      layout: bgl,
      entries: [
      {
        binding: 0,
        resource
      }]

    });
  }, state === 'invalid');
});

g.test('texture,resource_state').
desc('Test bind group creation with various texture resource states').
paramsSubcasesOnly((u) =>
u.
combine('state', kResourceStates).
combine('entry', sampledAndStorageBindingEntries(true, kTestFormat)).
combine('visibilityMask', [kAllShaderStages, GPUConst.ShaderStage.COMPUTE]).
combine('bindTextureResource', [false, true])
).
fn((t) => {
  const { state, entry, visibilityMask, bindTextureResource } = t.params;
  const info = texBindingTypeInfo(entry);

  const visibility = info.validStages & visibilityMask;
  skipIfResourceNotSupportedInStages(t, entry, visibility);

  const bgl = t.device.createBindGroupLayout({
    entries: [
    {
      ...entry,
      binding: 0,
      visibility
    }]

  });

  // The `RENDER_ATTACHMENT` usage must be specified if sampleCount > 1 according to WebGPU SPEC.
  const usage = entry.texture?.multisampled ?
  info.usage | GPUConst.TextureUsage.RENDER_ATTACHMENT :
  info.usage;
  const format = entry.storageTexture !== undefined ? 'r32float' : 'rgba8unorm';
  const texture = vtu.createTextureWithState(t, state, {
    usage,
    size: [1, 1],
    format,
    sampleCount: entry.texture?.multisampled ? 4 : 1
  });

  let resource;
  if (bindTextureResource) {
    resource = texture;
  } else {
    t.expectValidationError(() => {
      resource = texture.createView();
    }, state === 'invalid');
  }

  t.expectValidationError(() => {
    t.device.createBindGroup({
      layout: bgl,
      entries: [{ binding: 0, resource }]
    });
  }, state === 'invalid');
});

g.test('bind_group_layout,device_mismatch').
desc(
  'Tests createBindGroup cannot be called with a bind group layout created from another device'
).
paramsSubcasesOnly((u) => u.combine('mismatched', [true, false])).
beforeAllSubcases((t) => t.usesMismatchedDevice()).
fn((t) => {
  const mismatched = t.params.mismatched;

  const sourceDevice = mismatched ? t.mismatchedDevice : t.device;

  const bgl = sourceDevice.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUConst.ShaderStage.COMPUTE,
      buffer: {}
    }]

  });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      layout: bgl,
      entries: [
      {
        binding: 0,
        resource: { buffer: vtu.getUniformBuffer(t) }
      }]

    });
  }, mismatched);
});

g.test('binding_resources,device_mismatch').
desc(
  `
    Tests createBindGroup cannot be called with various resources created from another device
    Test with two resources to make sure all resources can be validated:
    - resource0 and resource1 from same device
    - resource0 and resource1 from different device

    TODO: test GPUExternalTexture as a resource
    `
).
params((u) =>
u.
combine('entry', [
{ buffer: { type: 'storage' } },
{ sampler: { type: 'filtering' } },
{ texture: { multisampled: false } },
{ storageTexture: { access: 'write-only', format: 'r32float' } },
{ storageTexture: { access: 'read-only', format: 'r32float' } },
{ storageTexture: { access: 'read-write', format: 'r32float' } }]
).
beginSubcases().
combineWithParams([
{ resource0Mismatched: false, resource1Mismatched: false }, //control case
{ resource0Mismatched: true, resource1Mismatched: false },
{ resource0Mismatched: false, resource1Mismatched: true }]
).
combine('visibilityMask', [kAllShaderStages, GPUConst.ShaderStage.COMPUTE])
).
beforeAllSubcases((t) => t.usesMismatchedDevice()).
fn((t) => {
  const { entry, resource0Mismatched, resource1Mismatched, visibilityMask } = t.params;

  const info = bindingTypeInfo(entry);
  const visibility = info.validStages & visibilityMask;
  skipIfResourceNotSupportedInStages(t, entry, visibility);

  const resource0 = resource0Mismatched ?
  vtu.getDeviceMismatchedBindingResource(t, info.resource) :
  vtu.getBindingResource(t, info.resource);
  const resource1 = resource1Mismatched ?
  vtu.getDeviceMismatchedBindingResource(t, info.resource) :
  vtu.getBindingResource(t, info.resource);

  const bgl = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility,
      ...entry
    },
    {
      binding: 1,
      visibility,
      ...entry
    }]

  });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      layout: bgl,
      entries: [
      {
        binding: 0,
        resource: resource0
      },
      {
        binding: 1,
        resource: resource1
      }]

    });
  }, resource0Mismatched || resource1Mismatched);
});

g.test('storage_texture,usage').
desc(
  `
    Test that the texture usage contains STORAGE_BINDING if the BindGroup entry defines
    storageTexture.
  `
).
params((u) =>
u //
// If usage0 and usage1 are the same, the usage being test is a single usage. Otherwise, it's
// a combined usage.
.combine('usage0', kTextureUsages).
combine('usage1', kTextureUsages).
unless(({ usage0, usage1 }) => {
  return !isValidTextureUsageCombination(usage0 | usage1);
})
).
fn((t) => {
  const { usage0, usage1 } = t.params;

  const usage = usage0 | usage1;

  // MAINTENANCE_TODO(#4509): Remove this after all implementations have TRANSIENT_ATTACHMENT.
  if ((usage & GPUConst.TextureUsage.TRANSIENT_ATTACHMENT) !== 0) {
    t.skipIfTransientAttachmentNotSupported();
  }

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      storageTexture: { access: 'write-only', format: 'rgba8unorm' }
    }]

  });

  const texture = t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage
  });

  const isValid = GPUTextureUsage.STORAGE_BINDING & usage;

  const textureView = texture.createView();
  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('storage_texture,mip_level_count').
desc(
  `
    Test that the mip level count of the resource of the BindGroup entry as a descriptor is 1 if the
    BindGroup entry defines storageTexture. If the mip level count is not 1, a validation error
    should be generated.
  `
).
params((u) =>
u //
.combine('baseMipLevel', [1, 2]).
combine('mipLevelCount', [1, 2])
).
fn((t) => {
  const { baseMipLevel, mipLevelCount } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      storageTexture: { access: 'write-only', format: 'rgba8unorm' }
    }]

  });

  const MIP_LEVEL_COUNT = 4;
  const texture = t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.STORAGE_BINDING,
    mipLevelCount: MIP_LEVEL_COUNT
  });

  const textureView = texture.createView({ baseMipLevel, mipLevelCount });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, mipLevelCount !== 1);
});

g.test('storage_texture,format').
desc(
  `
    Test that the format of the storage texture is equal to resource's descriptor format if the
    BindGroup entry defines storageTexture.
  `
).
params((u) =>
u //
.combine('storageTextureFormat', kPossibleStorageTextureFormats).
combine('resourceFormat', kPossibleStorageTextureFormats)
).
fn((t) => {
  const { storageTextureFormat, resourceFormat } = t.params;
  t.skipIfTextureFormatNotSupported(storageTextureFormat, resourceFormat);
  t.skipIfTextureFormatNotUsableWithStorageAccessMode(
    'write-only',
    storageTextureFormat,
    resourceFormat
  );

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      storageTexture: { access: 'write-only', format: storageTextureFormat }
    }]

  });

  const texture = t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: resourceFormat,
    usage: GPUTextureUsage.STORAGE_BINDING
  });

  const isValid = storageTextureFormat === resourceFormat;
  const textureView = texture.createView({ format: resourceFormat });
  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('buffer,usage').
desc(
  `
    Test that the buffer usage contains 'UNIFORM' if the BindGroup entry defines buffer and it's
    type is 'uniform', and the buffer usage contains 'STORAGE' if the BindGroup entry's buffer type
    is 'storage'|read-only-storage'.
  `
).
params((u) =>
u //
.combine('type', kBufferBindingTypes)
// If usage0 and usage1 are the same, the usage being test is a single usage. Otherwise, it's
// a combined usage.
.beginSubcases().
combine('usage0', kBufferUsages).
combine('usage1', kBufferUsages).
unless(
  ({ usage0, usage1 }) =>
  ((usage0 | usage1) & (GPUConst.BufferUsage.MAP_READ | GPUConst.BufferUsage.MAP_WRITE)) !==
  0
)
).
fn((t) => {
  const { type, usage0, usage1 } = t.params;

  const usage = usage0 | usage1;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      buffer: { type }
    }]

  });

  const buffer = t.createBufferTracked({
    size: 4,
    usage
  });

  let isValid = false;
  if (type === 'uniform') {
    isValid = GPUBufferUsage.UNIFORM & usage ? true : false;
  } else if (type === 'storage' || type === 'read-only-storage') {
    isValid = GPUBufferUsage.STORAGE & usage ? true : false;
  }

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: { buffer } }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('buffer,resource_offset').
desc(
  `
    Test that the resource.offset of the BindGroup entry is a multiple of limits.
    'minUniformBufferOffsetAlignment|minStorageBufferOffsetAlignment' if the BindGroup entry defines
    buffer and the buffer type is 'uniform|storage|read-only-storage'.
  `
).
params((u) =>
u //
.combine('type', kBufferBindingTypes).
beginSubcases().
combine('offsetAddMult', [
{ add: 0, mult: 0 },
{ add: 0, mult: 0.5 },
{ add: 0, mult: 1.5 },
{ add: 2, mult: 0 }]
)
).
fn((t) => {
  const { type, offsetAddMult } = t.params;
  const minAlignment =
  t.device.limits[
  type === 'uniform' ? 'minUniformBufferOffsetAlignment' : 'minStorageBufferOffsetAlignment'];

  const offset = makeValueTestVariant(minAlignment, offsetAddMult);

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      buffer: { type }
    }]

  });

  const usage = type === 'uniform' ? GPUBufferUsage.UNIFORM : GPUBufferUsage.STORAGE;
  const isValid = offset % minAlignment === 0;

  const buffer = t.createBufferTracked({
    size: 1024,
    usage
  });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: { buffer, offset } }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('buffer,resource_binding_size').
desc(
  `
    Test that the buffer binding size of the BindGroup entry is equal to or less than limits.
    'maxUniformBufferBindingSize|maxStorageBufferBindingSize' if the BindGroup entry defines
    buffer and the buffer type is 'uniform|storage|read-only-storage'.
  `
).
params((u) =>
u.
combine('type', kBufferBindingTypes).
beginSubcases()
// Test a size of 1 (for uniform buffer) or 4 (for storage and read-only storage buffer)
// then values just within and just above the limit.
.combine('bindingSize', [
{ base: 1, limit: 0 },
{ base: 0, limit: 1 },
{ base: 1, limit: 1 }]
)
).
fn((t) => {
  const {
    type,
    bindingSize: { base, limit }
  } = t.params;
  const mult = type === 'uniform' ? 1 : 4;
  const maxBindingSize =
  t.device.limits[
  type === 'uniform' ? 'maxUniformBufferBindingSize' : 'maxStorageBufferBindingSize'];

  const bindingSize = base * mult + maxBindingSize * limit;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      buffer: { type }
    }]

  });

  const usage = type === 'uniform' ? GPUBufferUsage.UNIFORM : GPUBufferUsage.STORAGE;
  const isValid = bindingSize <= maxBindingSize;

  // MAINTENANCE_TODO: Allocating the max size seems likely to fail. Refactor test.
  const buffer = t.createBufferTracked({
    size: maxBindingSize,
    usage
  });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: { buffer, size: bindingSize } }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('buffer,effective_buffer_binding_size').
desc(
  `
  Test that the effective buffer binding size of the BindGroup entry must be a multiple of 4 if the
  buffer type is 'storage|read-only-storage', while there is no such restriction on uniform buffers.
`
).
params((u) =>
u.
combine('type', kBufferBindingTypes).
beginSubcases().
combine('offsetMult', [0, 1]).
combine('bufferSizeAddition', [8, 10]).
combine('bindingSize', [undefined, 2, 4, 6])
).
fn((t) => {
  const { type, offsetMult, bufferSizeAddition, bindingSize } = t.params;
  const minAlignment =
  t.device.limits[
  type === 'uniform' ? 'minUniformBufferOffsetAlignment' : 'minStorageBufferOffsetAlignment'];

  const offset = minAlignment * offsetMult;
  const bufferSize = minAlignment + bufferSizeAddition;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      buffer: { type }
    }]

  });

  const effectiveBindingSize = bindingSize ?? bufferSize - offset;
  let usage, isValid;
  if (type === 'uniform') {
    usage = GPUBufferUsage.UNIFORM;
    isValid = true;
  } else {
    usage = GPUBufferUsage.STORAGE;
    isValid = effectiveBindingSize % 4 === 0;
  }

  const buffer = t.createBufferTracked({
    size: bufferSize,
    usage
  });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: { buffer, offset, size: bindingSize } }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('sampler,device_mismatch').
desc(`Tests createBindGroup cannot be called with a sampler created from another device.`).
paramsSubcasesOnly((u) => u.combine('mismatched', [true, false])).
beforeAllSubcases((t) => t.usesMismatchedDevice()).
fn((t) => {
  const { mismatched } = t.params;

  const sourceDevice = mismatched ? t.mismatchedDevice : t.device;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      sampler: { type: 'filtering' }
    }]

  });

  const sampler = sourceDevice.createSampler();
  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: sampler }],
      layout: bindGroupLayout
    });
  }, mismatched);
});

g.test('sampler,compare_function_with_binding_type').
desc(
  `
  Test that the sampler of the BindGroup has a 'compareFunction' value if the sampler type of the
  BindGroupLayout is 'comparison'. Other sampler types should not have 'compare' field in
  the descriptor of the sampler.
  `
).
params((u) =>
u //
.combine('bgType', kSamplerBindingTypes).
beginSubcases().
combine('compareFunction', [undefined, ...kCompareFunctions])
).
fn((t) => {
  const { bgType, compareFunction } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.COMPUTE,
      sampler: { type: bgType }
    }]

  });

  const isValid =
  bgType === 'comparison' ? compareFunction !== undefined : compareFunction === undefined;

  const sampler = t.device.createSampler({ compare: compareFunction });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: sampler }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('external_texture,texture_view,usage').
desc(
  `
    Test that the external texture usage contains TEXTURE_BINDING if the BindGroup entry defines
    externalTexture for a GPUTextureView resource.
  `
).
params((u) =>
u //
// If usage0 and usage1 are the same, the usage being test is a single usage. Otherwise, it's
// a combined usage.
.combine('usage0', kTextureUsages).
combine('usage1', kTextureUsages).
unless(({ usage0, usage1 }) => {
  return !isValidTextureUsageCombination(usage0 | usage1);
})
).
fn((t) => {
  const { usage0, usage1 } = t.params;

  const usage = usage0 | usage1;

  // MAINTENANCE_TODO(#4509): Remove this after all implementations have TRANSIENT_ATTACHMENT.
  if ((usage & GPUConst.TextureUsage.TRANSIENT_ATTACHMENT) !== 0) {
    t.skipIfTransientAttachmentNotSupported();
  }

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.FRAGMENT,
      externalTexture: {}
    }]

  });

  const texture = t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage
  });

  const isValid = GPUTextureUsage.TEXTURE_BINDING & usage;

  const textureView = texture.createView();
  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('external_texture,texture_view,dimension').
desc(
  `
    Test that the external texture dimension is 2d if the BindGroup entry defines
    externalTexture for a GPUTextureView resource.
  `
).
params((u) => u.combine('dimension', kTextureViewDimensions)).
fn((t) => {
  const { dimension } = t.params;
  t.skipIfTextureViewDimensionNotSupported(dimension);

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.FRAGMENT,
      externalTexture: {}
    }]

  });

  let height = 16;
  let depthOrArrayLayers = 6;
  if (dimension === '1d') {
    height = 1;
    depthOrArrayLayers = 1;
  }

  const texture = t.createTextureTracked({
    size: { width: 16, height, depthOrArrayLayers },
    format: 'rgba8unorm',
    usage: GPUConst.TextureUsage.TEXTURE_BINDING,
    dimension: getTextureDimensionFromView(dimension)
  });

  const shouldError = dimension !== '2d';
  const textureView = texture.createView({ dimension });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, shouldError);
});

g.test('external_texture,texture_view,mip_level_count').
desc(
  `
    Test that the mip level count of the resource of the BindGroup entry as a descriptor is 1 if the
    BindGroup entry defines externalTexture for a GPUTextureView resource. If the mip level count is
    not 1, a validation error should be generated.
  `
).
params((u) =>
u //
.combine('baseMipLevel', [1, 2]).
combine('mipLevelCount', [1, 2])
).
fn((t) => {
  const { baseMipLevel, mipLevelCount } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.FRAGMENT,
      externalTexture: {}
    }]

  });

  const MIP_LEVEL_COUNT = 4;
  const texture = t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.TEXTURE_BINDING,
    mipLevelCount: MIP_LEVEL_COUNT
  });

  const textureView = texture.createView({ baseMipLevel, mipLevelCount });

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, mipLevelCount !== 1);
});

g.test('external_texture,texture_view,format').
desc(
  `
    Test that the format of the external texture is "rgba8unorm", "bgra8unorm", or "rgba16float"
    if the BindGroup entry defines externalTexture for a GPUTextureView resource.
  `
).
params((u) => u.combine('format', kRegularTextureFormats)).
fn((t) => {
  const { format } = t.params;
  t.skipIfTextureFormatNotSupported(format);

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.FRAGMENT,
      externalTexture: {}
    }]

  });

  const texture = t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format,
    usage: GPUTextureUsage.TEXTURE_BINDING
  });

  const isValid = format === 'rgba8unorm' || format === 'bgra8unorm' || format === 'rgba16float';
  const textureView = texture.createView({ format });
  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, !isValid);
});

g.test('external_texture,texture_view,sample_count').
desc(
  `
  Test that the sample count of the resource of the BindGroup entry as a texture is 1 if the
  BindGroup entry defines externalTexture for a GPUTextureView resource. If the sample count is
  not 1, a validation error should be generated.
`
).
params((u) => u.combine('sampleCount', [1, 4])).
fn((t) => {
  const { sampleCount } = t.params;

  const bindGroupLayout = t.device.createBindGroupLayout({
    entries: [
    {
      binding: 0,
      visibility: GPUShaderStage.FRAGMENT,
      externalTexture: {}
    }]

  });

  const texture = t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.RENDER_ATTACHMENT,
    sampleCount
  });

  const textureView = texture.createView();

  t.expectValidationError(() => {
    t.device.createBindGroup({
      entries: [{ binding: 0, resource: textureView }],
      layout: bindGroupLayout
    });
  }, sampleCount !== 1);
});
});
__define('webgpu/api/validation/validation_test_utils.js', function (__exports) {
'use strict';
__get(__exports, 'createTextureWithState', () => createTextureWithState);
__get(__exports, 'createBufferWithState', () => createBufferWithState);
__get(__exports, 'createQuerySetWithState', () => createQuerySetWithState);
__get(__exports, 'getStorageBuffer', () => getStorageBuffer);
__get(__exports, 'getUniformBuffer', () => getUniformBuffer);
__get(__exports, 'getErrorBuffer', () => getErrorBuffer);
__get(__exports, 'getErrorSampler', () => getErrorSampler);
__get(__exports, 'getSampledTexture', () => getSampledTexture);
__get(__exports, 'getRenderTexture', () => getRenderTexture);
__get(__exports, 'getErrorTexture', () => getErrorTexture);
__get(__exports, 'getErrorTextureView', () => getErrorTextureView);
__get(__exports, 'getBindingResource', () => getBindingResource);
__get(__exports, 'getDeviceMismatchedStorageBuffer', () => getDeviceMismatchedStorageBuffer);
__get(__exports, 'getDeviceMismatchedUniformBuffer', () => getDeviceMismatchedUniformBuffer);
__get(__exports, 'getDeviceMismatchedTexture', () => getDeviceMismatchedTexture);
__get(__exports, 'getDeviceMismatchedSampledTexture', () => getDeviceMismatchedSampledTexture);
__get(__exports, 'getDeviceMismatchedStorageTexture', () => getDeviceMismatchedStorageTexture);
__get(__exports, 'getDeviceMismatchedRenderTexture', () => getDeviceMismatchedRenderTexture);
__get(__exports, 'getDeviceMismatchedBindingResource', () => getDeviceMismatchedBindingResource);
__get(__exports, 'getNoOpShaderCode', () => getNoOpShaderCode);
__get(__exports, 'createRenderPipelineWithState', () => createRenderPipelineWithState);
__get(__exports, 'createNoOpRenderPipeline', () => createNoOpRenderPipeline);
__get(__exports, 'createErrorRenderPipeline', () => createErrorRenderPipeline);
__get(__exports, 'createNoOpComputePipeline', () => createNoOpComputePipeline);
__get(__exports, 'createErrorComputePipeline', () => createErrorComputePipeline);
__get(__exports, 'createInvalidShaderModule', () => createInvalidShaderModule);
__get(__exports, 'doCreateRenderPipelineTest', () => doCreateRenderPipelineTest);
__get(__exports, 'doCreateComputePipelineTest', () => doCreateComputePipelineTest);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let kMaxQueryCount; __bind('webgpu/capability_info.js', (__m) => { ({ kMaxQueryCount } = __m); });


function createTextureWithState(
t,
state,
descriptor)
{
  descriptor = descriptor ?? {
    size: { width: 1, height: 1, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage:
    GPUTextureUsage.COPY_SRC |
    GPUTextureUsage.COPY_DST |
    GPUTextureUsage.TEXTURE_BINDING |
    GPUTextureUsage.STORAGE_BINDING |
    GPUTextureUsage.RENDER_ATTACHMENT
  };

  switch (state) {
    case 'valid':
      return t.createTextureTracked(descriptor);
    case 'invalid':
      return getErrorTexture(t);
    case 'destroyed':{
        const texture = t.createTextureTracked(descriptor);
        texture.destroy();
        return texture;
      }
  }
}

/**
 * Create a GPUTexture in the specified state. A `descriptor` may optionally be passed;
 * if `state` is `'invalid'`, it will be modified to add an invalid combination of usages.
 */
function createBufferWithState(
t,
state,
descriptor)
{
  descriptor = descriptor ?? {
    size: 4,
    usage: GPUBufferUsage.VERTEX
  };

  switch (state) {
    case 'valid':
      return t.createBufferTracked(descriptor);

    case 'invalid':{
        // Make the buffer invalid because of an invalid combination of usages but keep the
        // descriptor passed as much as possible (for mappedAtCreation and friends).
        t.device.pushErrorScope('validation');
        const buffer = t.createBufferTracked({
          ...descriptor,
          usage: descriptor.usage | GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_SRC
        });
        void t.device.popErrorScope();
        return buffer;
      }
    case 'destroyed':{
        const buffer = t.createBufferTracked(descriptor);
        buffer.destroy();
        return buffer;
      }
  }
}

/**
 * Create a GPUQuerySet in the specified state.
 * A `descriptor` may optionally be passed, which is used when `state` is not `'invalid'`.
 */
function createQuerySetWithState(
t,
state,
desc)
{
  const descriptor = { type: 'occlusion', count: 2, ...desc };

  switch (state) {
    case 'valid':
      return t.createQuerySetTracked(descriptor);
    case 'invalid':{
        // Make the queryset invalid because of the count out of bounds.
        descriptor.count = kMaxQueryCount + 1;
        return t.expectGPUError('validation', () => t.createQuerySetTracked(descriptor));
      }
    case 'destroyed':{
        const queryset = t.createQuerySetTracked(descriptor);
        queryset.destroy();
        return queryset;
      }
  }
}

/** Create an arbitrarily-sized GPUBuffer with the STORAGE usage. */
function getStorageBuffer(t) {
  return t.createBufferTracked({ size: 1024, usage: GPUBufferUsage.STORAGE });
}

/** Create an arbitrarily-sized GPUBuffer with the UNIFORM usage. */
function getUniformBuffer(t) {
  return t.createBufferTracked({ size: 1024, usage: GPUBufferUsage.UNIFORM });
}

/** Return an invalid GPUBuffer. */
function getErrorBuffer(t) {
  return createBufferWithState(t, 'invalid');
}

/** Return an invalid GPUSampler. */
function getErrorSampler(t) {
  t.device.pushErrorScope('validation');
  const sampler = t.device.createSampler({ lodMinClamp: -1 });
  void t.device.popErrorScope();
  return sampler;
}

/**
 * Return an arbitrarily-configured GPUTexture with the `TEXTURE_BINDING` usage and specified
 * sampleCount. The `RENDER_ATTACHMENT` usage will also be specified if sampleCount > 1 as is
 * required by WebGPU SPEC.
 */
function getSampledTexture(t, sampleCount = 1) {
  const usage =
  sampleCount > 1 ?
  GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.RENDER_ATTACHMENT :
  GPUTextureUsage.TEXTURE_BINDING;
  return t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage,
    sampleCount
  });
}

/** Return an arbitrarily-configured GPUTexture with the `STORAGE_BINDING` usage. */
function getStorageTexture(t, format) {
  return t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format,
    usage: GPUTextureUsage.STORAGE_BINDING
  });
}

/** Return an arbitrarily-configured GPUTexture with the `RENDER_ATTACHMENT` usage. */
function getRenderTexture(t, sampleCount = 1) {
  return t.createTextureTracked({
    size: { width: 16, height: 16, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.RENDER_ATTACHMENT,
    sampleCount
  });
}

/** Return an invalid GPUTexture. */
function getErrorTexture(t) {
  t.device.pushErrorScope('validation');
  const texture = t.createTextureTracked({
    size: { width: 0, height: 0, depthOrArrayLayers: 0 },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.TEXTURE_BINDING
  });
  void t.device.popErrorScope();
  return texture;
}

/** Return an invalid GPUTextureView (created from an invalid GPUTexture). */
function getErrorTextureView(t) {
  t.device.pushErrorScope('validation');
  const view = getErrorTexture(t).createView();
  void t.device.popErrorScope();
  return view;
}

/**
 * Return an arbitrary object of the specified {@link webgpu/capability_info!BindableResource} type
 * (e.g. `'errorBuf'`, `'nonFiltSamp'`, `sampledTexMS`, etc.)
 */
function getBindingResource(t, bindingType) {
  switch (bindingType) {
    case 'errorBuf':
      return { buffer: getErrorBuffer(t) };
    case 'errorSamp':
      return getErrorSampler(t);
    case 'errorTex':
      return getErrorTextureView(t);
    case 'uniformBuf':
      return { buffer: getUniformBuffer(t) };
    case 'storageBuf':
      return { buffer: getStorageBuffer(t) };
    case 'filtSamp':
      return t.device.createSampler({ minFilter: 'linear' });
    case 'nonFiltSamp':
      return t.device.createSampler();
    case 'compareSamp':
      return t.device.createSampler({ compare: 'never' });
    case 'sampledTex':
      return getSampledTexture(t, 1).createView();
    case 'sampledTexMS':
      return getSampledTexture(t, 4).createView();
    case 'readonlyStorageTex':
    case 'writeonlyStorageTex':
    case 'readwriteStorageTex':
      return getStorageTexture(t, 'r32float').createView();
  }
}

/** Create an arbitrarily-sized GPUBuffer with the STORAGE usage from mismatched device. */
function getDeviceMismatchedStorageBuffer(t) {
  return t.trackForCleanup(
    t.mismatchedDevice.createBuffer({ size: 4, usage: GPUBufferUsage.STORAGE })
  );
}

/** Create an arbitrarily-sized GPUBuffer with the UNIFORM usage from mismatched device. */
function getDeviceMismatchedUniformBuffer(t) {
  return t.trackForCleanup(
    t.mismatchedDevice.createBuffer({ size: 4, usage: GPUBufferUsage.UNIFORM })
  );
}

/** Return a GPUTexture with descriptor from mismatched device. */
function getDeviceMismatchedTexture(
t,
descriptor)
{
  return t.trackForCleanup(t.mismatchedDevice.createTexture(descriptor));
}

/** Return an arbitrarily-configured GPUTexture with the `SAMPLED` usage from mismatched device. */
function getDeviceMismatchedSampledTexture(t, sampleCount = 1) {
  return getDeviceMismatchedTexture(t, {
    size: { width: 4, height: 4, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.TEXTURE_BINDING,
    sampleCount
  });
}

/** Return an arbitrarily-configured GPUTexture with the `STORAGE` usage from mismatched device. */
function getDeviceMismatchedStorageTexture(
t,
format)
{
  return getDeviceMismatchedTexture(t, {
    size: { width: 4, height: 4, depthOrArrayLayers: 1 },
    format,
    usage: GPUTextureUsage.STORAGE_BINDING
  });
}

/** Return an arbitrarily-configured GPUTexture with the `RENDER_ATTACHMENT` usage from mismatched device. */
function getDeviceMismatchedRenderTexture(t, sampleCount = 1) {
  return getDeviceMismatchedTexture(t, {
    size: { width: 4, height: 4, depthOrArrayLayers: 1 },
    format: 'rgba8unorm',
    usage: GPUTextureUsage.RENDER_ATTACHMENT,
    sampleCount
  });
}

function getDeviceMismatchedBindingResource(
t,
bindingType)
{
  switch (bindingType) {
    case 'uniformBuf':
      return { buffer: getDeviceMismatchedUniformBuffer(t) };
    case 'storageBuf':
      return { buffer: getDeviceMismatchedStorageBuffer(t) };
    case 'filtSamp':
      return t.mismatchedDevice.createSampler({ minFilter: 'linear' });
    case 'nonFiltSamp':
      return t.mismatchedDevice.createSampler();
    case 'compareSamp':
      return t.mismatchedDevice.createSampler({ compare: 'never' });
    case 'sampledTex':
      return getDeviceMismatchedSampledTexture(t, 1).createView();
    case 'sampledTexMS':
      return getDeviceMismatchedSampledTexture(t, 4).createView();
    case 'readonlyStorageTex':
    case 'writeonlyStorageTex':
    case 'readwriteStorageTex':
      return getDeviceMismatchedStorageTexture(t, 'r32float').createView();
  }
}

/** Return a no-op shader code snippet for the specified shader stage. */
function getNoOpShaderCode(stage) {
  switch (stage) {
    case 'VERTEX':
      return `
        @vertex fn main() -> @builtin(position) vec4<f32> {
          return vec4<f32>();
        }
      `;
    case 'FRAGMENT':
      return `@fragment fn main() {}`;
    case 'COMPUTE':
      return `@compute @workgroup_size(1) fn main() {}`;
  }
}

/** Create a GPURenderPipeline in the specified state. */
function createRenderPipelineWithState(
t,
state)
{
  return state === 'valid' ? createNoOpRenderPipeline(t) : createErrorRenderPipeline(t);
}

/** Return a GPURenderPipeline with default options and no-op vertex and fragment shaders. */
function createNoOpRenderPipeline(
t,
layout = 'auto',
colorFormat = 'rgba8unorm')
{
  return t.device.createRenderPipeline({
    layout,
    vertex: {
      module: t.device.createShaderModule({
        code: getNoOpShaderCode('VERTEX')
      }),
      entryPoint: 'main'
    },
    fragment: {
      module: t.device.createShaderModule({
        code: getNoOpShaderCode('FRAGMENT')
      }),
      entryPoint: 'main',
      targets: [{ format: colorFormat, writeMask: 0 }]
    },
    primitive: { topology: 'triangle-list' }
  });
}

/** Return an invalid GPURenderPipeline. */
function createErrorRenderPipeline(t) {
  t.device.pushErrorScope('validation');
  const pipeline = t.device.createRenderPipeline({
    layout: 'auto',
    vertex: {
      module: t.device.createShaderModule({
        code: ''
      }),
      entryPoint: ''
    }
  });
  void t.device.popErrorScope();
  return pipeline;
}

/** Return a GPUComputePipeline with a no-op shader. */
function createNoOpComputePipeline(
t,
layout = 'auto')
{
  return t.device.createComputePipeline({
    layout,
    compute: {
      module: t.device.createShaderModule({
        code: getNoOpShaderCode('COMPUTE')
      }),
      entryPoint: 'main'
    }
  });
}

/** Return an invalid GPUComputePipeline. */
function createErrorComputePipeline(t) {
  t.device.pushErrorScope('validation');
  const pipeline = t.device.createComputePipeline({
    layout: 'auto',
    compute: {
      module: t.device.createShaderModule({
        code: ''
      }),
      entryPoint: ''
    }
  });
  void t.device.popErrorScope();
  return pipeline;
}

/** Return an invalid GPUShaderModule. */
function createInvalidShaderModule(t) {
  t.device.pushErrorScope('validation');
  const code = 'deadbeaf'; // Something make no sense
  const shaderModule = t.device.createShaderModule({ code });
  void t.device.popErrorScope();
  return shaderModule;
}

/** Helper for testing createRenderPipeline(Async) validation */
function doCreateRenderPipelineTest(
t,
isAsync,
_success,
descriptor,
errorTypeName = 'GPUPipelineError')
{
  if (isAsync) {
    if (_success) {
      t.shouldResolve(t.device.createRenderPipelineAsync(descriptor));
    } else {
      t.shouldReject(errorTypeName, t.device.createRenderPipelineAsync(descriptor));
    }
  } else {
    if (errorTypeName === 'GPUPipelineError') {
      t.expectValidationError(() => {
        t.device.createRenderPipeline(descriptor);
      }, !_success);
    } else {
      t.shouldThrow(_success ? false : errorTypeName, () => {
        t.device.createRenderPipeline(descriptor);
      });
    }
  }
}

/** Helper for testing createComputePipeline(Async) validation */
function doCreateComputePipelineTest(
t,
isAsync,
_success,
descriptor,
errorTypeName = 'GPUPipelineError')
{
  if (isAsync) {
    if (_success) {
      t.shouldResolve(t.device.createComputePipelineAsync(descriptor));
    } else {
      t.shouldReject(errorTypeName, t.device.createComputePipelineAsync(descriptor));
    }
  } else {
    if (errorTypeName === 'GPUPipelineError') {
      t.expectValidationError(() => {
        t.device.createComputePipeline(descriptor);
      }, !_success);
    } else {
      t.shouldThrow(_success ? false : errorTypeName, () => {
        t.device.createComputePipeline(descriptor);
      });
    }
  }
}
});
__define('webgpu/api/validation/error_scope.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Error scope validation tests.

Note these must create their own device, not use GPUTest (that one already has error scopes on it).

TODO: (POSTV1) Test error scopes of different threads and make sure they go to the right place.
TODO: (POSTV1) Test that unhandled errors go the right device, and nowhere if the device was dropped.
`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let kErrorScopeFilters, kGeneratableErrorScopeFilters; __bind('webgpu/capability_info.js', (__m) => { ({ kErrorScopeFilters, kGeneratableErrorScopeFilters } = __m); });
let ErrorTest; __bind('webgpu/error_test.js', (__m) => { ({ ErrorTest } = __m); });

const g = makeTestGroup(ErrorTest);

g.test('simple').
desc(
  `
Tests that error scopes catches their expected errors, firing an uncaptured error event otherwise.

- Same error and error filter (popErrorScope should return the error)
- Different error from filter (uncaptured error should result)
    `
).
params((u) =>
u.combine('errorType', kGeneratableErrorScopeFilters).combine('errorFilter', kErrorScopeFilters)
).
fn(async (t) => {
  const { errorType, errorFilter } = t.params;
  t.device.pushErrorScope(errorFilter);

  if (errorType !== errorFilter) {
    // Different error case
    const uncapturedErrorEvent = await t.expectUncapturedError(() => {
      t.generateError(errorType);
    });
    t.expect(t.isInstanceOfError(errorType, uncapturedErrorEvent.error));

    const error = await t.device.popErrorScope();
    t.expect(error === null);
  } else {
    // Same error as filter
    t.generateError(errorType);
    const error = await t.device.popErrorScope();
    t.expect(t.isInstanceOfError(errorType, error));
  }
});

g.test('empty').
desc(
  `
Tests that popping an empty error scope stack should reject.
    `
).
fn((t) => {
  const promise = t.device.popErrorScope();
  t.shouldReject('OperationError', promise, { allowMissingStack: true });
});

g.test('parent_scope').
desc(
  `
Tests that an error bubbles to the correct parent scope.

- Different error types as the parent scope
- Different depths of non-capturing filters for the generated error
    `
).
params((u) =>
u.
combine('errorFilter', kGeneratableErrorScopeFilters).
combine('stackDepth', [1, 10, 100, 1000])
).
fn(async (t) => {
  const { errorFilter, stackDepth } = t.params;
  t.device.pushErrorScope(errorFilter);

  // Push a bunch of error filters onto the stack (none that match errorFilter)
  const unmatchedFilters = kErrorScopeFilters.filter((filter) => {
    return filter !== errorFilter;
  });
  for (let i = 0; i < stackDepth; i++) {
    t.device.pushErrorScope(unmatchedFilters[i % unmatchedFilters.length]);
  }

  // Cause the error and then pop all the unrelated filters.
  t.generateError(errorFilter);

  await t.chunkedPopManyErrorScopes(stackDepth);

  // Finally the actual error should have been caught by the parent scope.
  const error = await t.device.popErrorScope();
  t.expect(t.isInstanceOfError(errorFilter, error));
});

g.test('current_scope').
desc(
  `
Tests that an error does not bubbles to parent scopes when local scope matches.

- Different error types as the current scope
- Different depths of non-capturing filters for the generated error
    `
).
params((u) =>
u.
combine('errorFilter', kGeneratableErrorScopeFilters).
combine('stackDepth', [1, 10, 100, 1000, 100000])
).
fn(async (t) => {
  const { errorFilter, stackDepth } = t.params;

  // Push a bunch of error filters onto the stack
  for (let i = 0; i < stackDepth; i++) {
    t.device.pushErrorScope(kErrorScopeFilters[i % kErrorScopeFilters.length]);
  }

  // Current scope should catch the error immediately.
  t.device.pushErrorScope(errorFilter);
  t.generateError(errorFilter);
  const error = await t.device.popErrorScope();
  t.expect(t.isInstanceOfError(errorFilter, error));

  // Remaining scopes shouldn't catch anything.
  await t.chunkedPopManyErrorScopes(stackDepth);
});

g.test('balanced_siblings').
desc(
  `
Tests that sibling error scopes need to be balanced.

- Different error types as the current scope
- Different number of sibling errors
    `
).
params((u) =>
u.combine('errorFilter', kErrorScopeFilters).combine('numErrors', [1, 10, 100, 1000])
).
fn(async (t) => {
  const { errorFilter, numErrors } = t.params;

  const promises = [];
  for (let i = 0; i < numErrors; i++) {
    t.device.pushErrorScope(errorFilter);
    promises.push(t.device.popErrorScope());
  }

  {
    // Trying to pop an additional non-existing scope should reject.
    const promise = t.device.popErrorScope();
    t.shouldReject('OperationError', promise, { allowMissingStack: true });
  }

  const errors = await Promise.all(promises);
  t.expect(errors.every((e) => e === null));
});

g.test('balanced_nesting').
desc(
  `
Tests that nested error scopes need to be balanced.

- Different error types as the current scope
- Different number of nested errors
    `
).
params((u) =>
u.combine('errorFilter', kErrorScopeFilters).combine('numErrors', [1, 10, 100, 1000])
).
fn(async (t) => {
  const { errorFilter, numErrors } = t.params;

  for (let i = 0; i < numErrors; i++) {
    t.device.pushErrorScope(errorFilter);
  }

  const promises = [];
  for (let i = 0; i < numErrors; i++) {
    promises.push(t.device.popErrorScope());
  }
  const errors = await Promise.all(promises);
  t.expect(errors.every((e) => e === null));

  {
    // Trying to pop an additional non-existing scope should reject.
    const promise = t.device.popErrorScope();
    t.shouldReject('OperationError', promise, { allowMissingStack: true });
  }
});
});
__define('webgpu/error_test.js', function (__exports) {
'use strict';
__get(__exports, 'ErrorTest', () => ErrorTest);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let Fixture; __bind('common/framework/fixture.js', (__m) => { ({ Fixture } = __m); });let getGPU; __bind('common/util/navigator_gpu.js', (__m) => { ({ getGPU } = __m); });let assert, raceWithRejectOnTimeout; __bind('common/util/util.js', (__m) => { ({ assert, raceWithRejectOnTimeout } = __m); });

/**
 * A test class to help test error scopes and uncapturederror events.
 */
class ErrorTest extends Fixture {
  _device = undefined;

  get device() {
    assert(this._device !== undefined);
    return this._device;
  }

  async init() {
    await super.init();
    const gpu = getGPU(this.rec);
    const adapter = await gpu.requestAdapter();
    assert(adapter !== null);

    // We need to max out the adapter limits related to texture dimensions to more reliably cause an
    // OOM error when asked for it, so set that on the device now.
    const device = await this.requestDeviceTracked(adapter, {
      requiredLimits: {
        maxTextureDimension2D: adapter.limits.maxTextureDimension2D,
        maxTextureArrayLayers: adapter.limits.maxTextureArrayLayers
      }
    });
    assert(device !== null);
    this._device = device;
  }

  /**
   * Generates an error of the given filter type. For now, the errors are generated by calling a
   * known code-path to cause the error. This can be updated in the future should there be a more
   * direct way to inject errors.
   */
  generateError(filter) {
    switch (filter) {
      case 'out-of-memory':
        this.trackForCleanup(
          this.device.createTexture({
            // One of the largest formats. With the base limits, the texture will be 256 GiB.
            format: 'rgba32float',
            usage: GPUTextureUsage.COPY_DST,
            size: [
            this.device.limits.maxTextureDimension2D,
            this.device.limits.maxTextureDimension2D,
            this.device.limits.maxTextureArrayLayers]

          })
        );
        break;
      case 'validation':
        // Generating a validation error by passing in an invalid usage when creating a buffer.
        this.trackForCleanup(
          this.device.createBuffer({
            size: 1024,
            usage: 0xffff // Invalid GPUBufferUsage
          })
        );
        break;
    }
    // MAINTENANCE_TODO: This is a workaround for Chromium not flushing. Remove when not needed.
    this.device.queue.submit([]);
  }

  /**
   * Checks whether the error is of the type expected given the filter.
   */
  isInstanceOfError(filter, error) {
    switch (filter) {
      case 'out-of-memory':
        return error instanceof GPUOutOfMemoryError;
      case 'validation':
        return error instanceof GPUValidationError;
      case 'internal':
        return error instanceof GPUInternalError;
    }
  }

  /**
   * Pop `count` error scopes, and assert they all return `null`. Chunks the
   * awaits so we only `Promise.all` 200 scopes at a time, instead of stalling
   * on a huge `Promise.all` all at once. This helps Chromium's "heartbeat"
   * mechanism know that the test is still running (and not hung).
   */
  async chunkedPopManyErrorScopes(count) {
    const promises = [];
    for (let i = 0; i < count; i++) {
      promises.push(this.device.popErrorScope());
      if (promises.length >= 200) {
        this.expect((await Promise.all(promises)).every((e) => e === null));
        promises.length = 0;
      }
    }
    this.expect((await Promise.all(promises)).every((e) => e === null));
  }

  /**
   * Expect an uncapturederror event to occur. Note: this MUST be awaited, because
   * otherwise it could erroneously pass by capturing an error from later in the test.
   */
  async expectUncapturedError(
  fn,
  useOnuncapturederror = false)
  {
    return this.immediateAsyncExpectation(() => {
      const promise = new Promise((resolve) => {
        const eventListener = (event) => {
          // Don't emit error to console.
          event.preventDefault();
          // Unregister before resolving so we can be certain these are cleaned
          // up before the next test.
          if (useOnuncapturederror) {
            this.device.onuncapturederror = null;
          } else {
            this.device.removeEventListener('uncapturederror', eventListener);
          }

          this.debug(`Got uncaptured error event with ${event.error}`);
          resolve(event);
        };

        if (useOnuncapturederror) {
          this.device.onuncapturederror = eventListener;
        } else {
          this.device.addEventListener('uncapturederror', eventListener, { once: true });
        }
      });

      fn();

      const kTimeoutMS = 1000;
      return raceWithRejectOnTimeout(
        promise,
        kTimeoutMS,
        'Timeout occurred waiting for uncaptured error'
      );
    });
  }
}
});
__define('webgpu/shader/validation/parse/semicolon.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `Validation tests for semicolon placements`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let ShaderValidationTest; __bind('webgpu/shader/validation/shader_validation_test.js', (__m) => { ({ ShaderValidationTest } = __m); });

const g = makeTestGroup(ShaderValidationTest);

g.test('module_scope_single').
desc(`Test that a semicolon can be placed at module scope.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `;`);
});

g.test('module_scope_multiple').
desc(`Test that multiple semicolons can be placed at module scope.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `;;;`);
});

g.test('after_enable').
desc(`Test that a semicolon must be placed after an enable directive.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `enable f16;`);
  t.expectCompileResult( /* pass */false, `enable f16`);
});

g.test('after_requires').
desc(`Test that a semicolon must be placed after a requires directive.`).
fn((t) => {
  t.skipIfLanguageFeatureNotSupported('readonly_and_readwrite_storage_textures');
  t.expectCompileResult( /* pass */true, `requires readonly_and_readwrite_storage_textures;`);
  t.expectCompileResult( /* pass */false, `requires readonly_and_readwrite_storage_textures`);
});

g.test('after_diagnostic').
desc(`Test that a semicolon must be placed after a requires directive.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `diagnostic(info, derivative_uniformity);`);
  t.expectCompileResult( /* pass */false, `diagnostic(info, derivative_uniformity)`);
});

g.test('after_struct_decl').
desc(`Test that a semicolon can be placed after an struct declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `struct S { x : i32 };`);
  t.expectCompileResult( /* pass */true, `struct S { x : i32 }`);
});

g.test('after_member').
desc(`Test that a semicolon must not be placed after an struct member declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `struct S { x : i32 }`);
  t.expectCompileResult( /* pass */false, `struct S { x : i32; }`);
});

g.test('after_func_decl').
desc(`Test that a semicolon can be placed after a function declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() {};`);
  t.expectCompileResult( /* pass */true, `fn f() {}`);
});

g.test('after_type_alias_decl').
desc(`Test that a semicolon must be placed after an type alias declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `alias T = i32;`);
  t.expectCompileResult( /* pass */false, `alias T = i32`);
});

g.test('after_return').
desc(`Test that a semicolon must be placed after a return statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { return; }`);
  t.expectCompileResult( /* pass */false, `fn f() { return }`);
});

g.test('after_call').
desc(`Test that a semicolon must be placed after a function call.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { workgroupBarrier(); }`);
  t.expectCompileResult( /* pass */false, `fn f() { workgroupBarrier() }`);
});

g.test('after_module_const_decl').
desc(`Test that a semicolon must be placed after a module-scope const declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `const v = 1;`);
  t.expectCompileResult( /* pass */false, `const v = 1`);
});

g.test('after_fn_const_decl').
desc(`Test that a semicolon must be placed after a function-scope const declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { const v = 1; }`);
  t.expectCompileResult( /* pass */false, `fn f() { const v = 1 }`);
});

g.test('after_module_var_decl').
desc(`Test that a semicolon must be placed after a module-scope var declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `var<private> v = 1;`);
  t.expectCompileResult( /* pass */false, `var<private> v = 1`);
});

g.test('after_fn_var_decl').
desc(`Test that a semicolon must be placed after a function-scope var declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { var v = 1; }`);
  t.expectCompileResult( /* pass */false, `fn f() { var v = 1 }`);
});

g.test('after_let_decl').
desc(`Test that a semicolon must be placed after a let declaration.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { let v = 1; }`);
  t.expectCompileResult( /* pass */false, `fn f() { let v = 1 }`);
});

g.test('after_discard').
desc(`Test that a semicolon must be placed after a discard statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { discard; }`);
  t.expectCompileResult( /* pass */false, `fn f() { discard }`);
});

g.test('after_assignment').
desc(`Test that a semicolon must be placed after an assignment statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { var v = 1; v = 2; }`);
  t.expectCompileResult( /* pass */false, `fn f() { var v = 1; v = 2 }`);
});

g.test('after_fn_const_assert').
desc(`Test that a semicolon must be placed after an function-scope static assert.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { const_assert(true); }`);
  t.expectCompileResult( /* pass */false, `fn f() { const_assert(true) }`);
});

g.test('function_body_single').
desc(`Test that a semicolon can be placed in a function body.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { ; }`);
});

g.test('function_body_multiple').
desc(`Test that multiple semicolons can be placed in a function body.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { ;;; }`);
});

g.test('compound_statement_single').
desc(`Test that a semicolon can be placed in a compound statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { { ; } }`);
});

g.test('compound_statement_multiple').
desc(`Test that multiple semicolons can be placed in a compound statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { { ;;; } }`);
});

g.test('after_compound_statement').
desc(`Test that a semicolon can be placed after a compound statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { {} ; }`);
});

g.test('after_if').
desc(`Test that a semicolon can be placed after an if-statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { if true {} ; }`);
});

g.test('after_if_else').
desc(`Test that a semicolon can be placed after an if-else-statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { if true {} else {} ; }`);
});

g.test('after_switch').
desc(`Test that a semicolon can be placed after an switch-statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { switch 1 { default {} } ; }`);
});

g.test('after_case').
desc(`Test that a semicolon cannot be placed after a non-default switch case.`).
fn((t) => {
  t.expectCompileResult( /* pass */false, `fn f() { switch 1 { case 1 {}; default {} } }`);
  t.expectCompileResult( /* pass */true, `fn f() { switch 1 { case 1 {} default {} } }`);
});

g.test('after_case_break').
desc(`Test that a semicolon must be placed after a case break statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */false, `fn f() { switch 1 { case 1 { break } default {} } }`);
  t.expectCompileResult( /* pass */true, `fn f() { switch 1 { case 1 { break; } default {} } }`);
});

g.test('after_default_case').
desc(`Test that a semicolon cannot be placed after a default switch case.`).
fn((t) => {
  t.expectCompileResult( /* pass */false, `fn f() { switch 1 { default {}; } }`);
  t.expectCompileResult( /* pass */true, `fn f() { switch 1 { default {} } }`);
});

g.test('after_default_case_break').
desc(`Test that a semicolon cannot be placed after a default switch case.`).
fn((t) => {
  t.expectCompileResult( /* pass */false, `fn f() { switch 1 { default { break } } }`);
  t.expectCompileResult( /* pass */true, `fn f() { switch 1 { default { break; } } }`);
});

g.test('after_for').
desc(`Test that a semicolon can be placed after a for-loop.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { for (; false;) {}; }`);
});

g.test('after_for_break').
desc(`Test that a semicolon must be placed after a for-loop break statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { for (; false;) { break; } }`);
  t.expectCompileResult( /* pass */false, `fn f() { for (; false;) { break } }`);
});

g.test('after_loop').
desc(`Test that a semicolon can be placed after a loop.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { loop { break; }; }`);
});

g.test('after_loop_break').
desc(`Test that a semicolon must be placed after a loop break statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { loop { break; }; }`);
  t.expectCompileResult( /* pass */false, `fn f() { loop { break }; }`);
});

g.test('after_loop_break_if').
desc(`Test that a semicolon must be placed after a loop break-if statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { loop { continuing { break if true; } }; }`);
  t.expectCompileResult( /* pass */false, `fn f() { loop { continuing { break if true } }; }`);
});

g.test('after_loop_continue').
desc(`Test that a semicolon must be placed after a loop continue statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { loop { if true { continue; } { break; } } }`);
  t.expectCompileResult( /* pass */false, `fn f() { loop { if true { continue } { break; } } }`);
});

g.test('after_continuing').
desc(`Test that a semicolon cannot be placed after a continuing.`).
fn((t) => {
  t.expectCompileResult( /* pass */false, `fn f() { loop { break; continuing{}; } }`);
  t.expectCompileResult( /* pass */true, `fn f() { loop { break; continuing{} } }`);
});

g.test('after_while').
desc(`Test that a semicolon cannot be placed after a while-loop.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { while false {}; }`);
});

g.test('after_while_break').
desc(`Test that a semicolon must be placed after a while break statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { while false { break; } }`);
  t.expectCompileResult( /* pass */false, `fn f() { while false { break } }`);
});

g.test('after_while_continue').
desc(`Test that a semicolon must be placed after a while continue statement.`).
fn((t) => {
  t.expectCompileResult( /* pass */true, `fn f() { while false { continue; } }`);
  t.expectCompileResult( /* pass */false, `fn f() { while false { continue } }`);
});
});
__define('webgpu/shader/validation/shader_validation_test.js', function (__exports) {
'use strict';
__get(__exports, 'ShaderValidationTest', () => ShaderValidationTest);
__get(__exports, 'UniqueFeaturesAndLimitsShaderValidationTest', () => UniqueFeaturesAndLimitsShaderValidationTest);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let keysOf; __bind('common/util/data_tables.js', (__m) => { ({ keysOf } = __m); });let AllFeaturesMaxLimitsGPUTest, UniqueFeaturesOrLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest, UniqueFeaturesOrLimitsGPUTest } = __m); });

const kEnables = {
  f16: 'shader-f16',
  subgroups: 'subgroups',
  clip_distances: 'clip-distances',
  chromium_experimental_primitive_id: 'chromium-experimental-primitive-id',
  atomic_vec2u_min_max: 'atomic-vec2u-min-max'
};

/**
 * Note: These regular expressions are not meant to be perfect. This is not production code expecting
 * to work with any WGSL passed by a user. It's only test code for working with WGSL written
 * in the CTS. A CTS test for which these regular expressions don't work should use a different set of
 * testing functions or options that don't use these regular expressions.
 */
const kEnableREs = Object.entries(kEnables).map(([enableName, feature]) => {
  return {
    re: new RegExp(`\\benable\\s+(?:\\s*\\w+\\s*,)*\\s*${enableName}\\s*(?:,\\s*\\w+)*\\s*;`),
    feature
  };
});

/**
 * Note: This function is not meant to be perfect. This is not production code expecting
 * to work with any WGSL passed by a user. It's only test code for working with WGSL written
 * in the CTS. A CTS test for which this check doesn't work can choose a different set of
 * testing functions or options that don't take this path.
 */
function skipIfCodeNeedsFeatureAndDeviceDoesNotHaveFeature(t, code) {
  for (const { re, feature } of kEnableREs) {
    if (re.test(code)) {
      t.skipIfDeviceDoesNotHaveFeature(feature);
    }
  }
}

/**
 * Base fixture for WGSL shader validation tests.
 */
class ShaderValidationTest extends AllFeaturesMaxLimitsGPUTest {
  /**
   * Add a test expectation for whether a createShaderModule call succeeds or not.
   * Note: skips test if 'enable X' exists in code and X's corresponding feature does not exist on device
   * unless you pass in autoSkipIfFeatureNotAvailable: false.
   *
   * @example
   * ```ts
   * t.expectCompileResult(true, `wgsl code`); // Expect success
   * t.expectCompileResult(false, `wgsl code`); // Expect validation error with any error string
   * ```
   */
  expectCompileResult(
  expectedResult,
  code,
  options)
  {
    if (options?.autoSkipIfFeatureNotAvailable !== false) {
      skipIfCodeNeedsFeatureAndDeviceDoesNotHaveFeature(this, code);
    }
    let shaderModule;
    this.expectGPUError(
      'validation',
      () => {
        shaderModule = this.device.createShaderModule({ code });
      },
      expectedResult !== true
    );

    const error = new Error();
    this.eventualAsyncExpectation(async () => {
      const compilationInfo = await shaderModule.getCompilationInfo();

      // MAINTENANCE_TODO: Pretty-print error messages with source context.
      const messagesLog =
      compilationInfo.messages.
      map((m) => `${m.lineNum}:${m.linePos}: ${m.type}: ${m.message}`).
      join('\n') +
      '\n\n---- shader ----\n' +
      code;

      if (compilationInfo.messages.some((m) => m.type === 'error')) {
        if (expectedResult) {
          error.message = `Unexpected compilationInfo 'error' message.\n` + messagesLog;
          this.rec.validationFailed(error);
        } else {
          error.message = `Found expected compilationInfo 'error' message.\n` + messagesLog;
          this.rec.debug(error);
        }
      } else {
        if (!expectedResult) {
          error.message = `Missing expected compilationInfo 'error' message.\n` + messagesLog;
          this.rec.validationFailed(error);
        } else {
          error.message = `No compilationInfo 'error' messages, as expected.\n` + messagesLog;
          this.rec.debug(error);
        }
      }
    });
  }

  /**
   * Add a test expectation for whether a createShaderModule call issues a warning.
   *
   * @example
   * ```ts
   * t.expectCompileWarning(true, `wgsl code`); // Expect compile success and any warning message
   * t.expectCompileWarning(false, `wgsl code`); // Expect compile success and no warning messages
   * ```
   */
  expectCompileWarning(expectWarning, code) {
    let shaderModule;
    this.expectGPUError(
      'validation',
      () => {
        shaderModule = this.device.createShaderModule({ code });
      },
      false
    );

    const error = new Error();
    this.eventualAsyncExpectation(async () => {
      const compilationInfo = await shaderModule.getCompilationInfo();

      // MAINTENANCE_TODO: Pretty-print error messages with source context.
      const messagesLog = compilationInfo.messages.
      map((m) => `${m.lineNum}:${m.linePos}: ${m.type}: ${m.message}`).
      join('\n');

      if (compilationInfo.messages.some((m) => m.type === 'warning')) {
        if (expectWarning) {
          error.message = `No 'warning' message as expected.\n` + messagesLog;
          this.rec.debug(error);
        } else {
          error.message = `Missing expected compilationInfo 'warning' message.\n` + messagesLog;
          this.rec.validationFailed(error);
        }
      } else {
        if (expectWarning) {
          error.message = `Missing expected 'warning' message.\n` + messagesLog;
          this.rec.validationFailed(error);
        } else {
          error.message = `Found a 'warning' message as expected.\n` + messagesLog;
          this.rec.debug(error);
        }
      }
    });
  }

  /**
   * Add a test expectation for whether a createComputePipeline call succeeds or not.
   * Note: skips test if 'enable X' exists in code and X's corresponding feature does not exist on device
   * unless you pass in autoSkipIfFeatureNotAvailable: false
   */
  expectPipelineResult(args)













  {
    const phonies = [];

    if (args.statements !== undefined) {
      phonies.push(...args.statements);
    }
    if (args.constants !== undefined) {
      phonies.push(...keysOf(args.constants).map((c) => `_ = ${c};`));
    }
    if (args.reference !== undefined) {
      phonies.push(...args.reference.map((c) => `_ = ${c};`));
    }

    const code =
    args.code + (
    args.addWorkgroupSize !== false ?
    `
@workgroup_size(1)` :
    ``) +
    `
      @compute fn main() {
  ${phonies.join('\n')}
}`;

    if (args.autoSkipIfFeatureNotAvailable !== false) {
      skipIfCodeNeedsFeatureAndDeviceDoesNotHaveFeature(this, code);
    }

    let shaderModule;
    this.expectGPUError(
      'validation',
      () => {
        shaderModule = this.device.createShaderModule({ code });
      },
      false
    );

    this.expectGPUError(
      'validation',
      () => {
        this.device.createComputePipeline({
          layout: 'auto',
          compute: { module: shaderModule, entryPoint: 'main', constants: args.constants }
        });
      },
      !args.expectedResult
    );
  }

  /**
   * Wraps the code fragment into an entry point.
   *
   * @example
   * ```ts
   * t.wrapInEntryPoint(`var i = 0;`);
   * ```
   */
  wrapInEntryPoint(code, enabledExtensions = []) {
    const enableDirectives = enabledExtensions.map((x) => `enable ${x};`).join('\n      ');

    return `
      ${enableDirectives}

      @compute @workgroup_size(1)
      fn main() {
        ${code}
      }`;
  }
}

// MAINTENANCE_TODO: Merge this with implementation above.
// NOTE: These things should probably not inherit. There is no relationship between
// these functions and a test. They could just as easily take a GPUTest as the first
// argument and then the all the problems associated with inheritance would disappear.
class UniqueFeaturesAndLimitsShaderValidationTest extends UniqueFeaturesOrLimitsGPUTest {
  /**
   * Add a test expectation for whether a createShaderModule call succeeds or not.
   *
   * @example
   * ```ts
   * t.expectCompileResult(true, `wgsl code`); // Expect success
   * t.expectCompileResult(false, `wgsl code`); // Expect validation error with any error string
   * ```
   */
  expectCompileResult(expectedResult, code) {
    let shaderModule;
    this.expectGPUError(
      'validation',
      () => {
        shaderModule = this.device.createShaderModule({ code });
      },
      expectedResult !== true
    );

    const error = new Error();
    this.eventualAsyncExpectation(async () => {
      const compilationInfo = await shaderModule.getCompilationInfo();

      // MAINTENANCE_TODO: Pretty-print error messages with source context.
      const messagesLog =
      compilationInfo.messages.
      map((m) => `${m.lineNum}:${m.linePos}: ${m.type}: ${m.message}`).
      join('\n') +
      '\n\n---- shader ----\n' +
      code;

      if (compilationInfo.messages.some((m) => m.type === 'error')) {
        if (expectedResult) {
          error.message = `Unexpected compilationInfo 'error' message.\n` + messagesLog;
          this.rec.validationFailed(error);
        } else {
          error.message = `Found expected compilationInfo 'error' message.\n` + messagesLog;
          this.rec.debug(error);
        }
      } else {
        if (!expectedResult) {
          error.message = `Missing expected compilationInfo 'error' message.\n` + messagesLog;
          this.rec.validationFailed(error);
        } else {
          error.message = `No compilationInfo 'error' messages, as expected.\n` + messagesLog;
          this.rec.debug(error);
        }
      }
    });
  }

  /**
   * Add a test expectation for whether a createShaderModule call issues a warning.
   *
   * @example
   * ```ts
   * t.expectCompileWarning(true, `wgsl code`); // Expect compile success and any warning message
   * t.expectCompileWarning(false, `wgsl code`); // Expect compile success and no warning messages
   * ```
   */
  expectCompileWarning(expectWarning, code) {
    let shaderModule;
    this.expectGPUError(
      'validation',
      () => {
        shaderModule = this.device.createShaderModule({ code });
      },
      false
    );

    const error = new Error();
    this.eventualAsyncExpectation(async () => {
      const compilationInfo = await shaderModule.getCompilationInfo();

      // MAINTENANCE_TODO: Pretty-print error messages with source context.
      const messagesLog = compilationInfo.messages.
      map((m) => `${m.lineNum}:${m.linePos}: ${m.type}: ${m.message}`).
      join('\n');

      if (compilationInfo.messages.some((m) => m.type === 'warning')) {
        if (expectWarning) {
          error.message = `No 'warning' message as expected.\n` + messagesLog;
          this.rec.debug(error);
        } else {
          error.message = `Missing expected compilationInfo 'warning' message.\n` + messagesLog;
          this.rec.validationFailed(error);
        }
      } else {
        if (expectWarning) {
          error.message = `Missing expected 'warning' message.\n` + messagesLog;
          this.rec.validationFailed(error);
        } else {
          error.message = `Found a 'warning' message as expected.\n` + messagesLog;
          this.rec.debug(error);
        }
      }
    });
  }

  /**
   * Add a test expectation for whether a createComputePipeline call succeeds or not.
   */
  expectPipelineResult(args)












  {
    const phonies = [];

    if (args.statements !== undefined) {
      phonies.push(...args.statements);
    }
    if (args.constants !== undefined) {
      phonies.push(...keysOf(args.constants).map((c) => `_ = ${c};`));
    }
    if (args.reference !== undefined) {
      phonies.push(...args.reference.map((c) => `_ = ${c};`));
    }

    const code =
    args.code + (
    args.addWorkgroupSize !== false ?
    `
@compute @workgroup_size(1)` :
    `
@compute`) +
    `
fn main() {
  ${phonies.join('\n')}
}`;

    let shaderModule;
    this.expectGPUError(
      'validation',
      () => {
        shaderModule = this.device.createShaderModule({ code });
      },
      false
    );

    this.expectGPUError(
      'validation',
      () => {
        this.device.createComputePipeline({
          layout: 'auto',
          compute: { module: shaderModule, entryPoint: 'main', constants: args.constants }
        });
      },
      !args.expectedResult
    );
  }

  /**
   * Wraps the code fragment into an entry point.
   *
   * @example
   * ```ts
   * t.wrapInEntryPoint(`var i = 0;`);
   * ```
   */
  wrapInEntryPoint(code, enabledExtensions = []) {
    const enableDirectives = enabledExtensions.map((x) => `enable ${x};`).join('\n      ');

    return `
      ${enableDirectives}

      @compute @workgroup_size(1)
      fn main() {
        ${code}
      }`;
  }
}
});
__define('webgpu/shader/validation/parse/comments.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `Validation tests for comments`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let ShaderValidationTest; __bind('webgpu/shader/validation/shader_validation_test.js', (__m) => { ({ ShaderValidationTest } = __m); });

const g = makeTestGroup(ShaderValidationTest);

g.test('comments').
desc(`Test that valid comments are handled correctly, including nesting.`).
fn((t) => {
  const code = `
/**
 * Here is my shader.
 *
 * /* I can nest /**/ comments. */
 * // I can nest line comments too.
 **/
@fragment // This is the stage
fn main(/*
no
parameters
*/) -> @location(0) vec4<f32> {
  return/*block_comments_delimit_tokens*/vec4<f32>(.4, .2, .3, .1);
}/* terminated block comments are OK at EOF...*/`;
  t.expectCompileResult(true, code);
});

g.test('line_comment_eof').
desc(`Test that line comments can come at EOF.`).
fn((t) => {
  const code = `
@fragment
fn main() -> @location(0) vec4<f32> {
  return vec4<f32>(.4, .2, .3, .1);
}
// line comments are OK at EOF...`;
  t.expectCompileResult(true, code);
});

g.test('line_comment_terminators').
desc(`Test that line comments are terminated by any blankspace other than space and \t`).
params((u) =>
u.
combine('blankspace', [
[' ', 'space'],
['\t', 'tab'],
['\u000a', 'line_feed'],
['\u000b', 'vertical_tab'],
['\u000c', 'form_feed'],
['\u000d', 'carriage_return'],
['\u000d\u000a', 'carriage_return_line_feed'],
['\u0085', 'next_line'],
['\u2028', 'line_separator'],
['\u2029', 'paragraph_separator']]
).
beginSubcases()
).
fn((t) => {
  const code = `// Line comment${t.params.blankspace[0]}const invalid_outside_comment = should_fail`;

  t.expectCompileResult([' ', '\t'].includes(t.params.blankspace[0]), code);
});

g.test('unterminated_block_comment').
desc(`Test that unterminated block comments cause an error`).
params((u) => u.combine('terminated', [true, false]).beginSubcases()).
fn((t) => {
  const code = `
/**
 * Unterminated block comment.
 *
 ${t.params.terminated ? '*/' : ''}`;

  t.expectCompileResult(t.params.terminated, code);
});
});
__define('webgpu/shader/execution/flow_control/loop.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Flow control tests for loops.
`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let AllFeaturesMaxLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest } = __m); });

let runFlowControlTest; __bind('webgpu/shader/execution/flow_control/harness.js', (__m) => { ({ runFlowControlTest } = __m); });

const g = makeTestGroup(AllFeaturesMaxLimitsGPUTest);

g.test('loop_break').
desc('Test that flow control exits a loop when reaching a break statement').
params((u) => u.combine('preventValueOptimizations', [true, false])).
fn((t) => {
  runFlowControlTest(
    t,
    (f) =>
    `
  ${f.expect_order(0)}
  var i = ${f.value(0)};
  loop {
    ${f.expect_order(1, 3, 5, 7)}
    if i == 3 {
      break;
    }
    ${f.expect_order(2, 4, 6)}
    i++;
  }
  ${f.expect_order(8)}
`
  );
});

g.test('loop_continue').
desc('Test flow control for a loop continue statement').
params((u) => u.combine('preventValueOptimizations', [true, false])).
fn((t) => {
  runFlowControlTest(
    t,
    (f) =>
    `
  ${f.expect_order(0)}
  var i = ${f.value(0)};
  loop {
    ${f.expect_order(1, 3, 5, 7, 8)}
    if i == 3 {
      i++;
      continue;
      ${f.expect_not_reached()}
    }
    ${f.expect_order(2, 4, 6, 9)}
    if i == 4 {
      break;
    }
    i++;
  }
  ${f.expect_order(10)}
`
  );
});

g.test('loop_continuing_basic').
desc('Test basic flow control for a loop continuing block').
params((u) => u.combine('preventValueOptimizations', [true, false])).
fn((t) => {
  runFlowControlTest(
    t,
    (f) =>
    `
  ${f.expect_order(0)}
  var i = ${f.value(0)};
  loop {
    ${f.expect_order(1, 3, 5)}
    i++;

    continuing {
      ${f.expect_order(2, 4, 6)}
      break if i == 3;
    }
  }
  ${f.expect_order(7)}
`
  );
});

g.test('nested_loops').
desc('Test flow control for a loop nested in another loop').
params((u) => u.combine('preventValueOptimizations', [true, false])).
fn((t) => {
  runFlowControlTest(
    t,
    (f) =>
    `
  ${f.expect_order(0)}
  var i = ${f.value(0)};
  loop {
    ${f.expect_order(1, 11, 21)}
    if i == ${f.value(6)} {
      ${f.expect_order(22)}
      break;
      ${f.expect_not_reached()}
    }
    ${f.expect_order(2, 12)}
    loop {
      i++;
      ${f.expect_order(3, 6, 9, 13, 16, 19)}
      if (i % ${f.value(3)}) == 0 {
        ${f.expect_order(10, 20)}
        break;
        ${f.expect_not_reached()}
      }
      ${f.expect_order(4, 7, 14, 17)}
      if (i & ${f.value(1)}) == 0 {
        ${f.expect_order(8, 15)}
        continue;
        ${f.expect_not_reached()}
      }
      ${f.expect_order(5, 18)}
    }
  }
  ${f.expect_order(23)}
`
  );
});

g.test('loop_break_if_logical_and_condition').
desc('Test flow control for a loop with a logical and break if').
params((u) => u.combine('preventValueOptimizations', [true, false])).
fn((t) => {
  runFlowControlTest(t, (f) => ({
    entrypoint: `
  ${f.expect_order(0)}
  var i = ${f.value(0)};
  loop {
    ${f.expect_order(1, 4, 7)}
    continuing {
      i++;
      break if !(a(i) && b(i));
    }
  }
  ${f.expect_order(9)}
      `,
    extra: `
fn a(i : i32) -> bool {
  ${f.expect_order(2, 5, 8)}
  return i < ${f.value(3)};
}
fn b(i : i32) -> bool {
  ${f.expect_order(3, 6)}
  return i < ${f.value(5)};
}
      `
  }));
});

g.test('loop_break_if_logical_or_condition').
desc('Test flow control for a loop with a logical or break if').
params((u) => u.combine('preventValueOptimizations', [true, false])).
fn((t) => {
  runFlowControlTest(t, (f) => ({
    entrypoint: `
  ${f.expect_order(0)}
  var i = ${f.value(0)};
  loop {
    ${f.expect_order(1, 3, 6, 9)}
    continuing {
      i++;
      break if !(a(i) || b(i));
    }
  }
  ${f.expect_order(12)}
      `,
    extra: `
fn a(i : i32) -> bool {
  ${f.expect_order(2, 4, 7, 10)}
  return i < ${f.value(2)};
}
fn b(i : i32) -> bool {
  ${f.expect_order(5, 8, 11)}
  return i < ${f.value(4)};
}
      `
  }));
});
});
__define('webgpu/shader/execution/flow_control/harness.js', function (__exports) {
'use strict';
__get(__exports, 'runFlowControlTest', () => runFlowControlTest);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let Colors; __bind('common/util/colors.js', (__m) => { ({ Colors } = __m); });
/**
 * Options for runFlowControlTest()
 */












/**
 * The builder interface for the runFlowControlTest() callback.
 * This interface is indented to be used to inject WGSL logic into the test
 * shader.
 * @see runFlowControlTest
 */
























/**
 * Builds, runs then checks the output of a flow control shader test.
 *
 * `build_wgsl` is a function that's called to build the WGSL shader.
 * This function takes a FlowControlTestBuilder as the single argument, and
 * returns either a string which is embedded into the WGSL entrypoint function,
 * or an object of the signature `{ entrypoint: string; extra: string }` which
 * contains the entrypoint code, along with additional module-scope code.
 *
 * The FlowControlTestBuilder should be used to insert expectations into WGSL to
 * validate control flow. FlowControlTestBuilder also can be used to add values
 * to the shader which cannot be optimized away.
 *
 * Example, testing that an if-statement behaves as expected:
 *
 * ```
 *   runFlowControlTest(t, f =>
 *   `
 *    ${f.expect_order(0)}
 *    if (${f.value(true)}) {
 *      ${f.expect_order(1)}
 *    } else {
 *      ${f.expect_not_reached()}
 *    }
 *    ${f.expect_order(2)}
 *  `);
 * ```
 *
 * @param t The test object
 * @param builder The shader builder function that takes a
 * FlowControlTestBuilder as the single argument, and returns either a WGSL
 * string which is embedded into the WGSL entrypoint function, or a structure
 * with entrypoint-scoped WGSL code and extra module-scope WGSL code.
 */
function runFlowControlTest(
t,
build_wgsl)
{
  const inputData = new Array();












  const expectations = new Array();

  const build_wgsl_result = build_wgsl({
    value: (v) => {
      if (t.params.preventValueOptimizations) {
        if (typeof v === 'boolean') {
          inputData.push(v ? 1 : 0);
          return `inputs[${inputData.length - 1}] != 0`;
        }
        inputData.push(v);
        return `inputs[${inputData.length - 1}]`;
      } else {
        return `${v}`;
      }
    },
    expect_order: (...expected) => {
      expectations.push({
        kind: 'events',
        stack: Error().stack,
        values: expected,
        counter: 0
      });
      // Expectation id starts from 1 to distinguish from initialization 0.
      return `push_output(${expectations.length}); // expect_order(${expected.join(', ')})`;
    },
    expect_not_reached: () => {
      expectations.push({
        kind: 'not-reached',
        stack: Error().stack
      });
      // Expectation id starts from 1 to distinguish from initialization 0.
      return `push_output(${expectations.length}); // expect_not_reached()`;
    }
  });

  const built_wgsl =
  typeof build_wgsl_result === 'string' ?
  { entrypoint: build_wgsl_result, extra: '' } :
  build_wgsl_result;

  const main_wgsl = built_wgsl.entrypoint !== undefined ? built_wgsl : built_wgsl.entrypoint;

  const wgsl = `
struct Outputs {
  count : u32,
  data  : array<u32>,
};
@group(0) @binding(0) var<storage, read>       inputs  : array<i32>;
@group(0) @binding(1) var<storage, read_write> outputs : Outputs;

fn push_output(value : u32) {
  outputs.data[outputs.count] = value;
  outputs.count++;
}

@compute @workgroup_size(1)
fn main() {
  _ = &inputs;
  _ = &outputs;
  ${main_wgsl.entrypoint}
}
${main_wgsl.extra}
`;

  const pipeline = t.device.createComputePipeline({
    layout: 'auto',
    compute: {
      module: t.device.createShaderModule({ code: wgsl }),
      entryPoint: 'main'
    }
  });

  // If there are no inputs, just put a single value in the buffer to keep
  // makeBufferWithContents() happy.
  if (inputData.length === 0) {
    inputData.push(0);
  }

  const inputBuffer = t.makeBufferWithContents(new Uint32Array(inputData), GPUBufferUsage.STORAGE);

  const maxOutputValues = 1000;
  const outputBuffer = t.createBufferTracked({
    size: 4 * (1 + maxOutputValues),
    usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC
  });

  const bindGroup = t.device.createBindGroup({
    layout: pipeline.getBindGroupLayout(0),
    entries: [
    { binding: 0, resource: { buffer: inputBuffer } },
    { binding: 1, resource: { buffer: outputBuffer } }]

  });

  // Run the shader.
  const encoder = t.device.createCommandEncoder({ label: 'runFlowControlTest' });
  const pass = encoder.beginComputePass();
  pass.setPipeline(pipeline);
  pass.setBindGroup(0, bindGroup);
  pass.dispatchWorkgroups(1);
  pass.end();
  t.queue.submit([encoder.finish()]);

  t.eventualExpectOK(
    t.
    readGPUBufferRangeTyped(outputBuffer, {
      type: Uint32Array,
      typedLength: outputBuffer.size / 4
    }).
    then((outputs) => {
      // outputs[0]    is the number of outputted values
      // outputs[1..N] holds the outputted values
      const outputCount = outputs.data[0];
      if (outputCount > maxOutputValues) {
        return new Error(
          `output data count (${outputCount}) exceeds limit of ${maxOutputValues}`
        );
      }

      // returns an Error with the given message and WGSL source
      const fail = (err) => Error(`${err}\nWGSL:\n${Colors.dim(Colors.blue(wgsl))}`);

      // returns a string that shows the outputted values to help understand the whole trace.
      const print_output_value = () => {
        const subarray = outputs.data.subarray(1, outputCount + 1);
        return `Output values (length: ${outputCount}): ${subarray.join(', ')}`;
      };

      // returns a colorized string of the expect_order() call, highlighting
      // the event number that caused an error.
      const expect_order_err = (expectation, err_idx) => {
        let out = 'expect_order(';
        for (let i = 0; i < expectation.values.length; i++) {
          if (i > 0) {
            out += ', ';
          }
          if (i < err_idx) {
            out += Colors.green(`${expectation.values[i]}`);
          } else if (i > err_idx) {
            out += Colors.dim(`${expectation.values[i]}`);
          } else {
            out += Colors.red(`${expectation.values[i]}`);
          }
        }
        out += ')';
        return out;
      };

      // Each of the outputted values represents an event
      // Check that each event is as expected
      for (let event = 0; event < outputCount; event++) {
        const eventValue = outputs.data[1 + event]; // outputs.data[0] is count
        // Expectation id starts from 1, and 0 is invalid value.
        if (eventValue === 0) {
          return fail(
            `outputs.data[${event}] is initial value 0, doesn't refer to any valid expectations)\n${print_output_value()}`
          );
        }
        const expectationIndex = eventValue - 1;
        if (expectationIndex >= expectations.length) {
          return fail(
            `outputs.data[${event}] value (${expectationIndex}) exceeds number of expectations (${
            expectations.length
            })\n${print_output_value()}`
          );
        }
        const expectation = expectations[expectationIndex];
        switch (expectation.kind) {
          case 'not-reached':
            return fail(
              `expect_not_reached() reached at event ${event}\n${print_output_value()}\n${
              expectation.stack
              }`
            );
          case 'events':
            if (expectation.counter >= expectation.values.length) {
              return fail(
                `${expect_order_err(
                  expectation,
                  expectation.counter
                )}) unexpectedly reached at event ${Colors.red(
                  `${event}`
                )}\n${print_output_value()}\n${expectation.stack}`
              );
            }
            if (event !== expectation.values[expectation.counter]) {
              return fail(
                `${expect_order_err(expectation, expectation.counter)} expected event ${
                expectation.values[expectation.counter]
                }, got ${event}\n${print_output_value()}\n${expectation.stack}`
              );
            }

            expectation.counter++;
            break;
        }
      }

      // Finally check that all expect_order() calls were reached
      for (const expectation of expectations) {
        if (expectation.kind === 'events' && expectation.counter !== expectation.values.length) {
          return fail(
            `${expect_order_err(expectation, expectation.counter)} event ${
            expectation.values[expectation.counter]
            } was not reached\n${expectation.stack}\n${print_output_value()}`
          );
        }
      }
      outputs.cleanup();
      return undefined;
    })
  );
}
});
__define('webgpu/web_platform/canvas/configure.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Tests for GPUCanvasContext.configure.

TODO:
- Test colorSpace
- Test toneMapping
`;let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });
let assert, hasFeature; __bind('common/util/util.js', (__m) => { ({ assert, hasFeature } = __m); });
let isValidTextureUsageCombination, kCanvasTextureFormats, kSomeBogusTextureUsage, kTextureUsages, kValidCombinationsOfOneOrTwoTextureUsages, usageIsTypeErrorForConfigure; __bind('webgpu/capability_info.js', (__m) => { ({ isValidTextureUsageCombination, kCanvasTextureFormats, kSomeBogusTextureUsage, kTextureUsages, kValidCombinationsOfOneOrTwoTextureUsages, usageIsTypeErrorForConfigure } = __m); });
let GPUConst; __bind('webgpu/constants.js', (__m) => { ({ GPUConst } = __m); });
let kAllTextureFormats, kFeaturesForFormats, filterFormatsByFeature, textureFormatsAreViewCompatible; __bind('webgpu/format_info.js', (__m) => { ({ kAllTextureFormats, kFeaturesForFormats, filterFormatsByFeature, textureFormatsAreViewCompatible } = __m); });
let AllFeaturesMaxLimitsGPUTest; __bind('webgpu/gpu_test.js', (__m) => { ({ AllFeaturesMaxLimitsGPUTest } = __m); });
let kAllCanvasTypes, createCanvas; __bind('webgpu/util/create_elements.js', (__m) => { ({ kAllCanvasTypes, createCanvas } = __m); });

const g = makeTestGroup(AllFeaturesMaxLimitsGPUTest);

g.test('defaults').
desc(
  `
    Ensure that the defaults for GPUCanvasConfiguration are correct.
    `
).
params((u) =>
u //
.combine('canvasType', kAllCanvasTypes)
).
fn((t) => {
  const { canvasType } = t.params;
  const canvas = createCanvas(t, canvasType, 2, 2);
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  ctx.configure({
    device: t.device,
    format: 'rgba8unorm'
  });

  const configuration = ctx.getConfiguration();
  assert(configuration !== null);
  t.expect(configuration.device === t.device);
  t.expect(configuration.format === 'rgba8unorm');
  t.expect(configuration.usage === GPUTextureUsage.RENDER_ATTACHMENT);
  t.expect(configuration.viewFormats.length === 0);
  t.expect(configuration.colorSpace === 'srgb');
  t.expect(configuration.toneMapping?.mode === 'standard');
  t.expect(configuration.alphaMode === 'opaque');

  const currentTexture = ctx.getCurrentTexture();
  t.expect(currentTexture.format === 'rgba8unorm');
  t.expect(currentTexture.usage === GPUTextureUsage.RENDER_ATTACHMENT);
  t.expect(currentTexture.dimension === '2d');
  t.expect(currentTexture.width === canvas.width);
  t.expect(currentTexture.height === canvas.height);
  t.expect(currentTexture.depthOrArrayLayers === 1);
  t.expect(currentTexture.mipLevelCount === 1);
  t.expect(currentTexture.sampleCount === 1);
});

g.test('device').
desc(
  `
    Ensure that configure reacts appropriately to various device states.
    `
).
params((u) =>
u //
.combine('canvasType', kAllCanvasTypes)
).
fn((t) => {
  const { canvasType } = t.params;
  const canvas = createCanvas(t, canvasType, 2, 2);
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  // getConfiguration returns null before configure.
  t.expect(ctx.getConfiguration() === null);

  // Calling configure without a device should throw a TypeError.
  t.shouldThrow('TypeError', () => {
    ctx.configure({
      format: 'rgba8unorm'
    });
  });

  // Device is not configured, so getCurrentTexture will throw an InvalidStateError.
  t.shouldThrow('InvalidStateError', () => {
    ctx.getCurrentTexture();
  });

  // Should throw on the second call to getCurrentTexture also.
  t.shouldThrow('InvalidStateError', () => {
    ctx.getCurrentTexture();
  });

  // Calling configure with a device should succeed.
  ctx.configure({
    device: t.device,
    format: 'rgba8unorm',
    alphaMode: 'opaque'
  });

  // getConfiguration will succeed after configure.
  const configuration = ctx.getConfiguration();
  assert(configuration !== null);
  t.expect(configuration.device === t.device);
  t.expect(configuration.format === 'rgba8unorm');
  t.expect(configuration.usage === GPUTextureUsage.RENDER_ATTACHMENT);
  t.expect(configuration.viewFormats.length === 0);
  t.expect(configuration.colorSpace === 'srgb');
  t.expect(configuration.toneMapping?.mode === 'standard');
  t.expect(configuration.alphaMode === 'opaque');

  // getCurrentTexture will succeed with a valid device.
  ctx.getCurrentTexture();

  // Unconfiguring should cause the device to be cleared.
  ctx.unconfigure();
  t.shouldThrow('InvalidStateError', () => {
    ctx.getCurrentTexture();
  });

  // getConfiguration returns null after unconfigure.
  t.expect(ctx.getConfiguration() === null);

  // Should be able to successfully configure again after unconfiguring.
  ctx.configure({
    device: t.device,
    format: 'rgba8unorm',
    alphaMode: 'premultiplied'
  });
  ctx.getCurrentTexture();

  // getConfiguration will succeed after configure.
  const newConfiguration = ctx.getConfiguration();
  assert(newConfiguration !== null);
  t.expect(newConfiguration.device === t.device);
  t.expect(newConfiguration.format === 'rgba8unorm');
  t.expect(newConfiguration.usage === GPUTextureUsage.RENDER_ATTACHMENT);
  t.expect(newConfiguration.viewFormats.length === 0);
  t.expect(newConfiguration.colorSpace === 'srgb');
  t.expect(newConfiguration.toneMapping?.mode === 'standard');
  t.expect(newConfiguration.alphaMode === 'premultiplied');
});

g.test('format').
desc(
  `
    Ensure that only valid texture formats are allowed when calling configure.
    `
).
params((u) =>
u //
.combine('canvasType', kAllCanvasTypes).
combine('format', kAllTextureFormats)
).
fn((t) => {
  const { canvasType, format } = t.params;
  t.skipIfTextureFormatNotSupported(format);
  const canvas = createCanvas(t, canvasType, 2, 2);
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  // Would prefer to use kCanvasTextureFormats.includes(format), but that's giving TS errors.
  let validFormat = false;
  for (const canvasFormat of kCanvasTextureFormats) {
    if (format === canvasFormat) {
      validFormat = true;
      break;
    }
  }

  if (validFormat) {
    ctx.configure({
      device: t.device,
      format
    });
    const configuration = ctx.getConfiguration();
    t.expect(configuration.format === format);
  } else {
    t.shouldThrow('TypeError', () => {
      ctx.configure({
        device: t.device,
        format
      });
    });
  }
});

g.test('usage').
desc(
  `
    Ensure that getCurrentTexture returns a texture with the configured usages.
    `
).
params((u) =>
u //
.combine('canvasType', kAllCanvasTypes).
beginSubcases().
combine(
  'usage',
  kValidCombinationsOfOneOrTwoTextureUsages
  // TypeError cases are tested in the next test.
  .filter((usage) => !usageIsTypeErrorForConfigure(usage))
)
).
fn((t) => {
  const { canvasType, usage } = t.params;

  // MAINTENANCE_TODO(#4509): Remove this after all implementations have TRANSIENT_ATTACHMENT.
  if ((usage & GPUConst.TextureUsage.TRANSIENT_ATTACHMENT) !== 0) {
    t.skipIfTransientAttachmentNotSupported();
  }

  const canvas = createCanvas(t, canvasType, 2, 2);
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  ctx.configure({
    device: t.device,
    format: 'rgba8unorm',
    usage
  });

  const configuration = ctx.getConfiguration();
  t.expect(configuration.usage === usage);

  const currentTexture = ctx.getCurrentTexture();
  t.expect(currentTexture instanceof GPUTexture);
  t.expect(currentTexture.usage === usage);

  // Try to use the texture with the given usage

  if (usage & GPUConst.TextureUsage.RENDER_ATTACHMENT) {
    const encoder = t.device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [
      {
        view: currentTexture.createView(),
        clearValue: [1.0, 0.0, 0.0, 1.0],
        loadOp: 'clear',
        storeOp: 'store'
      }]

    });
    pass.end();
    t.device.queue.submit([encoder.finish()]);
  }

  if (usage & GPUConst.TextureUsage.TEXTURE_BINDING) {
    const bgl = t.device.createBindGroupLayout({
      entries: [
      {
        binding: 0,
        visibility: GPUShaderStage.FRAGMENT,
        texture: {}
      }]

    });

    t.device.createBindGroup({
      layout: bgl,
      entries: [
      {
        binding: 0,
        resource: currentTexture.createView()
      }]

    });
  }

  const canUseStorageTextureInFragmentShader =
  !t.isCompatibility || t.device.limits.maxStorageTexturesInFragmentStage > 0;

  if (
  (usage & GPUConst.TextureUsage.STORAGE_BINDING) !== 0 &&
  canUseStorageTextureInFragmentShader)
  {
    const bgl = t.device.createBindGroupLayout({
      entries: [
      {
        binding: 0,
        visibility: GPUShaderStage.FRAGMENT,
        storageTexture: { access: 'write-only', format: currentTexture.format }
      }]

    });

    t.device.createBindGroup({
      layout: bgl,
      entries: [
      {
        binding: 0,
        resource: currentTexture.createView()
      }]

    });
  }

  if (usage & GPUConst.TextureUsage.COPY_DST) {
    const rgbaData = new Uint8Array([255, 0, 0, 255]);

    t.device.queue.writeTexture({ texture: currentTexture }, rgbaData, {}, [1, 1, 1]);
  }

  if (usage & GPUConst.TextureUsage.COPY_SRC) {
    const size = [currentTexture.width, currentTexture.height, 1];
    const dstTexture = t.createTextureTracked({
      format: currentTexture.format,
      usage: GPUTextureUsage.COPY_DST,
      size
    });

    const encoder = t.device.createCommandEncoder();
    encoder.copyTextureToTexture({ texture: currentTexture }, { texture: dstTexture }, size);
    t.device.queue.submit([encoder.finish()]);
  }
});

g.test('invalid_usage').
desc(`Test what happens when an invalid usage is passed to configure().`).
params((u) =>
u //
.combine('canvasType', kAllCanvasTypes)
// Test all formats because sometimes they're handled differently (require internal blits).
.combine('format', kCanvasTextureFormats).
combine('usage', [
0,
GPUConst.TextureUsage.COPY_DST,
// Special case for bgra8unorm vs other formats
GPUConst.TextureUsage.STORAGE_BINDING,
// Special case(s) for canvases vs regular textures
...kTextureUsages.filter(usageIsTypeErrorForConfigure),
GPUConst.TextureUsage.TRANSIENT_ATTACHMENT | GPUConst.TextureUsage.RENDER_ATTACHMENT,
// Various usages containing a bogus usage
kSomeBogusTextureUsage,
GPUConst.TextureUsage.COPY_DST | kSomeBogusTextureUsage,
GPUConst.TextureUsage.STORAGE_BINDING | kSomeBogusTextureUsage,
GPUConst.TextureUsage.TRANSIENT_ATTACHMENT | kSomeBogusTextureUsage]
)
).
fn((t) => {
  const { canvasType, format, usage } = t.params;

  // MAINTENANCE_TODO(#4509): Remove this after all implementations have TRANSIENT_ATTACHMENT.
  if ((usage & GPUConst.TextureUsage.TRANSIENT_ATTACHMENT) !== 0) {
    t.skipIfTransientAttachmentNotSupported();
  }

  const canvas = createCanvas(t, canvasType, 2, 2);
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  if (usageIsTypeErrorForConfigure(usage)) {
    t.shouldThrow('TypeError', () => {
      ctx.configure({ device: t.device, format, usage });
    });

    const configuration = ctx.getConfiguration();
    assert(configuration === null);

    t.shouldThrow('InvalidStateError', () => {
      ctx.getCurrentTexture();
    });
  } else {
    // Otherwise just the normal createTexture() usage validation applies.
    const hasStorageUsage = (usage & GPUConst.TextureUsage.STORAGE_BINDING) !== 0;
    const isUnsupportedBgra8unormStorage =
    format === 'bgra8unorm' &&
    hasStorageUsage &&
    !hasFeature(t.device.features, 'bgra8unorm-storage');
    const invalidUsage = !isValidTextureUsageCombination(usage) || isUnsupportedBgra8unormStorage;
    t.expectValidationError(() => {
      ctx.configure({ device: t.device, format, usage });
    }, invalidUsage);

    // The canvas still gets configured, and getCurrentTexture() can still be called.
    const configuration = ctx.getConfiguration();
    assert(configuration !== null);
    t.expect(configuration.usage === usage);

    let currentTexture;
    t.expectValidationError(() => {
      currentTexture = ctx.getCurrentTexture();
    }, invalidUsage);
    assert(currentTexture !== undefined && currentTexture instanceof GPUTexture);
    t.expect(currentTexture.usage === usage);
  }
});

g.test('alpha_mode').
desc(
  `
    Ensure that all valid alphaMode values are allowed when calling configure.
    `
).
params((u) =>
u //
.combine('canvasType', kAllCanvasTypes).
beginSubcases().
combine('alphaMode', ['opaque', 'premultiplied'])
).
fn((t) => {
  const { canvasType, alphaMode } = t.params;
  const canvas = createCanvas(t, canvasType, 2, 2);
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  ctx.configure({
    device: t.device,
    format: 'rgba8unorm',
    alphaMode
  });

  const configuration = ctx.getConfiguration();
  t.expect(configuration.alphaMode === alphaMode);

  const currentTexture = ctx.getCurrentTexture();
  t.expect(currentTexture instanceof GPUTexture);
});

g.test('size_zero_before_configure').
desc(`Ensure a validation error is raised in configure() if the size of the canvas is zero.`).
params((u) =>
u //
.combine('canvasType', kAllCanvasTypes).
combine('zeroDimension', ['width', 'height'])
).
fn((t) => {
  const { canvasType, zeroDimension } = t.params;
  const canvas = createCanvas(t, canvasType, 1, 1);
  canvas[zeroDimension] = 0;
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  // Validation error, the canvas size is 0 which doesn't make a valid GPUTextureDescriptor.
  t.expectValidationError(() => {
    ctx.configure({
      device: t.device,
      format: 'bgra8unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT
    });
  });

  canvas[zeroDimension] = 1;

  // The size being incorrect doesn't make for an invalid configuration. Now that it is fixed
  // getting textures from the canvas should work.
  const currentTexture = ctx.getCurrentTexture();

  // Try rendering to it even!
  const encoder = t.device.createCommandEncoder();
  const pass = encoder.beginRenderPass({
    colorAttachments: [
    {
      view: currentTexture.createView(),
      clearValue: [1.0, 0.0, 0.0, 1.0],
      loadOp: 'clear',
      storeOp: 'store'
    }]

  });
  pass.end();
  t.device.queue.submit([encoder.finish()]);
});

g.test('size_zero_after_configure').
desc(
  `Ensure a validation error is raised after configure() if the size of the canvas becomes zero.`
).
params((u) =>
u //
.combine('canvasType', kAllCanvasTypes).
combine('zeroDimension', ['width', 'height'])
).
fn((t) => {
  const { canvasType, zeroDimension } = t.params;
  const canvas = createCanvas(t, canvasType, 1, 1);
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  ctx.configure({
    device: t.device,
    format: 'bgra8unorm',
    usage: GPUTextureUsage.RENDER_ATTACHMENT
  });

  canvas[zeroDimension] = 0;

  // The size is incorrect, we should be getting an error texture and a validation error.
  let currentTexture;
  t.expectValidationError(() => {
    currentTexture = ctx.getCurrentTexture();
  });

  t.expect(currentTexture[zeroDimension] === 0);

  // Using the texture should produce a validation error.
  t.expectValidationError(() => {
    currentTexture.createView();
  });
});

g.test('viewFormats').
desc(
  `Test the validation that viewFormats are compatible with the format (for all canvas format / view formats)`
).
params((u) =>
u.
combine('canvasType', kAllCanvasTypes).
combine('format', kCanvasTextureFormats).
combine('viewFormatFeature', kFeaturesForFormats).
beginSubcases().
expand('viewFormat', ({ viewFormatFeature }) =>
filterFormatsByFeature(viewFormatFeature, kAllTextureFormats)
)
).
fn((t) => {
  const { canvasType, format, viewFormat } = t.params;

  t.skipIfTextureFormatNotSupported(viewFormat);

  const canvas = createCanvas(t, canvasType, 1, 1);
  const ctx = canvas.getContext('webgpu');
  assert(ctx instanceof GPUCanvasContext, 'Failed to get WebGPU context from canvas');

  const compatible = textureFormatsAreViewCompatible(t.device.features, format, viewFormat);

  // Test configure() produces an error if the formats aren't compatible.
  t.expectValidationError(() => {
    ctx.configure({
      device: t.device,
      format,
      viewFormats: [viewFormat]
    });
  }, !compatible);

  const viewFormats = ctx.getConfiguration().viewFormats;
  t.expect(viewFormats[0] === viewFormat);

  // Likewise for getCurrentTexture().
  let currentTexture;
  t.expectValidationError(() => {
    currentTexture = ctx.getCurrentTexture();
  }, !compatible);

  // The returned texture is an error texture.
  t.expectValidationError(() => {
    currentTexture.createView();
  }, !compatible);
});
});
__define('webgpu/util/create_elements.js', function (__exports) {
'use strict';
__get(__exports, 'kAllCanvasTypes', () => kAllCanvasTypes);
__get(__exports, 'kValidCanvasContextIds', () => kValidCanvasContextIds);
__get(__exports, 'createCanvas', () => createCanvas);
__get(__exports, 'createOnscreenCanvas', () => createOnscreenCanvas);
__get(__exports, 'createOffscreenCanvas', () => createOffscreenCanvas);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/let unreachable; __bind('common/util/util.js', (__m) => { ({ unreachable } = __m); });
// TESTING_TODO: This should expand to more canvas types (which will enhance a bunch of tests):
// - canvas element not in dom
// - canvas element in dom
// - offscreen canvas from transferControlToOffscreen from canvas not in dom
// - offscreen canvas from transferControlToOffscreen from canvas in dom
// - offscreen canvas from new OffscreenCanvas
const kAllCanvasTypes = ['onscreen', 'offscreen'];







/** Valid contextId for HTMLCanvasElement/OffscreenCanvas,
 *  spec: https://html.spec.whatwg.org/multipage/canvas.html#dom-canvas-getcontext
 */
const kValidCanvasContextIds = [
'2d',
'bitmaprenderer',
'webgl',
'webgl2',
'webgpu'];



/** Create HTMLCanvas/OffscreenCanvas. */
function createCanvas(
test,
canvasType,
width,
height)
{
  if (canvasType === 'onscreen') {
    if (typeof document !== 'undefined') {
      return createOnscreenCanvas(test, width, height);
    } else {
      test.skip('Cannot create HTMLCanvasElement');
    }
  } else if (canvasType === 'offscreen') {
    if (typeof OffscreenCanvas !== 'undefined') {
      return createOffscreenCanvas(test, width, height);
    } else {
      test.skip('Cannot create an OffscreenCanvas');
    }
  } else {
    unreachable();
  }
}

/** Create HTMLCanvasElement. */
function createOnscreenCanvas(
test,
width,
height)
{
  let canvas;
  if (typeof document !== 'undefined') {
    canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
  } else {
    test.skip('Cannot create HTMLCanvasElement');
  }
  return canvas;
}

/** Create OffscreenCanvas. */
function createOffscreenCanvas(
test,
width,
height)
{
  if (typeof OffscreenCanvas === 'undefined') {
    test.skip('OffscreenCanvas is not supported');
  }

  return new OffscreenCanvas(width, height);
}
});
__define('webgpu/web_platform/canvas/getPreferredCanvasFormat.spec.js', function (__exports) {
'use strict';
__get(__exports, 'description', () => description);
__get(__exports, 'g', () => g);
/**
* AUTO-GENERATED - DO NOT EDIT. Source: https://github.com/gpuweb/cts
**/const description = `
Tests for navigator.gpu.getPreferredCanvasFormat.
`;let Fixture, SkipTestCase; __bind('common/framework/fixture.js', (__m) => { ({ Fixture, SkipTestCase } = __m); });
let makeTestGroup; __bind('common/framework/test_group.js', (__m) => { ({ makeTestGroup } = __m); });

const g = makeTestGroup(Fixture);

g.test('value').
desc(
  `
    Ensure getPreferredCanvasFormat returns one of the valid values.
    `
).
beforeAllSubcases((t) => {
  if (typeof navigator === 'undefined') {
    throw new SkipTestCase('navigator does not exist in this environment');
  }
}).
fn((t) => {
  const preferredFormat = navigator.gpu.getPreferredCanvasFormat();
  t.expect(preferredFormat === 'bgra8unorm' || preferredFormat === 'rgba8unorm');
});
});
__require('common/runtime/wpt.js');
})();
