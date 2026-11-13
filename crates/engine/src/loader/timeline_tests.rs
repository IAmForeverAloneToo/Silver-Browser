//! A document's performance timeline (Performance Timeline, User
//! Timing, Navigation Timing): its marks and measures, its navigation's
//! entry and the legacy `performance.timing`, and the observers told of
//! new entries in a task, as the reference browser has them.

use net::test_server::TestServer;

use super::tests::load_all;
use crate::testing::*;

/// `body` served, loaded with scripts on and a clock the test moves; the
/// tab, the clock and the server, which must outlive the page's fetches.
fn timed(body: &'static str) -> (Tab, HandClock, TestServer) {
    let server = serve(move |path| match path {
        "/page.html" => Some(html(body)),
        _ => None,
    });
    let mut tab = Tab::new();
    let clock = tab.hand_clock();
    tab.engine.set_scripting(true);
    load_all(&mut tab, server.url("/page.html"));
    (tab, clock, server)
}

/// The console's lines since they were last taken.
fn lines(tab: &mut Tab) -> Vec<String> {
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

#[test]
fn marks_and_measures_are_made_kept_and_cleared() {
    let (mut tab, _clock, _server) = timed(
        "<script>\
         function tell(f) { try { f(); } catch (e) { console.log(e.name + ': ' + e.message); } }\
         performance.mark('a');\
         var b = performance.mark('b', { startTime: 5, detail: { x: [1] } });\
         console.log(b.name, b.entryType, b.startTime, b.duration, JSON.stringify(b.detail),\
           b.detail === b.detail, b instanceof PerformanceMark, b instanceof PerformanceEntry);\
         console.log(JSON.stringify(b));\
         var ab = performance.measure('ab', 'a', 'b');\
         console.log(ab.entryType, ab.startTime, ab.duration, ab.detail);\
         var o = performance.measure('o', { start: 2, duration: 3, detail: 'd' });\
         console.log(o.startTime, o.duration, o.detail);\
         var e = performance.measure('e', { end: 'b', duration: 1 });\
         console.log(e.startTime, e.duration);\
         console.log(performance.measure('n', 'navigationStart', 'b').duration);\
         console.log(performance.getEntries().map(function (x) {\
           return x.entryType + ':' + (x.entryType === 'navigation' ? '' : x.name); }).join());\
         console.log(new PerformanceMark('c', { startTime: 3 }).startTime,\
           performance.getEntriesByName('c').length);\
         tell(function () { performance.mark('navigationStart'); });\
         tell(function () { performance.mark('d', { startTime: -1 }); });\
         tell(function () { performance.mark('d', 5); });\
         tell(function () { performance.mark('d', { detail: function () {} }); });\
         tell(function () { performance.measure('x', 'nope'); });\
         tell(function () { performance.measure('x', { start: 0 }, 'a'); });\
         tell(function () { performance.measure('x', { detail: 1 }); });\
         tell(function () { performance.measure('x', { start: 0, end: 1, duration: 1 }); });\
         tell(function () { performance.measure('x', { start: -5 }); });\
         tell(function () { performance.measure('x', 'unloadEventStart'); });\
         tell(function () { new PerformanceEntry(); });\
         tell(function () { performance.mark(); });\
         performance.clearMarks('a');\
         console.log(performance.getEntriesByName('a').length,\
           performance.getEntriesByName('b', 'mark').length,\
           performance.getEntriesByName('b', 'measure').length);\
         performance.clearMeasures();\
         performance.clearMarks();\
         console.log(performance.getEntriesByType('measure').length,\
           performance.getEntriesByType('mark').length);\
         </script>",
    );
    assert_eq!(
        lines(&mut tab),
        [
            "b mark 5 0 {\"x\":[1]} true true true",
            "{\"name\":\"b\",\"entryType\":\"mark\",\"startTime\":5,\"duration\":0}",
            "measure 0 5 null",
            "2 3 d",
            "4 1",
            "5",
            "navigation:,mark:a,measure:ab,measure:n,measure:o,measure:e,mark:b",
            "3 0",
            "SyntaxError: Failed to execute 'mark' on 'Performance': 'navigationStart' is \
             part of the PerformanceTiming interface, and cannot be used as a mark name.",
            "TypeError: Failed to execute 'mark' on 'Performance': 'd' cannot have a negative \
             start time.",
            "TypeError: Failed to execute 'mark' on 'Performance': The provided value is not \
             of type 'PerformanceMarkOptions'.",
            "DataCloneError: Failed to execute 'mark' on 'Performance': function () {} could \
             not be cloned.",
            "SyntaxError: Failed to execute 'measure' on 'Performance': The mark 'nope' does \
             not exist.",
            "TypeError: Failed to execute 'measure' on 'Performance': If a non-empty \
             PerformanceMeasureOptions object was passed, |end_mark| must not be passed.",
            "TypeError: Failed to execute 'measure' on 'Performance': If a non-empty \
             PerformanceMeasureOptions object was passed, at least one of its 'start' or \
             'end' properties must be present.",
            "TypeError: Failed to execute 'measure' on 'Performance': If a non-empty \
             PerformanceMeasureOptions object was passed, it must not have all of its \
             'start', 'duration', and 'end' properties defined",
            "TypeError: Failed to execute 'measure' on 'Performance': 'x' cannot have a \
             negative time stamp.",
            "InvalidAccessError: Failed to execute 'measure' on 'Performance': \
             'unloadEventStart' is empty: either the event hasn't happened yet, or it would \
             provide cross-origin timing information.",
            "TypeError: Failed to construct 'PerformanceEntry': Illegal constructor",
            "TypeError: Failed to execute 'mark' on 'Performance': 1 argument required, but \
             only 0 present.",
            "0 1 0",
            "0 0",
        ]
    );
}

#[test]
fn the_navigation_has_its_entry_and_its_legacy_timing() {
    let (mut tab, clock, _server) = timed(
        "<script>\
         var nav = performance.getEntriesByType('navigation')[0];\
         console.log(nav instanceof PerformanceNavigationTiming,\
           nav instanceof PerformanceResourceTiming, nav.name === location.href,\
           nav.startTime, nav.type, nav.responseStatus, nav.initiatorType,\
           nav.responseStart >= 0, nav.loadEventEnd, nav.duration);\
         console.log(performance.timing.navigationStart === Math.floor(performance.timeOrigin),\
           performance.timing.responseStart >= performance.timing.navigationStart,\
           performance.timing.loadEventEnd, performance.navigation.type,\
           PerformanceNavigation.TYPE_RELOAD);\
         try { performance.measure('x', 'loadEventEnd'); } catch (e) { console.log(e.name); }\
         onload = function () { setTimeout(function () {\
           var nav = performance.getEntriesByType('navigation')[0];\
           console.log(nav.domInteractive <= nav.domContentLoadedEventStart,\
             nav.domContentLoadedEventStart <= nav.domContentLoadedEventEnd,\
             nav.domContentLoadedEventEnd <= nav.domComplete,\
             nav.domComplete <= nav.loadEventStart, nav.loadEventStart <= nav.loadEventEnd,\
             nav.loadEventEnd === nav.duration,\
             performance.timing.loadEventEnd >= performance.timing.navigationStart,\
             performance.measure('load', 'navigationStart', 'loadEventEnd').startTime);\
           console.log(Object.keys(nav.toJSON()).slice(0, 6).join(),\
             Object.keys(performance.toJSON()).join(),\
             Object.keys(performance.timing.toJSON()).length);\
         }); };\
         </script>",
    );
    clock.advance(1);
    tab.engine.run_tasks();
    assert_eq!(
        lines(&mut tab),
        [
            "true true true 0 navigate 200 navigation true 0 0",
            "true true 0 0 1",
            "InvalidAccessError",
            "true true true true true true true 0",
            "name,entryType,startTime,duration,initiatorType,deliveryType \
             timeOrigin,timing,navigation 21",
        ]
    );
    // A reload is one.
    tab.engine.reload();
    tab.wait_until(|engine| engine.load_state() == &crate::LoadState::Loaded);
    let reloaded: Vec<String> = lines(&mut tab)
        .into_iter()
        .filter(|line| line.starts_with("true true true 0"))
        .collect();
    assert_eq!(
        reloaded,
        ["true true true 0 reload 200 navigation true 0 0"]
    );
}

#[test]
fn observers_are_told_of_new_entries_in_a_task() {
    let (mut tab, clock, _server) = timed(
        "<script>\
         function tell(f) { try { f(); } catch (e) { console.log(e.name + ': ' + e.message); } }\
         var all = new PerformanceObserver(function (list, observer, options) {\
           console.log('all', list.getEntries().map(function (e) { return e.name; }).join(),\
             this === all, observer === all, JSON.stringify(options));\
         });\
         all.observe({ entryTypes: ['mark', 'not-a-type'] });\
         performance.mark('one');\
         performance.mark('two');\
         Promise.resolve().then(function () { console.log('microtask'); });\
         var late = new PerformanceObserver(function (list) {\
           console.log('buffered', list.getEntriesByType('mark').map(function (e) {\
             return e.name; }).join(), list.getEntriesByName('two').length);\
         });\
         late.observe({ type: 'mark', buffered: true });\
         var taken = new PerformanceObserver(function () { console.log('not told'); });\
         taken.observe({ type: 'measure' });\
         performance.measure('m');\
         console.log('taken', taken.takeRecords().length);\
         var gone = new PerformanceObserver(function () { console.log('not told'); });\
         gone.observe({ type: 'mark' });\
         performance.mark('three');\
         gone.disconnect();\
         tell(function () { new PerformanceObserver(function () {}).observe({}); });\
         tell(function () {\
           new PerformanceObserver(function () {}).observe({ entryTypes: ['mark'], type: 'mark' }); });\
         tell(function () { all.observe({ type: 'measure' }); });\
         tell(function () { late.observe({ entryTypes: ['measure'] }); });\
         tell(function () { new PerformanceObserver(1); });\
         tell(function () { new PerformanceObserverEntryList(); });\
         console.log(PerformanceObserver.supportedEntryTypes.join(),\
           Object.isFrozen(PerformanceObserver.supportedEntryTypes),\
           PerformanceObserver.supportedEntryTypes === PerformanceObserver.supportedEntryTypes);\
         setTimeout(function () { performance.mark('four'); }, 5);\
         </script>",
    );
    clock.advance(1);
    tab.engine.run_tasks();
    assert_eq!(
        lines(&mut tab),
        [
            "taken 1",
            "TypeError: Failed to execute 'observe' on 'PerformanceObserver': An observe() \
             call must include either entryTypes or type arguments.",
            "TypeError: Failed to execute 'observe' on 'PerformanceObserver': An observe() \
             call must not include both entryTypes and type arguments.",
            "InvalidModificationError: Failed to execute 'observe' on 'PerformanceObserver': \
             This observer has performed observe({entryTypes:...}, therefore it cannot \
             perform observe({type:...})",
            "InvalidModificationError: Failed to execute 'observe' on 'PerformanceObserver': \
             This PerformanceObserver has performed observe({type:...}, therefore it cannot \
             perform observe({entryTypes:...})",
            "TypeError: Failed to construct 'PerformanceObserver': parameter 1 is not of \
             type 'Function'.",
            "TypeError: Failed to construct 'PerformanceObserverEntryList': Illegal \
             constructor",
            "mark,measure,navigation true true",
            "microtask",
            "all one,two,three true true {\"droppedEntriesCount\":0}",
            "buffered one,two,three 1",
        ]
    );
    clock.advance(5);
    tab.engine.run_tasks();
    assert_eq!(
        lines(&mut tab),
        ["all four true true {}", "buffered four 0"]
    );
}
