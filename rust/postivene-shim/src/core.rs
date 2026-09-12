use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use deltachat_jsonrpc::{spawn_event_loop, CoreEvent, RpcClient};
use qmetaobject::*;

use crate::json;
use crate::models::{AccountItem, AccountListModel};
use crate::runtime::CoreRuntime;
use crate::signup::{self, Attempts, Outcome, Source, Taken, Transport};

/// The one live connection to the spawned server, shared with the models
/// QML instantiates per chat.
///
/// One per process in practice because the app makes one `DeltaChatCore`.
/// [`DeltaChatCore::start`]'s guard is per object, not global, so a second
/// instance would spawn a second server and take this over -- nothing
/// enforces the singleton, and nothing needs to yet.
static CONNECTION: Mutex<Option<(Arc<RpcClient>, CoreRuntime)>> = Mutex::new(None);

/// The transport and runtime, once [`DeltaChatCore::start`] has completed.
pub(crate) fn connection() -> Option<(Arc<RpcClient>, CoreRuntime)> {
    CONNECTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn set_connection(value: Option<(Arc<RpcClient>, CoreRuntime)>) {
    *CONNECTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
}

/// Default chatmail server, as the `dcaccount:` payload
/// `add_transport_from_qr` takes. See `docs/PROJECT.md`.
pub const DEFAULT_PROVIDER_QR: &str = "dcaccount:nine.testrun.org";

/// Where the RPM installs the server: beside the app, and not on `PATH`.
pub const BUNDLED_SERVER: &str = "/usr/libexec/harbour-postivene/deltachat-rpc-server";

/// How long the app waits for the server to go at exit.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Which server binary to run: `--rpc-server <path>` or
/// `--rpc-server=<path>` from `args`, else `POSTIVENE_RPC_SERVER` from
/// `env`, else [`BUNDLED_SERVER`].
///
/// Never a `PATH` lookup. The server is handed the mail password and holds
/// the keys, so it has to be the one this package installed or the one a
/// developer named -- not whichever binary of that name is first on a
/// search path. A bundled server that is missing fails at spawn with a
/// message saying which file, which is the right failure.
///
/// Behind a flag rather than taking `argv[1]`: Sailfish launches
/// `silica-qt5` apps through the invoker, which passes arguments of its
/// own, and a bare positional turned any of them into "the server binary".
#[must_use]
pub fn server_path<I>(args: I, env: Option<String>) -> String
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--rpc-server" {
            if let Some(path) = args.next() {
                return path;
            }
        } else if let Some(path) = arg.strip_prefix("--rpc-server=") {
            return path.to_string();
        }
    }
    env.filter(|path| !path.is_empty())
        .unwrap_or_else(|| BUNDLED_SERVER.to_string())
}

/// The running server's process id, or `None` before it is started or
/// after it has gone. For `POSTIVENE_MEMORY_LOG` in the app.
#[must_use]
pub fn server_pid() -> Option<u32> {
    CONNECTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|(rpc, _)| rpc.pid())
}

/// Stop the server, once the Qt event loop has returned.
///
/// Without this the child is only killed when the last `RpcClient` drops,
/// and the one held for the models never does: statics are not dropped at
/// exit. The server went anyway, on its stdin closing, but that was its
/// courtesy rather than this app's doing. Bounded, so a server that will
/// not die cannot hold the app open.
pub fn shutdown() {
    let Some((rpc, runtime)) = CONNECTION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return;
    };
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    runtime.spawn(async move {
        let _ = rpc.shutdown().await;
        let _ = done_tx.send(());
    });
    let _ = done_rx.recv_timeout(SHUTDOWN_TIMEOUT);
}

/// The last few lines the server wrote to stderr, as one message, or
/// nothing when it said nothing. The one clue to why it went that anyone
/// will ever see.
fn last_words(tail: &[String]) -> Option<String> {
    const KEEP: usize = 5;
    if tail.is_empty() {
        return None;
    }
    let start = tail.len().saturating_sub(KEEP);
    Some(format!(
        "the core's last output was: {}",
        tail[start..].join(" | ")
    ))
}

/// How long to wait before the first restart, and the ceiling the wait
/// doubles towards. The first is short because the overwhelmingly likely
/// cause is the phone reclaiming memory, and respawning then works
/// immediately; the ceiling is what keeps a server that cannot start from
/// spinning.
const RESTART_DELAY_MIN: Duration = Duration::from_secs(1);
const RESTART_DELAY_MAX: Duration = Duration::from_secs(30);

/// A server that stayed up this long counts as having worked. The next
/// failure then starts backing off from the minimum again instead of from
/// wherever the last crash loop left the delay.
const HEALTHY_UPTIME: Duration = Duration::from_secs(60);

/// How many restarts without a healthy run in between before giving up and
/// saying so. With the backoff above that is a little over three minutes of
/// trying -- long enough to outlast anything transient, short enough that a
/// binary which will never start does not keep the phone awake.
const RESTART_LIMIT: u32 = 12;

/// How long to wait before restart number `attempt`.
fn restart_delay(attempt: u32) -> Duration {
    RESTART_DELAY_MIN
        .saturating_mul(1_u32.checked_shl(attempt).unwrap_or(u32::MAX))
        .min(RESTART_DELAY_MAX)
}

/// The one `QObject` owning the connection to a spawned
/// `deltachat-rpc-server`.
///
/// Methods are fire-and-forget, each paired with a result signal. Every
/// async completion goes through [`qmetaobject::queued_callback`] so it
/// lands on the Qt thread before touching a `qt_property` or `qt_signal`.
///
/// Only a slice of the core's ~100 JSON-RPC methods is exposed; add more
/// the same way as the UI needs them.
#[derive(QObject, Default)]
// `io_all`, `supervising` and the two `_set` flags are independent facts,
// not states of one thing: what IO was asked for, whether a spawn has ever
// succeeded, and whether QML has handed each of two settings over. A state
// machine would be invented for them, not found.
#[allow(clippy::struct_excessive_bools)]
pub struct DeltaChatCore {
    base: qt_base_class!(trait QObject),

    /// One of: "idle", "starting", "ready", "stopped" (the server died),
    /// or `"error: ..."`.
    pub status: qt_property!(QString; NOTIFY status_changed),
    /// Emitted whenever [`DeltaChatCore::status`] changes.
    pub status_changed: qt_signal!(),

    /// The core's `get_system_info` answer, as raw JSON. Empty until
    /// [`DeltaChatCore::check_health`] has completed once.
    pub system_info: qt_property!(QString; NOTIFY system_info_changed),
    /// Emitted when [`DeltaChatCore::system_info`] is refreshed.
    pub system_info_changed: qt_signal!(),

    /// Raw core event: `kind` is the event's tag, `payload_json` the whole
    /// event as JSON. Untyped so QML can read any of the ~40 event kinds.
    pub core_event: qt_signal!(context_id: u32, kind: QString, payload_json: QString),

    /// A new, still unconfigured account was created.
    pub account_added: qt_signal!(account_id: u32),
    /// An account-scoped call (create, list, resume) failed.
    pub account_error: qt_signal!(message: QString),

    /// Spawn `rpc_server_path` and drain its event stream. No-op if
    /// already started.
    pub start: qt_method!(fn(&mut self, rpc_server_path: QString)),

    /// `get_system_info` round trip; result lands in `system_info`.
    pub check_health: qt_method!(fn(&mut self)),

