//! Taking a profile over: from a device that has it, or from a file.
//!
//! The core's own import, driven the way `src/signup.rs` drives a signup
//! and checked the same way: one Qt event loop against the recording
//! double, then its journal. What is pinned here is what the shim does
//! around the import:
//!
//! - the transfer reports itself while it runs, and the profile it
//!   brings over is announced with IO started on it, since an imported
//!   account has none;
//! - a code that is not a device offering a profile is refused before
//!   anything is transferred, with this app's own reason rather than the
//!   core's words about protocols;
//! - a failed import takes its account with it, unlike a refused signup:
//!   a half-written account is good for nothing, and the next attempt
//!   reuses whatever unconfigured account it finds;
//! - a transfer given up on answers nobody, and the core's process is
//!   stopped on the account it was holding.

// Qt harness: needs `unsafe` for `env::set_var` before Qt starts
// (`unused_unsafe` because it is only unsafe from edition 2024 on),
// `borrow_as_ptr` for the engine pointer, and `single_shot` with
// whole-second Durations.
#![allow(
    unsafe_code,
    unused_unsafe,
    clippy::borrow_as_ptr,
    clippy::disallowed_methods,
    clippy::expect_used
)]

use std::time::Duration;

use postivene_shim::DeltaChatCore;
use qmetaobject::*;
use serde_json::Value;

mod common;

/// Records what the shim signalled, in the Qt 5.6 dialect with the
/// shim's `snake_case` names.
const PROBE_QML: &str = r"
        import QtQuick 2.0
        Item {
            property int restored: 0
            property int lastAccount: 0
            property int steps: 0
            property int dones: 0
            property string refusals: ''
            property string failures: ''
            Connections {
                target: core
                onProfile_restored: {
                    restored = restored + 1
                    lastAccount = account_id
                }
                onRestore_progress: {
                    steps = steps + 1
                    if (permille === 1000) {
                        dones = dones + 1
                    }
                }
                onRestore_refused: refusals = refusals + reason + '|'
                onRestore_failed: failures = failures + 'failed|'
            }
            function summary() {
                return restored + '/' + lastAccount + '/' + steps + '/'
                    + dones + '/' + refusals + '/' + failures
            }
        }
    ";

#[test]
fn a_profile_is_taken_over_from_a_device_or_a_file_and_never_half_kept() {
    let temp = std::env::temp_dir().join(format!("postivene-restore-{}", std::process::id()));
    let journal = common::fresh_journal(&temp);
    let accounts = temp.join("accounts");
    std::fs::create_dir_all(&accounts).expect("create temp dirs");

    // SAFETY: single-threaded test binary, and all of these have to be
    // set before Qt initialises and before the shim spawns the server
    // that inherits them.
    unsafe {
        std::env::set_var("QT_QPA_PLATFORM", "offscreen");
        std::env::set_var("POSTIVENE_FAKE_JOURNAL", &journal);
        std::env::set_var("POSTIVENE_ACCOUNTS_DIR", &accounts);
        // The slow transfer answers between the ticks below, never on
        // one.
        std::env::set_var("POSTIVENE_FAKE_SLOW_MS", "2500");
    }

    let core_box = QObjectBox::new(DeltaChatCore::default());
    let mut engine = QmlEngine::new();
    engine.set_object_property("core".into(), core_box.pinned());
    engine.load_data(QByteArray::from(PROBE_QML));

    let server = QString::from(env!("CARGO_BIN_EXE_fake-core-server"));
    core_box.pinned().borrow_mut().start(server);

    // Whole seconds only (clippy.toml); one step per tick.
    let core_ptr: QPointer<DeltaChatCore> = QPointer::from(core_box.pinned().borrow());

    // 1s: the code another device shows while it offers its profile.
    let device = core_ptr.clone();
    single_shot(Duration::from_secs(1), move || {
        if let Some(this) = device.as_pinned() {
            this.borrow_mut()
                .restore_from_device(QString::from("DCBACKUP2:one.example"));
        }
    });

    // 2s: a code from a device running something newer than this core
    // can read. Refused before anything is transferred.
    let too_new = core_ptr.clone();
    single_shot(Duration::from_secs(2), move || {
        if let Some(this) = too_new.as_pinned() {
            this.borrow_mut()
                .restore_from_device(QString::from("DCBACKUP2:toonew.example"));
        }
    });

    // 3s: a file the core cannot read.
    let bad_file = core_ptr.clone();
    single_shot(Duration::from_secs(3), move || {
        if let Some(this) = bad_file.as_pinned() {
            this.borrow_mut()
                .restore_from_file(QString::from("/tmp/fail-backup.tar"));
        }
    });

    // 4s: one it can.
    let good_file = core_ptr.clone();
    single_shot(Duration::from_secs(4), move || {
        if let Some(this) = good_file.as_pinned() {
            this.borrow_mut()
                .restore_from_file(QString::from("/tmp/holiday-backup.tar"));
        }
    });

    // 5s: a transfer that takes its time, and 6s: the reader gives up on
    // it. What it answers afterwards is nobody's.
    let slow = core_ptr.clone();
    single_shot(Duration::from_secs(5), move || {
        if let Some(this) = slow.as_pinned() {
            this.borrow_mut()
                .restore_from_device(QString::from("DCBACKUP2:slow.example"));
        }
    });
    let cancel = core_ptr;
    single_shot(Duration::from_secs(6), move || {
        if let Some(this) = cancel.as_pinned() {
            this.borrow_mut().cancel_ongoing();
        }
    });

    let engine_ptr = &engine as *const QmlEngine;
    single_shot(Duration::from_secs(10), move || {
        // SAFETY: see tests/smoke.rs -- the callback only fires while
        // `exec()` is still running on this thread.
        unsafe {
            (*engine_ptr).quit();
        }
    });

    engine.exec();

    let summary = QString::from_qvariant(engine.invoke_method("summary".into(), &[]))
        .map(|value| value.to_string())
        .unwrap_or_default();
    let calls = common::calls(&journal);
    let context = format!("signals: {summary}\ncalls: {calls:?}");

    assert_signals(&summary, &context);
    assert_imports_and_io(&calls, &context);
    assert_nothing_half_kept(&calls, &context);
}

