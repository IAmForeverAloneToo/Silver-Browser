//! Live lists read whole once a change, not once an access (2026-11-18):
//! a script that reads `list[i]` for each `i` of a list of n items does
//! O(n) work, not O(n^2). Each test runs such a loop over a list long
//! enough that reading the whole list at each access spends a budget of
//! steps (QuickJS's interrupt checks and the host's work) many times
//! what the loop needs once.

use super::script_tests::scripted_bytes;

/// Whether `script`, after `body`, ends within `steps`.
fn fits(body: &str, script: &str, steps: u32) -> bool {
    let page = format!("{body}<script>{script}; console.log('finished')</script>");
    let limits = js::Limits {
        steps,
        ..js::Limits::default()
    };
    let (_, console) = scripted_bytes(page.into_bytes(), Some(limits));
    console == ["finished"]
}

/// The loop: every item of `list` read, by index, `rounds` times.
fn reads(list: &str) -> String {
    format!(
        "var list = {list}; var seen = 0; \
         for (var r = 0; r < 3; r++) for (var i = 0; i < list.length; i++) if (list[i]) seen++; \
         if (seen === 0) throw new Error('nothing read')"
    )
}

fn repeated(n: usize, f: impl Fn(usize) -> String) -> String {
    (0..n).map(f).collect()
}

fn rules(n: usize) -> String {
    format!(
        "<style>{}</style>",
        repeated(n, |i| format!(".c{i} {{ color: red }} "))
    )
}

fn sheets(n: usize) -> String {
    repeated(n, |i| format!("<style>.s{i} {{ color: red }}</style>"))
}

fn attributes(n: usize) -> String {
    format!(
        "<div id=t {}></div>",
        repeated(n, |i| format!("data-a{i}=x "))
    )
}

fn declarations(n: usize) -> String {
    format!(
        "<div id=t style=\"{}\"></div>",
        repeated(n, |i| format!("--v{i}: 1; "))
    )
}

fn classes(n: usize) -> String {
    format!(
        "<div id=t class=\"{}\"></div>",
        repeated(n, |i| format!("k{i} "))
    )
}

fn media(n: usize) -> String {
    let queries: Vec<String> = (0..n).map(|i| format!("(min-width: {i}px)")).collect();
    format!(
        "<style media=\"{}\">p {{ color: red }}</style>",
        queries.join(", ")
    )
}

fn children(n: usize) -> String {
    format!("<div id=t>{}</div>", repeated(n, |_| "<i>x</i>".to_owned()))
}

/// The items each list is read over: enough that reading the whole list
/// at each access spends many times the budget.
const N: usize = 1600;

/// Each list read over [`N`] items within `steps`: about four times what
/// the loop takes (2026-11-18, a debug build: cssRules 26, styleSheets
/// 59, attributes 42, style 31, classList 33, media 26, the node lists
/// 39 or 40), where reading the list whole at each access takes from
/// hundreds to hundreds of thousands.
fn read_within(body: fn(usize) -> String, list: &str, steps: u32) {
    assert!(
        fits(&body(N), &reads(list), steps),
        "{list} over {N} items, within {steps} steps"
    );
}

#[test]
fn a_rule_list_is_read_whole_once_a_change() {
    read_within(rules, "document.styleSheets[0].cssRules", 100);
}

#[test]
fn a_sheet_list_is_read_whole_once_a_change() {
    read_within(sheets, "document.styleSheets", 240);
}

#[test]
fn an_attribute_map_is_read_whole_once_a_change() {
    read_within(attributes, "document.getElementById('t').attributes", 170);
}

#[test]
fn a_declaration_block_is_parsed_once_a_change() {
    read_within(declarations, "document.getElementById('t').style", 125);
}

#[test]
fn a_token_list_is_parsed_once_a_change() {
    read_within(classes, "document.getElementById('t').classList", 135);
}

#[test]
fn a_media_list_is_parsed_once_a_change() {
    read_within(media, "document.styleSheets[0].media", 100);
}

#[test]
fn a_node_list_is_read_whole_once_a_change() {
    read_within(children, "document.getElementById('t').childNodes", 160);
}

#[test]
fn an_element_collection_is_read_whole_once_a_change() {
    read_within(children, "document.getElementById('t').children", 160);
    read_within(children, "document.getElementsByTagName('i')", 160);
}

/// And each still sees a change made between its reads.
#[test]
fn a_list_read_whole_sees_each_change() {
    let page = format!(
        "{}{}<div id=t class='a b' style='color: red' data-x=1><i>x</i></div>",
        rules(2),
        media(2)
    );
    let script = "var t = document.getElementById('t'); var sheet = document.styleSheets[0];         var r = sheet.cssRules, s = document.styleSheets, a = t.attributes, d = t.style,         c = t.classList, m = sheet.media, k = t.childNodes, e = t.children;         var before = [r.length, s.length, a.length, d.length, c.length, m.length, k.length, e.length];         sheet.insertRule('.z { color: blue }', 0); document.head.appendChild(document.createElement('style'));         t.setAttribute('data-y', '2'); t.style.setProperty('margin', '0px'); t.classList.add('c');         m.appendMedium('print'); t.appendChild(document.createElement('b'));         var after = [r.length, s.length, a.length, d.length, c.length, m.length, k.length, e.length];         console.log(before.join() + ' ' + after.join() + ' ' + r[0].selectorText + ' ' + c[2] + ' ' + a[a.length - 1].name)";
    let page = format!("{page}<script>{script}</script>");
    let (_, console) = scripted_bytes(page.into_bytes(), Some(js::Limits::default()));
    // Two sheets, the first without media; `margin` four longhands.
    assert_eq!(console, ["2,2,4,1,2,0,1,1 3,3,5,5,3,1,2,2 .z c data-y"]);
}
