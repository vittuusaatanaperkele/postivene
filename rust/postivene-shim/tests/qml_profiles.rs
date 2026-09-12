//! Switching profile must leave exactly one chat list on the page stack.
//!
//! `replace` swaps out only the profiles page, so the chat list for the
//! account just left stays underneath it -- one swipe back into the
//! account the reader thought they had left. `replaceAbove(null, ...)`
//! replaces the whole stack, which is what the onboarding pages already do
//! when they hand over to the chat list.

// Qt harness: see qml_pages.rs.
#![allow(
    unsafe_code,
    unused_unsafe,
    non_snake_case,
    clippy::borrow_as_ptr,
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::needless_pass_by_value
)]

use std::time::Duration;

use postivene_shim::DeltaChatCore;
use qmetaobject::*;

mod common;

/// Records navigation instead of performing it, and models the resulting
/// stack -- the depth is the whole point here.
#[derive(QObject, Default)]
struct StackProbe {
    base: qt_base_class!(trait QObject),
    /// The page names on the stack, innermost first.
    stack: qt_property!(QString; NOTIFY stack_changed),
    stack_changed: qt_signal!(),

    push: qt_method!(fn(&mut self, page: QString, properties: QVariantMap)),
    replace: qt_method!(fn(&mut self, page: QString, properties: QVariantMap)),
    replaceAbove:
        qt_method!(fn(&mut self, target: QVariant, page: QString, properties: QVariantMap)),
    pop: qt_method!(fn(&mut self)),

    pages: Vec<String>,
}

impl StackProbe {
    fn name_of(page: &QString) -> String {
        let page = page.to_string();
        page.rsplit('/').next().unwrap_or(&page).to_string()
    }

    fn publish(&mut self) {
        self.stack = self.pages.join(",").into();
        self.stack_changed();
    }

    fn push(&mut self, page: QString, _properties: QVariantMap) {
        let name = Self::name_of(&page);
        self.pages.push(name);
        self.publish();
    }

    fn replace(&mut self, page: QString, _properties: QVariantMap) {
        let name = Self::name_of(&page);
        self.pages.pop();
        self.pages.push(name);
        self.publish();
    }

    fn replaceAbove(&mut self, _target: QVariant, page: QString, _properties: QVariantMap) {
        let name = Self::name_of(&page);
        self.pages.clear();
        self.pages.push(name);
        self.publish();
    }

    fn pop(&mut self) {
        self.pages.pop();
        self.publish();
    }
}

const PROBE_QML: &str = r"
    import QtQuick 2.0
    Item {
        Loader { id: loader }
        // The stack as it really is when the profiles page is open: the
        // chat list for the current account, then the profiles page.
        function seed() {
            // Two accounts, so there is one to switch *to*.
            core.add_account()
            core.add_account()
            pageStack.push('qrc:/ChatListPage.qml', {})
            pageStack.push('qrc:/ProfilesPage.qml', {})
        }
        // add_account only signals; the list is refetched on demand.
        function refresh() { core.refresh_accounts() }
        // Counted off the page's own list, since the model exposes no
        // count of its own.
        // Rows are named per profile; account 2 is the one left once
        // the first row's profile has gone.
        function accountsLeft() {
            var view = findIn(loader.item, 'profileRow2')
            return view ? 'has-rows' : 'no-rows'
        }
        function load(url, currentAccountId) {
            loader.setSource('', {})
            loader.setSource(url, { currentAccountId: currentAccountId })
            return loader.status === Loader.Ready ? 'ok' : 'load-failed'
        }
        function findIn(node, name) {
            if (!node) { return null }
            if (node.objectName === name) { return node }
            var kids = node.data !== undefined ? node.data : node.children
            for (var i = 0; kids && i < kids.length; i++) {
                var hit = findIn(kids[i], name)
                if (hit) { return hit }
            }
            // A row's ContextMenu is held in a property, so it is in
            // neither list even though it is instantiated.
            if (node.menu) {
                var inMenu = findIn(node.menu, name)
                if (inMenu) { return inMenu }
            }
            if (node.contentItem && node.contentItem !== node) {
                return findIn(node.contentItem, name)
            }
            return null
        }
        function deleteFirstRow() {
            var item = findIn(loader.item, 'deleteProfileItem')
            if (!item) { return 'missing:deleteProfileItem' }
            item.clicked()
            return 'ok'
        }
        function tapFirstRow() {
            var row = findIn(loader.item, 'profileRow1')
            if (!row) { return 'missing:profileRow1' }
            row.clicked()
            return 'ok'
        }
        // The plus under the last row: the way to another profile.
        function addProfile() {
            var item = findIn(loader.item, 'addProfileButton')
            if (!item) { return 'missing:addProfileButton' }
            item.clicked()
            return 'ok'
        }
        // And the plus under that one: a profile this reader already has
        // on another device.
        function addSecondDevice() {
            var item = findIn(loader.item, 'secondDeviceButton')
            if (!item) { return 'missing:secondDeviceButton' }
            item.clicked()
            return 'ok'
        }
    }
