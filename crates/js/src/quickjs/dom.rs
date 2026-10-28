//! The DOM's natives (M25): the functions the bootstrap's classes call,
//! each turning its arguments into a [`Call`] of the host and the host's
//! [`Answer`] into a value, and charging the work the host reports to
//! the run's budget. Only the bootstrap's closure holds them.

use std::cell::Cell;
use std::rc::Rc;

use rquickjs::function::Rest;
use rquickjs::{Array, CaughtError, Coerced, Ctx, Exception, Function, Object, Value};

use super::{Entries, Slot, evaluate, report, thrown, with_host};
use crate::{
    Answer, Call, DomError, ErrorKind, Handle, MAX_HANDLE, RealmKind, ScriptError, ScriptingHost,
    WORK_PER_STEP,
};

/// The bindings, run in each new realm before any page's script.
pub(super) const BOOTSTRAP: &str = include_str!("bootstrap.js");
/// The name the bootstrap's frames carry in a stack, as its URL.
pub(super) const BOOTSTRAP_NAME: &str = "bootstrap";
/// WebGPU and WebGL's bindings (M41), read into a realm at their first
/// use: a function of the bootstrap's names they use.
pub(super) const GPU_BINDINGS: &str = include_str!("bootstrap_gpu.js");
/// The name their frames carry in a stack.
pub(super) const GPU_BINDINGS_NAME: &str = "bootstrap-gpu";

/// What the natives charge the work they report to: the run's steps,
/// the work not yet a whole step, and the budget's end.
#[derive(Clone)]
pub(super) struct Budget {
    pub(super) steps: Rc<Cell<u32>>,
    pub(super) work: Rc<Cell<u64>>,
    pub(super) exhausted: Rc<Cell<bool>>,
    pub(super) limit: u32,
}

impl Budget {
    /// Adds `units` of work, a step for each [`WORK_PER_STEP`]; past the
    /// budget the run is spent, which the interrupt handler ends at its
    /// next check, and no further native does anything meanwhile.
    pub(super) fn charge(&self, units: u64) {
        let work = self.work.get().saturating_add(units);
        let steps = u32::try_from(work / WORK_PER_STEP).unwrap_or(u32::MAX);
        self.work.set(work % WORK_PER_STEP);
        let total = self.steps.get().saturating_add(steps);
        self.steps.set(total);
        if total > self.limit {
            self.exhausted.set(true);
        }
    }
}

/// A native's arguments, read as the call it makes needs them.
struct Args<'a, 'js> {
    ctx: &'a Ctx<'js>,
    values: &'a [Value<'js>],
}

impl<'js> Args<'_, 'js> {
    fn value(&self, i: usize) -> Value<'js> {
        self.values
            .get(i)
            .cloned()
            .unwrap_or_else(|| Value::new_undefined(self.ctx.clone()))
    }

    /// A node's handle: a whole number no greater than [`MAX_HANDLE`].
    fn handle(&self, i: usize) -> rquickjs::Result<Handle> {
        let number = self.value(i).as_number().unwrap_or(f64::NAN);
        if number.fract() != 0.0 || !(0.0..=MAX_HANDLE as f64).contains(&number) {
            return Err(Exception::throw_type(self.ctx, "not a node"));
        }
        Ok(Handle(number as u64))
    }

    /// A handle, or `None` for `null` or `undefined`.
    fn maybe_handle(&self, i: usize) -> rquickjs::Result<Option<Handle>> {
        if self.value(i).is_null() || self.value(i).is_undefined() {
            return Ok(None);
        }
        self.handle(i).map(Some)
    }

    fn bool(&self, i: usize) -> bool {
        self.value(i).as_bool().unwrap_or(false)
    }

    /// A whole number the bootstrap has already converted as Web IDL
    /// says (a `long`, or a timer's id); anything else is 0.
    fn integer(&self, i: usize) -> f64 {
        let number = self.value(i).as_number().unwrap_or(0.0);
        if number.is_finite() {
            number.trunc()
        } else {
            0.0
        }
    }

    /// A number the bootstrap has already converted; anything else is 0.
    fn number(&self, i: usize) -> f64 {
        self.value(i).as_number().unwrap_or(0.0)
    }

    /// A list of strings, as an array of them.
    fn strings(&self, i: usize) -> rquickjs::Result<Vec<String>> {
        let Some(array) = self.value(i).into_array() else {
            return Err(Exception::throw_type(self.ctx, "not an array"));
        };
        array.iter::<String>().collect()
    }

    /// Bytes, as the bindings pass them: a string of code units 0 to 255
    /// (no byte crosses as a buffer, which `rquickjs` reads only through
    /// `unsafe`); `None` for `null`.
    fn bytes(&self, i: usize) -> rquickjs::Result<Option<Vec<u8>>> {
        if self.value(i).is_null() {
            return Ok(None);
        }
        let text = self.string(i)?;
        text.chars()
            .map(|c| u8::try_from(c).map_err(|_| Exception::throw_type(self.ctx, "not bytes")))
            .collect::<rquickjs::Result<Vec<u8>>>()
            .map(Some)
    }

    /// A canvas's call: the operation's name first, then its pixels as
    /// base64 when `bytes` holds them, then its arguments, each a number
    /// or a string (a boolean as 0 or 1).
    fn canvas(&self, bytes: Option<Vec<u8>>) -> rquickjs::Result<Call> {
        let op = self.string(0)?;
        let mut numbers = Vec::new();
        let mut strings = Vec::new();
        let first = if bytes.is_some() { 2 } else { 1 };
        for value in self.values.iter().skip(first) {
            if let Some(number) = value.as_number() {
                numbers.push(number);
            } else if let Some(flag) = value.as_bool() {
                numbers.push(f64::from(u8::from(flag)));
            } else if let Some(text) = value.as_string() {
                strings.push(text.to_string()?);
            } else {
                return Err(Exception::throw_type(self.ctx, "not a canvas argument"));
            }
        }
        Ok(Call::Canvas(Box::new(crate::dom::CanvasCall {
            op,
            numbers,
            strings,
            bytes,
        })))
    }

    /// A string, which the bootstrap has made well formed.
    fn string(&self, i: usize) -> rquickjs::Result<String> {
        match self.value(i).as_string() {
            Some(text) => text.to_string(),
            None => Err(Exception::throw_type(self.ctx, "not a string")),
        }
    }
}