    /// Create a new unconfigured account.
    pub add_account: qt_method!(fn(&mut self)),
    /// Delete a profile and everything in it, on this device.
    ///
    /// `remove_account` is the core's name for it -- verified against the
    /// pinned binary, which has no `delete_account`. There is no undo.
    pub remove_account: qt_method!(fn(&mut self, account_id: u32)),

    /// All accounts known to the core.
    pub account_list: qt_property!(RefCell<AccountListModel>; CONST),

    /// Attachments larger than this many bytes are fetched only when
    /// asked for; 0 fetches everything. The reader's setting, applied to
    /// every account the core has -- when it is set, and again each time
    /// the account list is read, so a profile added later gets it too.
    /// The core's `download_limit`, which it applies to future messages.
    pub download_limit: qt_property!(u32; WRITE set_download_limit NOTIFY download_limit_changed),
    /// Emitted when [`DeltaChatCore::download_limit`] changes.
    pub download_limit_changed: qt_signal!(),

    /// Messages older than this many seconds are deleted from this
    /// device; 0 keeps them. The reader's setting, applied to every
    /// account the way the download limit is. The core's
    /// `delete_device_after`, which it applies to every chat whatever
    /// that chat's own disappearing-messages timer says, and never to
    /// "Saved messages".
    pub delete_device_after: qt_property!(u32; WRITE set_delete_device_after NOTIFY delete_device_after_changed),
    /// Emitted when [`DeltaChatCore::delete_device_after`] changes.
    pub delete_device_after_changed: qt_signal!(),
    /// Ask how many messages `delete_device_after` set to `seconds` would
    /// delete now, across every account: what the settings page says
    /// before the reader confirms. Answers on `auto_deletion_estimated`,
    /// or on `core_error`.
    pub estimate_auto_deletion: qt_method!(fn(&mut self, seconds: u32)),
    /// The answer to `estimate_auto_deletion`, with the seconds it was
    /// asked for, so an answer to an earlier question can be told apart.
    pub auto_deletion_estimated: qt_signal!(seconds: u32, count: u32),

    /// Repopulate `account_list`. QML uses the `accounts_refreshed` result
    /// at startup to choose between onboarding and resuming an account.
    pub refresh_accounts: qt_method!(fn(&mut self)),
    /// [`DeltaChatCore::account_list`] was repopulated. `configured_count`
    /// is how many accounts are usable, and `resume_account_id` is the one
    /// to show (0 when there is none): the profile the core has marked as
    /// selected -- the one last shown, which the core remembers across
    /// runs -- or the first configured one when nothing is selected, or
    /// what is selected is not usable.
    pub accounts_refreshed: qt_signal!(configured_count: u32, resume_account_id: u32),

    /// Tell the core which profile is being shown, so the next start
    /// comes back to it. The core's `select_account`, which it writes to
    /// disk; the chat list calls this for whatever profile it opens on.
    pub select_account: qt_method!(fn(&mut self, account_id: u32)),

    /// Resume IO for an already-configured account.
    pub start_account_io: qt_method!(fn(&mut self, account_id: u32)),
    /// Resume IO for every configured account. Every profile receives,
    /// whichever the chat list is showing: the cover counts them all, and
    /// a profile switched away from is still one people write to.
    pub start_all_account_io: qt_method!(fn(&mut self)),
    /// Result of resuming IO. `account_id` is 0 for a resume of every
    /// account at once.
    pub io_started: qt_signal!(account_id: u32, success: bool, error: QString),

    /// The default chatmail server's `dcaccount:` payload.
    pub default_provider_qr: qt_method!(fn(&mut self) -> QString),

    /// Create a profile on a chatmail server: the core mints the address
    /// and credentials. Result via `profile_created`, `profile_error` or
    /// `profile_timed_out`, and only for the latest attempt: an attempt
    /// cancelled or superseded answers nobody (`signup.rs`).
    pub create_profile: qt_method!(fn(&mut self, display_name: QString, provider_qr: QString)),

    /// Configure an existing mailbox as this profile's transport.
    pub create_profile_with_email:
        qt_method!(fn(&mut self, display_name: QString, addr: QString, password: QString)),

    /// A profile is ready to use; `account_id` has a working transport.
    pub profile_created: qt_signal!(account_id: u32),
    /// Creating a profile failed. The message is the core's own.
    pub profile_error: qt_signal!(message: QString),
    /// The relay did not answer within `seconds`, and the attempt was
    /// given up on: the core's process is stopped, and a profile it
    /// makes after all is removed.
    pub profile_timed_out: qt_signal!(seconds: u32),
    /// Seconds a relay is given to answer before an attempt is given up
    /// on; 0 is the built-in thirty (`signup::DEADLINE`). Nothing sets
    /// it; a test turns it down rather than waiting.
    pub profile_timeout: qt_property!(u32; NOTIFY profile_timeout_changed),
    /// Emitted when [`DeltaChatCore::profile_timeout`] changes.
    pub profile_timeout_changed: qt_signal!(),

    /// Configuration progress for `account_id`, as the core reports it:
    /// 0 means failure, 1..=999 is permille, 1000 means done.
    pub configure_progress: qt_signal!(account_id: u32, permille: u32),

    /// Abort the running attempt at a profile. Takes no account id:
    /// onboarding has none to give, and the attempt knows its own.
    pub cancel_ongoing: qt_method!(fn(&mut self)),

    /// Take a profile over from the device showing this code.
    pub restore_from_device: qt_method!(fn(&mut self, qr_text: QString)),

    /// Take a profile over from a backup file.
    pub restore_from_file: qt_method!(fn(&mut self, path: QString)),

    /// How far the transfer has got, in permille, as the core reports
    /// it: 1000 is done, 0 is the core giving up.
    pub restore_progress: qt_signal!(permille: u32),

    /// The profile is on this device now, with IO started on it.
    pub profile_restored: qt_signal!(account_id: u32),

    /// Nothing was taken over, for a reason this app words itself:
    /// `not-a-backup`, `too-new` or `stalled`.
    pub restore_refused: qt_signal!(reason: QString),

    /// Nothing was taken over, in the core's own words.
    pub restore_failed: qt_signal!(message: QString),

    /// The account's email transports, as a JSON array of upstream
    /// `EnteredLoginParam`.
    pub list_transports: qt_method!(fn(&mut self, account_id: u32)),
    /// The account's transports, as a raw JSON array.
    pub transports_listed: qt_signal!(account_id: u32, transports_json: QString),

    /// Classify a QR payload. `qr_checked` carries the upstream `Qr`
    /// object as JSON: `kind` is camelCase, its fields `snake_case`.
    pub check_qr: qt_method!(fn(&mut self, account_id: u32, qr_content: QString)),
    /// A QR/invite payload was classified by the core.
    pub qr_checked: qt_signal!(account_id: u32, kind: QString, payload_json: QString),
    /// Classifying a QR/invite payload failed.
    pub qr_error: qt_signal!(message: QString),

    /// The core reported a failure of its own -- an `Error` event, which
    /// carries a message meant for the user. Typed so a page need not
    /// parse the event payload.
    pub core_error: qt_signal!(message: QString),

    rpc: Option<Arc<RpcClient>>,
    runtime: Option<CoreRuntime>,

