//! Loads the real onboarding pages and drives them headlessly.
//!
//! `Sailfish.Silica` ships only in the SDK, so `tests/silica-stubs/`
//! declares just enough of each component for the page files to instantiate
//! under host Qt. The stubs imitate no layout or behaviour: this says what a
//! page *does* -- which shim methods it calls, where it navigates, how it
//! reacts -- not how it looks.
//!
//! Interaction goes through `objectName`, which is production-safe, rather
//! than test hooks in the pages.

// Qt harness: needs `unsafe` for `env::set_var` before Qt starts
// (`unused_unsafe` because it is only unsafe from edition 2024 on),
// `borrow_as_ptr` for the engine pointer, and `single_shot` with
// whole-second Durations.
#![allow(
    unsafe_code,
    unused_unsafe,
    clippy::borrow_as_ptr,
    clippy::disallowed_methods,
    clippy::expect_used,
    // qt_method! declarations must match the generated dispatcher's
    // by-value parameters; see postivene-shim/src/lib.rs.
    clippy::needless_pass_by_value
)]

use std::cell::RefCell;
use std::ffi::CString;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use postivene_shim::DeltaChatCore;
use qmetaobject::*;
use serde_json::Value;

mod common;

/// QML forbids capitalised property names, so a `.qml` stub cannot provide
/// `BusyIndicatorSize.Large`; a registered enum can.
#[derive(QEnum)]
#[repr(u8)]
enum BusyIndicatorSize {
    Small = 0,
    Medium = 1,
    Large = 2,
}

/// Same, for `TruncationMode.Fade`.
#[derive(QEnum)]
#[repr(u8)]
enum TruncationMode {
    Elide = 0,
    Fade = 1,
}

/// Records navigation instead of performing it, and models the resulting
/// page stack. A context property, as in the app: a QML object in the probe
/// would not be visible inside a separately loaded page.
///
/// Method names are camelCase because they stand in for Silica's own API,
/// which the pages call; `qmetaobject` exposes Rust identifiers verbatim.
#[allow(non_snake_case)]
#[derive(QObject, Default)]
struct PageStackProbe {
    base: qt_base_class!(trait QObject),
    /// `push:AddProfileDialog.qml|replaceAbove:ChatListPage.qml|...`
    log: qt_property!(QString; NOTIFY log_changed),
    log_changed: qt_signal!(),
    /// The stack as it now stands, bottom first, comma separated.
    stack: qt_property!(QString; NOTIFY log_changed),
    /// The properties handed to the most recent navigation, as JSON.
    last_properties: qt_property!(QString; NOTIFY log_changed),

    push: qt_method!(fn(&mut self, page: QString, properties: QVariantMap)),
    replace: qt_method!(fn(&mut self, page: QString, properties: QVariantMap)),
    /// Silica's `replaceAbove(target, page, properties)`. A `null` target
    /// replaces the whole stack.
    replaceAbove:
        qt_method!(fn(&mut self, target: QVariant, page: QString, properties: QVariantMap)),
    pop: qt_method!(fn(&mut self)),

    pages: Vec<String>,
}

#[allow(non_snake_case)]
impl PageStackProbe {
    fn record(&mut self, action: &str, page: &QString, properties: &QVariantMap) -> String {
        // Only the file name matters; the rest is a checkout path.
        let page = page.to_string();
        let name = page.rsplit('/').next().unwrap_or(&page).to_string();
        let current = self.log.to_string();
        self.log = format!("{current}{action}:{name}|").into();

        // `QVariantMap` in qttypes 0.2 can be queried but not iterated, so
        // these are the keys the pages pass.
        let mut rendered = Vec::new();
        for key in [
            "accountId",
            "chatId",
            "chatName",
            "displayName",
            "providerQr",
        ] {
            let value = properties.value(QString::from(key), QVariant::default());
            let text = i32::from_qvariant(value.clone())
                .map(|number| number.to_string())
                .or_else(|| QString::from_qvariant(value).map(|text| text.to_string()));
            if let Some(text) = text {
                rendered.push(format!("{key}={text}"));
            }
        }
        self.last_properties = rendered.join(",").into();
        name
    }

    fn publish(&mut self) {
        self.stack = self.pages.join(",").into();
        self.log_changed();
    }