/// A native that makes `call` of the host from its arguments.
type Decode = fn(&Args<'_, '_>) -> rquickjs::Result<Call>;

/// The natives whose calls may make scripts ready to run at once: those
/// that insert nodes (a script made by a script runs when it is
/// connected, or given children) or set attributes (or a `src`), and
/// `document.write`.
const RUN_SCRIPTS: &[&str] = &[
    "insert",
    "replace",
    "setTextContent",
    "setAttribute",
    "setInnerHTML",
    "setOuterHTML",
    "insertAdjacentHTML",
    "write",
];

/// The natives, by the name the bootstrap calls them by.
const NATIVES: &[(&str, Decode)] = &[
    ("document", |_| Ok(Call::Document)),
    ("wrap", |a| Ok(Call::Wrap(a.handle(0)?))),
    ("takePins", |_| Ok(Call::TakePins)),
    ("kind", |a| Ok(Call::Kind(a.handle(0)?))),
    ("tag", |a| Ok(Call::Tag(a.handle(0)?))),
    ("parent", |a| Ok(Call::Parent(a.handle(0)?))),
    ("firstChild", |a| Ok(Call::FirstChild(a.handle(0)?))),
    ("lastChild", |a| Ok(Call::LastChild(a.handle(0)?))),
    ("previousSibling", |a| {
        Ok(Call::PreviousSibling(a.handle(0)?))
    }),
    ("nextSibling", |a| Ok(Call::NextSibling(a.handle(0)?))),
    ("children", |a| Ok(Call::Children(a.handle(0)?))),
    ("elementChildren", |a| {
        Ok(Call::ElementChildren(a.handle(0)?))
    }),
    ("version", |_| Ok(Call::Version)),
    ("connected", |a| Ok(Call::Connected(a.handle(0)?))),
    ("contains", |a| {
        Ok(Call::Contains {
            node: a.handle(0)?,
            other: a.handle(1)?,
        })
    }),
    ("data", |a| Ok(Call::Data(a.handle(0)?))),
    ("setData", |a| Ok(Call::SetData(a.handle(0)?, a.string(1)?))),
    ("textContent", |a| Ok(Call::TextContent(a.handle(0)?))),
    ("setTextContent", |a| {
        Ok(Call::SetTextContent(a.handle(0)?, a.string(1)?))
    }),
    ("attribute", |a| {
        Ok(Call::Attribute(a.handle(0)?, a.string(1)?))
    }),
    ("setAttribute", |a| {
        Ok(Call::SetAttribute(a.handle(0)?, a.string(1)?, a.string(2)?))
    }),
    ("removeAttribute", |a| {
        Ok(Call::RemoveAttribute(a.handle(0)?, a.string(1)?))
    }),
    ("attributeNames", |a| Ok(Call::AttributeNames(a.handle(0)?))),
    ("createElement", |a| Ok(Call::CreateElement(a.string(0)?))),
    ("createElementNS", |a| {
        let ns = a.string(0)?;
        Ok(Call::CreateElementNs(
            (!ns.is_empty()).then_some(ns),
            a.string(1)?,
        ))
    }),
    ("byTagNS", |a| {
        let ns = a.string(1)?;
        Ok(Call::ElementsByTagNameNs(
            a.handle(0)?,
            (!ns.is_empty()).then_some(ns),
            a.string(2)?,
        ))
    }),
    ("createText", |a| Ok(Call::CreateText(a.string(0)?))),
    ("insert", |a| {
        Ok(Call::Insert {
            parent: a.handle(0)?,
            node: a.handle(1)?,
            before: a.maybe_handle(2)?,
        })
    }),
    ("remove", |a| {
        Ok(Call::Remove {
            parent: a.handle(0)?,
            node: a.handle(1)?,
        })
    }),
    ("replace", |a| {
        Ok(Call::Replace {
            parent: a.handle(0)?,
            node: a.handle(1)?,
            child: a.handle(2)?,
        })
    }),
    ("clone", |a| {
        Ok(Call::Clone {
            node: a.handle(0)?,
            deep: a.bool(1),
        })
    }),
    ("elementById", |a| {
        Ok(Call::ElementById(a.handle(0)?, a.string(1)?))
    }),
    ("query", |a| {
        Ok(Call::Query {
            root: a.handle(0)?,
            selectors: a.string(1)?,
            all: a.bool(2),
        })
    }),
    ("matches", |a| Ok(Call::Matches(a.handle(0)?, a.string(1)?))),
    ("closest", |a| Ok(Call::Closest(a.handle(0)?, a.string(1)?))),
    ("byTag", |a| {
        Ok(Call::ElementsByTagName(a.handle(0)?, a.string(1)?))
    }),
    ("byClass", |a| {
        Ok(Call::ElementsByClassName(a.handle(0)?, a.string(1)?))
    }),
    ("innerHTML", |a| Ok(Call::InnerHtml(a.handle(0)?))),
    ("outerHTML", |a| Ok(Call::OuterHtml(a.handle(0)?))),
    ("setInnerHTML", |a| {
        Ok(Call::SetInnerHtml(a.handle(0)?, a.string(1)?))
    }),
    ("setOuterHTML", |a| {
        Ok(Call::SetOuterHtml(a.handle(0)?, a.string(1)?))
    }),
    ("insertAdjacentHTML", |a| {
        Ok(Call::InsertAdjacentHtml {
            node: a.handle(0)?,
            position: a.string(1)?,
            markup: a.string(2)?,
        })
    }),
    ("write", |a| Ok(Call::Write(a.string(0)?))),
    ("currentScript", |_| Ok(Call::CurrentScript)),
    ("readyState", |_| Ok(Call::ReadyState)),
    ("childText", |a| Ok(Call::ChildText(a.handle(0)?))),
    ("forceAsync", |a| Ok(Call::ForceAsync(a.handle(0)?))),
    ("setForceAsync", |a| {
        Ok(Call::SetForceAsync(a.handle(0)?, a.bool(1)))
    }),
    ("parseDeclarations", |a| {
        Ok(Call::ParseDeclarations(a.string(0)?))
    }),
    ("cssCanonical", |a| Ok(Call::CssCanonical(a.string(0)?))),
    ("cssValid", |a| {
        Ok(Call::CssValid(a.string(0)?, a.string(1)?))
    }),
    ("resolveUrl", |a| Ok(Call::ResolveUrl(a.string(0)?))),
    ("resolveUrlFromDocument", |a| {
        Ok(Call::ResolveUrlFromDocument(a.string(0)?))
    }),
    ("documentUrl", |_| Ok(Call::DocumentUrl)),
    ("location", |_| Ok(Call::Location)),
    ("origin", |_| Ok(Call::Origin)),
    ("secureContext", |_| Ok(Call::SecureContext)),
    ("fetch", |a| {
        Ok(Call::Fetch(Box::new(crate::dom::FetchCall {
            id: a.integer(0) as u32,
            method: a.string(1)?,
            url: a.string(2)?,
            headers: a.strings(3)?,
            body: a.bytes(4)?,
            mode: a.string(5)?,
            credentials: a.string(6)?,
            redirect: a.string(7)?,
            preflight: a.bool(8),
            cache: a.string(9)?,
            referrer: a.string(10)?,
            referrer_policy: a.string(11)?,
            target_address_space: a.string(12)?,
        })))
    }),
    ("fetchNow", |a| {
        Ok(Call::FetchNow(Box::new(crate::dom::FetchCall {
            id: 0,
            method: a.string(0)?,
            url: a.string(1)?,
            headers: a.strings(2)?,
            body: a.bytes(3)?,
            mode: a.string(4)?,
            credentials: a.string(5)?,
            redirect: a.string(6)?,
            preflight: a.bool(7),
            cache: "default".to_owned(),
            referrer: "about:client".to_owned(),
            referrer_policy: String::new(),
            target_address_space: String::new(),
        })))
    }),
    ("decode", |a| {
        Ok(Call::Decode(a.bytes(0)?.unwrap_or_default(), a.string(1)?))
    }),
    ("abortFetch", |a| Ok(Call::AbortFetch(a.integer(0) as u32))),
    ("textDecoderName", |a| {
        Ok(Call::TextDecoderName(a.string(0)?))
    }),
    ("textDecode", |a| {
        Ok(Call::TextDecode {
            id: a.integer(0) as u32,
            encoding: a.string(1)?,
            bytes: a.bytes(2)?.unwrap_or_default(),
            fatal: a.bool(3),
            stream: a.bool(4),
        })
    }),
    ("mimeCharset", |a| Ok(Call::MimeCharset(a.string(0)?))),
    ("matchMedia", |a| Ok(Call::MatchMedia(a.string(0)?))),
    ("mediaMatches", |a| {
        Ok(Call::MediaMatches(a.integer(0) as u32))
    }),
    ("mediaForget", |a| {
        Ok(Call::MediaForget(a.integer(0) as u32))
    }),
    ("idleDeadline", |_| Ok(Call::IdleDeadline)),
    ("cssSupports", |a| {
        // A value only when one is given, as a string.
        let value = match a.value(1).is_string() {
            true => Some(a.string(1)?),
            false => None,
        };
        Ok(Call::CssSupports(a.string(0)?, value))
    }),
    ("blobUrlCreate", |a| {
        Ok(Call::BlobUrlCreate(
            a.bytes(0)?.unwrap_or_default(),
            a.string(1)?,
        ))
    }),
    ("blobUrlRevoke", |a| Ok(Call::BlobUrlRevoke(a.string(0)?))),
    ("urlParse", |a| {
        // A base only when one is given, as a string.
        let base = match a.value(1).is_string() {
            true => Some(a.string(1)?),
            false => None,
        };
        Ok(Call::UrlParse(a.string(0)?, base))
    }),
    ("urlSet", |a| {
        Ok(Call::UrlSet {
            href: a.string(0)?,
            part: a.string(1)?,
            value: a.string(2)?,
        })
    }),
    ("urlHasCredentials", |a| {
        Ok(Call::UrlHasCredentials(a.string(0)?))
    }),
    ("sameOriginUrl", |a| Ok(Call::SameOriginUrl(a.string(0)?))),
    ("targetOrigin", |a| Ok(Call::TargetOrigin(a.string(0)?))),
    ("frameSelf", |_| Ok(Call::FrameSelf)),
    ("frameParent", |a| {
        Ok(Call::FrameParent(a.integer(0) as u32))
    }),
    ("frameTop", |a| Ok(Call::FrameTop(a.integer(0) as u32))),
    ("frameLength", |a| {
        Ok(Call::FrameLength(a.integer(0) as u32))
    }),
    ("frameChild", |a| {
        Ok(Call::FrameChild(a.integer(0) as u32, a.integer(1) as u32))
    }),
    ("frameNamed", |a| {
        Ok(Call::FrameNamed(a.integer(0) as u32, a.string(1)?))
    }),
    ("frameClosed", |a| {
        Ok(Call::FrameClosed(a.integer(0) as u32))
    }),
    ("frameOf", |a| Ok(Call::FrameOf(a.handle(0)?))),
    ("framePost", |a| {
        Ok(Call::FramePost {
            frame: a.integer(0) as u32,
            target_origin: a.string(1)?,
            data: a.string(2)?,
        })
    }),
    ("frameNavigate", |a| {
        Ok(Call::FrameNavigate {
            frame: a.integer(0) as u32,
            url: a.string(1)?,
            replace: a.bool(2),
        })
    }),
    ("windowName", |_| Ok(Call::WindowName)),
    ("setWindowName", |a| Ok(Call::SetWindowName(a.string(0)?))),
    ("workerStart", |a| {
        Ok(Call::WorkerStart {
            url: a.string(0)?,
            name: a.string(1)?,
            module: a.bool(2),
            credentials: a.string(3)?,
        })
    }),
    ("workerPost", |a| {
        Ok(Call::WorkerPost {
            worker: a.integer(0) as u32,
            data: a.string(1)?,
        })
    }),
    ("workerTerminate", |a| {
        Ok(Call::WorkerTerminate(a.integer(0) as u32))
    }),
    ("postToOwner", |a| Ok(Call::PostToOwner(a.string(0)?))),
    ("closeWorker", |_| Ok(Call::CloseWorker)),
    ("portChannel", |_| Ok(Call::PortChannel)),
    ("portPost", |a| {
        Ok(Call::PortPost {
            port: a.integer(0) as u32,
            data: a.string(1)?,
        })
    }),
    ("portStart", |a| Ok(Call::PortStart(a.integer(0) as u32))),
    ("portClose", |a| Ok(Call::PortClose(a.integer(0) as u32))),
    ("portAway", |a| Ok(Call::PortAway(a.integer(0) as u32))),
    ("portAdopt", |a| Ok(Call::PortAdopt(a.integer(0) as u32))),
    ("importScript", |a| Ok(Call::ImportScript(a.string(0)?))),
    ("postMessage", |a| {
        Ok(Call::PostMessage {
            target_origin: a.string(0)?,
            data: a.string(1)?,
        })
    }),
    ("formEntries", |a| {
        Ok(Call::FormEntries {
            form: a.handle(0)?,
            submitter: a.maybe_handle(1)?,
        })
    }),
    ("platformNewline", |_| Ok(Call::PlatformNewline)),
    ("cookie", |_| Ok(Call::Cookie)),
    ("setCookie", |a| Ok(Call::SetCookie(a.string(0)?))),
    ("documentDomain", |_| Ok(Call::DocumentDomain)),
    ("setDocumentDomain", |a| {
        Ok(Call::SetDocumentDomain(a.string(0)?))
    }),
    ("storageOpen", |a| Ok(Call::StorageOpen(a.bool(0)))),
    ("storageLength", |a| Ok(Call::StorageLength(a.bool(0)))),
    ("storageKey", |a| {
        Ok(Call::StorageKey(a.bool(0), a.integer(1) as u32))
    }),
    ("storageKeys", |a| Ok(Call::StorageKeys(a.bool(0)))),
    ("storageGet", |a| {
        Ok(Call::StorageGet(a.bool(0), a.string(1)?))
    }),
    ("storageSet", |a| {
        Ok(Call::StorageSet(a.bool(0), a.string(1)?, a.string(2)?))
    }),
    ("storageRemove", |a| {
        Ok(Call::StorageRemove(a.bool(0), a.string(1)?))
    }),
    ("storageClear", |a| Ok(Call::StorageClear(a.bool(0)))),
    ("devtoolsSend", |a| Ok(Call::DevToolsSend(a.string(0)?))),
    ("devtoolsAnswer", |a| Ok(Call::DevToolsAnswer(a.string(0)?))),
    ("idb", |a| Ok(Call::Idb(a.string(0)?))),
    ("serviceWorker", |a| Ok(Call::ServiceWorker(a.string(0)?))),
    ("serviceWorkerRespond", |a| {
        Ok(Call::ServiceWorkerRespond(
            a.string(0)?,
            a.bytes(1)?.unwrap_or_default(),
        ))
    }),
    ("caches", |a| {
        Ok(Call::Caches(a.string(0)?, a.bytes(1)?.unwrap_or_default()))
    }),
    ("idbWake", |_| Ok(Call::IdbWake)),
    ("media", |a| Ok(Call::Media(a.string(0)?))),
    ("audio", |a| {
        Ok(Call::Audio(a.string(0)?, a.bytes(1)?.unwrap_or_default()))
    }),
    ("gpu", |a| {
        Ok(Call::Gpu(a.string(0)?, a.bytes(1)?.unwrap_or_default()))
    }),
    // WebGL's calls: the operation, then numbers and strings; the data
    // variant's bytes, a binary string, second.
    ("webgl", |a| match a.canvas(None)? {
        Call::Canvas(call) => Ok(Call::WebGl(call)),
        other => Ok(other),
    }),
    ("webglData", |a| {
        let bytes = a.bytes(1)?.unwrap_or_default();
        match a.canvas(Some(bytes))? {
            Call::Canvas(call) => Ok(Call::WebGl(call)),
            other => Ok(other),
        }
    }),
    ("storageUsage", |_| Ok(Call::StorageUsage)),
    ("navigate", |a| {
        Ok(Call::Navigate {
            url: a.string(0)?,
            replace: a.bool(1),
        })
    }),
    ("reload", |_| Ok(Call::Reload)),
    ("openWindow", |a| {
        Ok(Call::OpenWindow {
            url: a.string(0)?,
            target: a.string(1)?,
        })
    }),
    ("closeWindow", |_| Ok(Call::CloseWindow)),
    ("navigator", |_| Ok(Call::NavigatorInfo)),
    ("viewport", |_| Ok(Call::Viewport)),
    ("documentInfo", |_| Ok(Call::DocumentInfo)),
    ("computedValue", |a| {
        Ok(Call::ComputedValue {
            node: a.handle(0)?,
            pseudo: a.string(1)?,
            property: a.string(2)?,
        })
    }),
    ("computedProperties", |_| Ok(Call::ComputedProperties)),
    ("svgQuery", |a| {
        Ok(Call::SvgQuery {
            node: a.handle(0)?,
            query: a.string(1)?,
            args: vec![a.number(2), a.number(3)],
        })
    }),
    ("released", |a| Ok(Call::Released(a.handle(0)?))),
    ("setTimer", |a| {
        Ok(Call::SetTimer {
            id: a.integer(0) as u32,
            delay: a.integer(1) as i32,
        })
    }),
    ("clearTimer", |a| Ok(Call::ClearTimer(a.integer(0) as u32))),
    ("now", |_| Ok(Call::Now)),
    ("timeOrigin", |_| Ok(Call::TimeOrigin)),
    ("ancestors", |a| Ok(Call::Ancestors(a.handle(0)?))),
    ("activator", |a| Ok(Call::Activator(a.handle(0)?))),
    ("activate", |a| Ok(Call::Activate(a.handle(0)?))),
    ("disabled", |a| Ok(Call::Disabled(a.handle(0)?))),
    ("formOwner", |a| Ok(Call::FormOwner(a.handle(0)?))),
    ("wantFrame", |_| Ok(Call::WantFrame)),
    ("focusable", |a| Ok(Call::Focusable(a.handle(0)?))),
    ("tabbable", |a| Ok(Call::Tabbable(a.handle(0)?))),
    ("focused", |_| Ok(Call::Focused)),
    ("setFocus", |a| {
        Ok(Call::SetFocus {
            node: a.maybe_handle(0)?,
            prevent_scroll: a.bool(1),
        })
    }),
    ("blurToward", |a| Ok(Call::BlurToward(a.handle(0)?))),
    ("hasFocus", |_| Ok(Call::HasFocus)),
    ("controlValue", |a| Ok(Call::ControlValue(a.handle(0)?))),
    ("valueMode", |a| Ok(Call::ValueMode(a.handle(0)?))),
    ("setControlValue", |a| {
        Ok(Call::SetControlValue {
            node: a.handle(0)?,
            value: a.string(1)?,
        })
    }),
    ("inputType", |a| Ok(Call::InputType(a.handle(0)?))),
    ("checkedness", |a| Ok(Call::Checkedness(a.handle(0)?))),
    ("setCheckedness", |a| {
        Ok(Call::SetCheckedness {
            node: a.handle(0)?,
            checked: a.bool(1),
        })
    }),
    ("indeterminate", |a| Ok(Call::Indeterminate(a.handle(0)?))),
    ("setIndeterminate", |a| {
        Ok(Call::SetIndeterminate {
            node: a.handle(0)?,
            indeterminate: a.bool(1),
        })
    }),
    ("selectedness", |a| Ok(Call::Selectedness(a.handle(0)?))),
    ("setSelectedness", |a| {
        Ok(Call::SetSelectedness {
            node: a.handle(0)?,
            selected: a.bool(1),
        })
    }),
    ("options", |a| Ok(Call::Options(a.handle(0)?))),
    ("ownerSelect", |a| Ok(Call::OwnerSelect(a.handle(0)?))),
    ("selectedIndex", |a| Ok(Call::SelectedIndex(a.handle(0)?))),
    ("optionText", |a| Ok(Call::OptionText(a.handle(0)?))),
    ("textSelection", |a| Ok(Call::TextSelection(a.handle(0)?))),
    ("setTextSelection", |a| {
        let unit = |n: f64| n.clamp(0.0, f64::from(u32::MAX)) as u32;
        Ok(Call::SetTextSelection {
            node: a.handle(0)?,
            start: unit(a.integer(1)),
            end: unit(a.integer(2)),
            direction: a.integer(3).clamp(0.0, 2.0) as u8,
        })
    }),
    ("formControls", |a| Ok(Call::FormControls(a.handle(0)?))),
    ("labeledControl", |a| Ok(Call::LabeledControl(a.handle(0)?))),
    ("labels", |a| Ok(Call::Labels(a.handle(0)?))),
    ("commitEdit", |a| Ok(Call::CommitEdit(a.handle(0)?))),
    ("radioGroup", |a| Ok(Call::RadioGroup(a.handle(0)?))),
    ("submitForm", |a| {
        Ok(Call::SubmitForm {
            form: a.handle(0)?,
            submitter: a.maybe_handle(1)?,
        })
    }),
    ("resetForm", |a| Ok(Call::ResetForm(a.handle(0)?))),
    ("validity", |a| Ok(Call::Validity(a.handle(0)?))),
    ("willValidate", |a| Ok(Call::WillValidate(a.handle(0)?))),
    ("validationMessage", |a| {
        Ok(Call::ValidationMessage(a.handle(0)?))
    }),
    ("setCustomValidity", |a| {
        Ok(Call::SetCustomValidity {
            node: a.handle(0)?,
            message: a.string(1)?,
        })
    }),
    ("invalidControls", |a| {
        Ok(Call::InvalidControls(a.handle(0)?))
    }),
    ("markSubmitted", |a| Ok(Call::MarkSubmitted(a.handle(0)?))),
    ("setUserValidity", |a| {
        Ok(Call::SetUserValidity(a.handle(0)?))
    }),
    ("reportInvalid", |a| Ok(Call::ReportInvalid(a.handle(0)?))),
    ("hidden", |_| Ok(Call::Hidden)),
    ("canvas", |a| a.canvas(None)),
    // `putImageData`'s pixels, as base64 (`toBase64` makes it at the
    // speed of C; `rquickjs` lends no buffer's bytes safely).
    ("canvasData", |a| {
        let bytes = super::wasm::from_base64(&a.string(1)?)
            .ok_or_else(|| Exception::throw_type(a.ctx, "not base64"))?;
        a.canvas(Some(bytes))
    }),
    ("dialog", |a| {
        Ok(Call::Dialog {
            kind: a.string(0)?,
            message: a.string(1)?,
        })
    }),
    ("codeAllowed", |a| {
        let element = match a.integer(2) {
            n if n >= 0.0 => Some(a.handle(2)?),
            _ => None,
        };
        Ok(Call::CodeAllowed {
            kind: a.string(0)?,
            source: a.string(1)?,
            element,
        })
    }),
    ("nonce", |a| Ok(Call::Nonce(a.handle(0)?))),
    ("cssomStyled", |a| Ok(Call::CssomStyled(a.handle(0)?))),
    ("setNonce", |a| {
        Ok(Call::SetNonce(a.handle(0)?, a.string(1)?))
    }),
];