    /// The server binary `start` was given, kept so a restart need not be
    /// told again.
    rpc_server_path: String,
    /// Accounts whose IO was resumed, replayed after a restart. A fresh
    /// server has the account files but no IO running, so without this the
    /// app comes back able to read history and unable to receive.
    io_accounts: BTreeSet<u32>,
    /// IO was resumed for every account at once, and is to be again after
    /// a restart.
    io_all: bool,
    /// Restarts since the last healthy run; drives the backoff and the
    /// give-up limit.
    restart_attempt: u32,
    /// When the current server was spawned, for [`HEALTHY_UPTIME`].
    server_started_at: Option<Instant>,
    /// True once a spawn has succeeded. Until then a failure is the app
    /// failing to start, which is reported and left alone; after it, a
    /// failure is something to retry.
    supervising: bool,
    /// True once QML has handed over a download limit. Until then the
    /// property holds a default nobody chose, and writing that to every
    /// account would be the app deciding for the reader.
    download_limit_set: bool,
    /// The same, for the deletion period.
    delete_device_after_set: bool,
    /// The attempts at a profile there have been, shared with the tasks
    /// that carry each one out.
    attempts: Arc<Attempts>,
}

impl DeltaChatCore {
    /// Spawn the server and begin draining its event stream. No-op if
    /// already started. See the `start` declaration above.
    pub fn start(&mut self, rpc_server_path: QString) {
        if self.runtime.is_some() {
            return;
        }

        // Built on its own thread; see `crate::runtime`.
        let runtime = match CoreRuntime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                self.status = format!("error: failed to start async runtime: {err}").into();
                self.status_changed();
                return;
            }
        };

        let path = rpc_server_path.to_string();
        self.rpc_server_path.clone_from(&path);
        self.runtime = Some(runtime.clone());
        self.restart_attempt = 0;
        self.supervising = false;

        self.status = QString::from("starting");
        self.status_changed();

        Self::spawn_server(QPointer::from(&*self), path, runtime);
    }

    /// Spawn the server and wire the result up. Shared by `start` and every
    /// restart.
    ///
    /// An associated function taking what it needs, rather than a method:
    /// the object must not be borrowed while any of this runs. `start` is
    /// called from QML, which holds a mutable borrow for the duration, and
    /// `status_changed` is handled in QML by code that calls straight back
    /// in here. So every mutation below is scoped to a callback, and every
    /// signal is emitted with no borrow held.
    fn spawn_server(ptr: QPointer<Self>, path: String, runtime: CoreRuntime) {
        let started_ptr = ptr.clone();
        let retry_path = path.clone();
        let started = queued_callback(move |result: Result<Arc<RpcClient>, String>| {
            let Some(this) = started_ptr.as_pinned() else {
                return;
            };
            match result {
                Ok(rpc) => {
                    let (runtime, accounts, all) = {
                        let mut this_mut = this.borrow_mut();
                        this_mut.rpc = Some(rpc.clone());
                        this_mut.status = QString::from("ready");
                        this_mut.server_started_at = Some(Instant::now());
                        this_mut.supervising = true;
                        set_connection(this_mut.runtime.clone().map(|rt| (rpc.clone(), rt)));
                        (
                            this_mut.runtime.clone(),
                            this_mut.io_accounts.clone(),
                            this_mut.io_all,
                        )
                    };
                    // Draining first: IO resumed before anything is reading
                    // the stream would deliver its events to nobody.
                    if let Some(runtime) = runtime {
                        Self::forward_events(started_ptr.clone(), rpc, runtime, retry_path.clone());
                    }
                    if all {
                        Self::resume_io(started_ptr.clone(), None);
                    }
                    for account_id in accounts {
                        Self::resume_io(started_ptr.clone(), Some(account_id));
                    }
                    // Last, because a handler may call back in here.
                    this.borrow().status_changed();
                }
                Err(err) => {
                    // Only a server that worked once is worth retrying; a
                    // first spawn that fails is reported and left alone,
                    // which is what the first screen reads.
                    if this.borrow().supervising {
                        Self::schedule_restart(
                            started_ptr.clone(),
                            retry_path.clone(),
                            Some(format!("could not restart the core: {err}")),
                        );
                        return;
                    }
                    // Never started: this is the app failing to start.
                    {
                        // Dropped so a later `start()` is not blocked by the
                        // already-started guard.
                        let mut this_mut = this.borrow_mut();
                        this_mut.runtime = None;
                        set_connection(None);
                        this_mut.status = format!("error: {err}").into();
                    }
                    this.borrow().status_changed();
                }
            }
        });

        runtime.spawn(async move {
            let result = async {
                let accounts_dir = Self::accounts_dir()?;
                RpcClient::spawn_with_env(
                    path,
                    Vec::<&str>::new(),
                    [("DC_ACCOUNTS_PATH", accounts_dir)],
                )
                .await
                .map(Arc::new)
                .map_err(|err| err.to_string())
            }
            .await;
            started(result);
        });
    }

    /// The server is gone: put the app into `reconnecting` and spawn
    /// another one after a backoff, or give up and say `stopped`.
    ///
    /// `failure` is what to report if this round gives up: the spawn error
    /// when a restart attempt is what failed, the server's last words when
    /// a running server exited, and `None` when it left in silence.
    fn schedule_restart(ptr: QPointer<Self>, path: String, failure: Option<String>) {
        let Some(this) = ptr.as_pinned() else { return };
        set_connection(None);

        let next = {
            let mut this_mut = this.borrow_mut();
            this_mut.rpc = None;
            // Nothing to restart: either `start` was never called, or a
            // previous round already gave up and dropped the runtime.
            let Some(runtime) = this_mut.runtime.clone() else {
                return;
            };
            // A server that stayed up long enough to be useful resets the
            // backoff, so an app running for a week does not treat its
            // second ever restart as if it were in a crash loop.
            if this_mut
                .server_started_at
                .is_some_and(|at| at.elapsed() >= HEALTHY_UPTIME)
            {
                this_mut.restart_attempt = 0;
            }
            this_mut.server_started_at = None;

            if this_mut.restart_attempt >= RESTART_LIMIT {
                // Same reason as the failed first spawn: leaving the runtime
                // in place would make a later `start()` a silent no-op.
                this_mut.runtime = None;
                // "stopped" and not an `error:` status: this is the state
                // the pages have a message for, and the reason -- when
                // there is one -- goes out on `core_error` instead, which
                // is where a detail the reader cannot act on belongs.
                this_mut.status = QString::from("stopped");
                None
            } else {
                let delay = restart_delay(this_mut.restart_attempt);
                this_mut.restart_attempt += 1;
                this_mut.status = QString::from("reconnecting");
                Some((delay, runtime))
            }
        };
        this.borrow().status_changed();

        let Some((delay, runtime)) = next else {
            if let Some(detail) = failure {
                this.borrow().core_error(detail.into());
            }
            return;
        };
        let spawn_runtime = runtime.clone();
        let retry = queued_callback(move |()| {
            Self::spawn_server(ptr.clone(), path.clone(), spawn_runtime.clone());
        });
        runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            retry(());
        });
    }

    /// Resume IO after a restart -- for one account, or for all of them
    /// with `None` -- without touching the `io_started` signal: nothing
    /// asked for this, and a page that reacted to it would be reacting to
    /// a reconnection it never requested.
    fn resume_io(ptr: QPointer<Self>, account_id: Option<u32>) {
        let Some(this) = ptr.as_pinned() else { return };
        let Some((rpc, runtime)) = this.borrow().connection() else {
            return;
        };
        let failed = queued_callback(move |err: String| {
            if let Some(this) = ptr.as_pinned() {
                this.borrow().core_error(err.into());
            }
        });
        runtime.spawn(async move {
            if let Err(err) = start_io(&rpc, account_id).await {
                failed(err);
            }
        });
    }

    /// Where the core keeps account state: `POSTIVENE_ACCOUNTS_DIR`, else
    /// inside the directory sailjail grants the app. Without it the core
    /// would use `./accounts`, relative to whatever directory the app was
    /// launched from.
    ///
    /// The nesting is not a typo. Sailjail grants write access to
    /// `~/.local/share/<OrganizationName>/<ApplicationName>`, and
    /// postivene.desktop declares both as `postivene`, so the account
    /// directory has to sit under *both* to be writable once confined.
    /// The old `postivene/accounts` was a sibling of that grant, not a
    /// child, and would have become unwritable the moment the
    /// `[X-Sailjail]` section took effect.
    ///
    /// # Errors
    ///
    /// If neither `XDG_DATA_HOME` nor `HOME` is set, so there is nowhere
    /// to put the directory, or if it cannot be created.
    pub fn accounts_dir() -> Result<String, String> {
        let dir = if let Ok(dir) = std::env::var("POSTIVENE_ACCOUNTS_DIR") {
            std::path::PathBuf::from(dir)
        } else {
            let base = std::env::var("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|_| {
                    std::env::var("HOME")
                        .map(|home| std::path::PathBuf::from(home).join(".local/share"))
                })
                .map_err(|_| "neither XDG_DATA_HOME nor HOME is set".to_string())?;
            let dir = base.join("postivene/postivene/accounts");
            Self::adopt_legacy_accounts(&base.join("postivene/accounts"), &dir);
            dir
        };
        // Private to this user: the directory holds the keys, the mail
        // password and every message. The mode is asked for at creation
        // and set again afterwards, because a directory that already
        // exists -- adopted from before the sandbox, or made by an older
        // build with the umask default -- keeps whatever it was given.
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&dir)
            .map_err(|err| format!("cannot create accounts dir {}: {err}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|err| format!("cannot restrict accounts dir {}: {err}", dir.display()))?;
        }
        Ok(dir.to_string_lossy().into_owned())
    }

    /// Move a profile left at the pre-sandbox location, once.
    ///
    /// Best effort by necessity: a confined app cannot see the old
    /// directory at all, since it is outside the grant. This helps the
    /// run that happens before confinement takes effect, and does nothing
    /// otherwise -- a profile stranded by an upgrade straight into the
    /// sandbox has to be moved by hand, or pointed at with
    /// `POSTIVENE_ACCOUNTS_DIR`. Never overwrites a profile that is
    /// already in the new place.
    fn adopt_legacy_accounts(legacy: &std::path::Path, wanted: &std::path::Path) {
        if wanted.exists() || !legacy.is_dir() {
            return;
        }
        let Some(parent) = wanted.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_ok() {
            // A failure here leaves the old directory untouched, which is
            // the right way to fail: the account is still there to move.
            let _ = std::fs::rename(legacy, wanted);
        }
    }

    /// Forward the core's event stream to `core_event` via queued
    /// callbacks, for as long as the transport lives.
    ///
    /// The setup and the loop share one `runtime.spawn` because
    /// `spawn_event_loop`'s internal `tokio::spawn` needs ambient runtime
    /// context.
    fn forward_events(
        ptr: QPointer<Self>,
        rpc: Arc<RpcClient>,
        runtime: CoreRuntime,
        path: String,
    ) {
        // The stream ends when the server dies -- killed for memory,
        // crashed, whatever. That is the one failure the app cannot see
        // for itself: every model goes quiet and the UI still looks fine.
        // So it is also where the next server gets started.
        let stopped_ptr = ptr.clone();
        let stopped = queued_callback(move |tail: Vec<String>| {
            Self::schedule_restart(stopped_ptr.clone(), path.clone(), last_words(&tail));
        });
        let emit = queued_callback(move |event: CoreEvent| {
            if let Some(this) = ptr.as_pinned() {
                this.borrow().relay(&event);
            }
        });
        runtime.spawn(async move {
            let (mut events, _handle) = spawn_event_loop(rpc.clone());
            while let Some(event) = events.recv().await {
                emit(event);
            }
            // The stream ends only when the transport does (see
            // `spawn_event_loop`), so this is the server gone -- and what
            // it wrote to stderr on the way out is the only clue why.
            stopped(rpc.stderr_tail());
        });
    }

    /// One event off the stream, as the signals the QML side listens for.
    ///
    /// Every event fires `core_event`, with its payload as JSON. A few
    /// kinds fire a typed signal too, so a progress bar need not parse
    /// JSON, and a few are the ones that could have moved an unread count.
    fn relay(&self, event: &CoreEvent) {
        let kind = match json::str_at(&event.event, "kind") {
            "" => "Unknown".to_string(),
            kind => kind.to_string(),
        };
        let payload = serde_json::to_string(&event.event).unwrap_or_default();
        if kind == "Error" {
            let text = match json::str_at(&event.event, "msg") {
                "" => "the core reported an error",
                text => text,
            };
            self.core_error(text.into());
        }
        if kind == "ConfigureProgress" {
            if let Some(permille) = json::u32_opt(&event.event, "progress") {
                self.configure_progress(event.context_id, permille);
            }
        }
        // The same for a profile being taken over: the core reports an
        // import the way it reports a configure, and one is running at a
        // time, so the account it is on is the page's own business
        // rather than something to match up here.
        if kind == "ImexProgress" {
            if let Some(permille) = json::u32_opt(&event.event, "progress") {
                self.restore_progress(permille);
            }
        }
        // The count on the profile's row follows whatever could have
        // moved it: a message in, a chat read or marked unread, a chat
        // gone. The overflow could have hidden any of those.
        if matches!(
            kind.as_str(),
            "IncomingMsg"
                | "IncomingMsgBunch"
                | "MsgsChanged"
                | "MsgsNoticed"
                | "ChatModified"
                | "ChatDeleted"
                | "ChatlistChanged"
                | "ChatlistItemChanged"
                | "EventChannelOverflow"
        ) {
            self.refresh_unread(event.context_id);
        }
        self.core_event(event.context_id, kind.into(), payload.into());
    }

    /// Run a `get_system_info` round trip into
    /// [`DeltaChatCore::system_info`].
    pub fn check_health(&mut self) {
        let Some((rpc, runtime)) = self.connection() else {
            self.core_error(QString::from("not started"));
            return;
        };

        let ptr: QPointer<Self> = QPointer::from(&*self);
        let done = queued_callback(move |result: Result<String, String>| {
            let Some(this) = ptr.as_pinned() else { return };
            match result {
                Ok(info) => {
                    this.borrow_mut().system_info = info.into();
                    this.borrow().system_info_changed();
                }
                // Reported on its own signal rather than written into
                // `status`: that is the app's answer to "is the core
                // there", and a health check that times out is not the
                // same as a core that has gone away -- but every button
                // enabled on `status === "ready"` would go dead.
                Err(err) => this.borrow().core_error(err.into()),
            }
        });

        runtime.spawn(async move {
            let result = rpc
                .call_unit::<std::collections::BTreeMap<String, String>>("get_system_info")
                .await
                .map(|info| serde_json::to_string(&info).unwrap_or_default())
                .map_err(|err| err.to_string());
            done(result);
        });
    }

    /// Create a new, unconfigured account.
    pub fn add_account(&mut self) {
        let Some((rpc, runtime)) = self.connection() else {
            self.account_error(QString::from("not started"));
            return;
        };

        let ptr: QPointer<Self> = QPointer::from(&*self);
        let done = queued_callback(move |result: Result<u32, String>| {
            let Some(this) = ptr.as_pinned() else { return };
            match result {
                Ok(account_id) => this.borrow().account_added(account_id),
                Err(err) => this.borrow().account_error(err.into()),
            }
        });

        runtime.spawn(async move {
            let result = rpc
                .call_unit::<u32>("add_account")
                .await
                .map_err(|err| err.to_string());
            done(result);
        });
    }

    /// Delete a profile and everything in it.
    ///
    /// Refreshes afterwards rather than trusting the caller to: the list
    /// this leaves behind is what decides whether the app still has a
    /// profile to show, and `accounts_refreshed` is how that is learnt.
    pub fn remove_account(&mut self, account_id: u32) {
        let Some((rpc, runtime)) = self.connection() else {
            self.account_error(QString::from("not started"));
            return;
        };

        let ptr: QPointer<Self> = QPointer::from(&*self);
        let done = queued_callback(move |result: Result<(), String>| {
            let Some(this) = ptr.as_pinned() else { return };
            match result {
                Ok(()) => this.borrow_mut().refresh_accounts(),
                Err(err) => this.borrow().account_error(err.into()),
            }
        });

        runtime.spawn(async move {
            let result = rpc
                .call::<_, ()>("remove_account", (account_id,))
                .await
                .map_err(|err| err.to_string());
            done(result);
        });
    }

    /// Set the download limit and apply it to every account.
    pub fn set_download_limit(&mut self, bytes: u32) {
        if self.download_limit_set && self.download_limit == bytes {
            return;
        }
        self.download_limit = bytes;
        self.download_limit_set = true;
        self.download_limit_changed();
        self.spread("download_limit", bytes.to_string());
    }

    /// Set the deletion period and apply it to every account.
    pub fn set_delete_device_after(&mut self, seconds: u32) {
        if self.delete_device_after_set && self.delete_device_after == seconds {
            return;
        }
        self.delete_device_after = seconds;
        self.delete_device_after_set = true;
        self.delete_device_after_changed();
        self.spread("delete_device_after", seconds.to_string());
    }

    /// Write one setting to whatever accounts the core has now. Nothing
    /// to apply to before the core is up, and `refresh_accounts` covers
    /// that.
    fn spread(&self, key: &'static str, value: String) {
        let Some((rpc, runtime)) = self.connection() else {
            return;
        };
        let ptr: QPointer<Self> = QPointer::from(self);
        let done = queued_callback(move |result: Result<Vec<u32>, String>| {
            let Some(this) = ptr.as_pinned() else { return };
            match result {
                Ok(ids) => this.borrow().write_config(&ids, key, value.clone()),
                Err(err) => this.borrow().core_error(err.into()),
            }
        });
        runtime.spawn(async move {
            done(account_ids(&rpc).await);
        });
    }

    /// Write every setting QML has handed over to each of these accounts:
    /// what a profile added later gets when the list is next read.
    fn apply_settings(&self, ids: &[u32]) {
        if self.download_limit_set {
            self.write_config(ids, "download_limit", self.download_limit.to_string());
        }
        if self.delete_device_after_set {
            self.write_config(
                ids,
                "delete_device_after",
                self.delete_device_after.to_string(),
            );
        }
    }

    /// Write one config value to each of these accounts. A failure is
    /// reported and the rest still written.
    fn write_config(&self, ids: &[u32], key: &'static str, value: String) {
        if ids.is_empty() {
            return;
        }
        let Some((rpc, runtime)) = self.connection() else {
            return;
        };
        let ids = ids.to_vec();
        let ptr: QPointer<Self> = QPointer::from(self);
        let failed = queued_callback(move |err: String| {
            if let Some(this) = ptr.as_pinned() {
                this.borrow().core_error(err.into());
            }
        });
        runtime.spawn(async move {
            for account_id in ids {
                if let Err(err) = rpc
                    .call::<_, ()>("set_config", (account_id, key, Some(value.clone())))
                    .await
                {
                    failed(format!("could not set {key}: {err}"));
                }
            }
        });
    }

    /// Count what a deletion period would delete now, across every
    /// account.
    pub fn estimate_auto_deletion(&mut self, seconds: u32) {
        let Some((rpc, runtime)) = self.connection() else {
            self.core_error(QString::from("not started"));
            return;
        };
        let ptr: QPointer<Self> = QPointer::from(&*self);
        let done = queued_callback(move |result: Result<u32, String>| {
            let Some(this) = ptr.as_pinned() else { return };
            match result {
                Ok(count) => this.borrow().auto_deletion_estimated(seconds, count),
                Err(err) => this.borrow().core_error(err.into()),
            }
        });
        runtime.spawn(async move {
            let result = async {
                let mut total: u64 = 0;
                for account_id in account_ids(&rpc).await? {
                    // estimate_auto_deletion_count params: account,
                    // from_server, seconds. From this device, not the
                    // server: the setting is the device's.
                    let count: u64 = rpc
                        .call(
                            "estimate_auto_deletion_count",
                            (account_id, false, i64::from(seconds)),
                        )
                        .await
                        .map_err(|err| err.to_string())?;
                    total = total.saturating_add(count);
                }
                Ok::<_, String>(u32::try_from(total).unwrap_or(u32::MAX))
            }
            .await;
            done(result);
        });
    }

    /// Repopulate [`DeltaChatCore::account_list`] from the core.
    pub fn refresh_accounts(&mut self) {
        let Some((rpc, runtime)) = self.connection() else {
            self.account_error(QString::from("not started"));
            return;
        };

        let ptr: QPointer<Self> = QPointer::from(&*self);
        let done = queued_callback(
            move |result: Result<(Vec<AccountItem>, Option<u32>), String>| {
                let Some(this) = ptr.as_pinned() else { return };
                match result {
                    Ok((items, selected)) => {
                        // Saturating: "very many" is the right answer for a
                        // has-any-configured-account check.
                        let configured_count =
                            u32::try_from(items.iter().filter(|item| item.is_configured).count())
                                .unwrap_or(u32::MAX);
                        let resume_account_id = resume_account(&items, selected);
                        let ids: Vec<u32> = items.iter().map(|item| item.account_id).collect();
                        reconcile_accounts(&mut this.borrow_mut().account_list.borrow_mut(), items);
                        this.borrow().apply_settings(&ids);
                        this.borrow()
                            .accounts_refreshed(configured_count, resume_account_id);
                    }
                    Err(err) => this.borrow().account_error(err.into()),
                }
            },
        );

        runtime.spawn(async move {
            let result = async {
                // Upstream `Account`: `kind` is "Configured"/"Unconfigured",
                // fields camelCase.
                let accounts: Vec<serde_json::Value> = rpc
                    .call_unit("get_all_accounts")
                    .await
                    .map_err(|err| err.to_string())?;
                // The profile the core has marked as selected: what the
                // chat list last told it, kept on disk, so it survives the
                // app being closed. Null when nothing was ever selected.
                // Nice to have rather than needed: a core that cannot say
                // still has a first configured profile to open on.
                let selected: Option<u32> = rpc
                    .call_unit("get_selected_account_id")
                    .await
                    .unwrap_or(None);
                let mut items: Vec<AccountItem> = accounts
                    .iter()
                    .filter_map(|account| {
                        Some(AccountItem {
                            account_id: json::u32_opt(account, "id")?,
                            display_name: json::text(account, "displayName"),
                            addr: json::text(account, "addr"),
                            is_configured: json::str_at(account, "kind") == "Configured",
                            avatar_path: json::text(account, "profileImage"),
                            color: json::text(account, "color"),
                            unread_count: 0,
                        })
                    })
                    .collect();
                // One more call per profile, for the badge on its row.
                // Only the configured ones: an unconfigured account has
                // no chats to count.
                for item in items.iter_mut().filter(|item| item.is_configured) {
                    item.unread_count = fresh_count(&rpc, item.account_id).await;
                }
                Ok::<_, String>((items, selected))
            }
            .await;
            done(result);
        });
    }

    /// Tell the core which profile is being shown.
    pub fn select_account(&mut self, account_id: u32) {
        if account_id == 0 {
            return;
        }
        let Some((rpc, runtime)) = self.connection() else {
            return;
        };
        let ptr: QPointer<Self> = QPointer::from(&*self);
        let failed = queued_callback(move |err: String| {
            if let Some(this) = ptr.as_pinned() {
                this.borrow().account_error(err.into());
            }
        });
        runtime.spawn(async move {
            if let Err(err) = rpc.call::<_, ()>("select_account", (account_id,)).await {
                failed(format!("could not remember the profile: {err}"));
            }
        });
    }

    /// Re-read one profile's unread count, after an event that could have
    /// moved it, and change its row in place. Cheap enough to do on every
    /// such event: it is one query, and the profiles page's badge is
    /// wrong until it is done.
    pub fn refresh_unread(&self, account_id: u32) {
        if account_id == 0 {
            return;
        }
        let Some((rpc, runtime)) = self.connection() else {
            return;
        };
        let ptr: QPointer<Self> = QPointer::from(self);
        let done = queued_callback(move |count: u32| {
            let Some(this) = ptr.as_pinned() else { return };
            let this_ref = this.borrow();
            let mut rows = this_ref.account_list.borrow_mut();
            // Read before the change: the iterator borrows the rows.
            let changed = rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.account_id == account_id && row.unread_count != count)
                .map(|(index, row)| (index, row.clone()));
            if let Some((index, mut row)) = changed {
                row.unread_count = count;
                rows.change_line(index, row);
            }
        });
        runtime.spawn(async move {
            done(fresh_count(&rpc, account_id).await);
        });
    }

    /// Resume IO for an already-configured account.
    pub fn start_account_io(&mut self, account_id: u32) {
        // Remembered before the call, not after it succeeds: a restart has
        // to resume whatever the app asked for, and an attempt that failed
        // against a dying server is exactly what the next one should redo.
        self.io_accounts.insert(account_id);
        let Some((rpc, runtime)) = self.connection() else {
            self.io_started(account_id, false, QString::from("not started"));
            return;
        };

        let ptr: QPointer<Self> = QPointer::from(&*self);
        let done = queued_callback(move |result: (u32, Result<(), String>)| {
            let Some(this) = ptr.as_pinned() else { return };
            let (account_id, result) = result;
            match result {
                Ok(()) => this
                    .borrow()
                    .io_started(account_id, true, QString::default()),
                Err(err) => this.borrow().io_started(account_id, false, err.into()),
            }
        });

        runtime.spawn(async move {
            let result = start_io(&rpc, Some(account_id)).await;
            done((account_id, result));
        });
    }

    /// Resume IO for every configured account.
    pub fn start_all_account_io(&mut self) {
        // Remembered before the call, for the same reason as above.
        self.io_all = true;
        let Some((rpc, runtime)) = self.connection() else {
            self.io_started(0, false, QString::from("not started"));
            return;
        };

        let ptr: QPointer<Self> = QPointer::from(&*self);
        let done = queued_callback(move |result: Result<(), String>| {
            let Some(this) = ptr.as_pinned() else { return };
            match result {
                Ok(()) => this.borrow().io_started(0, true, QString::default()),
                Err(err) => this.borrow().io_started(0, false, err.into()),
            }
        });

        runtime.spawn(async move {
            done(start_io(&rpc, None).await);
        });
    }

    /// The `dcaccount:` payload for the default chatmail server.
    pub fn default_provider_qr(&mut self) -> QString {
        QString::from(DEFAULT_PROVIDER_QR)
    }

    /// Create a profile on a chatmail server from a `dcaccount:`/`dclogin:`
    /// payload.
    pub fn create_profile(&mut self, display_name: QString, provider_qr: QString) {
        self.begin_profile(
            display_name.to_string(),
            Transport::Qr(provider_qr.to_string()),
        );
    }

    /// Create a profile backed by an existing mailbox. `addr` and
    /// `password` only; the rest of `EnteredLoginParam` autoconfigures
    /// (docs/PROJECT.md).
    pub fn create_profile_with_email(
        &mut self,
        display_name: QString,
        addr: QString,
        password: QString,
    ) {
        self.begin_profile(
            display_name.to_string(),
            Transport::Mailbox {
                addr: addr.to_string(),
                password: password.to_string(),
            },
        );
    }

    /// The shared start of both `create_profile*` methods: one attempt,
    /// which becomes the one that counts, carried out on the runtime.
    fn begin_profile(&mut self, display_name: String, transport: Transport) {
        let Some((rpc, runtime)) = self.connection() else {
            self.profile_error(QString::from("not started"));
            return;
        };
        let deadline = match self.profile_timeout {
            0 => signup::DEADLINE,
            seconds => Duration::from_secs(u64::from(seconds)),
        };
        let attempt = self.attempts.begin(deadline);
        let done = self.profile_callback(attempt.id());
        let task_runtime = runtime.clone();
        runtime.spawn(async move {
            done(
                attempt
                    .run(&task_runtime, rpc, display_name, transport)
                    .await,
            );
        });
    }

    /// Abort the running attempt, whether it is making a profile or
    /// taking one over: nothing it answers is waited for any more, and
    /// the core's process is stopped on every account an attempt is
    /// still holding -- the one being cancelled, and any
    /// earlier one the core has not let go of yet.
    pub fn cancel_ongoing(&mut self) {
        let held = self.attempts.cancel();
        let Some((rpc, runtime)) = self.connection() else {
            return;
        };
        runtime.spawn(async move {
            // Fire and forget: the UI reacts to the core's final
            // ConfigureProgress(0), not to this call's return.
            for account_id in held {
                let _ = rpc
                    .call::<_, ()>("stop_ongoing_process", (account_id,))
                    .await;
            }
        });
    }

    /// Take a profile over from the device showing `qr_text`, which is
    /// what its own "add second device" offers.
    pub fn restore_from_device(&mut self, qr_text: QString) {
        self.begin_restore(Source::Device(qr_text.to_string()));
    }

    /// Take a profile over from the backup file at `path`.
    pub fn restore_from_file(&mut self, path: QString) {
        self.begin_restore(Source::File(path.to_string()));
    }

    /// The shared start of both `restore_from_*` methods. The same
    /// bookkeeping as a signup, so that one at a time holds across both
    /// and `cancel_ongoing` stops whichever is running.
    fn begin_restore(&mut self, source: Source) {
        let Some((rpc, runtime)) = self.connection() else {
            self.restore_failed(QString::from("not started"));
            return;
        };
        let attempt = self.attempts.begin(signup::TRANSFER_DEADLINE);
        let done = self.restore_callback(attempt.id());
        let task_runtime = runtime.clone();
        runtime.spawn(async move {
            done(attempt.restore(&task_runtime, rpc, source).await);
        });
    }

    /// List the account's email transports.
    pub fn list_transports(&mut self, account_id: u32) {
        let Some((rpc, runtime)) = self.connection() else {
            self.profile_error(QString::from("not started"));
            return;
        };

        let ptr: QPointer<Self> = QPointer::from(&*self);
        let done = queued_callback(move |result: (u32, Result<String, String>)| {
            let Some(this) = ptr.as_pinned() else { return };
            let (account_id, result) = result;
            match result {
                Ok(json) => this.borrow().transports_listed(account_id, json.into()),
                Err(err) => this.borrow().profile_error(err.into()),
            }
        });

        runtime.spawn(async move {
            let result = rpc
                .call::<_, serde_json::Value>("list_transports", (account_id,))
                .await
                .map(|value| value.to_string())
                .map_err(|err| err.to_string());
            done((account_id, result));
        });
    }

    /// The completion path of `check_qr`: the classification, as the
    /// core gave it, or the error.
    fn qr_callback(&self) -> impl Fn((u32, Result<serde_json::Value, String>)) {
        let ptr: QPointer<Self> = QPointer::from(self);
        queued_callback(move |result: (u32, Result<serde_json::Value, String>)| {
            let Some(this) = ptr.as_pinned() else { return };
            let (account_id, result) = result;
            match result {
                Ok(qr) => {
                    let kind = match json::str_at(&qr, "kind") {
                        "" => "unknown".to_string(),
                        kind => kind.to_string(),
                    };
                    let payload = serde_json::to_string(&qr).unwrap_or_default();
                    this.borrow()
                        .qr_checked(account_id, kind.into(), payload.into());
                }
                Err(err) => this.borrow().qr_error(err.into()),
            }
        })
    }

    /// The shared completion path of both `create_profile*` methods:
    /// the outcome, signalled if `attempt` is still the one the reader
    /// is waiting for. A profile made for an attempt that no longer is
    /// -- the relay answered after the reader cancelled -- is removed
    /// rather than announced: nobody asked for it, and left in place it
    /// would be the profile the app opened on next time.
    fn profile_callback(&self, attempt: u64) -> impl Fn(Outcome) {
        let ptr: QPointer<Self> = QPointer::from(self);
        queued_callback(move |outcome: Outcome| {
            let Some(this) = ptr.as_pinned() else { return };
            let wanted = this.borrow().attempts.is_current(attempt);
            match outcome {
                Outcome::Created(account_id) if wanted => {
                    this.borrow().profile_created(account_id);
                }
                Outcome::Created(account_id) => {
                    if let Some((rpc, runtime)) = this.borrow().connection() {
                        runtime.spawn(async move { signup::discard(&rpc, account_id).await });
                    }
                }
                Outcome::Failed(err) if wanted => this.borrow().profile_error(err.into()),
                Outcome::TimedOut(seconds) if wanted => {
                    this.borrow().profile_timed_out(seconds);
                }
                Outcome::Failed(_) | Outcome::TimedOut(_) => {}
            }
        })
    }

    /// The completion path of both `restore_from_*` methods, with the
    /// same rule as a signup's: an answer for an attempt the reader is
    /// no longer waiting for is not theirs, and the profile it brought
    /// over is removed rather than announced.
    ///
    /// IO is started here rather than left to the page. An imported
    /// account has none running -- the import writes the database and
    /// stops -- and a profile that does not fetch is not a profile.
    fn restore_callback(&self, attempt: u64) -> impl Fn(Taken) {
        let ptr: QPointer<Self> = QPointer::from(self);
        queued_callback(move |taken: Taken| {
            let Some(this) = ptr.as_pinned() else { return };
            let wanted = this.borrow().attempts.is_current(attempt);
            match taken {
                Taken::Done(account_id) if wanted => {
                    if let Some((rpc, runtime)) = this.borrow().connection() {
                        runtime.spawn(async move {
                            let _ = start_io(&rpc, Some(account_id)).await;
                        });
                    }
                    this.borrow().profile_restored(account_id);
                }
                Taken::Done(account_id) => {
                    if let Some((rpc, runtime)) = this.borrow().connection() {
                        runtime.spawn(async move { signup::discard(&rpc, account_id).await });
                    }
                }
                Taken::Refused(reason) if wanted => {
                    this.borrow().restore_refused(reason.into());
                }
                Taken::Failed(err) if wanted => this.borrow().restore_failed(err.into()),
                Taken::Refused(_) | Taken::Failed(_) => {}
            }
        })
    }

    /// The transport and runtime, once [`DeltaChatCore::start`] completed.
    /// Callers report `None` on their own error signal.
    fn connection(&self) -> Option<(Arc<RpcClient>, CoreRuntime)> {
        Some((self.rpc.clone()?, self.runtime.clone()?))
    }

    /// Classify a QR/invite payload via the core.
    pub fn check_qr(&mut self, account_id: u32, qr_content: QString) {
        let Some((rpc, runtime)) = self.connection() else {
            self.qr_error(QString::from("not started"));
            return;
        };

        let done = self.qr_callback();

        let qr_content = qr_content.to_string();
        runtime.spawn(async move {
            let result = rpc
                .call::<_, serde_json::Value>("check_qr", (account_id, qr_content))
                .await
                .map_err(|err| err.to_string());
            done((account_id, result));
        });
    }
}

