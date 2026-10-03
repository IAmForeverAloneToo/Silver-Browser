//! The acceptance layouts of M17 and M18 (M17.10 and M18.7, the flexbox
//! and grid notes' "Acceptance"): pages under `tests/layouts/`, real
//! layouts with text, whose every element's box `tests/wpt.rs` checks as
//! the web-platform-tests runner checks a check-layout test.
//!
//! `cargo run -p engine --example layouts -- capture [page.html ...]`
//! writes the expectations into the pages, as the reference browser
//! lays them out: every element's `offsetWidth` and `offsetHeight` into
//! its `data-expected-width` and `-height`, and its `offsetLeft` and
//! `offsetTop` into `data-offset-x` and `-y`, the body's offsets aside
//! (CSSOM View makes them zero). Elements whose box is an inline one, or
//! that have none, are left without: an inline box's offsets are its
//! first fragment's, not the rectangle around them all. The pages are
//! captured alone when named, all of them when none is.
//!
//! The browser is `$BROWSER`, else `chrome-headless-shell` on the PATH,
//! with `$BROWSER_FLAGS` added (a container running as root needs
//! `--no-sandbox`), at 800 by 600 and device scale one, as
//! `tests/reference/capture.sh` runs it. A page's `@font-face` gives the
//! browser this engine's own font, DejaVu Sans, from `crates/text/fonts`:
//! the capture stops if the browser did not load it. Each page is
//! measured from a copy beside it that carries the measuring script in
//! its head, and the copy is removed after. The page's start tags from
//! its body on are paired with the elements the browser reports, in
//! order: a page whose tags the browser's parser would rearrange (an
//! implied `tbody`, a `p` closed by a `div`) stops the capture rather
//! than putting a box's numbers on another element. Look at what
//! changed before committing it.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The expectations a capture writes, each removed before it is written
/// again.
const ATTRIBUTES: &[&str] = &[
    "data-expected-width",
    "data-expected-height",
    "data-offset-x",
    "data-offset-y",
];

