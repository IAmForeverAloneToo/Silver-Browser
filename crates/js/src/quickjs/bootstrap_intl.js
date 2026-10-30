// Intl (ECMA-402, gap batch D, D63): a binding chunk (D64), compiled
// apart from the bootstrap and read into a realm only when a page first
// uses it, by `Intl` or a locale form of the language's methods. A
// function of what of the bootstrap's it uses, as the bootstrap names it,
// giving the locale forms the bootstrap's methods call. The bootstrap's
// rules hold here: what is kept of the built-ins was kept before any page
// ran, nothing looked up on a built-in's prototype when a page calls in,
// no iterator.
(function (env) {
  "use strict";
  const {
    $Date, $Number, $Object, $RangeError, $String, $TypeError, $WeakMap, ArrayIndexOf, ArrayPush,
    ArraySlice, ArraySort, BigIntToString, BigIntValueOf, CharCodeAt, DateGetTime,
    DateGetTimezoneOffset, DateNow, MathAbs, MathFloor, MathMax, MathMin, MathTrunc, NumberValueOf,
    ObjectCreate, ObjectPrototype, StringIndexOf, StringSlice, ToUpperCase, WeakMapGet, WeakMapSet,
    defineProperty, getOwnPropertyDescriptor, getOwnPropertyNames, global, languageList, native,
    toString, toStringTag,
  } = env;

  // ---- Intl (ECMA-402, gap batch D, D63) ------------------------------------

  // The namespace (ECMA-402 section 8): an ordinary object, not a
  // constructor, its members not enumerable, as the language's own
  // built-ins are. What it computes is the natives' (`intl...`), by
  // ICU4X with its compiled data; RelativeTimeFormat, ListFormat,
  // Segmenter and DisplayNames are not here.
  const IntlNamespace = ObjectCreate(ObjectPrototype);
  defineProperty(IntlNamespace, toStringTag, { value: "Intl", configurable: true });
  defineProperty(global, "Intl", {
    value: IntlNamespace,
    writable: true,
    configurable: true,
    enumerable: false,
  });
  // The internal slots of the objects the constructors make, by object.
  const intlSlots = new $WeakMap();

  // A built-in's data property: writable, configurable, not enumerable.
  function intlDefine(target, name, value) {
    defineProperty(target, name, { value, writable: true, configurable: true, enumerable: false });
  }

  // Makes `ctor` one of the namespace's constructors: its prototype's
  // members those of `members` (an object literal, whose methods are
  // not constructors), not enumerable, its class string "Intl." and its
  // name, its `prototype` not writable, and `supportedLocalesOf` on it.
  function intlClass(ctor, name, members) {
    const proto = ctor.prototype;
    const keys = getOwnPropertyNames(members);
    for (let i = 0; i < keys.length; i++) {
      const desc = getOwnPropertyDescriptor(members, keys[i]);
      desc.enumerable = false;
      defineProperty(proto, keys[i], desc);
    }
    defineProperty(proto, toStringTag, { value: "Intl." + name, configurable: true });
    defineProperty(ctor, "prototype", { writable: false, enumerable: false, configurable: false });
    if (name !== "Locale") {
      intlDefine(ctor, "supportedLocalesOf", {
        supportedLocalesOf(locales) {
          const requested = canonicalLocales(locales);
          const options = coerceOptions(arguments[1]);
          getOption(options, "localeMatcher", "string", MATCHERS, "best fit", name);
          return native.intlSupported(requested);
        },
      }.supportedLocalesOf);
    }
    intlDefine(IntlNamespace, name, ctor);
  }

  // The slot of `object` if it is one `kind` made, else a TypeError.
  function intlSlot(object, kind, method) {
    const slot = (typeof object === "object" || typeof object === "function") && object !== null
      ? WeakMapGet(intlSlots, object)
      : undefined;
    if (slot === undefined || slot.kind !== kind) {
      throw new $TypeError("Method Intl." + kind + ".prototype." + method +
        " called on incompatible receiver " + toDisplay(object));
    }
    return slot;
  }

  function toDisplay(value) {
    if (typeof value === "string") return value;
    if (typeof value === "symbol") return "Symbol()";
    if ((typeof value === "object" || typeof value === "function") && value !== null) return "#<Object>";
    return $String(value);
  }

  const MATCHERS = ["lookup", "best fit"];

  // ECMA-402 section 9.2.12, CoerceOptionsToObject.
  function coerceOptions(options) {
    if (options === undefined) return ObjectCreate(null);
    if (options === null) throw new $TypeError("Cannot convert undefined or null to object");
    return $Object(options);
  }

  // Section 9.2.13, GetOption: the option read once, made a boolean or a
  // string, and held to its allowed values.
  function getOption(options, property, type, values, fallback, where) {
    let value = options[property];
    if (value === undefined) return fallback;
    value = type === "boolean" ? !!value : toString(value);
    if (values !== undefined && ArrayIndexOf(values, value) < 0) {
      throw new $RangeError("Value " + value + " out of range for Intl." + where +
        " options property " + property);
    }
    return value;
  }

  // Section 9.2.15 and section 9.2.16, DefaultNumberOption and
  // GetNumberOption.
  function defaultNumberOption(value, minimum, maximum, fallback, property) {
    if (value === undefined) return fallback;
    value = $Number(value);
    if (value !== value || value < minimum || value > maximum) {
      throw new $RangeError(property + " value is out of range.");
    }
    return MathFloor(value);
  }

  function getNumberOption(options, property, minimum, maximum, fallback) {
    return defaultNumberOption(options[property], minimum, maximum, fallback, property);
  }

  // The tag of an `Intl.Locale`, or undefined for anything else.
  function localeTag(value) {
    const slot = (typeof value === "object" || typeof value === "function") && value !== null
      ? WeakMapGet(intlSlots, value)
      : undefined;
    return slot !== undefined && slot.kind === "Locale" ? slot.tag : undefined;
  }

  // Section 9.2.1, CanonicalizeLocaleList: the tags of a string, a
  // Locale or a list of them, canonical, each once (at most a thousand).
  function canonicalLocales(locales) {
    if (locales === undefined) return [];
    const tags = [];
    const tag = localeTag(locales);
    if (typeof locales === "string" || tag !== undefined) {
      ArrayPush(tags, tag !== undefined ? tag : locales);
      return native.intlCanonical(tags);
    }
    if (locales === null) throw new $TypeError("Cannot convert undefined or null to object");
    const list = $Object(locales);
    const length = MathMin(MathMax(MathTrunc($Number(list.length)) || 0, 0), 9007199254740991);
    if (length > 1000) throw new $RangeError("Too many locales");
    for (let k = 0; k < length; k++) {
      if (!(k in list)) continue;
      const value = list[k];
      if (typeof value !== "string" && (typeof value !== "object" || value === null)) {
        throw new $TypeError("Language ID should be string or object.");
      }
      const its = localeTag(value);
      ArrayPush(tags, its !== undefined ? its : toString(value));
    }
    return native.intlCanonical(tags);
  }

  // The user's languages, which a negotiation falls back to before
  // `en-US` (section 6.2.4, DefaultLocale).
  function preferredLocales() {
    try {
      return languageList();
    } catch (error) {
      return [];
    }
  }

  // Whether a keyword's value is a `type` (UTS #35): subtags of three
  // to eight letters and digits.
  function isTypeSequence(value) {
    let run = 0;
    for (let i = 0; i < value.length; i++) {
      const c = CharCodeAt(value, i);
      if (c === 45) {
        if (run < 3) return false;
        run = 0;
      } else if ((c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122)) {
        if (++run > 8) return false;
      } else {
        return false;
      }
    }
    return run >= 3;
  }

  // What the reference browser's ICU supports of each keyword (section 9.1's
  // [[LocaleData]]); the lists `Intl.supportedValuesOf` answers.
  const INTL_CALENDARS = [
    "buddhist", "chinese", "coptic", "dangi", "ethioaa", "ethiopic", "gregory", "hebrew", "indian",
    "islamic", "islamic-civil", "islamic-rgsa", "islamic-tbla", "islamic-umalqura", "iso8601",
    "japanese", "persian", "roc",
  ];
  const INTL_COLLATIONS = [
    "compat", "emoji", "eor", "phonebk", "pinyin", "searchjl", "stroke", "trad", "unihan",
    "zhuyin",
  ];
  const INTL_NUMBERING = [
    "adlm", "ahom", "arab", "arabext", "bali", "beng", "bhks", "brah", "cakm", "cham", "deva",
    "diak", "fullwide", "gara", "gong", "gonm", "gujr", "gukh", "guru", "hanidec", "hmng", "hmnp",
    "java", "kali", "kawi", "khmr", "knda", "krai", "lana", "lanatham", "laoo", "latn", "lepc",
    "limb", "mathbold", "mathdbl", "mathmono", "mathsanb", "mathsans", "mlym", "modi", "mong",
    "mroo", "mtei", "mymr", "mymrepka", "mymrpao", "mymrshan", "mymrtlng", "nagm", "newa", "nkoo",
    "olck", "onao", "orya", "osma", "outlined", "rohg", "saur", "segment", "shrd", "sind", "sinh",
    "sora", "sund", "sunu", "takr", "talu", "tamldec", "telu", "thai", "tibt", "tirh", "tnsa",
    "tols", "vaii", "wara", "wcho",
  ];
  // The units section 6.6.2 sanctions.
  const INTL_UNITS = [
    "acre", "bit", "byte", "celsius", "centimeter", "day", "degree", "fahrenheit",
    "fluid-ounce", "foot", "gallon", "gigabit", "gigabyte", "gram", "hectare", "hour", "inch",
    "kilobit", "kilobyte", "kilogram", "kilometer", "liter", "megabit", "megabyte", "meter",
    "microsecond", "mile", "mile-scandinavian", "milliliter", "millimeter", "millisecond",
    "minute", "month", "nanosecond", "ounce", "percent", "petabyte", "pound", "second", "stone",
    "terabit", "terabyte", "week", "yard", "year",
  ];
  const INTL_CURRENCIES = [
    "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
    "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD",
    "CDF", "CHF", "CLP", "CNY", "COP", "CRC", "CUC", "CUP", "CVE", "CZK", "DJF", "DKK", "DOP",
    "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS", "GIP", "GMD", "GNF",
    "GTQ", "GYD", "HKD", "HNL", "HRK", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR", "ISK",
    "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT", "LAK",
    "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP", "MRU",
    "MUR", "MVR", "MWK", "MXN", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR", "NZD", "OMR",
    "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB", "RWF", "SAR",
    "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLL", "SOS", "SRD", "SSP", "STN", "SVC", "SYP",
    "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH", "UGX", "USD",
    "UYU", "UZS", "VES", "VND", "VUV", "WST", "XAF", "XCD", "XDR", "XOF", "XPF", "XSU", "YER",
    "ZAR", "ZMW", "ZWL",
  ];

  function validKeyword(key, value) {
    switch (key) {
      case "ca": return ArrayIndexOf(INTL_CALENDARS, value) >= 0;
      case "co": return ArrayIndexOf(INTL_COLLATIONS, value) >= 0;
      case "nu": return ArrayIndexOf(INTL_NUMBERING, value) >= 0;
      case "hc": return value === "h11" || value === "h12" || value === "h23" || value === "h24";
      case "kn": return value === "true" || value === "false";
      case "kf": return value === "upper" || value === "lower" || value === "false";
    }
    return false;
  }

  // Section 9.2.7, ResolveLocale: the locale negotiated, its keywords
  // of `keys` from its Unicode extension unless an option overrides
  // them, and the locale's name with those that came from it.
  function resolveLocale(requested, keys, options) {
    const found = native.intlResolve(requested, preferredLocales());
    const values = { __proto__: null };
    let extension = "";
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      let value;
      let fromTag = false;
      for (let j = 1; j + 1 < found.length; j += 2) {
        if (found[j] === key) {
          value = found[j + 1] === "" ? "true" : found[j + 1];
          fromTag = validKeyword(key, value);
          if (!fromTag) value = undefined;
        }
      }
      const option = options[key];
      if (option !== undefined && validKeyword(key, option)) {
        if (option !== value) fromTag = false;
        value = option;
      }
      if (fromTag) extension += "-" + key + (value === "true" ? "" : "-" + value);
      values[key] = value;
    }
    return {
      locale: extension === "" ? found[0] : found[0] + "-u" + extension,
      dataLocale: found[0],
      values,
    };
  }

  // The tag a formatter is made for: the data locale and the keywords
  // ICU4X reads.
  function formatterTag(dataLocale, keywords) {
    let extension = "";
    for (let i = 0; i + 1 < keywords.length; i += 2) {
      if (keywords[i + 1] !== undefined) extension += "-" + keywords[i] + "-" + keywords[i + 1];
    }
    return extension === "" ? dataLocale : dataLocale + "-u" + extension;
  }

  // Parts as the natives give them, [type, text, ...], as one string or
  // as `formatToParts`'s objects.
  function joinParts(flat) {
    let text = "";
    for (let i = 1; i < flat.length; i += 2) text += flat[i];
    return text;
  }

  function partObjects(flat) {
    const parts = [];
    for (let i = 0; i + 1 < flat.length; i += 2) {
      ArrayPush(parts, { type: flat[i], value: flat[i + 1] });
    }
    return parts;
  }

  // A range's parts as the natives give them, [type, text, source, ...],
  // as one string or as `formatRangeToParts`'s objects.
  function joinRange(flat) {
    let text = "";
    for (let i = 1; i < flat.length; i += 3) text += flat[i];
    return text;
  }

  function rangeObjects(flat) {
    const parts = [];
    for (let i = 0; i + 2 < flat.length; i += 3) {
      ArrayPush(parts, { type: flat[i], value: flat[i + 1], source: flat[i + 2] });
    }
    return parts;
  }

  // ---- NumberFormat (section 15) ----

  const ROUNDING_MODES = [
    "halfExpand", "ceil", "floor", "expand", "trunc", "halfCeil", "halfFloor", "halfTrunc",
    "halfEven",
  ];
  const ROUNDING_INCREMENTS = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 2500, 5000];

  // The fraction digits the reference browser's ICU gives a currency
  // where they are not 2.
  function currencyDigits(currency) {
    switch (currency) {
      case "AFN": case "ALL": case "BIF": case "CLP": case "COP": case "DJF": case "GNF": case "HUF":
      case "IDR": case "IQD": case "IRR": case "ISK": case "JPY": case "KMF": case "KPW": case "KRW":
      case "LAK": case "LBP": case "MGA": case "MMK": case "PKR": case "PYG": case "RSD": case "RWF":
      case "SLL": case "SOS": case "SYP": case "UGX": case "VND": case "VUV": case "XAF": case "XOF":
      case "XPF": case "YER":
        return 0;
      case "BHD": case "JOD": case "KWD": case "LYD": case "OMR": case "TND":
        return 3;
    }
    return 2;
  }

  // Section 6.3.1, IsWellFormedCurrencyCode, and section 6.6.1,
  // IsWellFormedUnitIdentifier.
  function isCurrencyCode(code) {
    if (code.length !== 3) return false;
    for (let i = 0; i < 3; i++) {
      const c = CharCodeAt(code, i) | 32;
      if (c < 97 || c > 122) return false;
    }
    return true;
  }

  function isUnit(unit) {
    if (ArrayIndexOf(INTL_UNITS, unit) >= 0) return true;
    const per = StringIndexOf(unit, "-per-");
    return per > 0 && ArrayIndexOf(INTL_UNITS, StringSlice(unit, 0, per)) >= 0 &&
      ArrayIndexOf(INTL_UNITS, StringSlice(unit, per + 5)) >= 0;
  }

  // Section 15.1.3, SetNumberFormatDigitOptions.
  function setDigitOptions(slot, options, mnfdDefault, mxfdDefault, notation, where) {
    const mnid = getNumberOption(options, "minimumIntegerDigits", 1, 21, 1);
    let mnfd = options.minimumFractionDigits;
    let mxfd = options.maximumFractionDigits;
    let mnsd = options.minimumSignificantDigits;
    let mxsd = options.maximumSignificantDigits;
    slot.minimumIntegerDigits = mnid;
    const increment = getNumberOption(options, "roundingIncrement", 1, 5000, 1);
    if (ArrayIndexOf(ROUNDING_INCREMENTS, increment) < 0) {
      throw new $RangeError("roundingIncrement value is out of range.");
    }
    const mode = getOption(options, "roundingMode", "string", ROUNDING_MODES, "halfExpand", where);
    const priority = getOption(options, "roundingPriority", "string",
      ["auto", "morePrecision", "lessPrecision"], "auto", where);
    const trailing = getOption(options, "trailingZeroDisplay", "string",
      ["auto", "stripIfInteger"], "auto", where);
    if (increment !== 1) mxfdDefault = mnfdDefault;
    slot.roundingIncrement = increment;
    slot.roundingMode = mode;
    slot.trailingZeroDisplay = trailing;
    const hasSd = mnsd !== undefined || mxsd !== undefined;
    const hasFd = mnfd !== undefined || mxfd !== undefined;
    let needSd = true;
    let needFd = true;
    if (priority === "auto") {
      needSd = hasSd;
      if (needSd || (!hasFd && notation === "compact")) needFd = false;
    }
    if (needSd) {
      if (hasSd) {
        mnsd = defaultNumberOption(mnsd, 1, 21, 1, "minimumSignificantDigits");
        mxsd = defaultNumberOption(mxsd, mnsd, 21, 21, "maximumSignificantDigits");
        slot.minimumSignificantDigits = mnsd;
        slot.maximumSignificantDigits = mxsd;
      } else {
        slot.minimumSignificantDigits = 1;
        slot.maximumSignificantDigits = 21;
      }
    }
    if (needFd) {
      if (hasFd) {
        mnfd = defaultNumberOption(mnfd, 0, 100, undefined, "minimumFractionDigits");
        mxfd = defaultNumberOption(mxfd, 0, 100, undefined, "maximumFractionDigits");
        if (mnfd === undefined) {
          mnfd = MathMin(mnfdDefault, mxfd);
        } else if (mxfd === undefined) {
          mxfd = MathMax(mxfdDefault, mnfd);
        } else if (mnfd > mxfd) {
          throw new $RangeError("maximumFractionDigits value is out of range.");
        }
        slot.minimumFractionDigits = mnfd;
        slot.maximumFractionDigits = mxfd;
      } else {
        slot.minimumFractionDigits = mnfdDefault;
        slot.maximumFractionDigits = mxfdDefault;
      }
    }
    if (!needSd && !needFd) {
      slot.minimumFractionDigits = 0;
      slot.maximumFractionDigits = 0;
      slot.minimumSignificantDigits = 1;
      slot.maximumSignificantDigits = 2;
      slot.roundingType = 2;
      slot.roundingPriority = "morePrecision";
    } else if (priority === "auto") {
      slot.roundingType = needSd ? 1 : 0;
      slot.roundingPriority = "auto";
    } else {
      slot.roundingType = priority === "morePrecision" ? 2 : 3;
      slot.roundingPriority = priority;
    }
    if (increment !== 1) {
      if (slot.roundingType !== 0) {
        throw new $TypeError("roundingIncrement is only allowed with fraction digits rounding");
      }
      if (slot.maximumFractionDigits !== slot.minimumFractionDigits) {
        throw new $RangeError("maximumFractionDigits value is out of range.");
      }
    }
  }

  // The digit options as `intlNumber`'s and `intlPlural`'s codes.
  function digitCodes(slot, style) {
    const grouping = slot.useGrouping === false ? 0 : slot.useGrouping === "always" ? 2
      : slot.useGrouping === "min2" ? 3 : 1;
    return [
      style,
      slot.minimumIntegerDigits,
      slot.minimumFractionDigits === undefined ? 0 : slot.minimumFractionDigits,
      slot.maximumFractionDigits === undefined ? 0 : slot.maximumFractionDigits,
      slot.minimumSignificantDigits === undefined ? 1 : slot.minimumSignificantDigits,
      slot.maximumSignificantDigits === undefined ? 21 : slot.maximumSignificantDigits,
      slot.roundingType,
      grouping,
      ArrayIndexOf(["auto", "never", "always", "exceptZero", "negative"], slot.signDisplay),
      ArrayIndexOf(["standard", "scientific", "engineering", "compact"], slot.notation),
      ArrayIndexOf(ROUNDING_MODES, slot.roundingMode),
      slot.trailingZeroDisplay === "stripIfInteger" ? 1 : 0,
      slot.roundingIncrement,
      MathMax(ArrayIndexOf(["symbol", "narrowSymbol", "code", "name"], slot.currencyDisplay), 0),
      slot.currencySign === "accounting" ? 1 : 0,
      MathMax(ArrayIndexOf(["short", "narrow", "long"], slot.unitDisplay), 0),
      slot.compactDisplay === "long" ? 1 : 0,
    ];
  }

  // Section 15.1.2, InitializeNumberFormat.
  function NumberFormat() {
    if (new.target === undefined) {
      return new NumberFormat(arguments[0], arguments[1]);
    }
    const requested = canonicalLocales(arguments[0]);
    const options = coerceOptions(arguments[1]);
    const where = "NumberFormat";
    getOption(options, "localeMatcher", "string", MATCHERS, "best fit", where);
    const nu = getOption(options, "numberingSystem", "string", undefined, undefined, where);
    if (nu !== undefined && !isTypeSequence(nu)) {
      throw new $RangeError("Invalid numberingSystem : " + nu);
    }
    const r = resolveLocale(requested, ["nu"], { nu });
    const slot = { kind: "NumberFormat", locale: r.locale, dataLocale: r.dataLocale, format: undefined };
    slot.tag = formatterTag(r.dataLocale, ["nu", r.values.nu]);
    slot.numberingSystem = native.intlNumberingSystem(slot.tag);
    // Section 15.1.4, SetNumberFormatUnitOptions.
    const style = getOption(options, "style", "string",
      ["decimal", "percent", "currency", "unit"], "decimal", where);
    slot.style = style;
    let currency = getOption(options, "currency", "string", undefined, undefined, where);
    if (currency !== undefined && !isCurrencyCode(currency)) {
      throw new $RangeError("Invalid currency code : " + currency);
    }
    const currencyDisplay = getOption(options, "currencyDisplay", "string",
      ["code", "symbol", "narrowSymbol", "name"], "symbol", where);
    const currencySign = getOption(options, "currencySign", "string",
      ["standard", "accounting"], "standard", where);
    const unit = getOption(options, "unit", "string", undefined, undefined, where);
    if (unit !== undefined && !isUnit(unit)) {
      throw new $RangeError("Invalid unit argument for Intl.NumberFormat() '" + unit + "'");
    }
    const unitDisplay = getOption(options, "unitDisplay", "string",
      ["short", "narrow", "long"], "short", where);
    if (style === "currency") {
      if (currency === undefined) throw new $TypeError("Currency code is required with currency style.");
      currency = ToUpperCase(currency);
      slot.currency = currency;
      slot.currencyDisplay = currencyDisplay;
      slot.currencySign = currencySign;
    }
    if (style === "unit") {
      if (unit === undefined) throw new $TypeError("Unit is required with unit style.");
      slot.unit = unit;
      slot.unitDisplay = unitDisplay;
    }
    const notation = getOption(options, "notation", "string",
      ["standard", "scientific", "engineering", "compact"], "standard", where);
    slot.notation = notation;
    let mnfdDefault = 0;
    let mxfdDefault = style === "percent" ? 0 : 3;
    if (style === "currency" && notation === "standard") {
      mnfdDefault = mxfdDefault = currencyDigits(currency);
    }
    setDigitOptions(slot, options, mnfdDefault, mxfdDefault, notation, where);
    const compactDisplay = getOption(options, "compactDisplay", "string", ["short", "long"], "short", where);
    const groupingDefault = notation === "compact" ? "min2" : "auto";
    let grouping = options.useGrouping;
    if (grouping === undefined) {
      grouping = groupingDefault;
    } else if (grouping === true) {
      grouping = "always";
    } else if (!grouping) {
      grouping = false;
    } else {
      grouping = toString(grouping);
      if (grouping === "true" || grouping === "false") {
        grouping = groupingDefault;
      } else if (grouping !== "min2" && grouping !== "auto" && grouping !== "always") {
        throw new $RangeError("Value " + grouping + " out of range for Intl.NumberFormat options property useGrouping");
      }
    }
    slot.useGrouping = grouping;
    if (notation === "compact") slot.compactDisplay = compactDisplay;
    slot.signDisplay = getOption(options, "signDisplay", "string",
      ["auto", "never", "always", "exceptZero", "negative"], "auto", where);
    slot.codes = digitCodes(slot, ArrayIndexOf(["decimal", "percent", "currency", "unit"], style));
    WeakMapSet(intlSlots, this, slot);
  }

  // Section 15.5.16, ToIntlMathematicalValue: a number, or a string of
  // digits for a BigInt or a numeric string, which formats exactly.
  function intlNumeric(value) {
    if (typeof value === "bigint") return BigIntToString(value);
    if (typeof value === "string") return value;
    return $Number(value);
  }

  function formatNumberParts(slot, value) {
    return native.intlNumber(slot.tag, slot.codes, slotUnit(slot), intlNumeric(value));
  }

  function slotUnit(slot) {
    return slot.style === "currency" ? slot.currency : slot.style === "unit" ? slot.unit : "";
  }

  // Section 15.5.22, FormatNumericRange's checks: both ends given and
  // neither NaN.
  function rangeEnd(value, which) {
    if (value === undefined) throw new $TypeError(which + " is undefined");
    const x = intlNumeric(value);
    if ((typeof x === "number" && x !== x) || (typeof x === "string" && $Number(x) !== $Number(x))) {
      throw new $RangeError(which + " is NaN");
    }
    return x;
  }

  function formatNumberRange(slot, start, end) {
    const x = rangeEnd(start, "start");
    const y = rangeEnd(end, "end");
    return native.intlNumberRange(slot.tag, slot.codes, slotUnit(slot), x, y);
  }

  intlClass(NumberFormat, "NumberFormat", {
    get format() {
      const slot = intlSlot(this, "NumberFormat", "format");
      if (slot.format === undefined) {
        slot.format = (value) => joinParts(formatNumberParts(slot, value));
      }
      return slot.format;
    },
    formatToParts(value) {
      return partObjects(formatNumberParts(intlSlot(this, "NumberFormat", "formatToParts"), value));
    },
    formatRange(start, end) {
      return joinRange(formatNumberRange(intlSlot(this, "NumberFormat", "formatRange"), start, end));
    },
    formatRangeToParts(start, end) {
      const slot = intlSlot(this, "NumberFormat", "formatRangeToParts");
      return rangeObjects(formatNumberRange(slot, start, end));
    },
    resolvedOptions() {
      const slot = intlSlot(this, "NumberFormat", "resolvedOptions");
      const options = { locale: slot.locale, numberingSystem: slot.numberingSystem, style: slot.style };
      const names = [
        "currency", "currencyDisplay", "currencySign", "unit", "unitDisplay",
        "minimumIntegerDigits", "minimumFractionDigits", "maximumFractionDigits",
        "minimumSignificantDigits", "maximumSignificantDigits", "useGrouping", "notation",
        "compactDisplay", "signDisplay", "roundingIncrement", "roundingMode", "roundingPriority",
        "trailingZeroDisplay",
      ];
      for (let i = 0; i < names.length; i++) {
        if (slot[names[i]] !== undefined) options[names[i]] = slot[names[i]];
      }
      return options;
    },
  });

  // ---- PluralRules (section 17) ----

  function PluralRules() {
    if (new.target === undefined) {
      throw new $TypeError("Constructor Intl.PluralRules requires 'new'");
    }
    const requested = canonicalLocales(arguments[0]);
    const options = coerceOptions(arguments[1]);
    const where = "PluralRules";
    getOption(options, "localeMatcher", "string", MATCHERS, "best fit", where);
    const type = getOption(options, "type", "string", ["cardinal", "ordinal"], "cardinal", where);
    const slot = { kind: "PluralRules", type, signDisplay: "auto", notation: "standard" };
    setDigitOptions(slot, options, 0, 3, "standard", where);
    const r = resolveLocale(requested, [], {});
    slot.locale = r.locale;
    slot.dataLocale = r.dataLocale;
    slot.codes = digitCodes(slot, 0);
    WeakMapSet(intlSlots, this, slot);
  }

  intlClass(PluralRules, "PluralRules", {
    select(value) {
      const slot = intlSlot(this, "PluralRules", "select");
      return native.intlPlural(slot.dataLocale, slot.type === "ordinal", slot.codes, $Number(value));
    },
    // Section 17.3.4, Intl.PluralRules.prototype.selectRange.
    selectRange(start, end) {
      const slot = intlSlot(this, "PluralRules", "selectRange");
      if (start === undefined || end === undefined) {
        throw new $TypeError("start or end is undefined");
      }
      const x = $Number(start);
      const y = $Number(end);
      if (x !== x || y !== y) throw new $RangeError("start or end is NaN");
      return native.intlPluralRange(slot.dataLocale, slot.type === "ordinal", slot.codes, x, y);
    },
    resolvedOptions() {
      const slot = intlSlot(this, "PluralRules", "resolvedOptions");
      const options = { locale: slot.locale, type: slot.type };
      const names = [
        "minimumIntegerDigits", "minimumFractionDigits", "maximumFractionDigits",
        "minimumSignificantDigits", "maximumSignificantDigits",
      ];
      for (let i = 0; i < names.length; i++) {
        if (slot[names[i]] !== undefined) options[names[i]] = slot[names[i]];
      }
      options.pluralCategories = native.intlPluralCategories(slot.dataLocale, slot.type === "ordinal");
      options.roundingIncrement = slot.roundingIncrement;
      options.roundingMode = slot.roundingMode;
      options.roundingPriority = slot.roundingPriority;
      options.trailingZeroDisplay = slot.trailingZeroDisplay;
      return options;
    },
  });

  // ---- Collator (section 10) ----

  const SENSITIVITIES = ["base", "accent", "case", "variant"];

  // Section 10.1.2, InitializeCollator.
  function Collator() {
    if (new.target === undefined) {
      return new Collator(arguments[0], arguments[1]);
    }
    const requested = canonicalLocales(arguments[0]);
    const options = coerceOptions(arguments[1]);
    const where = "Collator";
    const usage = getOption(options, "usage", "string", ["sort", "search"], "sort", where);
    getOption(options, "localeMatcher", "string", MATCHERS, "best fit", where);
    const co = getOption(options, "collation", "string", undefined, undefined, where);
    if (co !== undefined && !isTypeSequence(co)) {
      throw new $RangeError("Invalid collation : " + co);
    }
    let kn = getOption(options, "numeric", "boolean", undefined, undefined, where);
    if (kn !== undefined) kn = kn ? "true" : "false";
    const kf = getOption(options, "caseFirst", "string", ["upper", "lower", "false"], undefined, where);
    const r = resolveLocale(requested, ["co", "kf", "kn"], { co, kf, kn });
    const sensitivity = getOption(options, "sensitivity", "string", SENSITIVITIES, "variant", where);
    const ignore = getOption(options, "ignorePunctuation", "boolean", undefined, undefined, where);
    const slot = {
      kind: "Collator",
      locale: r.locale,
      usage,
      sensitivity,
      ignorePunctuation: ignore === undefined ? StringIndexOf(r.dataLocale + "-", "th-") === 0 : ignore,
      collation: r.values.co === undefined ? "default" : r.values.co,
      numeric: r.values.kn === "true",
      caseFirst: r.values.kf === undefined ? "false" : r.values.kf,
      compare: undefined,
    };
    slot.tag = formatterTag(r.dataLocale, ["co", r.values.co, "kf", r.values.kf, "kn", r.values.kn]);
    slot.codes = [
      ArrayIndexOf(SENSITIVITIES, sensitivity), slot.ignorePunctuation ? 1 : 0, usage === "search" ? 1 : 0,
    ];
    WeakMapSet(intlSlots, this, slot);
  }

  function compareStrings(slot, x, y) {
    return native.intlCompare(slot.tag, slot.codes, toString(x), toString(y));
  }

  intlClass(Collator, "Collator", {
    get compare() {
      const slot = intlSlot(this, "Collator", "compare");
      if (slot.compare === undefined) {
        slot.compare = (x, y) => compareStrings(slot, x, y);
      }
      return slot.compare;
    },
    resolvedOptions() {
      const slot = intlSlot(this, "Collator", "resolvedOptions");
      return {
        locale: slot.locale,
        usage: slot.usage,
        sensitivity: slot.sensitivity,
        ignorePunctuation: slot.ignorePunctuation,
        collation: slot.collation,
        numeric: slot.numeric,
        caseFirst: slot.caseFirst,
      };
    },
  });

  // ---- DateTimeFormat (section 11) ----

  // The component options (section 11.2.3's table) in order, with the
  // values each takes; a value's code is its index plus one.
  const DATE_COMPONENTS = [
    ["weekday", ["narrow", "short", "long"]],
    ["era", ["narrow", "short", "long"]],
    ["year", ["numeric", "2-digit"]],
    ["month", ["numeric", "2-digit", "narrow", "short", "long"]],
    ["day", ["numeric", "2-digit"]],
    ["dayPeriod", ["narrow", "short", "long"]],
    ["hour", ["numeric", "2-digit"]],
    ["minute", ["numeric", "2-digit"]],
    ["second", ["numeric", "2-digit"]],
    ["fractionalSecondDigits", undefined],
    ["timeZoneName", ["short", "long", "shortOffset", "longOffset", "shortGeneric", "longGeneric"]],
  ];
  const DATE_STYLES = ["full", "long", "medium", "short"];

  // The system's time zone: its IANA name, or "" when the system does
  // not say, when the zone is named by its offset now (section 6.4.3).
  let systemZoneName = null;
  function systemZone() {
    if (systemZoneName === null) systemZoneName = native.intlSystemZone();
    return systemZoneName;
  }

  function offsetName(minutes) {
    const sign = minutes < 0 ? "-" : "+";
    const abs = MathAbs(minutes);
    const h = MathFloor(abs / 60);
    const m = abs % 60;
    return sign + (h < 10 ? "0" : "") + h + ":" + (m < 10 ? "0" : "") + m;
  }

  const UTC_NAMES = [
    "UTC", "ETC/UTC", "GMT", "ETC/GMT", "UCT", "ETC/UCT", "ZULU", "ETC/ZULU", "UNIVERSAL",
    "ETC/UNIVERSAL", "GREENWICH", "ETC/GREENWICH", "GMT0", "ETC/GMT0", "GMT+0", "ETC/GMT+0",
    "GMT-0", "ETC/GMT-0",
  ];

  // The minutes of an offset time zone's name (section 6.4.1,
  // IsTimeZoneOffsetString: +HH, +HHMM or +HH:MM), or undefined.
  function offsetMinutes(text) {
    const c = CharCodeAt(text, 0);
    if (c !== 43 && c !== 45 && c !== 0x2212) return undefined;
    const digit = (i) => {
      const d = CharCodeAt(text, i) - 48;
      return d >= 0 && d <= 9 ? d : -100;
    };
    let minutes;
    if (text.length === 3) {
      minutes = (digit(1) * 10 + digit(2)) * 60;
    } else if (text.length === 5) {
      minutes = (digit(1) * 10 + digit(2)) * 60 + digit(3) * 10 + digit(4);
    } else if (text.length === 6 && CharCodeAt(text, 3) === 58) {
      minutes = (digit(1) * 10 + digit(2)) * 60 + digit(4) * 10 + digit(5);
    } else {
      return undefined;
    }
    if (minutes < 0 || minutes >= 24 * 60 || (minutes % 60) >= 60) return undefined;
    return c === 43 ? minutes : -minutes;
  }

  // The time zone a DateTimeFormat formats in: UTC, a fixed offset, the
  // system's, or an IANA zone by its name as ICU canonicalizes it, its
  // offsets the natives' (section 11.1.2 steps 29-35).
  function timeZoneOf(value) {
    if (value === undefined) {
      const name = systemZone();
      if (name !== "") return { kind: "system", name };
      const now = -DateGetTimezoneOffset(new $Date(DateNow()));
      return { kind: "system", name: now === 0 ? "UTC" : offsetName(now) };
    }
    const text = toString(value);
    const upper = ToUpperCase(text);
    if (ArrayIndexOf(UTC_NAMES, upper) >= 0) return { kind: "utc", name: "UTC" };
    const minutes = offsetMinutes(text);
    if (minutes !== undefined) return { kind: "offset", name: offsetName(minutes), offset: minutes };
    const name = native.intlTimeZone(text);
    if (name === "") throw new $RangeError("Invalid time zone specified: " + text);
    if (name === "UTC") return { kind: "utc", name };
    if (name === systemZone()) return { kind: "system", name };
    return { kind: "zone", name };
  }

  // Section 11.1.2, CreateDateTimeFormat, `required` and `defaults` as
  // section 11.1.2 names them ("date", "time" or "any"; "date", "time"
  // or "all").
  function createDateTimeFormat(object, locales, options, required, defaults) {
    const requested = canonicalLocales(locales);
    options = coerceOptions(options);
    const where = "DateTimeFormat";
    getOption(options, "localeMatcher", "string", MATCHERS, "best fit", where);
    const ca = getOption(options, "calendar", "string", undefined, undefined, where);
    if (ca !== undefined && !isTypeSequence(ca)) throw new $RangeError("Invalid calendar : " + ca);
    const nu = getOption(options, "numberingSystem", "string", undefined, undefined, where);
    if (nu !== undefined && !isTypeSequence(nu)) {
      throw new $RangeError("Invalid numberingSystem : " + nu);
    }
    const hour12 = getOption(options, "hour12", "boolean", undefined, undefined, where);
    let hourCycle = getOption(options, "hourCycle", "string", ["h11", "h12", "h23", "h24"], undefined, where);
    if (hour12 !== undefined) hourCycle = null;
    const r = resolveLocale(requested, ["ca", "hc", "nu"],
      { ca, hc: hourCycle === null ? undefined : hourCycle, nu });
    const zone = timeZoneOf(options.timeZone);
    const slot = { kind: "DateTimeFormat", locale: r.locale, zone, format: undefined };
    const codes = [];
    const chosen = [];
    let explicit = undefined;
    for (let i = 0; i < DATE_COMPONENTS.length; i++) {
      const name = DATE_COMPONENTS[i][0];
      let value;
      if (name === "fractionalSecondDigits") {
        value = getNumberOption(options, name, 1, 3, undefined);
      } else {
        value = getOption(options, name, "string", DATE_COMPONENTS[i][1], undefined, where);
      }
      ArrayPush(chosen, value);
      if (value !== undefined && explicit === undefined) explicit = name;
    }
    getOption(options, "formatMatcher", "string", ["basic", "best fit"], "best fit", where);
    const dateStyle = getOption(options, "dateStyle", "string", DATE_STYLES, undefined, where);
    const timeStyle = getOption(options, "timeStyle", "string", DATE_STYLES, undefined, where);
    if (dateStyle !== undefined || timeStyle !== undefined) {
      if (explicit !== undefined) {
        throw new $TypeError("Can't set option " + explicit + " when " +
          (dateStyle !== undefined ? "dateStyle" : "timeStyle") + " is used");
      }
      if (required === "date" && timeStyle !== undefined) {
        throw new $TypeError("Invalid option : timeStyle");
      }
      if (required === "time" && dateStyle !== undefined) {
        throw new $TypeError("Invalid option : dateStyle");
      }
    } else {
      // Section 11.1.2 steps 39-41: the defaults when no field is asked for.
      let needDefaults = true;
      if (required === "date" || required === "any") {
        for (let i = 0; i < 5; i++) if (i !== 1 && chosen[i] !== undefined) needDefaults = false;
      }
      if (required === "time" || required === "any") {
        for (let i = 5; i < 10; i++) if (chosen[i] !== undefined) needDefaults = false;
      }
      if (needDefaults && (defaults === "date" || defaults === "all")) {
        chosen[2] = chosen[3] = chosen[4] = "numeric";
      }
      if (needDefaults && (defaults === "time" || defaults === "all")) {
        chosen[6] = chosen[7] = chosen[8] = "numeric";
      }
    }
    // The codes `intlDateTime` takes: the components but the day period,
    // the styles, then the day period.
    for (let i = 0; i < DATE_COMPONENTS.length; i++) {
      if (i === 5) continue;
      const values = DATE_COMPONENTS[i][1];
      const value = chosen[i];
      ArrayPush(codes, value === undefined ? 0 : values === undefined ? value : ArrayIndexOf(values, value) + 1);
    }
    ArrayPush(codes, dateStyle === undefined ? 0 : ArrayIndexOf(DATE_STYLES, dateStyle) + 1);
    ArrayPush(codes, timeStyle === undefined ? 0 : ArrayIndexOf(DATE_STYLES, timeStyle) + 1);
    ArrayPush(codes, chosen[5] === undefined ? 0 : ArrayIndexOf(DATE_COMPONENTS[5][1], chosen[5]) + 1);
    slot.components = chosen;
    slot.dateStyle = dateStyle;
    slot.timeStyle = timeStyle;
    // The hour cycle asked for: the locale's 12- or 24-hour one for
    // `hour12`, else the option's or the keyword's.
    const hc = hour12 !== undefined ? (hour12 ? "c12" : "c24") : r.values.hc;
    slot.tag = formatterTag(r.dataLocale, ["ca", r.values.ca, "hc", hc, "nu", r.values.nu]);
    slot.codes = codes;
    // The calendar, the hour cycle, and the components the pattern the
    // natives found resolves them to (V8 reports those).
    const info = native.intlDateTimeInfo(slot.tag, codes);
    slot.calendar = info[0];
    slot.hourCycle = info[1] === "" ? undefined : info[1];
    if (info.length > 2) {
      const resolved = [];
      for (let i = 0; i < DATE_COMPONENTS.length; i++) ArrayPush(resolved, undefined);
      for (let j = 2; j + 1 < info.length; j += 2) {
        for (let i = 0; i < DATE_COMPONENTS.length; i++) {
          if (DATE_COMPONENTS[i][0] === info[j]) {
            resolved[i] = i === 9 ? $Number(info[j + 1]) : info[j + 1];
          }
        }
      }
      slot.components = resolved;
    }
    slot.numberingSystem = native.intlNumberingSystem(formatterTag(r.dataLocale, ["nu", r.values.nu]));
    WeakMapSet(intlSlots, object, slot);
    return slot;
  }

  // Section 11.5.6's time value, TimeClip'd, or a RangeError.
  function timeValue(date) {
    const t = $Number(date);
    if (t !== t || t === Infinity || t === -Infinity || MathAbs(t) > 8.64e15) {
      throw new $RangeError("Invalid time value");
    }
    return MathTrunc(t) + 0;
  }

  // The zone's name for the natives ("UTC", an IANA name, or "" for one
  // known by its offset alone) and its offset at `t`, in minutes.
  function zoneAt(slot, t) {
    const zone = slot.zone;
    if (zone.kind === "utc") return ["UTC", 0];
    if (zone.kind === "offset") return ["", zone.offset];
    if (zone.kind === "zone") return [zone.name, native.intlZoneOffset(zone.name, t)];
    return [systemZone(), -DateGetTimezoneOffset(new $Date(t))];
  }

  // Section 11.5.6, FormatDateTimePattern's parts of a time value.
  function formatDateParts(slot, date) {
    const t = date === undefined ? DateNow() : timeValue(date);
    const at = zoneAt(slot, t);
    const flat = native.intlDateTime(slot.tag, slot.codes, at[0], t, at[1]);
    flat.length -= 1;
    return flat;
  }

  // Section 11.5.9, PartitionDateTimeRangePattern's parts of two.
  function formatDateRange(slot, start, end) {
    if (start === undefined || end === undefined) {
      throw new $TypeError("startDate or endDate is undefined");
    }
    const x = timeValue(start);
    const y = timeValue(end);
    const xAt = zoneAt(slot, x);
    const yAt = zoneAt(slot, y);
    return native.intlDateTimeRange(slot.tag, slot.codes, xAt[0], [x, xAt[1]], [y, yAt[1]]);
  }

  function DateTimeFormat() {
    if (new.target === undefined) {
      return new DateTimeFormat(arguments[0], arguments[1]);
    }
    createDateTimeFormat(this, arguments[0], arguments[1], "any", "date");
  }

  intlClass(DateTimeFormat, "DateTimeFormat", {
    get format() {
      const slot = intlSlot(this, "DateTimeFormat", "format");
      if (slot.format === undefined) {
        slot.format = (date) => joinParts(formatDateParts(slot, date));
      }
      return slot.format;
    },
    formatToParts(date) {
      return partObjects(formatDateParts(intlSlot(this, "DateTimeFormat", "formatToParts"), date));
    },
    formatRange(startDate, endDate) {
      const slot = intlSlot(this, "DateTimeFormat", "formatRange");
      return joinRange(formatDateRange(slot, startDate, endDate));
    },
    formatRangeToParts(startDate, endDate) {
      const slot = intlSlot(this, "DateTimeFormat", "formatRangeToParts");
      return rangeObjects(formatDateRange(slot, startDate, endDate));
    },
    resolvedOptions() {
      const slot = intlSlot(this, "DateTimeFormat", "resolvedOptions");
      const options = {
        locale: slot.locale,
        calendar: slot.calendar,
        numberingSystem: slot.numberingSystem,
        timeZone: slot.zone.name,
      };
      if (slot.hourCycle !== undefined) {
        options.hourCycle = slot.hourCycle;
        options.hour12 = slot.hourCycle === "h11" || slot.hourCycle === "h12";
      }
      if (slot.dateStyle === undefined && slot.timeStyle === undefined) {
        for (let i = 0; i < DATE_COMPONENTS.length; i++) {
          if (slot.components[i] !== undefined) options[DATE_COMPONENTS[i][0]] = slot.components[i];
        }
      }
      if (slot.dateStyle !== undefined) options.dateStyle = slot.dateStyle;
      if (slot.timeStyle !== undefined) options.timeStyle = slot.timeStyle;
      return options;
    },
  });

  // ---- Locale (section 14) ----

  // Section 14.1.1, the Intl.Locale constructor.
  function Locale(tag) {
    if (new.target === undefined) {
      throw new $TypeError("Constructor Intl.Locale requires 'new'");
    }
    if (typeof tag !== "string" && (typeof tag !== "object" || tag === null)) {
      throw new $TypeError("First argument to Intl.Locale constructor can't be empty or missing");
    }
    const its = localeTag(tag);
    const text = its !== undefined ? its : toString(tag);
    const options = coerceOptions(arguments[1]);
    const where = "Locale";
    const read = (name, values) => {
      const value = getOption(options, name, "string", values, undefined, where);
      return value === undefined ? "" : value;
    };
    const language = read("language");
    const script = read("script");
    const region = read("region");
    const ca = read("calendar");
    const co = read("collation");
    const hc = read("hourCycle", ["h11", "h12", "h23", "h24"]);
    const kf = read("caseFirst", ["upper", "lower", "false"]);
    const numeric = getOption(options, "numeric", "boolean", undefined, undefined, where);
    const kn = numeric === undefined ? "" : numeric ? "true" : "false";
    const nu = read("numberingSystem");
    const info = native.intlLocale(text, [language, script, region, ca, co, hc, kf, kn, nu]);
    WeakMapSet(intlSlots, this, {
      kind: "Locale",
      tag: info[0],
      language: info[1],
      script: info[2],
      region: info[3],
      baseName: info[4],
      calendar: info[5],
      collation: info[6],
      hourCycle: info[7],
      caseFirst: info[8],
      numeric: info[9],
      numberingSystem: info[10],
    });
  }

  function localeField(object, name) {
    const value = intlSlot(object, "Locale", name)[name];
    return value === "" ? undefined : value;
  }

  // The Intl Locale Info proposal's answers, each a new object or array:
  // the locale's own keyword's value if it has one, else its data's.
  function localeInfo(object, name) {
    const slot = intlSlot(object, "Locale", name);
    switch (name) {
      case "getCalendars":
        return slot.calendar !== "" ? [slot.calendar] : native.intlCalendars(slot.tag);
      case "getCollations":
        return slot.collation !== "" ? [slot.collation] : native.intlCollations(slot.tag);
      case "getHourCycles":
        return [slot.hourCycle !== "" ? slot.hourCycle : native.intlHourCycle(slot.baseName)];
      case "getNumberingSystems":
        return [slot.numberingSystem !== "" ? slot.numberingSystem
          : native.intlNumberingSystem(slot.baseName)];
      case "getTimeZones":
        return slot.region === "" ? undefined : native.intlTimeZones(slot.region);
      case "getTextInfo":
        return { direction: native.intlTextDirection(slot.tag) };
    }
    const week = native.intlWeekInfo(slot.tag);
    return { firstDay: week[0], weekend: ArraySlice(week, 1) };
  }

  intlClass(Locale, "Locale", {
    maximize() {
      return new Locale(native.intlLikely(intlSlot(this, "Locale", "maximize").tag, true));
    },
    minimize() {
      return new Locale(native.intlLikely(intlSlot(this, "Locale", "minimize").tag, false));
    },
    toString() {
      return intlSlot(this, "Locale", "toString").tag;
    },
    get baseName() { return localeField(this, "baseName"); },
    get calendar() { return localeField(this, "calendar"); },
    get caseFirst() { return localeField(this, "caseFirst"); },
    get collation() { return localeField(this, "collation"); },
    get hourCycle() { return localeField(this, "hourCycle"); },
    get numeric() { return intlSlot(this, "Locale", "numeric").numeric === "true"; },
    get numberingSystem() { return localeField(this, "numberingSystem"); },
    get language() { return localeField(this, "language"); },
    get script() { return localeField(this, "script"); },
    get region() { return localeField(this, "region"); },
    getCalendars() { return localeInfo(this, "getCalendars"); },
    getCollations() { return localeInfo(this, "getCollations"); },
    getHourCycles() { return localeInfo(this, "getHourCycles"); },
    getNumberingSystems() { return localeInfo(this, "getNumberingSystems"); },
    getTimeZones() { return localeInfo(this, "getTimeZones"); },
    getTextInfo() { return localeInfo(this, "getTextInfo"); },
    getWeekInfo() { return localeInfo(this, "getWeekInfo"); },
  });

  // ---- The namespace's functions (section 8.3) ----

  intlDefine(IntlNamespace, "getCanonicalLocales", {
    getCanonicalLocales(locales) {
      return canonicalLocales(locales);
    },
  }.getCanonicalLocales);

  intlDefine(IntlNamespace, "supportedValuesOf", {
    supportedValuesOf(key) {
      switch (toString(key)) {
        case "calendar": return ArraySlice(INTL_CALENDARS);
        case "collation": return ArraySlice(INTL_COLLATIONS);
        case "currency": return ArraySlice(INTL_CURRENCIES);
        case "numberingSystem": return ArraySlice(INTL_NUMBERING);
        case "timeZone": return native.intlTimeZones("");
        case "unit": return ArraySlice(INTL_UNITS);
      }
      throw new $RangeError("Invalid key : " + toString(key));
    },
  }.supportedValuesOf);

  // ---- The locale forms of the language's methods (sections 19-21) ----

  // The formatters with no locales nor options asked for, made once.
  let defaultNumberFormat = null;
  let defaultCollator = null;
  const defaultDateFormats = { __proto__: null };

  function numberFormatFor(locales, options) {
    if (locales === undefined && options === undefined) {
      if (defaultNumberFormat === null) defaultNumberFormat = new NumberFormat();
      return WeakMapGet(intlSlots, defaultNumberFormat);
    }
    return WeakMapGet(intlSlots, new NumberFormat(locales, options));
  }

  const numberToLocaleString = {
    toLocaleString() {
      const x = typeof this === "number" ? this : NumberValueOf(this);
      return joinParts(formatNumberParts(numberFormatFor(arguments[0], arguments[1]), x));
    },
  }.toLocaleString;

  const bigintToLocaleString = {
    toLocaleString() {
      const x = typeof this === "bigint" ? this : BigIntValueOf(this);
      return joinParts(formatNumberParts(numberFormatFor(arguments[0], arguments[1]), x));
    },
  }.toLocaleString;

  // Section 21.4.4.39 and kin: a date formatted with section 11.1.2's
  // `required` and `defaults`.
  function dateToLocale(date, locales, options, required, defaults) {
    const t = DateGetTime(date);
    if (t !== t) return "Invalid Date";
    let slot;
    if (locales === undefined && options === undefined) {
      slot = defaultDateFormats[required];
      if (slot === undefined) {
        slot = createDateTimeFormat(ObjectCreate(null), undefined, undefined, required, defaults);
        defaultDateFormats[required] = slot;
      }
    } else {
      slot = createDateTimeFormat(ObjectCreate(null), locales, options, required, defaults);
    }
    return joinParts(formatDateParts(slot, t));
  }

  const dateToLocaleString = {
    toLocaleString() {
      return dateToLocale(this, arguments[0], arguments[1], "any", "all");
    },
  }.toLocaleString;
  const dateToLocaleDateString = {
    toLocaleDateString() {
      return dateToLocale(this, arguments[0], arguments[1], "date", "date");
    },
  }.toLocaleDateString;
  const dateToLocaleTimeString = {
    toLocaleTimeString() {
      return dateToLocale(this, arguments[0], arguments[1], "time", "time");
    },
  }.toLocaleTimeString;

  const localeCompare = {
    localeCompare(that) {
      if (this === undefined || this === null) {
        throw new $TypeError("String.prototype.localeCompare called on null or undefined");
      }
      const locales = arguments[1];
      const options = arguments[2];
      let slot;
      if (locales === undefined && options === undefined) {
        if (defaultCollator === null) defaultCollator = new Collator();
        slot = WeakMapGet(intlSlots, defaultCollator);
      } else {
        slot = WeakMapGet(intlSlots, new Collator(locales, options));
      }
      return compareStrings(slot, this, that);
    },
  }.localeCompare;

  return {
    numberToLocaleString, bigintToLocaleString, dateToLocaleString, dateToLocaleDateString,
    dateToLocaleTimeString, localeCompare,
  };
})