/// Bring the account rows in line with `wanted` without rebuilding them.
///
/// How many messages wait to be read across an account's chats: the
/// core's `get_fresh_msgs`, which is the figure the reference clients put
/// on their account switchers. It leaves muted chats out, as they do. An
/// account that cannot be asked counts as having nothing waiting: the
/// badge is a nicety, and the list is not worth failing over it.
/// The id of every account the core has, configured or not.
async fn account_ids(rpc: &RpcClient) -> Result<Vec<u32>, String> {
    rpc.call_unit::<Vec<serde_json::Value>>("get_all_accounts")
        .await
        .map(|accounts| {
            accounts
                .iter()
                .filter_map(|account| json::u32_opt(account, "id"))
                .collect()
        })
        .map_err(|err| err.to_string())
}

async fn fresh_count(rpc: &RpcClient, account_id: u32) -> u32 {
    rpc.call::<_, Vec<u32>>("get_fresh_msgs", (account_id,))
        .await
        .map_or(0, |ids| u32::try_from(ids.len()).unwrap_or(u32::MAX))
}

/// Start IO for one account, or with `None` for every account the core
/// has: `start_io` and `start_io_for_all_accounts`, the core's own pair.
async fn start_io(rpc: &RpcClient, account_id: Option<u32>) -> Result<(), String> {
    match account_id {
        Some(account_id) => rpc.call::<_, ()>("start_io", (account_id,)).await,
        None => rpc.call_unit::<()>("start_io_for_all_accounts").await,
    }
    .map_err(|err| err.to_string())
}