    fn push(&mut self, page: QString, properties: QVariantMap) {
        let name = self.record("push", &page, &properties);
        self.pages.push(name);
        self.publish();
    }

    fn replace(&mut self, page: QString, properties: QVariantMap) {
        let name = self.record("replace", &page, &properties);
        self.pages.pop();
        self.pages.push(name);
        self.publish();
    }

    fn replaceAbove(&mut self, target: QVariant, page: QString, properties: QVariantMap) {
        let name = self.record("replaceAbove", &page, &properties);
        // Only the whole-stack form is used here; a non-null target would
        // need the page it names to decide where to cut.
        // QML's `null` arrives as an empty QVariant.
        let target_is_null =
            QString::from_qvariant(target).map_or(true, |value| value.to_string().is_empty());
        assert!(
            target_is_null,
            "replaceAbove with a non-null target is not modelled"
        );
        self.pages.clear();
        self.pages.push(name);
        self.publish();
    }

    fn pop(&mut self) {
        let current = self.log.to_string();
        self.log = format!("{current}pop|").into();
        self.pages.pop();
        self.publish();
    }
}

/// Owns a `Loader` and walks the loaded page's children by `objectName`.
const PROBE_QML: &str = r"
    import QtQuick 2.0
    Item {
        id: root
        Loader { id: loader }

        function load(url) {
            loader.source = url
            return loader.status === Loader.Ready ? 'ok' : 'load-failed'
        }
        // With initial properties, as JSON: what a dialog hands its
        // accept destination.
        function loadWith(url, json) {
            loader.setSource(url, JSON.parse(json))
            return loader.status === Loader.Ready ? 'ok' : 'load-failed'
        }
        // What Silica makes of a dialog's accept destination as soon as
        // the dialog is on screen, for the dialog to fill in on accept.
        QtObject {
            id: destination
            property string displayName
            property string providerQr
        }
        // Silica accepts a dialog on a tap of its header or a swipe;
        // the stub's accept() is the same thing.
        function accept() {
            loader.item.acceptDestinationInstance = destination
            loader.item.accept()
            return 'ok'
        }
        function handed() { return destination.displayName + ',' + destination.providerQr }
        // What Silica does when a page becomes the one on screen.
        function activate() { loader.item.status = 2; return 'ok' }
        function findIn(node, name) {
            if (!node) { return null }
            if (node.objectName === name) { return node }
            var kids = node.children
            for (var i = 0; kids && i < kids.length; i++) {
                var hit = findIn(kids[i], name)
                if (hit) { return hit }
            }
            // List delegates hang off the view's contentItem, not its
            // children.
            if (node.contentItem && node.contentItem !== node) {
                return findIn(node.contentItem, name)
            }
            return null
        }
        function click(name) {
            var item = findIn(loader.item, name)
            if (!item) { return 'missing:' + name }
            item.clicked()
            return 'ok'
        }
        function setText(name, value) {
            var item = findIn(loader.item, name)
            if (!item) { return 'missing:' + name }
            item.text = value
            return 'ok'
        }
        // What Silica's ComboBox does on a tap of one of its items.
        function pick(name, index) {
            var item = findIn(loader.item, name)
            if (!item) { return 'missing:' + name }
            item.currentIndex = parseInt(index)
            return 'ok'
        }
        function get(name, property) {
            var item = findIn(loader.item, name)
            if (!item) { return 'missing:' + name }
            return '' + item[property]
        }
        function pageProperty(property) {
            return loader.item ? '' + loader.item[property] : 'no-page'
        }
    }
";

fn qml_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml")
}

fn stubs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/silica-stubs")
}

fn page_url(name: &str) -> String {
    format!("file://{}", qml_dir().join("pages").join(name).display())
}

/// Each step and what it recorded. One step per tick: the shim answers
/// asynchronously and `single_shot` only handles whole seconds.
type Steps = Rc<RefCell<Vec<(String, String)>>>;

fn record(steps: &Steps, label: &str, value: QString) {
    steps
        .borrow_mut()
        .push((label.to_string(), value.to_string()));
}

