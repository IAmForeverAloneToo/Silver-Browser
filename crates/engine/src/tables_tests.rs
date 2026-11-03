//! The table API (gap batch H, HTML section 4.9): `HTMLTableElement`'s
//! sections, rows and the methods that make and remove them, a section's
//! and a row's, and a cell's index, with live collections, as the
//! reference browser answers.

use crate::{Engine, differential};

/// What each of `cases`, an expression, gives as a string, or the error
/// it throws, its name and message.
fn answers(cases: &[&str]) -> Vec<String> {
    let mut engine = Engine::with_fonts(
        text::FontCollection::from_bytes(text::TEST_FONT.to_vec(), 0)
            .expect("the bundled font parses"),
    );
    engine.set_scripting(true);
    engine.load_document("<p>x</p>");
    engine.take_console();
    let mut script = String::from(
        "const tags = (e) => Array.from(e.children).map(c => c.tagName).join();
         const ids = (list) => Array.from(list).map(e => e.id).join();",
    );
    for case in cases {
        script.push_str(&format!(
            "try {{ console.log(String({case})); }} \
             catch (e) {{ console.log(e.name + ': ' + e.message); }}\n"
        ));
    }
    differential::run_script(&mut engine, &script);
    engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

#[test]
fn a_tables_rows_are_its_heads_then_its_own_and_its_bodies_then_its_feet() {
    let said = answers(&[
        "(() => { const t = document.createElement('table'); t.innerHTML = '<tfoot><tr id=f>\
           </tr></tfoot><tr id=x></tr><tbody><tr id=b></tr></tbody><thead><tr id=h></tr>\
           </thead><thead><tr id=h2></tr></thead>'; window.t2 = t; return ids(t.rows); })()",
        "t2.tHead.firstChild.id + ' ' + t2.tFoot.firstChild.id + ' ' + t2.tBodies.length",
        "t2.rows === t2.rows && t2.tBodies === t2.tBodies",
        // Live: a row added shows at once.
        "(() => { const rows = t2.rows; t2.tBodies[0].appendChild(document.createElement('tr')); \
           return rows.length; })()",
        "t2.insertRow().parentNode.tagName + ' ' + t2.rows.length",
        "t2.insertRow(0).parentNode.tagName",
        "t2.rows[3].rowIndex + ' ' + t2.rows[3].sectionRowIndex",
        "document.createElement('tr').rowIndex",
        "(() => { const t = document.createElement('table'); const r = t.insertRow(); \
           return r.parentNode.tagName + ' ' + tags(t); })()",
        "(() => { const t = document.createElement('table'); t.appendChild(document\
           .createElement('tbody')); t.appendChild(document.createElement('tbody')); \
           return t.insertRow().parentNode === t.lastChild; })()",
        "(() => { const t = document.createElement('table'); t.innerHTML = '<tr id=a></tr>\
           <tr id=b></tr><tr id=c></tr>'; t.deleteRow(1); t.deleteRow(-1); \
           return ids(t.rows); })()",
    ]);
    assert_eq!(
        said,
        [
            "h,h2,x,b,f",
            "h f 2",
            "true",
            "6",
            "TFOOT 7",
            "THEAD",
            "3 0",
            "-1",
            "TBODY TBODY",
            "true",
            "a"
        ]
    );
}

#[test]
fn captions_and_sections_go_where_html_puts_them() {
    let said = answers(&[
        "(() => { const t = document.createElement('table'); t.innerHTML = '<caption></caption>\
           <colgroup></colgroup><tbody></tbody>'; window.t5 = t; t.createTHead(); \
           return tags(t); })()",
        "(() => { t5.createTFoot(); t5.createTBody(); return tags(t5); })()",
        "(() => { const h = document.createElement('thead'); t5.tHead = h; \
           return tags(t5) + ' ' + (t5.tHead === h); })()",
        "(() => { t5.deleteCaption(); t5.deleteTHead(); t5.deleteTFoot(); return tags(t5); })()",
        "(() => { const c = document.createElement('caption'); t5.caption = c; \
           return t5.firstChild === c && t5.createCaption() === c; })()",
        "(() => { t5.caption = null; return t5.caption; })()",
        "(() => { t5.caption = document.createElement('div'); })()",
        "(() => { t5.tHead = document.createElement('div'); })()",
        "(() => { t5.tFoot = document.createElement('thead'); })()",
        "(() => { const x = document.implementation.createDocument(\
           'http://www.w3.org/1999/xhtml', 'html', null); const t = x.createElementNS(\
           'http://www.w3.org/1999/xhtml', 'table'); const c = t.createCaption(); \
           return c.namespaceURI + ' ' + c.localName + ' ' + (c.ownerDocument === x); })()",
        "Object.getOwnPropertyNames(HTMLTableElement.prototype).filter(n => \
           /^(caption|tHead|tFoot|tBodies|rows|create|delete|insert)/.test(n)).sort().join()",
    ]);
    assert_eq!(
        said,
        [
            "CAPTION,COLGROUP,THEAD,TBODY",
            "CAPTION,COLGROUP,THEAD,TBODY,TBODY,TFOOT",
            "CAPTION,COLGROUP,THEAD,TBODY,TBODY,TFOOT true",
            "COLGROUP,TBODY,TBODY",
            "true",
            "null",
            "TypeError: Failed to set the 'caption' property on 'HTMLTableElement': Failed to \
             convert value to 'HTMLTableCaptionElement'.",
            "TypeError: Failed to set the 'tHead' property on 'HTMLTableElement': Failed to \
             convert value to 'HTMLTableSectionElement'.",
            "HierarchyRequestError: Failed to set the 'tFoot' property on 'HTMLTableElement': Not \
             a tfoot element.",
            "http://www.w3.org/1999/xhtml caption true",
            "caption,createCaption,createTBody,createTFoot,createTHead,deleteCaption,deleteRow,\
             deleteTFoot,deleteTHead,insertRow,rows,tBodies,tFoot,tHead",
        ]
    );
}

#[test]
fn rows_and_cells_are_made_and_removed_by_index_with_the_reference_browsers_errors() {
    let said = answers(&[
        "document.createElement('table').insertRow(-2)",
        "document.createElement('table').insertRow(1)",
        "document.createElement('table').deleteRow(0)",
        "document.createElement('table').deleteRow(-1)",
        "document.createElement('tbody').insertRow(3)",
        "document.createElement('tbody').deleteRow(0)",
        "document.createElement('tbody').deleteRow(-1)",
        "document.createElement('tr').insertCell(2)",
        "document.createElement('tr').deleteCell(0)",
        "document.createElement('tr').deleteCell(-1)",
        "(() => { const s = document.createElement('tbody'); s.innerHTML = '<tr id=a></tr>\
           <tr id=b></tr>'; s.insertRow(1).id = 'n'; s.deleteRow(0); \
           return ids(s.rows) + ' ' + s.rows[1].sectionRowIndex; })()",
        "(() => { const r = document.createElement('tr'); r.innerHTML = \
           '<td></td><th></th><td></td>'; return r.cells.length + ' ' + r.cells[2].cellIndex + \
           ' ' + r.insertCell(1).cellIndex + ' ' + r.cells[1].localName; })()",
        "(() => { const r = document.createElement('tr'); r.innerHTML = '<td id=a></td><th \
           id=b></th>'; r.deleteCell(-1); return ids(r.cells); })()",
        "document.createElement('td').cellIndex",
        "HTMLTableElement.prototype.insertRow.length + ' ' + \
           HTMLTableElement.prototype.deleteRow.length + ' ' + \
           HTMLTableRowElement.prototype.insertCell.length",
    ]);
    assert_eq!(
        said,
        [
            "IndexSizeError: Failed to execute 'insertRow' on 'HTMLTableElement': The index \
             provided (-2) is less than -1.",
            "IndexSizeError: Failed to execute 'insertRow' on 'HTMLTableElement': The index \
             provided (1) is greater than the number of rows in the table (0).",
            "IndexSizeError: Failed to execute 'deleteRow' on 'HTMLTableElement': The index \
             provided (0) is greater than the number of rows in the table (0).",
            "undefined",
            "IndexSizeError: Failed to execute 'insertRow' on 'HTMLTableSectionElement': The \
             provided index (3 is outside the range [-1, 0].",
            "IndexSizeError: Failed to execute 'deleteRow' on 'HTMLTableSectionElement': The \
             provided index (0 is outside the range [-1, 0].",
            "undefined",
            "IndexSizeError: Failed to execute 'insertCell' on 'HTMLTableRowElement': The value \
             provided (2) is outside the range [-1, 0].",
            "IndexSizeError: Failed to execute 'deleteCell' on 'HTMLTableRowElement': The value \
             provided (0) is outside the range [0, 0).",
            "undefined",
            "n,b 1",
            "3 2 1 td",
            "a",
            "-1",
            "0 1 0",
        ]
    );
}