/// Two profiles arrived, the second on its own account; the transfers
/// reported themselves, each ending in the core's "done"; the
/// code from the newer device was refused as such and nothing else was;
/// the unreadable file failed once, in the core's words.
fn assert_signals(summary: &str, context: &str) {
    assert!(!summary.is_empty(), "the probe QML never loaded. {context}");
    let parts: Vec<&str> = summary.split('/').collect();
    assert_eq!(parts.len(), 6, "unexpected summary shape. {context}");
    assert_eq!(parts[0], "2", "two profiles should have arrived. {context}");
    assert_eq!(
        parts[1], "2",
        "the second profile is not on its own account. {context}"
    );
    assert!(
        parts[2].parse::<u32>().unwrap_or(0) >= 4,
        "the transfers did not report themselves. {context}"
    );
    assert_eq!(
        parts[3], "2",
        "the transfers that finished did not report the core's done. \
         {context}"
    );
    assert_eq!(
        parts[4], "too-new|",
        "the newer device's code was not refused as such. {context}"
    );
    assert_eq!(
        parts[5], "failed|",
        "the unreadable file did not fail exactly once. {context}"
    );
}

/// The device transfers went to `get_backup` with the code, the files to
/// `import_backup` with the path, and IO was started on each profile
/// that arrived -- an imported account has none running.
fn assert_imports_and_io(calls: &[(String, Value)], context: &str) {
    let of = |name: &str| -> Vec<(u64, String)> {
        calls
            .iter()
            .filter(|(method, _)| method == name)
            .map(|(_, params)| {
                (
                    params.get(0).and_then(Value::as_u64).unwrap_or(0),
                    params
                        .get(1)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                )
            })
            .collect()
    };
    assert_eq!(
        of("get_backup"),
        vec![
            (1, "DCBACKUP2:one.example".to_string()),
            (3, "DCBACKUP2:slow.example".to_string()),
        ],
        "the transfers from a device, in order. {context}"
    );
    assert_eq!(
        of("import_backup"),
        vec![
            (2, "/tmp/fail-backup.tar".to_string()),
            (2, "/tmp/holiday-backup.tar".to_string()),
        ],
        "the imports from a file, in order. {context}"
    );
    let started: Vec<u64> = calls
        .iter()
        .filter(|(method, _)| method == "start_io")
        .filter_map(|(_, params)| params.get(0).and_then(Value::as_u64))
        .collect();
    assert_eq!(
        started,
        vec![1, 2],
        "IO was not started on each profile that arrived. {context}"
    );
}

/// The code that was refused cost no transfer at all; the file that
/// failed took its account with it; and the transfer the reader gave up
/// on was stopped in the core rather than left running, taking its own
/// account with it when it answered too late to matter.
fn assert_nothing_half_kept(calls: &[(String, Value)], context: &str) {
    let checked: Vec<String> = calls
        .iter()
        .filter(|(method, _)| method == "check_qr")
        .map(|(_, params)| {
            params
                .get(1)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert!(
        checked.contains(&"DCBACKUP2:toonew.example".to_string()),
        "the code was not read before the transfer. {context}"
    );
    let removed: Vec<u64> = calls
        .iter()
        .filter(|(method, _)| method == "remove_account")
        .filter_map(|(_, params)| params.get(0).and_then(Value::as_u64))
        .collect();
    assert_eq!(
        removed,
        vec![2, 3],
        "an account a transfer left behind was kept: the failed import's, \
         and the one the reader gave up on. {context}"
    );
    let stopped: Vec<u64> = calls
        .iter()
        .filter(|(method, _)| method == "stop_ongoing_process")
        .filter_map(|(_, params)| params.get(0).and_then(Value::as_u64))
        .collect();
    assert_eq!(
        stopped,
        vec![3],
        "the transfer given up on was not stopped. {context}"
    );
}
