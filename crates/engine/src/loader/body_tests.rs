//! The bodies scripts make (M27, docs/design/fetch.md): `Blob` and
//! `File`, `FormData` and `URLSearchParams`, run in a page and checked
//! by what it logs.

use super::script_tests::scripted;

/// The console's lines of `script` run in a page with scripting on.
fn logged(script: &str) -> Vec<String> {
    let page: &'static str =
        Box::leak(format!("<form id=f></form><script>{script}</script>").into_boxed_str());
    scripted(page, true).1
}

/// The console's single line after `script`, failing on an error.
fn one(script: &str) -> String {
    let lines = logged(script);
    assert_eq!(lines.len(), 1, "{lines:?}");
    lines.into_iter().next().unwrap()
}

#[test]
fn a_blob_holds_its_parts_bytes_and_its_type() {
    assert_eq!(
        one(
            r#"const b = new Blob(['a\u00e9', new Uint8Array([98, 99]), new Blob(['d']), new DataView(new Uint8Array([1, 101, 2]).buffer, 1, 1)], {type: 'Text/Plain'});
            b.text().then(t => console.log(b.size, b.type, t, b instanceof Blob, Object.prototype.toString.call(b)))"#
        ),
        "7 text/plain a\u{e9}bcde true [object Blob]"
    );
    assert_eq!(
        one(
            r#"console.log(JSON.stringify([new Blob().size, new Blob([], {type: 'a\u00ff'}).type, new Blob(['x'], {type: ' A '}).type]))"#
        ),
        "[0,\"\",\" a \"]"
    );
    assert_eq!(
        one(r#"const b = new Blob(['0123456789'], {type: 'x/y'});
            Promise.all([b.slice(2, 5).text(), b.slice(-3).text(), b.slice(5, 2).text(), b.slice(1, 3, 'A/B').type, b.slice().type])
            .then(r => console.log(JSON.stringify(r)))"#),
        "[\"234\",\"789\",\"\",\"a/b\",\"\"]"
    );
    // Its bytes come back as a buffer and as bytes, copies each.
    assert_eq!(
        one(
            r#"const b = new Blob(['\u20ac']); Promise.all([b.arrayBuffer(), b.bytes()]).then(([a, u]) => {
            new Uint8Array(a)[0] = 0; console.log(a.byteLength, u.join(), u instanceof Uint8Array) })"#
        ),
        "3 226,130,172 true"
    );
    // A lone surrogate is U+FFFD's three bytes; the endings are
    // transparent unless native ones are asked for.
    let native = if cfg!(windows) { "7" } else { "5" };
    assert_eq!(
        one(
            r#"console.log(new Blob(['\ud800']).size, new Blob(['a\r\nb\nc'], {endings: 'native'}).size, new Blob(['a\nb'], {endings: 'transparent'}).size)"#
        ),
        format!("3 {native} 3")
    );
    assert_eq!(
        logged(
            r#"try { new Blob('ab') } catch (e) { console.log(e.name) }
            try { new Blob([], {endings: 'odd'}) } catch (e) { console.log(e.name) }
            try { Blob.prototype.text.call({}).catch(e => console.log('rejected', e.name)) } catch (e) { console.log('threw') }"#
        ),
        ["TypeError", "TypeError", "rejected TypeError"]
    );
}

