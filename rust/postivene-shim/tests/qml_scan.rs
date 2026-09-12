//! Scanning a QR code: the view runs the camera only while it is the one
//! on screen, a frame with a code in it comes back as the code's text,
//! and the view then stays for its host to take down.
//!
//! The frame is made here rather than by a camera: the test's own
//! `QrCode` encodes an invite, the test draws those modules into the PGM
//! Qt would have written, and the view's `QrScanner` reads it back
//! through the same call the viewfinder timer makes. That is the whole
//! shim path under the Qt event loop, less the optics.
//!
//! The other way in is typed: the button under the viewfinder opens a
//! panel over it, with the clipboard's contents in it when they look
//! like an invite, and what is entered there is handed back the way a
//! scanned code is.

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

use std::path::Path;
use std::time::Duration;

use qmetaobject::*;

mod common;

const INVITE: &str = "https://i.delta.chat/#0123456789ABCDEF&a=them%40example.org&n=Them";

const PROBE_QML: &str = r"
    import QtQuick 2.0
    import Sailfish.Silica 1.0
    import Postivene 1.0
    Item {
        property string heard: ''
        Loader { id: loader }
        // What the page draws for the test to photograph.
        QrCode { id: code }
        function encode(text) {
            code.text = text
            return code.size + ':' + code.modules
        }
        function load(url) {
            loader.setSource(url, {})
            if (loader.status !== Loader.Ready) { return 'load-failed' }
            loader.item.scanned.connect(function(text) { heard = text })
            return 'ok'
        }
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
        function get(name, property) {
            var item = findIn(loader.item, name)
            if (!item) { return 'missing:' + name }
            return '' + item[property]
        }
        // What the host says when the view is the one on screen.
        function setActive(active) { loader.item.active = active; return 'ok' }
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
        function setClipboard(text) { Clipboard.text = text; return 'ok' }
        // The Loader sizes what it holds, so this is the viewfinder's
        // own size too.
        function sizeView(width, height) {
            loader.width = parseInt(width, 10)
            loader.height = parseInt(height, 10)
            return 'ok'
        }
        function grabShape() {
            if (!loader.item) { return 'no-view' }
            var size = loader.item.grabSize()
            return size.width + 'x' + size.height
        }
        function decode(path) {
            var scanner = findIn(loader.item, 'scanner')
            if (!scanner) { return 'missing:scanner' }
            scanner.decode(path)
            return 'ok'
        }
        function heardText() { return heard }
    }
";