/// Runs the bootstrap with the natives, which nothing else then holds;
/// gives its entry points, the functions the realm's tasks call (M26),
/// which a nested script's error reaches through `entries`.
pub(super) fn install<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    kind: &RealmKind,
    secure: bool,
    host: &Slot<H>,
    budget: &Budget,
    entries: &Entries,
    console: Function<'js>,
) -> rquickjs::Result<Object<'js>> {
    let natives = Object::new(ctx.clone())?;
    natives.set("consoleMessage", console)?;
    // Whether the realm is a secure context's (M39).
    natives.set("secure", secure)?;
    // The kind of global the bootstrap makes (M31).
    match kind {
        RealmKind::Window => natives.set("realmKind", "window")?,
        RealmKind::DevTools => natives.set("realmKind", "devtools")?,
        RealmKind::ServiceWorker => natives.set("realmKind", "serviceworker")?,
        RealmKind::DedicatedWorker { name } => {
            natives.set("realmKind", "worker")?;
            natives.set("workerName", name.as_str())?;
        }
    }
    for &(name, decode) in NATIVES {
        let (host, budget, entries) = (host.clone(), budget.clone(), entries.clone());
        let runs_scripts = RUN_SCRIPTS.contains(&name);
        let native = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                if budget.exhausted.get() {
                    return Err(Exception::throw_internal(&ctx, "interrupted"));
                }
                let call = decode(&Args {
                    ctx: &ctx,
                    values: &args.0,
                })?;
                let (answer, work) = with_host(&ctx, &host, |host| {
                    let answer = host.dom(call);
                    (answer, host.take_work())
                })?;
                budget.charge(work);
                let answer = match answer {
                    Ok(answer) => answer,
                    Err(error) => return Err(throw(&ctx, &error)),
                };
                if runs_scripts {
                    run_nested(&ctx, &host, &budget, &entries)?;
                }
                value(&ctx, answer)
            },
        )?
        .with_name(name)?;
        natives.set(name, native)?;
    }
    // `document.title` keeps M24's two methods of the host.
    let read = host.clone();
    natives.set(
        "title",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>| -> rquickjs::Result<String> {
                with_host(&ctx, &read, |host| host.title())
            },
        )?,
    )?;
    let write = host.clone();
    natives.set(
        "setTitle",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, title: Coerced<String>| -> rquickjs::Result<()> {
                with_host(&ctx, &write, |host| host.set_title(title.0))
            },
        )?,
    )?;
    install_errors(ctx, host, &natives)?;
    super::wasm::install(ctx, &natives, budget)?;
    // WebGPU and WebGL's bindings' function, read when first asked for.
    natives.set(
        "gpuBindings",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>| -> rquickjs::Result<Function<'js>> {
                super::bytecode::gpu_bindings(&ctx)
            },
        )?,
    )?;
    let bootstrap: Function<'js> = super::bytecode::bootstrap(ctx)?;
    bootstrap.call::<_, Object<'js>>((natives, ctx.globals()))
}

