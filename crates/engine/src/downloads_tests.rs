//! Downloads (M37): names made safe, the file written under its partial
//! name and renamed when complete, the mark of the web, a file that runs
//! code held until kept, a download cancelled or broken off removed, and
//! a navigation's download saved by the engine.

use std::path::PathBuf;

use net::Url;

use crate::downloads::{
    DownloadState, Downloads, MAX_DOWNLOADS, MAX_FILE_NAME, PARTIAL, is_dangerous, safe_name,
};
use crate::testing::*;

/// A folder of its own for test `name`, empty.
fn folder(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("silver-downloads-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn url(text: &str) -> Url {
    Url::parse(text).expect("parses")
}

fn files(folder: &PathBuf) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_name_is_made_safe() {
    let page = url("https://example.com/files/report.pdf");
    let name = |suggested: &str| safe_name(Some(suggested), &page, "application/pdf");
    assert_eq!(name("q3 report.pdf"), "q3 report.pdf");
    assert_eq!(name("..\\..\\windows\\evil.pdf"), "evil.pdf");
    assert_eq!(name("/etc/passwd"), "passwd.pdf");
    assert_eq!(name("a<b>c:d\"e|f?g*h.txt"), "a_b_c_d_e_f_g_h.txt");
    assert_eq!(name("bell\u{7}.txt"), "bell_.txt");
    // A right-to-left override cannot hide an extension.
    assert_eq!(name("invoice\u{202e}fdp.exe"), "invoice_fdp.exe");
    assert_eq!(name("..hidden.txt"), "hidden.txt");
    assert_eq!(name("trailing.txt. . "), "trailing.txt");
    assert_eq!(name("CON.txt"), "_CON.txt");
    assert_eq!(name("nul"), "_nul.pdf");
    assert_eq!(name("   "), "report.pdf", "the URL's when none is given");
    // The URL's last segment, decoded; else `download`, with the type's
    // extension.
    assert_eq!(
        safe_name(
            None,
            &url("https://a.test/x/My%20File.zip"),
            "application/zip"
        ),
        "My File.zip"
    );
    assert_eq!(
        safe_name(None, &url("https://a.test/"), "text/plain"),
        "download.txt"
    );
    assert_eq!(
        safe_name(None, &url("data:text/plain,hello"), "text/plain"),
        "download.txt"
    );
    assert_eq!(
        safe_name(None, &url("https://a.test/data"), "application/x-weird"),
        "data"
    );
    // Cut to its bound, its extension kept, at a character's boundary.
    let long = format!("{}.tar.gz", "\u{e9}".repeat(300));
    let cut = name(&long);
    assert!(cut.len() <= MAX_FILE_NAME && cut.ends_with(".gz"), "{cut}");
}

#[test]
fn a_download_is_written_under_its_partial_name_then_takes_its_own() {
    let folder = folder("written");
    let downloads = Downloads::new();
    assert!(
        downloads
            .start(&url("https://a.test/f.txt"), None, "text/plain", None)
            .is_err()
    );
    downloads.set_folder(folder.clone());
    let mut sink = downloads
        .start(&url("https://a.test/f.txt"), None, "text/plain", Some(10))
        .expect("started");
    assert_eq!(files(&folder), [format!("f.txt.{PARTIAL}")]);
    assert!(sink.write(b"hello "));
    assert!(sink.write(b"file"));
    assert_eq!(downloads.list()[0].received, 10);
    sink.finish();
    assert_eq!(files(&folder), ["f.txt"]);
    assert_eq!(std::fs::read(folder.join("f.txt")).unwrap(), b"hello file");
    let listed = &downloads.list()[0];
    assert_eq!(listed.state, DownloadState::Complete);
    assert_eq!(listed.path, folder.join("f.txt"));
    assert_eq!(downloads.take_notices(), ["Saving f.txt", "Saved f.txt"]);
    // The mark of the web, on Windows: the internet zone.
    #[cfg(windows)]
    {
        let mark = std::fs::read_to_string(folder.join("f.txt:Zone.Identifier")).unwrap();
        assert!(mark.contains("ZoneId=3"), "{mark}");
        assert!(mark.contains("HostUrl=https://a.test/f.txt"), "{mark}");
    }
    // The name taken, the next is numbered.
    let sink = downloads
        .start(&url("https://a.test/f.txt"), None, "text/plain", None)
        .expect("started");
    sink.finish();
    assert_eq!(files(&folder), ["f (1).txt", "f.txt"]);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_file_that_runs_code_is_held_until_kept() {
    let folder = folder("held");
    let downloads = Downloads::new();
    downloads.set_folder(folder.clone());
    let mut sink = downloads
        .start(
            &url("https://a.test/setup.exe"),
            None,
            "application/octet-stream",
            None,
        )
        .expect("started");
    assert!(sink.write(b"MZ"));
    sink.finish();
    assert_eq!(files(&folder), [format!("setup.exe.{PARTIAL}")]);
    let id = downloads.list()[0].id;
    assert_eq!(downloads.list()[0].state, DownloadState::Held);
    downloads.keep(id).expect("kept");
    assert_eq!(files(&folder), ["setup.exe"]);
    assert_eq!(downloads.list()[0].state, DownloadState::Complete);
    assert!(downloads.keep(id).is_err(), "kept once");
    assert!(is_dangerous("a.PS1") && is_dangerous("x.tar.bat"));
    assert!(!is_dangerous("a.pdf") && !is_dangerous("exe"));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_download_cancelled_or_broken_off_leaves_no_file() {
    let folder = folder("cancelled");
    let downloads = Downloads::new();
    downloads.set_folder(folder.clone());
    let mut sink = downloads
        .start(
            &url("https://a.test/a.bin"),
            Some("a.bin"),
            "application/octet-stream",
            None,
        )
        .expect("started");
    assert!(sink.write(b"part"));
    downloads.cancel(downloads.list()[0].id);
    assert!(!sink.write(b"more"), "the response is dropped");
    drop(sink);
    assert_eq!(downloads.list()[0].state, DownloadState::Cancelled);
    // A writer gone before its end: failed.
    let mut sink = downloads
        .start(
            &url("https://a.test/b.bin"),
            Some("b.bin"),
            "application/octet-stream",
            None,
        )
        .expect("started");
    assert!(sink.write(b"part"));
    drop(sink);
    assert!(matches!(
        downloads.list()[1].state,
        DownloadState::Failed(_)
    ));
    let failed = downloads
        .start(
            &url("https://a.test/c.bin"),
            Some("c.bin"),
            "application/octet-stream",
            None,
        )
        .expect("started");
    failed.fail("the connection was reset".to_owned());
    assert_eq!(
        downloads.list()[2].state,
        DownloadState::Failed("the connection was reset".to_owned())
    );
    assert!(files(&folder).is_empty(), "{:?}", files(&folder));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn the_list_keeps_the_newest() {
    let folder = folder("list");
    let downloads = Downloads::new();
    downloads.set_folder(folder.clone());
    for n in 0..MAX_DOWNLOADS + 5 {
        let sink = downloads
            .start(
                &url(&format!("https://a.test/{n}.txt")),
                None,
                "text/plain",
                None,
            )
            .expect("started");
        sink.finish();
    }
    let list = downloads.list();
    assert_eq!(list.len(), MAX_DOWNLOADS);
    assert_eq!(list[0].name, "5.txt");
    downloads.remove(list[0].id);
    assert_eq!(downloads.list().len(), MAX_DOWNLOADS - 1);
    assert!(folder.join("5.txt").exists(), "its file stays");
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_navigation_to_an_attachment_is_saved_and_the_page_stays() {
    let folder = folder("navigation");
    let server = serve(|path| match path {
        "/" => Some(html("<title>page</title>")),
        "/get" => Some(net::test_server::Reply::with_fields(
            200,
            &[
                ("Content-Type", "text/csv"),
                ("Content-Disposition", "attachment; filename=\"rates.csv\""),
            ],
            b"a,b\n1,2\n",
        )),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.downloads().set_folder(folder.clone());
    tab.engine.navigate(server.url("/"));
    tab.wait_until(|engine| engine.title() == "page");
    tab.engine.navigate(server.url("/get"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while files(&folder) != ["rates.csv"] {
        assert!(std::time::Instant::now() < deadline, "{:?}", files(&folder));
        tab.engine.poll_network();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read(folder.join("rates.csv")).unwrap(),
        b"a,b\n1,2\n"
    );
    assert_eq!(tab.engine.url(), Some(&server.url("/")));
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_link_with_a_download_attribute_is_saved_under_its_name() {
    let folder = folder("attribute");
    let server = serve(|path| match path {
        "/" => Some(html(
            "<style>body { margin: 0 } a { display: block; height: 40px }</style>\
             <a href=/files/raw.txt download=renamed.txt>named</a>\
             <a href=/files/disposed.txt download=ignored.txt>disposed</a>\
             <a href='data:text/plain,inline%20text' download>data</a>\
             <a href=http://localhost:1/elsewhere.txt download>elsewhere</a>",
        )),
        "/files/disposed.txt" => Some(net::test_server::Reply::with_fields(
            200,
            &[("Content-Disposition", "attachment; filename=\"server.txt\"")],
            b"disposed",
        )),
        _ => Some(net::test_server::Reply::ok("text/plain", b"raw")),
    });
    let mut tab = Tab::new();
    tab.engine.downloads().set_folder(folder.clone());
    tab.engine.navigate(server.url("/"));
    tab.wait_until(|engine| engine.load_state() == &crate::LoadState::Loaded);
    tab.engine.frame(400, 300);
    let click = |tab: &mut Tab, y: f32| {
        tab.engine.pointer_move(10.0, y);
        tab.engine.pointer_down(10.0, y, crate::Button::Primary, 1);
        tab.engine.pointer_up(10.0, y, crate::Button::Primary, 1);
    };
    let wait = |want: &[&str]| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while files(&folder) != want {
            assert!(std::time::Instant::now() < deadline, "{:?}", files(&folder));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    click(&mut tab, 20.0);
    wait(&["renamed.txt"]);
    // The response's own name comes first.
    click(&mut tab, 60.0);
    wait(&["renamed.txt", "server.txt"]);
    click(&mut tab, 100.0);
    wait(&["download.txt", "renamed.txt", "server.txt"]);
    assert_eq!(
        std::fs::read(folder.join("download.txt")).unwrap(),
        b"inline text"
    );
    // Another origin's link is followed, not saved.
    click(&mut tab, 140.0);
    tab.wait_until(|engine| {
        engine
            .url()
            .is_some_and(|url| url.as_str().ends_with("/elsewhere.txt"))
    });
    assert_eq!(files(&folder).len(), 3);
    let _ = std::fs::remove_dir_all(&folder);
}