fn value_of<'a>(steps: &'a [(String, String)], label: &str) -> &'a str {
    steps
        .iter()
        .find(|(name, _)| name == label)
        .map_or("<step did not run>", |(_, value)| value.as_str())
}

/// Temp directories and environment, returning the journal path.
fn prepare_environment() -> PathBuf {
    let temp = std::env::temp_dir().join(format!("postivene-qml-pages-{}", std::process::id()));
    let journal = common::fresh_journal(&temp);
    std::fs::create_dir_all(temp.join("accounts")).expect("create temp dirs");
    // SAFETY: single-threaded, and set before Qt starts and before the
    // server inherits them.
    unsafe {
        std::env::set_var("QT_QPA_PLATFORM", "offscreen");
        std::env::set_var("POSTIVENE_FAKE_JOURNAL", &journal);
        std::env::set_var("POSTIVENE_ACCOUNTS_DIR", temp.join("accounts"));
    }
    journal
}

// The engine, the QObject boxes and every step share one scope: all must
// outlive `exec()`. The assertions are in the helpers below.
#[allow(clippy::too_many_lines)]
#[test]
fn onboarding_pages_drive_the_core_and_navigate() {
    let journal = prepare_environment();

    let core_box = QObjectBox::new(DeltaChatCore::default());
    let stack_box = QObjectBox::new(PageStackProbe::default());

    let mut engine = QmlEngine::new();
    // Without this the page files do not parse their imports.
    engine.add_import_path(QString::from(stubs_dir().to_string_lossy().into_owned()));
    // The two enum namespaces the .qml stubs cannot express.
    let uri = CString::new("Sailfish.Silica").expect("static uri");
    qml_register_enum::<BusyIndicatorSize>(
        &uri,
        1,
        0,
        &CString::new("BusyIndicatorSize").expect("static name"),
    );
    qml_register_enum::<TruncationMode>(
        &uri,
        1,
        0,
        &CString::new("TruncationMode").expect("static name"),
    );
    engine.set_object_property("core".into(), core_box.pinned());
    engine.set_object_property("pageStack".into(), stack_box.pinned());
    engine.load_data(QByteArray::from(PROBE_QML));

    core_box
        .pinned()
        .borrow_mut()
        .start(QString::from(env!("CARGO_BIN_EXE_fake-core-server")));

    let steps: Steps = Rc::new(RefCell::new(Vec::new()));
    let engine_ptr = std::ptr::addr_of_mut!(engine);

    // SAFETY: these callbacks fire only while `exec()` is running on this
    // thread, and `engine` outlives it.
    macro_rules! call {
        ($name:expr $(, $arg:expr)*) => {{
            let result = unsafe {
                (*engine_ptr).invoke_method(
                    $name.into(),
                    &[$(QVariant::from(QString::from($arg))),*],
                )
            };
            QString::from_qvariant(result).unwrap_or_default()
        }};
    }

    // One step per tick; whole seconds only (clippy.toml).
    let s = steps.clone();
    single_shot(Duration::from_secs(1), move || {
        record(
            &s,
            "welcome-load",
            call!("load", page_url("WelcomePage.qml")),
        );
    });

    let s = steps.clone();
    single_shot(Duration::from_secs(2), move || {
        record(&s, "welcome-probing", call!("pageProperty", "probing"));
        record(&s, "welcome-click", call!("click", "setupButton"));
    });

    // Where that lands: the two ways into a profile. One asks where the
    // reader's existing profile is; the other goes on to the relay
    // dialog.
    let s = steps.clone();
    single_shot(Duration::from_secs(3), move || {
        record(
            &s,
            "start-load",
            call!("load", page_url("ProfileStartPage.qml")),
        );
        record(
            &s,
            "start-existing",
            call!("click", "existingProfileButton"),
        );
        record(&s, "start-create", call!("click", "createProfileButton"));
    });

    // The dialog: nothing to accept until there is a name; the first
    // relay unless another is picked or one is typed.
    let s = steps.clone();
    single_shot(Duration::from_secs(4), move || {
        record(
            &s,
            "dialog-load",
            call!("load", page_url("AddProfileDialog.qml")),
        );
        record(&s, "dialog-empty", call!("pageProperty", "canAccept"));
        record(
            &s,
            "dialog-index",
            call!("get", "relayCombo", "currentIndex"),
        );
        record(&s, "dialog-provider", call!("pageProperty", "providerQr"));
        record(&s, "dialog-name", call!("setText", "nameField", " Ada "));
        record(&s, "dialog-named", call!("pageProperty", "canAccept"));
        record(&s, "dialog-pick", call!("pick", "relayCombo", "1"));
        record(&s, "dialog-picked", call!("pageProperty", "providerQr"));
        record(
            &s,
            "dialog-custom",
            call!("setText", "customField", " chat.example.org "),
        );
        record(&s, "dialog-typed", call!("pageProperty", "providerQr"));
        record(&s, "dialog-list-off", call!("get", "relayCombo", "enabled"));
        record(&s, "dialog-uncustom", call!("setText", "customField", ""));
        record(&s, "dialog-unpick", call!("pick", "relayCombo", "0"));
        record(&s, "dialog-accept", call!("accept"));
        record(&s, "dialog-handed", call!("handed"));
    });

    // The setup page, made before it is on screen the way Silica makes
    // a dialog's destination, with what the dialog hands it: nothing is
    // asked of the core until the page is the one on screen.
    let s = steps.clone();
    single_shot(Duration::from_secs(5), move || {
        record(
            &s,
            "setup-load",
            call!(
                "loadWith",
                page_url("ProfileSetupPage.qml"),
                r#"{"displayName":"Ada","providerQr":"dcaccount:nine.testrun.org","status":0}"#
            ),
        );
    });

    let s = steps.clone();
    let journal_early = journal.clone();
    single_shot(Duration::from_secs(6), move || {
        let asked_early = common::records(&journal_early).iter().any(|call| {
            call.get("method").and_then(Value::as_str) == Some("add_transport_from_qr")
        });
        record(&s, "setup-early", QString::from(asked_early.to_string()));
        record(&s, "setup-activate", call!("activate"));
    });

    let s = steps.clone();
    single_shot(Duration::from_secs(8), move || {
        record(&s, "setup-permille", call!("pageProperty", "permille"));
        record(&s, "setup-error", call!("pageProperty", "errorMessage"));
        record(&s, "setup-busy", call!("pageProperty", "busy"));
    });

    single_shot(Duration::from_secs(11), move || unsafe {
        (*engine_ptr).quit();
    });

    engine.exec();

    let steps = steps.borrow();
    let navigation = stack_box.pinned().borrow().log.to_string();
    let properties = stack_box.pinned().borrow().last_properties.to_string();
    let context = format!("steps: {steps:?}\nnavigation: {navigation}");

    assert_pages_loaded(&steps, &context);
    assert_welcome_and_navigation(&steps, &navigation, &properties, &context);
    assert_dialog(&steps, &context);
    assert_profile_creation(&steps, &context);
    assert_wire_calls(&journal);
}