";

#[test]
#[allow(clippy::too_many_lines)]
fn switching_profile_leaves_one_chat_list_on_the_stack() {
    let temp = std::env::temp_dir().join(format!("postivene-qml-accounts-{}", std::process::id()));
    std::fs::create_dir_all(temp.join("accounts")).expect("create temp dirs");

    // SAFETY: single-threaded test binary; set before Qt starts.
    unsafe {
        std::env::set_var("QT_QPA_PLATFORM", "offscreen");
        std::env::set_var("POSTIVENE_ACCOUNTS_DIR", temp.join("accounts"));
    }

    postivene_shim::register_qml_types();

    let core_box = QObjectBox::new(DeltaChatCore::default());
    let stack_box = QObjectBox::new(StackProbe::default());
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
    let stack_ptr = std::ptr::addr_of!(stack_box);
    let mut steps: Vec<(&str, String)> = Vec::new();
    let steps_ptr: *mut Vec<(&str, String)> = std::ptr::addr_of_mut!(steps);

    macro_rules! call {
        ($name:expr $(, $arg:expr)*) => {{
            let result = (*engine_ptr).invoke_method(
                $name.into(),
                &[$(QVariant::from($arg)),*],
            );
            QString::from_qvariant(result)
                .map(|value| value.to_string())
                .unwrap_or_default()
        }};
    }

    single_shot(Duration::from_secs(1), move || unsafe {
        call!("seed");
    });

    single_shot(Duration::from_secs(3), move || unsafe {
        call!("refresh");
    });

    single_shot(Duration::from_secs(5), move || unsafe {
        // The *second* account is current, so the first row -- the one
        // findIn reaches -- is one to switch to rather than the one
        // already open.
        (*steps_ptr).push((
            "load",
            call!(
                "load",
                QString::from(common::page_url("ProfilesPage.qml")),
                2
            ),
        ));
    });

    single_shot(Duration::from_secs(7), move || unsafe {
        (*steps_ptr).push(("seeded", (*stack_ptr).pinned().borrow().stack.to_string()));
        // Another profile is made from the plus under the list, on top
        // of this page.
        (*steps_ptr).push(("add", call!("addProfile")));
        (*steps_ptr).push(("added", (*stack_ptr).pinned().borrow().stack.to_string()));
        // And the plus under it, for a profile that exists already on
        // the phone in the reader's other hand.
        (*steps_ptr).push(("second", call!("addSecondDevice")));
        (*steps_ptr).push((
            "took-over",
            (*stack_ptr).pinned().borrow().stack.to_string(),
        ));
        (*steps_ptr).push(("tap", call!("tapFirstRow")));
    });

    single_shot(Duration::from_secs(9), move || unsafe {
        (*steps_ptr).push(("after", (*stack_ptr).pinned().borrow().stack.to_string()));
        // Reload the profiles page and delete from it. Two profiles
        // remain, so this is not the last-one case.
        call!(
            "load",
            QString::from(common::page_url("ProfilesPage.qml")),
            2
        );
    });

    single_shot(Duration::from_secs(11), move || unsafe {
        (*steps_ptr).push(("delete", call!("deleteFirstRow")));
    });

    single_shot(Duration::from_secs(14), move || unsafe {
        (*steps_ptr).push(("deleted", call!("accountsLeft")));
        (*engine_ptr).quit();
    });

    engine.exec();

    let value = |label: &str| {
        steps
            .iter()
            .find(|(name, _)| *name == label)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    let context = format!("steps: {steps:?}");

    assert_eq!(
        value("load"),
        "ok",
        "the profiles page did not load. {context}"
    );
    assert_eq!(
        value("seeded"),
        "ChatListPage.qml,ProfilesPage.qml",
        "the stack was not seeded as the app really has it. {context}"
    );
    assert_eq!(
        value("add"),
        "ok",
        "no plus under the profile list, so no way to add a profile. {context}"
    );
    assert!(
        value("added").ends_with(",AddProfileDialog.qml"),
        "adding a profile did not open the add-profile dialog on top: \
         {}. {context}",
        value("added")
    );
    assert_eq!(
        value("second"),
        "ok",
        "no second plus under the profile list, so a profile on another \
         device cannot be taken over from here. {context}"
    );
    assert!(
        value("took-over").ends_with(",RestoreProfilePage.qml"),
        "the second plus did not open the take-over page on top: {}. \
         {context}",
        value("took-over")
    );
    // Also the guard that the list really had rows: with no accounts there
    // is no row to tap and nothing here would be under test.
    assert_eq!(
        value("tap"),
        "ok",
        "no account row was reachable, so the list was empty and this test \
         proves nothing. {context}"
    );
    assert_eq!(
        value("delete"),
        "ok",
        "no delete entry on a profile row, so a profile cannot be removed. \
         {context}"
    );
    assert_eq!(
        value("after"),
        "ChatListPage.qml",
        "switching account left more than the new chat list on the stack, so \
         the account just left is one swipe away. {context}"
    );
}