/// The modules as the page's `QrCode` reports them, drawn into a P5 PGM
/// with a quiet zone, each module four pixels: what Qt writes for a
/// `.pgm` path, less the camera.
fn write_frame(encoded: &str, path: &Path) -> Result<(), String> {
    let (size, modules) = encoded
        .split_once(':')
        .ok_or_else(|| format!("no size in {encoded:?}"))?;
    let size: usize = size.parse().map_err(|err| format!("size: {err}"))?;
    if size == 0 {
        return Err("the code has no modules".to_string());
    }
    let (quiet, scale) = (4, 4);
    let edge = (size + 2 * quiet) * scale;
    let mut pixels = vec![u8::MAX; edge * edge];
    for (row, line) in modules.lines().enumerate() {
        for (column, module) in line.chars().enumerate() {
            if module != '1' {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    pixels[((row + quiet) * scale + dy) * edge + (column + quiet) * scale + dx] = 0;
                }
            }
        }
    }
    let mut bytes = format!("P5\n{edge} {edge}\n255\n").into_bytes();
    bytes.extend_from_slice(&pixels);
    std::fs::write(path, bytes).map_err(|err| err.to_string())
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_code_held_up_to_the_page_comes_back_as_its_text() {
    let temp = std::env::temp_dir().join(format!("postivene-scan-{}", std::process::id()));
    std::fs::create_dir_all(&temp).expect("create temp dir");
    let frame = temp.join("frame.pgm");

    // SAFETY: single-threaded test binary; set before Qt starts.
    unsafe {
        std::env::set_var("QT_QPA_PLATFORM", "offscreen");
        std::env::set_var("XDG_CACHE_HOME", &temp);
    }

    postivene_shim::register_qml_types();

    let mut engine = QmlEngine::new();
    engine.add_import_path(QString::from(
        common::stubs_dir().to_string_lossy().into_owned(),
    ));
    engine.load_data(QByteArray::from(PROBE_QML));

    let engine_ptr = std::ptr::addr_of_mut!(engine);
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
    macro_rules! get {
        ($name:expr, $property:expr) => {
            call!("get", QString::from($name), QString::from($property))
        };
    }
    macro_rules! record {
        ($label:expr, $value:expr) => {
            (*steps_ptr).push(($label, $value))
        };
    }

    let frame_path = frame.clone();
    single_shot(Duration::from_secs(1), move || unsafe {
        // The picture: the page's own encoder, drawn into a file.
        let encoded = call!("encode", QString::from(INVITE));
        record!(
            "frame",
            write_frame(&encoded, &frame_path).map_or_else(|err| err, |()| "ok".to_string())
        );
        record!(
            "load",
            call!("load", QString::from(common::component_url("ScanView.qml")))
        );
        // Not yet on screen: the camera is not running and nothing is
        // grabbed.
        record!("camera-before", get!("camera", "running"));
        record!("grabbing-before", get!("grabber", "running"));
        call!("setActive", true);
        record!("camera-active", get!("camera", "running"));
        record!("grabbing-active", get!("grabber", "running"));
        // Focus is asked for while nothing has been read.
        record!("focusing-active", get!("refocus", "running"));
        record!("acting-before", get!("acting", "running"));
        // The viewfinder timer's own call, with the frame made above.
        record!(
            "decode",
            call!(
                "decode",
                QString::from(frame_path.to_string_lossy().into_owned())
            )
        );
        record!("busy", get!("scanner", "busy"));
    });

    single_shot(Duration::from_secs(3), move || unsafe {
        record!("heard", call!("heardText"));
        record!("camera-after", get!("camera", "running"));
        record!("grabbing-after", get!("grabber", "running"));
        record!("focusing-after", get!("refocus", "running"));
        record!("acting-after", get!("acting", "running"));
        // The first search ran as soon as the view came on screen, on
        // the event loop turn after it did.
        record!("searches", get!("camera", "searches"));
        // The other way in, on a fresh view: a link typed rather than
        // scanned, with the clipboard offering it first.
        record!(
            "reload",
            call!("load", QString::from(common::component_url("ScanView.qml")))
        );
        call!("setActive", true);
        record!("panel-before", get!("linkPanel", "visible"));
        record!(
            "shopping",
            call!("setClipboard", QString::from("milk, eggs"))
        );
        record!("open", call!("click", QString::from("typeLinkButton")));
        record!("panel-open", get!("linkPanel", "visible"));
        record!("not-pasted", get!("linkField", "text"));
        record!(
            "reload-again",
            call!("load", QString::from(common::component_url("ScanView.qml")))
        );
        call!("setActive", true);
        record!("copied", call!("setClipboard", QString::from(TYPED)));
        record!(
            "open-again",
            call!("click", QString::from("typeLinkButton"))
        );
        record!("pasted", get!("linkField", "text"));
        record!("connect", call!("click", QString::from("followButton")));
        record!("typed-heard", call!("heardText"));
        record!("typed-camera", get!("camera", "running"));
        record!("typed-acting", get!("acting", "running"));
        // A frame is grabbed in the viewfinder's own shape, with its
        // long side capped, and a viewfinder smaller than the cap is
        // taken as it is rather than blown up.
        call!("sizeView", QString::from("540"), QString::from("800"));
        record!("grab-tall", call!("grabShape"));
        call!("sizeView", QString::from("800"), QString::from("540"));
        record!("grab-wide", call!("grabShape"));
        call!("sizeView", QString::from("300"), QString::from("200"));
        record!("grab-small", call!("grabShape"));
        (*engine_ptr).quit();
    });

    engine.exec();

    assert_scan(&steps);
    assert_typed(&steps);
}

/// An invite the reader types, or copied and lets the panel paste.
const TYPED: &str = "https://i.delta.chat/#FEDCBA9876543210&a=typed%40example.org&n=Typed";

