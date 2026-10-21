//! The windows' contexts and the profiles (M37): the windows of one
//! profile share its context, private ones one held in memory, let go
//! of with their last window; a profile's name holds to its rules, and
//! the profiles are folders beside the default's.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use profile::{Memory, Profile};

use super::*;

/// A folder of its own for a test, empty.
fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("silver-windows-tests")
        .join(format!("{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a test folder");
    dir
}

/// Contexts whose default profile is `default`, the folders they open
/// noted in `opened`, each a profile in memory.
fn contexts(default: &Path, opened: &Rc<RefCell<Vec<PathBuf>>>) -> Contexts {
    let opened = opened.clone();
    Contexts::new(
        Some(default.to_owned()),
        Box::new(move |folder: &Path| {
            opened.borrow_mut().push(folder.to_owned());
            Profile::in_memory(&Memory::new()).map_err(|e| e.to_string())
        }),
    )
}

#[test]
fn a_profiles_name_holds_to_its_rules() {
    for name in ["Work", "a b", "x-1_2", "\u{e9}t\u{e9}", &"n".repeat(64)] {
        assert_eq!(check_name(name), Ok(()), "{name}");
    }
    for name in [
        "",
        "   ",
        " lead",
        "trail ",
        "a/b",
        "a\\b",
        "..",
        "a.b",
        "c:",
        "Default",
        "default",
        "CON",
        "nul",
        "com1",
        "Lpt9",
        &"n".repeat(65),
    ] {
        assert!(check_name(name).is_err(), "{name:?}");
    }
}

#[test]
fn the_windows_of_a_profile_share_its_context_and_private_ones_their_own() {
    let dir = folder("share");
    let default = dir.join("SilverBrowser").join("Profile");
    let opened = Rc::default();
    let mut contexts = contexts(&default, &opened);
    let first = engine::Context::new();
    let kept = Kept::Folder(default.clone());
    contexts.add(kept.clone(), first.clone());
    // Ctrl+N: the same context.
    let (again, context) = contexts.open_for(&WindowRequest::New, &kept).unwrap();
    assert_eq!(again, kept);
    assert!(context.same(&first));
    // The default profile by its name is it too.
    let (_, context) = contexts
        .open_for(
            &WindowRequest::Profile(DEFAULT_NAME.to_owned()),
            &Kept::Private,
        )
        .unwrap();
    assert!(context.same(&first));
    assert!(opened.borrow().is_empty(), "nothing opened again");
    // Private windows share one context of their own.
    let (private, one) = contexts.open_for(&WindowRequest::Private, &kept).unwrap();
    assert_eq!(private, Kept::Private);
    assert!(!one.same(&first));
    let (_, two) = contexts.open_for(&WindowRequest::New, &private).unwrap();
    assert!(
        two.same(&one),
        "Ctrl+N in a private window: another private one"
    );
    // Gone with the last of them; the next is a new one.
    assert!(contexts.closed(&Kept::Private).is_none());
    assert!(
        contexts
            .closed(&Kept::Private)
            .is_some_and(|gone| gone.same(&one))
    );
    let (_, three) = contexts.open_for(&WindowRequest::Private, &kept).unwrap();
    assert!(!three.same(&one));
    // A named profile: its folder beside the default's, opened once.
    let (work, context) = contexts
        .open_for(&WindowRequest::Profile("Work".to_owned()), &kept)
        .unwrap();
    let place = dir.join("SilverBrowser").join("Profiles").join("Work");
    assert_eq!(work, Kept::Folder(place.clone()));
    assert_eq!(*opened.borrow(), std::slice::from_ref(&place));
    let (_, shared) = contexts
        .open_for(&WindowRequest::Profile("Work".to_owned()), &kept)
        .unwrap();
    assert!(shared.same(&context));
    assert_eq!(opened.borrow().len(), 1);
    assert_eq!(contexts.name_of(&work), "Work");
    assert_eq!(contexts.name_of(&kept), DEFAULT_NAME);
    // A name it may not have opens nothing.
    assert!(
        contexts
            .open_for(&WindowRequest::Profile("../x".to_owned()), &kept)
            .is_err()
    );
    // The default's windows: three, closed one by one.
    assert!(contexts.closed(&kept).is_none());
    assert!(contexts.closed(&kept).is_none());
    assert!(contexts.closed(&kept).is_some());
}

#[test]
fn a_profile_that_will_not_open_opens_no_window() {
    let dir = folder("refused");
    let mut contexts = Contexts::new(
        Some(dir.join("Profile")),
        Box::new(|_: &Path| Err("it is open in another browser".to_owned())),
    );
    let Err(why) = contexts.open_for(&WindowRequest::Profile("Work".to_owned()), &Kept::Nothing)
    else {
        panic!("opened");
    };
    assert!(why.contains("another browser"), "{why}");
    assert!(contexts.contexts().is_empty(), "nothing kept open");
}

#[test]
fn profiles_are_folders_made_and_listed_by_name() {
    let dir = folder("listed");
    let root = dir.join("Profiles");
    assert_eq!(profiles_in(Some(&root)), [DEFAULT_NAME]);
    make_profile(Some(&root), "Work").unwrap();
    make_profile(Some(&root), "Home").unwrap();
    assert!(root.join("Work").is_dir());
    assert!(make_profile(Some(&root), "Work").is_err(), "there already");
    assert!(make_profile(Some(&root), "a/b").is_err());
    assert!(make_profile(None, "Work").is_err(), "no place for them");
    // A folder of a name no profile may have is not one.
    std::fs::create_dir_all(root.join("not.one")).unwrap();
    std::fs::write(root.join("file"), b"x").unwrap();
    assert_eq!(profiles_in(Some(&root)), [DEFAULT_NAME, "Home", "Work"]);
    let _ = std::fs::remove_dir_all(&dir);
}
