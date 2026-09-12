//! The pages a reader with a profile already meets.
//!
//! Two of them: the question of where that profile is
//! (`ExistingProfilePage.qml`), and the page that brings it over
//! (`RestoreProfilePage.qml`), which serves both answers. What is
//! checked here is what the pages do -- which way each button leads,
//! what the file half offers and the device half does not, that a
//! transfer takes the reader to the profile it brought over, and that a
//! code the shim refuses is put into words rather than shown as a
//! protocol complaint. The transfer itself is `restore.rs`.

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
use std::rc::Rc;
use std::time::Duration;

use postivene_shim::DeltaChatCore;
use qmetaobject::*;

mod common;

/// Records navigation instead of performing it, with the one property
/// that matters here: which way the take-over page was opened.
#[allow(non_snake_case)]
#[derive(QObject, Default)]
struct PageStackProbe {
    base: qt_base_class!(trait QObject),
    /// `push:RestoreProfilePage.qml(device)|replaceAbove:ChatListPage.qml|`
    log: qt_property!(QString; NOTIFY log_changed),
    log_changed: qt_signal!(),

    push: qt_method!(fn(&mut self, page: QString, properties: QVariantMap)),
    replaceAbove:
        qt_method!(fn(&mut self, target: QVariant, page: QString, properties: QVariantMap)),
    pop: qt_method!(fn(&mut self)),
}

#[allow(non_snake_case)]
impl PageStackProbe {
    fn record(&mut self, entry: &str) {
        let current = self.log.to_string();
        self.log = format!("{current}{entry}|").into();
        self.log_changed();
    }

    fn name(page: &QString) -> String {
        let page = page.to_string();
        page.rsplit('/').next().unwrap_or(&page).to_string()
    }

    fn push(&mut self, page: QString, properties: QVariantMap) {
        // `QVariantMap` in qttypes 0.2 can be queried but not iterated.
        let from =
            QString::from_qvariant(properties.value(QString::from("from"), QVariant::default()))
                .map(|value| value.to_string())
                .unwrap_or_default();
        let name = Self::name(&page);
        if from.is_empty() {
            self.record(&format!("push:{name}"));
        } else {
            self.record(&format!("push:{name}({from})"));
        }
    }

    fn replaceAbove(&mut self, _target: QVariant, page: QString, _properties: QVariantMap) {
        self.record(&format!("replaceAbove:{}", Self::name(&page)));
    }

    fn pop(&mut self) {
        self.record("pop");
    }
}

