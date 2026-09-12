//! Load the catalog for the reader's language.
//!
//! The one `unsafe` in the tree, and the reason for the C++ toolchain in
//! this crate's build: `QTranslator` is not bound by qmetaobject, and the
//! only way to install one is to call it. The exception is kept to this
//! module, and to a block short enough to be checked by reading.
//!
//! Silica's own strings come translated with the system; these are the
//! app's, from `translations/postivene-<lang>.qm`, which the RPM builds
//! from the `.ts` catalogs with `lrelease`. Which file is loaded follows
//! the reader's Language setting, through the locale the system starts
//! the app under: `postivene-de_DE.qm` if there is one, else
//! `postivene-de.qm`, and nothing at all for a language the app has no
//! catalog for -- which leaves every string as it was written.

// `cpp!` expands to a call across the FFI boundary, which is `unsafe` by
// construction. Scoped to this file: the workspace denies it everywhere
// else, and docs/PROJECT.md says why this one is allowed.
#![allow(unsafe_code)]

use cpp::cpp;
use qmetaobject::QString;

cpp! {{
    #include <QtCore/QCoreApplication>
    #include <QtCore/QLocale>
    #include <QtCore/QString>
    #include <QtCore/QTranslator>
}}

/// The file name every catalog starts with.
const CATALOG: &str = "postivene";

/// Install the catalog for `locale` from `dir`, and say whether one was
/// found. An empty `locale` is the system's, which is the one the reader
/// chose in Settings; anything else is a name such as `de_DE`, for tests.
///
/// Must run after the application object exists -- after the view is
/// made, in `main` -- and before any QML is loaded: Qt 5.6 has no way to
/// retranslate a page that is already built.
pub fn install(dir: &str, locale: &str) -> bool {
    let dir = QString::from(dir);
    let locale = QString::from(locale);
    let name = QString::from(CATALOG);
    // SAFETY: the three arguments are QStrings owned by this frame, read
    // by value on the C++ side. The translator is parented to the
    // application object, which owns it from here on, and nothing keeps a
    // pointer to it in Rust.
    cpp!(unsafe [dir as "QString", locale as "QString", name as "QString"] -> bool as "bool" {
        QCoreApplication *app = QCoreApplication::instance();
        if (!app) {
            return false;
        }
        QTranslator *translator = new QTranslator(app);
        QLocale which = locale.isEmpty() ? QLocale() : QLocale(locale);
        // "-" is what sits between the name and the locale in the file
        // name; load() tries the locale's languages from the most specific
        // form down, so postivene-pt_BR.qm wins over postivene-pt.qm where
        // both exist.
        if (!translator->load(which, name, QStringLiteral("-"), dir)) {
            delete translator;
            return false;
        }
        return app->installTranslator(translator);
    })
}

// Qt harness, as the shim's tests are: `unused_unsafe` because
// `env::set_var` is only unsafe from edition 2024 on, `borrow_as_ptr` for
// the engine pointer, `disallowed_methods` for `single_shot`'s Duration,
// and `expect_used` for a test's own setup.
#[cfg(test)]
#[allow(
    unused_unsafe,
    clippy::borrow_as_ptr,
    clippy::disallowed_methods,
    clippy::expect_used
)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use qmetaobject::*;

    use super::install;

    /// The catalogs in the source tree.
    fn translations_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../translations")
    }

    /// Compile one catalog into `out`, with the `lrelease` the packaging
    /// job installs. None if there is no `lrelease` on this machine,
    /// which is a skip locally and a failure under CI.
    fn compile(language: &str, out: &std::path::Path) -> Option<()> {
        let lrelease = ["lrelease", "lrelease-qt5"].into_iter().find(|tool| {
            std::process::Command::new(tool)
                .arg("-version")
                .output()
                .is_ok()
        });
        let Some(lrelease) = lrelease else {
            assert!(
                std::env::var_os("CI").is_none(),
                "lrelease is not installed, so the catalogs cannot be compiled; \
                 install qttools5-dev-tools"
            );
            eprintln!("skipping: lrelease not found (install qttools5-dev-tools)");
            return None;
        };
        let source = translations_dir().join(format!("postivene-{language}.ts"));
        let target = out.join(format!("postivene-{language}.qm"));
        let status = std::process::Command::new(lrelease)
            .arg(&source)
            .arg("-qm")
            .arg(&target)
            .status()
            .expect("run lrelease");
        assert!(status.success(), "lrelease failed on {}", source.display());
        Some(())
    }

    // One test: the application object can be made once per process, and
    // Qt 5's translators apply to what is loaded after them.
    #[test]
    fn the_readers_language_is_loaded_and_its_plurals_counted() {
        let out =
            std::env::temp_dir().join(format!("postivene-translations-{}", std::process::id()));
        std::fs::create_dir_all(&out).expect("create the output dir");
        if compile("de", &out).is_none() {
            return;
        }

        // SAFETY: single-threaded test binary; set before Qt starts.
        unsafe {
            std::env::set_var("QT_QPA_PLATFORM", "offscreen");
        }
        // The application object, which the translator needs to exist.
        let mut engine = QmlEngine::new();
        let dir = out.to_string_lossy().into_owned();

        assert!(
            !install(&dir, "xx_YY"),
            "a language with no catalog reports one loaded, so a reader of \
             it would get the wrong language rather than the source text"
        );
        assert!(
            install(&dir, "de_DE"),
            "the German catalog did not load from {dir}"
        );

        // Asked with the context each string is under in its page, as a
        // qsTr() in that file would be.
        engine.load_data(QByteArray::from(
            r"
            import QtQuick 2.0
            Item {
                function plain() { return qsTranslate('WelcomePage', 'Set up my profile') }
                function one() { return qsTranslate('GroupPage', '%n member(s)', '', 1) }
                function many() { return qsTranslate('GroupPage', '%n member(s)', '', 3) }
                function untranslated() { return qsTranslate('Nowhere', 'not in any catalog') }
            }
            ",
        ));
        let engine_ptr = std::ptr::addr_of_mut!(engine);
        let mut seen: Vec<(&str, String)> = Vec::new();
        let seen_ptr: *mut Vec<(&str, String)> = std::ptr::addr_of_mut!(seen);
        single_shot(Duration::from_millis(200), move || unsafe {
            for name in ["plain", "one", "many", "untranslated"] {
                let value = (*engine_ptr).invoke_method(name.into(), &[]);
                let text = QString::from_qvariant(value)
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                (*seen_ptr).push((name, text));
            }
            (*engine_ptr).quit();
        });
        engine.exec();

        let value = |label: &str| {
            seen.iter()
                .find(|(name, _)| *name == label)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        assert_eq!(value("plain"), "Profil einrichten", "seen: {seen:?}");
        assert_eq!(value("one"), "1 Mitglied", "seen: {seen:?}");
        assert_eq!(value("many"), "3 Mitglieder", "seen: {seen:?}");
        assert_eq!(
            value("untranslated"),
            "not in any catalog",
            "a string with no translation does not fall back to itself. seen: {seen:?}"
        );
        let _ = std::fs::remove_dir_all(&out);
    }
}