#[test]
fn a_file_is_a_blob_with_a_name_and_a_time() {
    assert_eq!(
        one(
            r#"const f = new File(['ab'], 'n.txt', {type: 'TEXT/x', lastModified: 42});
            const g = new File([], 'g');
            console.log(f.name, f.size, f.type, f.lastModified, f instanceof Blob, typeof g.lastModified, g.lastModified > 0, Object.prototype.toString.call(f))"#
        ),
        "n.txt 2 text/x 42 true number true [object File]"
    );
    assert_eq!(
        logged(r#"try { new File(['a']) } catch (e) { console.log(e.message) }"#),
        ["Failed to construct 'File': 2 arguments required, but only 1 present."]
    );
}

#[test]
fn form_data_keeps_its_entries_in_order() {
    assert_eq!(
        one(r#"const d = new FormData();
            d.append('a', '1'); d.append('b', 2); d.append('a', '3'); d.set('b', 'x'); d.set('c', 'y');
            const seen = []; for (const [k, v] of d) seen.push(k + '=' + v);
            console.log(seen.join('&'), d.get('a'), JSON.stringify(d.getAll('a')), d.has('c'), d.get('z'), [...d.keys()].join(), [...d.values()].join())"#),
        "a=1&b=x&a=3&c=y 1 [\"1\",\"3\"] true null a,b,a,c 1,x,3,y"
    );
    // A Blob is a File named "blob" or as asked; a File keeps its name.
    assert_eq!(
        one(r#"const d = new FormData();
            d.append('b', new Blob(['x'], {type: 't/u'})); d.append('n', new Blob(['y']), 'named');
            d.append('f', new File(['z'], 'f.txt')); d.append('r', new File(['z'], 'f.txt'), 'other');
            console.log(d.getAll('b')[0].name, d.get('b').type, d.get('n').name, d.get('f').name, d.get('r').name, d.get('b') instanceof File)"#),
        "blob t/u named f.txt other true"
    );
    // Changes made while iterating show; deleting goes by name.
    assert_eq!(
        one(
            r#"const d = new FormData(); d.append('a', '1'); d.append('b', '2');
            const seen = []; d.forEach((v, k) => { seen.push(k); if (k === 'a') d.append('c', '3'); });
            d.delete('b'); console.log(seen.join(), [...d.entries()].map(e => e.join('=')).join('&'))"#
        ),
        "a,b,c a=1&c=3"
    );
    assert_eq!(
        one(
            r#"const d = new FormData(); d.append('a', '1'); d.append('b', '2'); d.append('c', '3');
            const seen = []; d.forEach((v, k) => { seen.push(k); if (k === 'a') d.delete('b'); });
            console.log(seen.join())"#
        ),
        "a,c"
    );
    assert_eq!(
        logged(
            r#"const d = new FormData();
            try { d.append('a', 'b', 'c') } catch (e) { console.log(e.message) }
            try { d.append('a') } catch (e) { console.log(e.name) }
            try { FormData.prototype.get.call({}, 'a') } catch (e) { console.log(e.message) }"#
        ),
        [
            "Failed to execute 'append' on 'FormData': parameter 2 is not of type 'Blob'.",
            "TypeError",
            "Illegal invocation",
        ]
    );
}

#[test]
fn form_data_from_a_form_takes_its_entry_list() {
    let page = "<form id=f><input name=q value=hello><input type=checkbox name=c checked>\
                <input type=file name=up><button name=go value=1>Go</button>\
                <button name=other value=2>Other</button></form><form id=g><button id=out>x</button></form>\
                <p id=p></p><script>\
                const f = document.getElementById('f'), buttons = f.querySelectorAll('button');\
                const d = new FormData(f);\
                console.log([...d].map(([k, v]) => k + '=' + (typeof v === 'string' ? v : 'File:' + v.name + ':' + v.type + ':' + v.size)).join('&'));\
                console.log([...new FormData(f, buttons[1])].map(e => e.join('=')).join('&'));\
                for (const bad of [[document.getElementById('p')], [f, document.getElementById('p')], [f, document.getElementById('out')], ['x']]) {\
                  try { new FormData(...bad) } catch (e) { console.log(e.name + ': ' + e.message) }\
                }</script>";
    let console = super::script_tests::scripted(page, true).1;
    assert_eq!(
        console,
        [
            "q=hello&c=on&up=File::application/octet-stream:0",
            "q=hello&c=on&up=[object File]&other=2",
            "TypeError: Failed to construct 'FormData': parameter 1 is not of type 'HTMLFormElement'.",
            "TypeError: Failed to construct 'FormData': The specified element is not a submit button.",
            "NotFoundError: Failed to construct 'FormData': The specified element is not owned by this form element.",
            "TypeError: Failed to construct 'FormData': parameter 1 is not of type 'HTMLFormElement'.",
        ]
    );
}

#[test]
fn search_params_parse_and_serialize_as_the_url_standard_says() {
    assert_eq!(
        one(
            r#"const p = new URLSearchParams('?a=1&b=2+3&c=%41%zz%4&=x&&y&a=%E2%82%AC&%FF=z');
            console.log(JSON.stringify([...p]), p.size)"#
        ),
        "[[\"a\",\"1\"],[\"b\",\"2 3\"],[\"c\",\"A%zz%4\"],[\"\",\"x\"],[\"y\",\"\"],[\"a\",\"\u{20ac}\"],[\"\u{fffd}\",\"z\"]] 7"
    );
    assert_eq!(
        one(
            r#"const p = new URLSearchParams(); p.append('a b', 'c+d&e=f'); p.append('\u00e9*-._~!', '');
            console.log(p.toString(), String(new URLSearchParams([['x', '1'], ['y', '2']])), new URLSearchParams({k: 'v', n: 1}).toString())"#
        ),
        "a+b=c%2Bd%26e%3Df&%C3%A9*-._%7E%21= x=1&y=2 k=v&n=1"
    );
    assert_eq!(
        one(r#"const p = new URLSearchParams('b=2&a=1&b=1&a=0&c=3');
            p.sort(); const s1 = p.toString();
            p.delete('b', '1'); const has = [p.has('b'), p.has('b', '1'), p.has('a', '0')];
            p.set('a', 'x'); p.delete('c');
            console.log(s1, JSON.stringify(has), p.toString(), p.get('a'), JSON.stringify(p.getAll('b')), p.get('zz'))"#),
        "a=1&a=0&b=2&b=1&c=3 [true,false,true] a=x&b=2 x [\"2\"] null"
    );
    assert_eq!(
        logged(
            r#"try { new URLSearchParams([['a']]) } catch (e) { console.log(e.name) }
            try { new URLSearchParams([1]) } catch (e) { console.log(e.name) }
            console.log(new URLSearchParams(null).toString(), new URLSearchParams(new URLSearchParams('q=1')).toString(),
              Object.prototype.toString.call(new URLSearchParams().entries()))"#
        ),
        [
            "TypeError",
            "TypeError",
            "null= q=1 [object URLSearchParams Iterator]"
        ]
    );
}

#[test]
fn text_is_utf_8_both_ways_with_replacement_for_what_is_not() {
    // A pair of surrogates is one code point's four bytes.
    assert_eq!(one(r#"console.log(new Blob(['\u{1F600}']).size)"#), "4");
    // Overlong, surrogate and too-high sequences are each a U+FFFD a
    // byte, the byte that broke one read again; a sequence cut short at
    // the end is one; a leading byte order mark goes.
    assert_eq!(
        one(
            r#"new Blob([new Uint8Array([0xE0, 0x80, 0x80, 0xED, 0xA0, 0x80, 0xF4, 0x90, 0x80, 0x80, 0x61, 0xE2, 0x82])]).text()
            .then(t => console.log(t === '\ufffd'.repeat(10) + 'a\ufffd'))"#
        ),
        "true"
    );
    assert_eq!(
        one(
            r#"Promise.all([new Blob([new Uint8Array([0xEF, 0xBB, 0xBF, 0x61, 0xEF, 0xBB, 0xBF])]).text(),
            new Blob(['\u0416\u20ac']).bytes()]).then(([t, b]) => console.log(t.length, t.charCodeAt(1), b.join()))"#
        ),
        "2 65279 208,150,226,130,172"
    );
}