/// Every page file must instantiate; nothing below means anything if not.
fn assert_pages_loaded(steps: &[(String, String)], context: &str) {
    for step in ["welcome-load", "start-load", "dialog-load", "setup-load"] {
        assert_eq!(value_of(steps, step), "ok", "{step} failed. {context}");
    }
}

/// With no configured account the welcome page stops probing and shows its
/// buttons; the setup one starts the profile path, and the page it opens
/// leads both ways from there: to where an existing profile is taken over
/// from, and on to the relay dialog.
fn assert_welcome_and_navigation(
    steps: &[(String, String)],
    navigation: &str,
    properties: &str,
    context: &str,
) {
    assert_eq!(
        value_of(steps, "welcome-probing"),
        "false",
        "the welcome page never stopped probing. {context}"
    );
    assert_eq!(value_of(steps, "welcome-click"), "ok", "{context}");
    assert!(
        navigation.contains("push:ProfileStartPage.qml"),
        "Set up my profile did not start the profile path. {context}"
    );
    assert!(
        navigation.contains("push:ExistingProfilePage.qml"),
        "I already have a profile did not ask where it is. {context}"
    );
    assert!(
        navigation.contains("push:AddProfileDialog.qml"),
        "Create a profile did not open the dialog. {context}"
    );
    assert!(
        navigation.contains("replaceAbove:ChatListPage.qml"),
        "a created profile did not land in the chat list. {context}"
    );
    assert!(
        properties.contains("accountId="),
        "the chat list was opened without an account id: {properties}"
    );
}