/// The panel opens from the button, takes the clipboard only when it
/// looks like an invite, and hands the link back the way a code is.
fn assert_typed(steps: &[(&str, String)]) {
    let context = format!("steps: {steps:?}");
    let value = |label: &str| {
        steps
            .iter()
            .find(|(name, _)| *name == label)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    for label in ["reload", "open", "reload-again", "open-again", "connect"] {
        assert_eq!(value(label), "ok", "step {label} failed. {context}");
    }
    assert_eq!(
        value("panel-before"),
        "false",
        "the link panel is up before anyone asked for it. {context}"
    );
    assert_eq!(
        value("panel-open"),
        "true",
        "the button did not open the link panel. {context}"
    );
    assert_eq!(
        value("not-pasted"),
        "",
        "a clipboard that holds no invite was pasted into the field. {context}"
    );
    assert_eq!(
        value("pasted"),
        TYPED,
        "an invite on the clipboard was not offered in the field. {context}"
    );
    assert_eq!(
        value("typed-heard"),
        TYPED,
        "the typed link was not handed back the way a scanned code is. {context}"
    );
    // 640 on the long side, the short one in proportion. A square grab
    // of an oblong viewfinder stretches the modules, and a decoder
    // reading a square symbol as an oblong one has that much less to
    // work with.
    assert_eq!(
        value("grab-tall"),
        "432x640",
        "a tall viewfinder was not grabbed in its own shape. {context}"
    );
    assert_eq!(
        value("grab-wide"),
        "640x432",
        "a wide viewfinder was not grabbed in its own shape. {context}"
    );
    assert_eq!(
        value("grab-small"),
        "300x200",
        "a viewfinder smaller than the cap was blown up, which invents \
         no detail. {context}"
    );
    assert_eq!(
        value("typed-camera"),
        "false",
        "the camera keeps running after a link was entered. {context}"
    );
    assert_eq!(
        value("typed-acting"),
        "true",
        "the view does not show that the link is being acted on. {context}"
    );
}

/// The camera follows the page's status, the frame decodes to the invite,
/// and the page hands it on and waits.
fn assert_scan(steps: &[(&str, String)]) {
    let context = format!("steps: {steps:?}");
    let value = |label: &str| {
        steps
            .iter()
            .find(|(name, _)| *name == label)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };

    for label in ["frame", "load", "decode"] {
        assert_eq!(value(label), "ok", "step {label} failed. {context}");
    }
    assert_eq!(
        value("camera-before"),
        "false",
        "the camera runs before the view is on screen. {context}"
    );
    assert_eq!(
        value("grabbing-before"),
        "false",
        "frames are grabbed before the view is on screen. {context}"
    );
    assert_eq!(
        value("camera-active"),
        "true",
        "the camera does not start with the view. {context}"
    );
    assert_eq!(
        value("grabbing-active"),
        "true",
        "frames are not grabbed while the view is on screen. {context}"
    );
    assert_eq!(
        value("focusing-active"),
        "true",
        "focus is not being asked for while scanning. {context}"
    );
    assert!(
        value("searches").parse::<u32>().unwrap_or(0) >= 1,
        "no focus search was run when the view came on screen. {context}"
    );
    assert_eq!(
        value("busy"),
        "true",
        "the scanner does not say it is busy while decoding, so frames \
         would queue behind it. {context}"
    );
    assert_eq!(
        value("heard"),
        INVITE,
        "the frame did not come back as the invite it carried. {context}"
    );
    assert_eq!(
        value("camera-after"),
        "false",
        "the camera keeps running after a code was read. {context}"
    );
    assert_eq!(
        value("grabbing-after"),
        "false",
        "frames keep being grabbed after a code was read. {context}"
    );
    assert_eq!(
        value("focusing-after"),
        "false",
        "focus keeps being asked for after a code was read. {context}"
    );
    assert_eq!(
        value("acting-before"),
        "false",
        "the view says it is acting on a code before one was read. {context}"
    );
    assert_eq!(
        value("acting-after"),
        "true",
        "the view does not show that the code is being acted on. {context}"
    );
}