/// The profile to open on: the one the core has selected when it is a
/// configured one, else the first configured, else 0.
///
/// The selected profile is the one the chat list last told the core
/// about, and the core keeps it on disk -- so this is what brings the app
/// back to the profile it was closed on. It can point at nothing usable:
/// a profile deleted from another client, or one whose setup never
/// finished, and then the first that works is the honest answer.
fn resume_account(items: &[AccountItem], selected: Option<u32>) -> u32 {
    let usable = |item: &&AccountItem| item.is_configured;
    selected
        .and_then(|id| {
            items
                .iter()
                .filter(usable)
                .find(|item| item.account_id == id)
        })
        .or_else(|| items.iter().find(usable))
        .map_or(0, |item| item.account_id)
}

/// `reset_data` destroys every delegate, and a profile counting down to
/// its deletion *is* a delegate: Silica's remorse timer lives on the row.
/// Deleting the first of several profiles reloaded the list and tore the
/// other countdowns down with it, so only the first deletion happened and
/// the rest had to be asked for again. Rows are removed, inserted and
/// changed in place instead, so a row that is still there keeps its
/// delegate and whatever that delegate is in the middle of.
fn reconcile_accounts(rows: &mut AccountListModel, wanted: Vec<AccountItem>) {
    let wanted_ids: BTreeSet<u32> = wanted.iter().map(|item| item.account_id).collect();
    let gone: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| !wanted_ids.contains(&row.account_id))
        .map(|(index, _)| index)
        .collect();
    // Backwards, so each index still means what it did when it was found.
    for index in gone.into_iter().rev() {
        rows.remove(index);
    }
    let count = wanted.len();
    for (index, item) in wanted.into_iter().enumerate() {
        // Read before the match: the iterator borrows the rows, and the
        // arms write to them.
        let current = rows.iter().nth(index).map(|row| row.account_id);
        match current {
            Some(id) if id == item.account_id => {
                if rows[index] != item {
                    rows.change_line(index, item);
                }
            }
            _ => rows.insert(index, item),
        }
    }
    // Only a reordering leaves anything past the end: a row inserted at
    // its new place while its old one was still there.
    while rows.iter().count() > count {
        rows.remove(count);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        last_words, reconcile_accounts, restart_delay, resume_account, server_path, AccountItem,
        AccountListModel, BUNDLED_SERVER, RESTART_DELAY_MAX, RESTART_DELAY_MIN, RESTART_LIMIT,
    };

    fn account(id: u32, name: &str) -> AccountItem {
        AccountItem {
            account_id: id,
            display_name: name.into(),
            is_configured: true,
            ..AccountItem::default()
        }
    }

    fn ids(rows: &AccountListModel) -> Vec<(u32, String)> {
        rows.iter()
            .map(|row| (row.account_id, row.display_name.to_string()))
            .collect()
    }

    #[test]
    fn a_refresh_removes_inserts_and_changes_rows_where_they_stand() {
        let mut rows = AccountListModel::default();
        reconcile_accounts(
            &mut rows,
            vec![account(1, "a"), account(2, "b"), account(3, "c")],
        );
        assert_eq!(
            ids(&rows),
            vec![(1, "a".into()), (2, "b".into()), (3, "c".into())]
        );
        // One gone from the middle, one renamed, one new at the end.
        reconcile_accounts(
            &mut rows,
            vec![account(1, "ada"), account(3, "c"), account(4, "d")],
        );
        assert_eq!(
            ids(&rows),
            vec![(1, "ada".into()), (3, "c".into()), (4, "d".into())]
        );
        // A reordering still ends with exactly the wanted rows.
        reconcile_accounts(&mut rows, vec![account(4, "d"), account(1, "ada")]);
        assert_eq!(ids(&rows), vec![(4, "d".into()), (1, "ada".into())]);
        reconcile_accounts(&mut rows, Vec::new());
        assert_eq!(ids(&rows), Vec::<(u32, String)>::new());
    }

    #[test]
    fn the_app_comes_back_to_the_selected_profile_when_it_is_usable() {
        let unconfigured = AccountItem {
            account_id: 3,
            is_configured: false,
            ..AccountItem::default()
        };
        let items = vec![account(1, "a"), account(2, "b"), unconfigured];
        // What the core has selected wins, whichever position it is in.
        assert_eq!(resume_account(&items, Some(2)), 2);
        assert_eq!(resume_account(&items, Some(1)), 1);
        // Nothing selected, a profile that is gone, and one that never
        // finished setting up all fall back to the first that works.
        assert_eq!(resume_account(&items, None), 1);
        assert_eq!(resume_account(&items, Some(9)), 1);
        assert_eq!(resume_account(&items, Some(3)), 1);
        // And with nothing usable there is nothing to come back to.
        assert_eq!(resume_account(&[], Some(1)), 0);
        assert_eq!(resume_account(&items[2..], None), 0);
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_server_is_the_flag_then_the_environment_then_the_bundle() {
        assert_eq!(
            server_path(args(&["--rpc-server", "/x/server"]), Some("/env".into())),
            "/x/server"
        );
        assert_eq!(
            server_path(args(&["--rpc-server=/y/server"]), None),
            "/y/server"
        );
        // The invoker's own arguments are not a server.
        assert_eq!(
            server_path(
                args(&["-prestart", "--type=silica-qt5"]),
                Some("/env".into())
            ),
            "/env"
        );
        assert_eq!(server_path(args(&[]), None), BUNDLED_SERVER);
    }

    #[test]
    fn the_server_is_never_looked_up_on_path() {
        // A bare name would go to PATH, and whatever answered there would
        // be handed the mail password. Nothing here produces one, an
        // empty environment variable included.
        for env in [None, Some(String::new())] {
            let path = server_path(args(&["--rpc-server"]), env);
            assert!(path.starts_with('/'), "{path:?} would be looked up on PATH");
        }
    }

    #[test]
    fn last_words_keep_the_tail_and_say_nothing_for_silence() {
        assert_eq!(last_words(&[]), None);
        let lines: Vec<String> = (1..=7).map(|n| format!("line {n}")).collect();
        let words = last_words(&lines).unwrap_or_default();
        assert!(words.ends_with("line 3 | line 4 | line 5 | line 6 | line 7"));
        assert!(!words.contains("line 2"));
    }

    #[test]
    fn the_backoff_doubles_and_then_stops_doubling() {
        assert_eq!(restart_delay(0), RESTART_DELAY_MIN);
        assert_eq!(restart_delay(1), RESTART_DELAY_MIN * 2);
        assert_eq!(restart_delay(2), RESTART_DELAY_MIN * 4);
        assert_eq!(restart_delay(5), RESTART_DELAY_MAX);
        // Every later attempt waits the ceiling, and the shift that would
        // overflow a u32 does not panic.
        assert_eq!(restart_delay(RESTART_LIMIT), RESTART_DELAY_MAX);
        assert_eq!(restart_delay(u32::MAX), RESTART_DELAY_MAX);
    }

    #[test]
    fn giving_up_takes_longer_than_anything_transient() {
        let total: std::time::Duration = (0..RESTART_LIMIT).map(restart_delay).sum();
        assert!(
            total >= std::time::Duration::from_secs(180),
            "the app gives up after {total:?}, which is not long enough to \
             outlast a phone that is briefly out of memory"
        );
    }
}