/// The dialog: no accepting without a name, the first relay by default,
/// a picked or typed one otherwise, and what was chosen is what the
/// setup page is handed.
fn assert_dialog(steps: &[(String, String)], context: &str) {
    for step in [
        "dialog-name",
        "dialog-pick",
        "dialog-custom",
        "dialog-uncustom",
        "dialog-unpick",
        "dialog-accept",
    ] {
        assert_eq!(value_of(steps, step), "ok", "{step} failed. {context}");
    }
    assert_eq!(
        value_of(steps, "dialog-empty"),
        "false",
        "the dialog can be accepted without a name. {context}"
    );
    // One of the list, at random: not the first every time.
    let index: usize = value_of(steps, "dialog-index")
        .parse()
        .unwrap_or(usize::MAX);
    assert!(
        index < 26,
        "the relay picked on arrival is not one of the list. {context}"
    );
    let provider = value_of(steps, "dialog-provider");
    assert!(
        provider.starts_with("dcaccount:") && provider.contains('.'),
        "the relay picked on arrival is not a dcaccount: payload with a domain. {context}"
    );
    assert_eq!(
        value_of(steps, "dialog-named"),
        "true",
        "a name does not make the dialog acceptable. {context}"
    );
    assert_eq!(
        value_of(steps, "dialog-picked"),
        "dcaccount:mehl.cloud",
        "picking the second relay did not change the payload. {context}"
    );
    assert_eq!(
        value_of(steps, "dialog-typed"),
        "dcaccount:chat.example.org",
        "a typed server does not take over from the list, trimmed. {context}"
    );
    assert_eq!(
        value_of(steps, "dialog-list-off"),
        "false",
        "the list is still offered while a server is typed. {context}"
    );
    assert_eq!(
        value_of(steps, "dialog-handed"),
        "Ada,dcaccount:nine.testrun.org",
        "the setup page was not handed the trimmed name and the relay. {context}"
    );
}

/// The setup page asks the core once it is on screen, not when it is
/// made, and progress reaches it.
fn assert_profile_creation(steps: &[(String, String)], context: &str) {
    assert_eq!(
        value_of(steps, "setup-early"),
        "false",
        "the setup page asked the core before it was on screen, which is \
         when Silica makes it, with nothing filled in yet. {context}"
    );
    assert_eq!(value_of(steps, "setup-activate"), "ok", "{context}");
    assert_eq!(
        value_of(steps, "setup-permille"),
        "1000",
        "ConfigureProgress never reached the page. {context}"
    );
    assert_eq!(
        value_of(steps, "setup-error"),
        "",
        "a successful profile left an error on the page. {context}"
    );
    assert_eq!(
        value_of(steps, "setup-busy"),
        "false",
        "the page is still busy after the core answered. {context}"
    );
}

/// A tap produced the right calls on the wire.
fn assert_wire_calls(journal: &std::path::Path) {
    let calls = common::records(journal);
    let method_names: Vec<&str> = calls
        .iter()
        .filter_map(|call| call.get("method").and_then(Value::as_str))
        .collect();
    assert!(
        method_names.contains(&"add_transport_from_qr"),
        "tapping create did not reach add_transport_from_qr: {method_names:?}"
    );
    assert!(
        !method_names.contains(&"configure"),
        "the pages called the deprecated `configure`: {method_names:?}"
    );
    let transport = calls
        .iter()
        .find(|call| call.get("method").and_then(Value::as_str) == Some("add_transport_from_qr"));
    assert_eq!(
        transport
            .and_then(|call| call.pointer("/params/1"))
            .and_then(Value::as_str),
        Some("dcaccount:nine.testrun.org"),
        "the relay the dialog chose did not reach the core"
    );
    let display_name = calls.iter().find(|call| {
        call.get("method").and_then(Value::as_str) == Some("set_config")
            && call.pointer("/params/1").and_then(Value::as_str) == Some("displayname")
    });
    assert_eq!(
        display_name
            .and_then(|call| call.pointer("/params/2"))
            .and_then(Value::as_str),
        Some("Ada"),
        "the name typed into the page did not reach the core"
    );
}