/// Owns a `Loader` and works the loaded page by `objectName`.
const PROBE_QML: &str = r"
    import QtQuick 2.0
    Item {
        Loader { id: loader }

        function loadWith(url, json) {
            loader.setSource('', {})
            loader.setSource(url, JSON.parse(json))
            return loader.status === Loader.Ready ? 'ok' : 'load-failed'
        }
        // What Silica does when a page becomes the one on screen.
        function activate() { loader.item.status = 2; return 'ok' }
        function findIn(node, name) {
            if (!node) { return null }
            if (node.objectName === name) { return node }
            var kids = node.data !== undefined ? node.data : node.children
            for (var i = 0; kids && i < kids.length; i++) {
                var hit = findIn(kids[i], name)
                if (hit) { return hit }
            }
            return null
        }
        function click(name) {
            var item = findIn(loader.item, name)
            if (!item) { return 'missing:' + name }
            item.clicked()
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
        // What the camera or the file browser hands the page.
        function begin(text) {
            if (!loader.item) { return 'no-page' }
            loader.item.begin(text)
            return 'ok'
        }
    }
";

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

// The engine, the QObject boxes and every step share one scope: all of
// them have to outlive `exec()`. The assertions are in the helper below.
#[allow(clippy::too_many_lines)]
#[test]
fn a_profile_that_exists_already_is_asked_after_and_brought_over() {
    let temp = std::env::temp_dir().join(format!("postivene-qml-restore-{}", std::process::id()));
    let journal = common::fresh_journal(&temp);
    std::fs::create_dir_all(temp.join("accounts")).expect("create temp dirs");
    // SAFETY: single-threaded, and set before Qt starts and before the
    // server inherits them.
    unsafe {
        std::env::set_var("QT_QPA_PLATFORM", "offscreen");
        std::env::set_var("POSTIVENE_FAKE_JOURNAL", &journal);
        std::env::set_var("POSTIVENE_ACCOUNTS_DIR", temp.join("accounts"));
    }

    let core_box = QObjectBox::new(DeltaChatCore::default());
    let stack_box = QObjectBox::new(PageStackProbe::default());

    let mut engine = QmlEngine::new();
    engine.add_import_path(QString::from(
        common::stubs_dir().to_string_lossy().into_owned(),
    ));
    engine.set_object_property("core".into(), core_box.pinned());
    engine.set_object_property("pageStack".into(), stack_box.pinned());
    engine.load_data(QByteArray::from(PROBE_QML));

    core_box
        .pinned()
        .borrow_mut()
        .start(QString::from(env!("CARGO_BIN_EXE_fake-core-server")));

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

    let steps: Steps = Rc::new(RefCell::new(Vec::new()));

    // 1s: the question, and both answers.
    let s = steps.clone();
    single_shot(Duration::from_secs(1), move || {
        record(
            &s,
            "ask-load",
            call!(
                "loadWith",
                common::page_url("ExistingProfilePage.qml"),
                "{}"
            ),
        );
        record(&s, "ask-device", call!("click", "secondDeviceButton"));
        record(&s, "ask-file", call!("click", "backupFileButton"));
    });

    // 2s: the file half. The browser is offered, the camera is not, and
    // choosing opens the browser.
    let s = steps.clone();
    single_shot(Duration::from_secs(2), move || {
        record(
            &s,
            "file-load",
            call!(
                "loadWith",
                common::page_url("RestoreProfilePage.qml"),
                r#"{"from":"file","status":2}"#
            ),
        );
        record(
            &s,
            "file-button",
            call!("get", "chooseFileButton", "visible"),
        );
        record(&s, "file-scanner", call!("get", "scanArea", "visible"));
        record(&s, "file-choose", call!("click", "chooseFileButton"));
        // And what the browser reports back: the import starts.
        record(&s, "file-begin", call!("begin", "/tmp/holiday-backup.tar"));
        record(&s, "file-busy", call!("pageProperty", "busy"));
    });

    // 4s: the core has answered, and the page is on the profile it
    // brought over.
    let s = steps.clone();
    single_shot(Duration::from_secs(4), move || {
        record(&s, "file-done", call!("pageProperty", "busy"));
    });

    // 5s: the device half, handed a code from a newer Delta Chat than
    // this core can read.
    let s = steps.clone();
    single_shot(Duration::from_secs(5), move || {
        record(
            &s,
            "device-load",
            call!(
                "loadWith",
                common::page_url("RestoreProfilePage.qml"),
                r#"{"from":"device","status":2}"#
            ),
        );
        record(
            &s,
            "device-button",
            call!("get", "chooseFileButton", "visible"),
        );
        record(
            &s,
            "device-begin",
            call!("begin", "DCBACKUP2:toonew.example"),
        );
    });

    let s = steps.clone();
    single_shot(Duration::from_secs(7), move || {
        record(&s, "device-busy", call!("pageProperty", "busy"));
        record(&s, "device-said", call!("pageProperty", "errorMessage"));
    });

    single_shot(Duration::from_secs(8), move || unsafe {
        (*engine_ptr).quit();
    });

    engine.exec();

    let navigation = stack_box.pinned().borrow().log.to_string();
    assert_pages(&steps.borrow(), &navigation);
}

/// The question leads both ways; the file half offers the browser and
/// not the camera, and what it chooses is imported; the profile that
/// arrives is what the app lands on; and a code the shim refuses is
/// shown as a sentence rather than left to the core's words.
fn assert_pages(steps: &[(String, String)], navigation: &str) {
    let context = format!("steps: {steps:?}\nnavigation: {navigation}");

    assert_eq!(
        value_of(steps, "ask-load"),
        "ok",
        "the question did not load. {context}"
    );
    assert!(
        navigation.contains("push:RestoreProfilePage.qml(device)"),
        "the second-device answer did not open the take-over page on the \
         device half. {context}"
    );
    assert!(
        navigation.contains("push:RestoreProfilePage.qml(file)"),
        "the backup answer did not open the take-over page on the file \
         half. {context}"
    );

    assert_eq!(
        value_of(steps, "file-load"),
        "ok",
        "the take-over page did not load. {context}"
    );
    assert_eq!(
        value_of(steps, "file-button"),
        "true",
        "the file half does not offer the file browser. {context}"
    );
    assert_eq!(
        value_of(steps, "file-scanner"),
        "false",
        "the file half put the camera up anyway. {context}"
    );
    assert!(
        navigation.contains("push:BackupFilePage.qml"),
        "choosing a backup did not open the file browser. {context}"
    );
    assert_eq!(
        value_of(steps, "file-busy"),
        "true",
        "the import did not start on the file that was chosen. {context}"
    );
    assert_eq!(
        value_of(steps, "file-done"),
        "false",
        "the page is still transferring after the core has answered. \
         {context}"
    );
    assert!(
        navigation.contains("replaceAbove:ChatListPage.qml"),
        "the profile that arrived is not what the app landed on. {context}"
    );

    assert_eq!(
        value_of(steps, "device-button"),
        "false",
        "the device half offers the file browser as well. {context}"
    );
    assert_eq!(
        value_of(steps, "device-begin"),
        "ok",
        "the code from the camera was not acted on. {context}"
    );
    assert_eq!(
        value_of(steps, "device-busy"),
        "false",
        "the page waits on a code the shim already refused. {context}"
    );
    let said = value_of(steps, "device-said");
    assert!(
        said.contains("newer"),
        "the refusal was not put into words for the reader: {said:?}. \
         {context}"
    );
}
