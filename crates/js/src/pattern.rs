//! The `pattern` attribute's regular expressions (M26.9b,
//! `docs/design/events.md`, "Constraint validation", HTML section
//! 4.10.5.3.6): an ECMAScript regular expression with the `v` flag,
//! matched by QuickJS-ng whether the page's scripts run or not, in a
//! small context of its own on each thread, with no DOM, a small heap
//! and a small step budget, the page's own scripts having no part in it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::{Context, Function, Persistent, Runtime, Value};

/// The heap the patterns' context may use.
const PATTERN_HEAP: usize = 8 << 20;
/// Its stack.
const PATTERN_STACK: usize = 256 << 10;
/// The interrupt checks a match may take, as a script's steps are
/// counted: what stops a catastrophic backtrack.
const PATTERN_STEPS: u32 = 200;
/// The answers kept, by pattern and value, before they are forgotten.
const MAX_ANSWERS: usize = 1024;

/// The context and its matcher function, and the answers so far.
struct Patterns {
    context: Context,
    test: Persistent<Function<'static>>,
    steps: Rc<Cell<u32>>,
    answers: HashMap<(String, String), Option<bool>>,
    // Dropped last: the context and the function are the runtime's.
    _runtime: Runtime,
}

impl Patterns {
    fn new() -> Option<Patterns> {
        let runtime = Runtime::new().ok()?;
        runtime.set_memory_limit(PATTERN_HEAP);
        runtime.set_max_stack_size(PATTERN_STACK);
        let steps = Rc::new(Cell::new(0u32));
        let taken = steps.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let n = taken.get().saturating_add(1);
            taken.set(n);
            n > PATTERN_STEPS
        })));
        let context = Context::full(&runtime).ok()?;
        let test = context.with(|ctx| {
            let function: Function = ctx
                .eval(
                    "(function (p, v) { var r; \
                       try { r = new RegExp('^(?:' + p + ')$', 'v'); } \
                       catch (e) { return null; } \
                       return r.test(v); })",
                )
                .ok()?;
            Some(Persistent::save(&ctx, function))
        })?;
        Some(Patterns {
            context,
            test,
            steps,
            answers: HashMap::new(),
            _runtime: runtime,
        })
    }

    fn check(&mut self, pattern: &str, value: &str) -> Option<bool> {
        let key = (pattern.to_owned(), value.to_owned());
        if let Some(&answer) = self.answers.get(&key) {
            return answer;
        }
        self.steps.set(0);
        let answer = self.context.with(|ctx| {
            let test = self.test.clone().restore(&ctx).ok()?;
            let result: Value = test.call((pattern, value)).ok()?;
            result.as_bool()
        });
        if self.answers.len() >= MAX_ANSWERS {
            self.answers.clear();
        }
        self.answers.insert(key, answer);
        answer
    }
}

thread_local! {
    static PATTERNS: RefCell<Option<Patterns>> = const { RefCell::new(None) };
}

/// Whether `value` matches `pattern` as HTML's `pattern` attribute
/// does: the whole value against the pattern compiled as
/// `^(?:pattern)$` with the `v` flag. `None` where there is no
/// constraint: the pattern does not compile, or the match ran past its
/// budget.
pub fn pattern_matches(pattern: &str, value: &str) -> Option<bool> {
    PATTERNS.with(|cell| {
        let mut patterns = cell.borrow_mut();
        if patterns.is_none() {
            *patterns = Patterns::new();
        }
        patterns.as_mut()?.check(pattern, value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_matches_the_whole_value_with_the_v_flag() {
        assert_eq!(pattern_matches("[0-9]{3}", "123"), Some(true));
        assert_eq!(pattern_matches("[0-9]{3}", "1234"), Some(false), "anchored");
        assert_eq!(pattern_matches("a|b", "ab"), Some(false), "grouped whole");
        assert_eq!(pattern_matches("a|b", "b"), Some(true));
        // The v flag's set notation.
        assert_eq!(pattern_matches("[\\p{L}--[a-z]]+", "AB"), Some(true));
        assert_eq!(pattern_matches("[\\p{L}--[a-z]]+", "Ab"), Some(false));
        // One that does not compile is no constraint.
        assert_eq!(pattern_matches("(", "x"), None);
        // A catastrophic backtrack runs out of its budget.
        let value = format!("{}!", "a".repeat(40));
        assert_eq!(pattern_matches("(a+)+", &value), None);
        // Asked again, the same answer.
        assert_eq!(pattern_matches("[0-9]{3}", "123"), Some(true));
    }
}