/// The natives the bootstrap reports exceptions and compiles code with
/// (M26): `describe`, a thrown value's report as the console words it
/// (its message, URL, line and column); `reportError`, such a report
/// given to the host; and `evaluate`, a string run as a classic script
/// naming a URL, what it throws thrown on (a timer's string, an event
/// handler's text).
fn install_errors<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    host: &Slot<H>,
    natives: &Object<'js>,
) -> rquickjs::Result<()> {
    // What a structured clone cannot tell of an object by script (M31):
    // 1 a function, 2 a proxy, 3 a promise, 4 an error, else 0.
    natives.set(
        "cloneKind",
        Function::new(ctx.clone(), |value: Value<'js>| -> i32 {
            if value.is_proxy() {
                2
            } else if value.is_function() {
                1
            } else if value.is_promise() {
                3
            } else if value.is_error() {
                4
            } else {
                0
            }
        })?,
    )?;
    natives.set(
        "describe",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, value: Value<'js>| -> rquickjs::Result<Array<'js>> {
                let error = thrown(&ctx, value, "", false);
                let report = Array::new(ctx.clone())?;
                report.set(0, error.message)?;
                report.set(1, error.url)?;
                report.set(2, error.line)?;
                report.set(3, error.column)?;
                Ok(report)
            },
        )?,
    )?;
    let reporting = host.clone();
    natives.set(
        "reportError",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  message: String,
                  url: String,
                  line: f64,
                  column: f64|
                  -> rquickjs::Result<()> {
                with_host(&ctx, &reporting, |host| {
                    host.report(ScriptError {
                        kind: ErrorKind::Exception,
                        message,
                        url,
                        line: line as u32,
                        column: column as u32,
                        stack: Vec::new(),
                    });
                })
            },
        )?,
    )?;
    natives.set(
        "evaluate",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, source: String, url: String| -> rquickjs::Result<Value<'js>> {
                evaluate(&ctx, &source, &url, 1).map_err(|caught| rethrow(&ctx, caught))
            },
        )?,
    )?;
    Ok(())
}