/// The script the measured copy carries: once the page and its fonts
/// have loaded, every face's status, then the body and each element in
/// it, in document order, as `name display width height left top`.
const SCRIPT: &str = "<script>\
addEventListener('load', () => document.fonts.ready.then(() => {\
  const lines = ['FONTS ' + [...document.fonts].map(f => f.status).join(' ')];\
  for (const e of [document.body, ...document.body.querySelectorAll('*')]) {\
    lines.push([e.localName, getComputedStyle(e).display, e.offsetWidth, e.offsetHeight,\
      e.offsetLeft, e.offsetTop].join(' '));\
  }\
  const pre = document.createElement('pre');\
  pre.textContent = 'CAPTURE\\n' + lines.join('\\n') + '\\nEND';\
  document.documentElement.appendChild(pre);\
}));\
</script>";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some((command, pages)) = args.split_first() else {
        eprintln!("usage: layouts capture [page.html ...]");
        return ExitCode::FAILURE;
    };
    if command != "capture" {
        eprintln!("usage: layouts capture [page.html ...]");
        return ExitCode::FAILURE;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/layouts");
    let pages: Vec<PathBuf> = if pages.is_empty() {
        let mut all: Vec<PathBuf> = match fs::read_dir(&dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "html"))
                .collect(),
            Err(error) => {
                eprintln!("{}: {error}", dir.display());
                return ExitCode::FAILURE;
            }
        };
        all.sort();
        all
    } else {
        pages.iter().map(|p| dir.join(p)).collect()
    };
    let mut failed = false;
    for page in &pages {
        match capture(page) {
            Ok(count) => println!("{}: {count} elements", page.display()),
            Err(error) => {
                eprintln!("{}: {error}", page.display());
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// One element as the browser reports it.
struct Measured {
    name: String,
    display: String,
    width: String,
    height: String,
    left: String,
    top: String,
}

/// Captures `page`, returning how many elements were given expectations.
fn capture(page: &Path) -> Result<usize, String> {
    let source = fs::read_to_string(page).map_err(|e| e.to_string())?;
    let source = without_expectations(&source);
    let measured = measure(page, &source)?;
    let (out, count) = annotated(&source, &measured)?;
    fs::write(page, out).map_err(|e| e.to_string())?;
    Ok(count)
}

/// `source`, its expectations taken out, with those of `measured` put
/// on its elements, paired with its start tags from the body's on; and
/// how many elements were given them.
fn annotated(source: &str, measured: &[Measured]) -> Result<(String, usize), String> {
    let tags = start_tags(source)?;
    if tags.len() != measured.len() {
        return Err(format!(
            "the page has {} start tags from its body on, the browser {} elements",
            tags.len(),
            measured.len()
        ));
    }
    let mut out = source.to_string();
    let mut count = 0;
    // From the end, so that the positions before stay where they were.
    for (index, (tag, m)) in tags.iter().zip(measured).enumerate().rev() {
        if tag.name != m.name {
            return Err(format!(
                "element {index} is <{}> in the page and <{}> in the browser",
                tag.name, m.name
            ));
        }
        if m.display == "inline" || m.display == "none" || m.display == "contents" {
            continue;
        }
        let mut attributes = format!(
            " data-expected-width=\"{}\" data-expected-height=\"{}\"",
            m.width, m.height
        );
        if index > 0 {
            attributes.push_str(&format!(
                " data-offset-x=\"{}\" data-offset-y=\"{}\"",
                m.left, m.top
            ));
        }
        out.insert_str(tag.end, &attributes);
        count += 1;
    }
    Ok((out, count))
}

/// `source` with the attributes a capture writes taken out.
fn without_expectations(source: &str) -> String {
    let mut out = source.to_string();
    for name in ATTRIBUTES {
        let needle = format!(" {name}=\"");
        while let Some(start) = out.find(&needle) {
            let value = start + needle.len();
            let Some(end) = out[value..].find('"') else {
                break;
            };
            out.replace_range(start..value + end + 1, "");
        }
    }
    out
}

/// What the browser reports of `source`, the text of `page`, measured
/// from a copy beside it.
fn measure(page: &Path, source: &str) -> Result<Vec<Measured>, String> {
    let Some(head) = source.find("</head>") else {
        return Err("the page has no </head> to put the measuring script before".into());
    };
    let mut probe = source.to_string();
    probe.insert_str(head, SCRIPT);
    let name = page
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the page's name is not text")?;
    let copy = page.with_file_name(format!(".capture-{name}"));
    fs::write(&copy, probe).map_err(|e| e.to_string())?;
    let output = run_browser(&copy);
    let _ = fs::remove_file(&copy);
    let dom = output?;
    let start = dom
        .find("CAPTURE\n")
        .ok_or("the browser reported nothing")?;
    let end = dom[start..]
        .find("\nEND")
        .ok_or("the browser's report has no end")?;
    let mut lines = dom[start + "CAPTURE\n".len()..start + end].lines();
    let fonts = lines.next().unwrap_or("");
    let statuses: Vec<&str> = fonts.split_whitespace().skip(1).collect();
    if statuses.is_empty() || statuses.iter().any(|s| *s != "loaded") {
        return Err(format!("the browser did not load the page's font: {fonts}"));
    }
    lines
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [name, display, width, height, left, top] = fields[..] else {
                return Err(format!("a malformed line in the browser's report: {line}"));
            };
            Ok(Measured {
                name: name.to_string(),
                display: display.to_string(),
                width: width.to_string(),
                height: height.to_string(),
                left: left.to_string(),
                top: top.to_string(),
            })
        })
        .collect()
}

/// The DOM the browser serializes for `page` once its layout is done.
fn run_browser(page: &Path) -> Result<String, String> {
    let browser = env::var("BROWSER").unwrap_or_else(|_| "chrome-headless-shell".into());
    let flags = env::var("BROWSER_FLAGS").unwrap_or_default();
    let page = fs::canonicalize(page).map_err(|e| e.to_string())?;
    let output = Command::new(&browser)
        .args([
            "--disable-gpu",
            "--hide-scrollbars",
            "--force-device-scale-factor=1",
            "--force-color-profile=srgb",
            "--window-size=800,600",
            "--virtual-time-budget=5000",
        ])
        .args(flags.split_whitespace())
        .arg("--dump-dom")
        .arg(format!("file://{}", page.display()))
        .output()
        .map_err(|e| format!("{browser}: {e}"))?;
    if !output.status.success() {
        return Err(format!("{browser} exited with {}", output.status));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

/// A start tag of the page: its element's name, and where its
/// attributes end (before its `>`, or its `/>`).
struct Tag {
    name: String,
    end: usize,
}

/// The start tags of `source` from its body's on, in order: comments and
/// end tags passed over, and the text of a `script` or `style` element.
fn start_tags(source: &str) -> Result<Vec<Tag>, String> {
    let bytes = source.as_bytes();
    // Tag names are not case-sensitive; lowering ASCII keeps every
    // position where it was.
    let mut at = source
        .to_ascii_lowercase()
        .find("<body")
        .ok_or("the page has no <body>")?;
    let mut tags = Vec::new();
    while let Some(offset) = source[at..].find('<') {
        let open = at + offset;
        let rest = &source[open..];
        if rest.starts_with("<!--") {
            let close = rest.find("-->").ok_or("a comment is not closed")?;
            at = open + close + 3;
            continue;
        }
        let Some(&next) = bytes.get(open + 1) else {
            break;
        };
        if !next.is_ascii_alphabetic() {
            let close = rest.find('>').ok_or("a tag is not closed")?;
            at = open + close + 1;
            continue;
        }
        let name_end = rest[1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .map_or(rest.len(), |n| n + 1);
        let name = rest[1..name_end].to_ascii_lowercase();
        // The tag ends at the first `>` outside a quoted value.
        let mut quote = None;
        let mut close = None;
        for (i, c) in rest.char_indices().skip(name_end) {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(c),
                (None, '>') => {
                    close = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let close = close.ok_or("a start tag is not closed")?;
        let end = if rest[..close].ends_with('/') {
            open + close - 1
        } else {
            open + close
        };
        at = open + close + 1;
        if name == "script" || name == "style" {
            let closing = format!("</{name}");
            let text_end = source[at..]
                .find(&closing)
                .ok_or_else(|| format!("a {name} element is not closed"))?;
            at += text_end;
        }
        tags.push(Tag { name, end });
    }
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(name: &str, display: &str, numbers: [&str; 4]) -> Measured {
        Measured {
            name: name.to_string(),
            display: display.to_string(),
            width: numbers[0].to_string(),
            height: numbers[1].to_string(),
            left: numbers[2].to_string(),
            top: numbers[3].to_string(),
        }
    }

    #[test]
    fn the_start_tags_are_the_bodys_on_past_comments_end_tags_and_raw_text() {
        let source = "<html><head><title>t</title></head>\
            <BODY class=a><!-- <p> --><div title='a > b'><img src=\"x\"/>\
            <style>p > q { }</style></div><p>x</p></body></html>";
        let tags = start_tags(source).expect("the page scans");
        let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["body", "div", "img", "style", "p"]);
        // Each ends where its attributes do: before `>`, or `/>`.
        assert_eq!(&source[tags[0].end..tags[0].end + 1], ">");
        assert_eq!(&source[tags[1].end - 1..tags[1].end + 1], "'>");
        assert_eq!(&source[tags[2].end..tags[2].end + 2], "/>");
        assert!(start_tags("<p>no body</p>").is_err());
        assert!(start_tags("<body><div title='open>").is_err());
    }

    #[test]
    fn a_capture_takes_out_what_one_before_wrote() {
        let source = "<body data-expected-width=\"8\" data-expected-height=\"6\">\
            <div id=a data-offset-x=\"1\" data-offset-y=\"2\" data-test=\"kept\"></div></body>";
        assert_eq!(
            without_expectations(source),
            "<body><div id=a data-test=\"kept\"></div></body>"
        );
    }

    #[test]
    fn expectations_go_on_the_elements_with_boxes_the_bodys_without_offsets() {
        let source = "<head></head><body><div class=a><span>x</span></div><p hidden></p></body>";
        let measured = [
            measured("body", "block", ["800", "40", "0", "0"]),
            measured("div", "flex", ["800", "20", "0", "0"]),
            measured("span", "inline", ["8", "19", "0", "0"]),
            measured("p", "none", ["0", "0", "0", "0"]),
        ];
        let (out, count) = annotated(source, &measured).expect("the tags pair");
        assert_eq!(count, 2);
        assert_eq!(
            out,
            "<head></head><body data-expected-width=\"800\" data-expected-height=\"40\">\
             <div class=a data-expected-width=\"800\" data-expected-height=\"20\" \
             data-offset-x=\"0\" data-offset-y=\"0\"><span>x</span></div><p hidden></p></body>"
        );
        // A page whose elements are not the browser's stops the capture.
        assert!(annotated(source, &measured[..3]).is_err());
        let mut renamed = measured;
        renamed[1].name = "section".to_string();
        assert!(annotated(source, &renamed).is_err());
    }
}
