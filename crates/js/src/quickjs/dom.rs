//! The DOM's natives (M25): the functions the bootstrap's classes call,
//! each turning its arguments into a [`Call`] of the host and the host's
//! [`Answer`] into a value, and charging the work the host reports to
//! the run's budget. Only the bootstrap's closure holds them.

use std::cell::Cell;
use std::rc::Rc;

use rquickjs::function::Rest;
use rquickjs::{Array, CaughtError, Coerced, Ctx, Exception, Function, Object, Value};

use super::{Slot, caught_error, evaluate, with_host};
use crate::{Answer, Call, DomError, Handle, MAX_HANDLE, ScriptingHost, WORK_PER_STEP};

/// The bindings, run in each new realm before any page's script.
const BOOTSTRAP: &str = include_str!("bootstrap.js");

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
    fn charge(&self, units: u64) {
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
    ("documentUrl", |_| Ok(Call::DocumentUrl)),
    ("released", |a| Ok(Call::Released(a.handle(0)?))),
];

/// Runs the bootstrap with the natives, which nothing else then holds.
pub(super) fn install<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    host: &Slot<H>,
    budget: &Budget,
) -> rquickjs::Result<()> {
    let natives = Object::new(ctx.clone())?;
    for &(name, decode) in NATIVES {
        let (host, budget) = (host.clone(), budget.clone());
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
                    run_nested(&ctx, &host, &budget)?;
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
    let bootstrap: Function<'js> = ctx.eval(BOOTSTRAP)?;
    bootstrap.call::<_, ()>((natives, ctx.globals()))?;
    Ok(())
}

/// Runs the scripts the host has ready to run now, each nested in the
/// running script (HTML: before the call that made it ready returns),
/// its errors the page's. A run past its budget is stopped whole.
fn run_nested<'js, H: ScriptingHost>(
    ctx: &Ctx<'js>,
    host: &Slot<H>,
    budget: &Budget,
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
                return Err(match caught {
                    CaughtError::Exception(exception) => ctx.throw(exception.into_value()),
                    CaughtError::Value(value) => ctx.throw(value),
                    CaughtError::Error(error) => error,
                });
            }
            Err(caught) => Some(caught_error(ctx, caught, &script.url, false)),
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