/// Throws again what was caught, as it was: an exception the budget's
/// stop made goes on uncatchable.
fn rethrow<'js>(ctx: &Ctx<'js>, caught: CaughtError<'js>) -> rquickjs::Error {
    match caught {
        CaughtError::Exception(exception) => ctx.throw(exception.into_value()),
        CaughtError::Value(value) => ctx.throw(value),
        CaughtError::Error(error) => error,
    }
}

/// Runs the scripts the host has ready to run now, each nested in the
/// running script (HTML: before the call that made it ready returns),
/// its errors the page's, reported as a script's are (M26). A run past
/// its budget is stopped whole.
fn run_nested<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    host: &Slot<H>,
    budget: &Budget,
    entries: &Entries,
) -> rquickjs::Result<()> {
    while let Some(script) = with_host(ctx, host, |host| host.take_script())? {
        let result = evaluate(ctx, &script.source, &script.url, script.line);
        let caught = match result {
            Ok(_) => None,
            // A script stopped by the budget is no error of its own: the
            // stop, uncatchable, goes on up through the script it is
            // nested in, which is stopped too.
            Err(caught) if budget.exhausted.get() => {
                with_host(ctx, host, |host| host.script_ran())?;
                return Err(rethrow(ctx, caught));
            }
            Err(caught) => Some(report(
                ctx,
                entries,
                &budget.exhausted,
                caught,
                &script.url,
                false,
            )),
        };
        with_host(ctx, host, |host| {
            if let Some(error) = caught {
                host.report(error);
            }
            host.script_ran();
        })?;
    }
    Ok(())
}

/// An answer as a script's value.
fn value<'js>(ctx: &Ctx<'js>, answer: Answer) -> rquickjs::Result<Value<'js>> {
    let handle = |handle: Handle| Value::new_float(ctx.clone(), handle.0 as f64);
    Ok(match answer {
        Answer::Nothing => Value::new_undefined(ctx.clone()),
        Answer::Bool(value) => Value::new_bool(ctx.clone(), value),
        Answer::Number(value) => Value::new_number(ctx.clone(), value),
        Answer::Node(Some(node)) => handle(node),
        Answer::Node(None) | Answer::MaybeString(None) => Value::new_null(ctx.clone()),
        Answer::Nodes(nodes) => {
            let array = Array::new(ctx.clone())?;
            for (i, node) in nodes.into_iter().enumerate() {
                array.set(i, handle(node))?;
            }
            array.into_value()
        }
        Answer::String(text) | Answer::MaybeString(Some(text)) => {
            rquickjs::String::from_str(ctx.clone(), &text)?.into_value()
        }
        Answer::Fetched(None) => Value::new_null(ctx.clone()),
        Answer::Fetched(Some(answer)) => crate::quickjs::answer_object(ctx, &answer)?.into_value(),
        Answer::Numbers(numbers) => {
            let array = Array::new(ctx.clone())?;
            for (i, number) in numbers.into_iter().enumerate() {
                array.set(i, number)?;
            }
            array.into_value()
        }
        Answer::Pins(pins) => {
            let array = Array::new(ctx.clone())?;
            for (i, (node, connected)) in pins.into_iter().enumerate() {
                array.set(2 * i, handle(node))?;
                array.set(2 * i + 1, connected)?;
            }
            array.into_value()
        }
        Answer::Strings(texts) => {
            let array = Array::new(ctx.clone())?;
            for (i, text) in texts.iter().enumerate() {
                array.set(i, rquickjs::String::from_str(ctx.clone(), text)?)?;
            }
            array.into_value()
        }
        Answer::Bytes(bytes) => rquickjs::ArrayBuffer::new(ctx.clone(), bytes)?.into_value(),
        Answer::Pair(text, bytes) => {
            let array = Array::new(ctx.clone())?;
            array.set(0, rquickjs::String::from_str(ctx.clone(), &text)?)?;
            array.set(1, rquickjs::ArrayBuffer::new(ctx.clone(), bytes)?)?;
            array.into_value()
        }
    })
}

/// Throws the exception a failed call is: one of ECMAScript's errors,
/// or a `DOMException` of the realm's own class, whatever a page did to
/// the global one.
fn throw(ctx: &Ctx<'_>, error: &DomError) -> rquickjs::Error {
    match error.name {
        "TypeError" => Exception::throw_type(ctx, &error.message),
        "RangeError" => Exception::throw_range(ctx, &error.message),
        name => Exception::throw_dom(ctx, name, &error.message),
    }
}
