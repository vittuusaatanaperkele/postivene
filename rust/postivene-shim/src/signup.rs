//! Making a profile, or taking one over: one attempt at a time, each
//! given a deadline.
//!
//! Two ways in. A profile can be *made*, on a chatmail relay or a mailbox
//! of the reader's own, which is the long transport call described below.
//! Or an existing one can be *taken over* from somewhere else -- another
//! device holding it, over the local network, or a backup file -- which
//! is the core's import, `restore` below. Both end with an account this
//! device can write from, both are one at a time, and both can be given
//! up on, so they share the bookkeeping.
//!
//! An attempt is an account for the profile to be built on, the display
//! name, and then the core's one transport call -- which asks the relay
//! for an address (or tries the mailbox it was given), stores what comes
//! back and starts IO. That last call is the long one, and the length of
//! it is the core's: a relay that does not answer holds the call for as
//! long as the core's own connection attempts take, which is minutes.
//!
//! Two things follow from that, and both were met on a phone.
//!
//! An attempt has to be given up on. A relay that has not answered in
//! [`DEADLINE`] is not going to, so the attempt stops the core's process
//! and says so (`profile_timed_out`); the page has had a Cancel button
//! all along and, since the fourth second, its suggestion to try another
//! relay. The call's own limit is the client's minute; this is shorter,
//! and it is what the reader sees.
//!
//! An attempt given up on is still running in the core. The core allows
//! one ongoing process per account, and the account stays unconfigured
//! until that process ends -- so a retry that picked "the unconfigured
//! account" back up, as the first retry did, was refused with "There is
//! already another ongoing process running". The accounts an attempt is
//! still holding are kept here (`busy`), and a retry takes a fresh one.
//! And an attempt given up on can still succeed -- the relay answers
//! late, after the reader cancelled -- and its result is not the
//! reader's any more: the profile it made is removed, and nothing is
//! signalled for it.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deltachat_jsonrpc::RpcClient;

use crate::json;
use crate::runtime::CoreRuntime;

/// How long a relay is given, in all. Long enough for a slow connection
/// to be asked for an address and then logged into twice; short enough
/// that a relay which is down is reported as such while the reader is
/// still looking at the page.
pub const DEADLINE: Duration = Duration::from_secs(30);

/// What a profile is reached through: a chatmail relay's `dcaccount:`
/// payload, or a mailbox of the reader's own.
pub(crate) enum Transport {
    /// `add_transport_from_qr`: the relay mints the address.
    Qr(String),
    /// `add_or_update_transport` with the two fields of an
    /// `EnteredLoginParam` that cannot be autoconfigured.
    Mailbox {
        /// The address.
        addr: String,
        /// Its password.
        password: String,
    },
}

/// Where a profile taken over from elsewhere comes from.
pub(crate) enum Source {
    /// `get_backup`: the code another device shows while it offers
    /// itself, and then a transfer over the local network.
    Device(String),
    /// `import_backup`: a backup file the reader points at.
    File(String),
}

/// How taking a profile over ended.
pub(crate) enum Taken {
    /// The profile is on this device, as `account_id`.
    Done(u32),
    /// A reason of this app's own, for the page to put into the reader's
    /// language: `not-a-backup`, `too-new` or `stalled`.
    Refused(&'static str),
    /// The core refused, in its own words.
    Failed(String),
}

/// How long a transfer is given. Not a deadline the reader should ever
/// meet: a backup is megabytes over a local network, and the page shows
/// it arriving the whole way. It is here so that a transfer whose other
/// end went away ends by itself rather than leaving a progress bar up
/// for the rest of the day.
pub const TRANSFER_DEADLINE: Duration = Duration::from_secs(30 * 60);

/// How an attempt ended.
pub(crate) enum Outcome {
    /// The profile is made and `account_id` has a working transport.
    Created(u32),
    /// The core refused, with its own words.
    Failed(String),
    /// The relay did not answer in this many seconds.
    TimedOut(u32),
}

/// The attempts there have been, shared with each attempt's tasks.
///
/// Held by the core object and read from its thread; the tasks share it
/// through the `Arc`, so the counters are atomics and the set is under a
/// lock that is never held across an await.
#[derive(Default)]
pub(crate) struct Attempts {
    /// The attempt whose result the reader is waiting for: the latest
    /// begun, or 0 once cancelled.
    current: AtomicU64,
    /// How many have begun, which is where the next id comes from.
    begun: AtomicU64,
    /// Accounts whose transport call has not returned yet.
    busy: Mutex<BTreeSet<u32>>,
}

impl Attempts {
    /// Start an attempt, which becomes the one that counts.
    pub(crate) fn begin(self: &Arc<Self>, deadline: Duration) -> Attempt {
        let id = self.begun.fetch_add(1, Ordering::SeqCst) + 1;
        self.current.store(id, Ordering::SeqCst);
        Attempt {
            id,
            shared: self.clone(),
            deadline,
            wanted: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The reader gave up: no attempt counts any more. The accounts
    /// still held are returned, for the caller to stop the core's
    /// process on each.
    pub(crate) fn cancel(&self) -> Vec<u32> {
        self.current.store(0, Ordering::SeqCst);
        self.busy().iter().copied().collect()
    }

    /// Whether `attempt` is still the one the reader is waiting for.
    pub(crate) fn is_current(&self, attempt: u64) -> bool {
        self.current.load(Ordering::SeqCst) == attempt
    }

    fn busy(&self) -> std::sync::MutexGuard<'_, BTreeSet<u32>> {
        self.busy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The account is nobody's now.
    fn release(&self, account_id: u32) {
        self.busy().remove(&account_id);
    }
}

/// One attempt at making a profile.
pub(crate) struct Attempt {
    id: u64,
    shared: Arc<Attempts>,
    deadline: Duration,
    /// Cleared when the deadline passes. The call may still complete
    /// after that, and what it made is then not wanted.
    wanted: Arc<AtomicBool>,
}

impl Attempt {
    /// Which attempt this is, for the completion path to compare.
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// Do the work: an account, the name, the transport -- the last
    /// within the deadline.
    pub(crate) async fn run(
        &self,
        runtime: &CoreRuntime,
        rpc: Arc<RpcClient>,
        display_name: String,
        transport: Transport,
    ) -> Outcome {
        let account_id = match self.pick_account(&rpc).await {
            Ok(account_id) => account_id,
            Err(err) => return Outcome::Failed(err),
        };
        // Before the transport call, so the name is in place when the
        // core announces the account. Cancelled meanwhile: nothing is
        // waiting for the answer, so the relay is not asked.
        let prepared = set_display_name(&rpc, account_id, display_name).await;
        if prepared.is_err() || !self.shared.is_current(self.id) {
            self.shared.release(account_id);
            return prepared.map_or_else(Outcome::Failed, |()| {
                Outcome::Failed("cancelled".to_string())
            });
        }

        // A task of its own, so that giving up on it here leaves it to
        // finish: the account is held until the core answers, whenever
        // that is, and a late success is cleaned up by the task itself.
        let call = runtime.spawn(transport_call(
            rpc.clone(),
            self.shared.clone(),
            self.wanted.clone(),
            account_id,
            transport,
        ));
        match tokio::time::timeout(self.deadline, call).await {
            Ok(Ok(Ok(()))) => Outcome::Created(account_id),
            Ok(Ok(Err(err))) => Outcome::Failed(err),
            Ok(Err(err)) => Outcome::Failed(err.to_string()),
            Err(_) => {
                self.wanted.store(false, Ordering::SeqCst);
                let _ = rpc
                    .call::<_, ()>("stop_ongoing_process", (account_id,))
                    .await;
                Outcome::TimedOut(u32::try_from(self.deadline.as_secs()).unwrap_or(u32::MAX))
            }
        }
    }

    /// Take a profile over from elsewhere: an account to put it on, the
    /// code read first if there is one, and then the core's import --
    /// which reports itself in `ImexProgress` events the whole way.
    ///
    /// A failed import leaves the account behind, so it is removed
    /// rather than released: unlike a refused signup, which leaves a
    /// clean unconfigured account for the next attempt to reuse, a
    /// half-written one is good for nothing.
    pub(crate) async fn restore(
        &self,
        runtime: &CoreRuntime,
        rpc: Arc<RpcClient>,
        source: Source,
    ) -> Taken {
        let account_id = match self.pick_account(&rpc).await {
            Ok(account_id) => account_id,
            Err(err) => return Taken::Failed(err),
        };
        if let Source::Device(qr) = &source {
            if let Err(reason) = from_a_device(&rpc, account_id, qr).await {
                self.shared.release(account_id);
                return Taken::Refused(reason);
            }
        }
        // A task of its own, as the transport call is, so that giving up
        // on it here leaves it to finish and clean up after itself.
        let call = runtime.spawn(import_call(
            rpc.clone(),
            self.shared.clone(),
            self.wanted.clone(),
            account_id,
            source,
        ));
        match tokio::time::timeout(self.deadline, call).await {
            Ok(Ok(Ok(()))) => Taken::Done(account_id),
            Ok(Ok(Err(err))) => {
                discard(&rpc, account_id).await;
                Taken::Failed(err)
            }
            Ok(Err(err)) => {
                discard(&rpc, account_id).await;
                Taken::Failed(err.to_string())
            }
            Err(_) => {
                self.wanted.store(false, Ordering::SeqCst);
                let _ = rpc
                    .call::<_, ()>("stop_ongoing_process", (account_id,))
                    .await;
                Taken::Refused("stalled")
            }
        }
    }

    /// The account the profile is built on: an unconfigured one no
    /// attempt is still holding, else a fresh one. Reuse keeps a failed
    /// signup from stranding an account per retry; the holding check
    /// keeps a retry off an account whose configure is still running.
    async fn pick_account(&self, rpc: &RpcClient) -> Result<u32, String> {
        let accounts: Vec<serde_json::Value> = rpc
            .call_unit("get_all_accounts")
            .await
            .map_err(|err| err.to_string())?;
        {
            let mut busy = self.shared.busy();
            let free = accounts
                .iter()
                .filter(|account| json::str_at(account, "kind") == "Unconfigured")
                .filter_map(|account| json::u32_opt(account, "id"))
                .find(|id| !busy.contains(id));
            if let Some(account_id) = free {
                busy.insert(account_id);
                return Ok(account_id);
            }
        }
        let account_id = rpc
            .call_unit::<u32>("add_account")
            .await
            .map_err(|err| err.to_string())?;
        self.shared.busy().insert(account_id);
        Ok(account_id)
    }
}

/// The transport call, and the account's release once it has returned.
/// A success nobody is waiting for any more is removed here: the reader
/// gave up on it, and a profile they did not ask for must not be what
/// the app opens on next time.
async fn transport_call(
    rpc: Arc<RpcClient>,
    shared: Arc<Attempts>,
    wanted: Arc<AtomicBool>,
    account_id: u32,
    transport: Transport,
) -> Result<(), String> {
    let result = match transport {
        Transport::Qr(qr) => {
            rpc.call::<_, ()>("add_transport_from_qr", (account_id, qr))
                .await
        }
        Transport::Mailbox { addr, password } => {
            let param = serde_json::json!({ "addr": addr, "password": password });
            rpc.call::<_, ()>("add_or_update_transport", (account_id, param))
                .await
        }
    }
    .map_err(|err| err.to_string());
    shared.release(account_id);
    if result.is_ok() && !wanted.load(Ordering::SeqCst) {
        discard(&rpc, account_id).await;
    }
    result
}

/// The import call, and the account's release once it has returned. A
/// profile taken over after the reader gave up is removed here, for the
/// same reason a late signup's is: nobody asked for it, and left in
/// place it would be the profile the app opened on next time.
async fn import_call(
    rpc: Arc<RpcClient>,
    shared: Arc<Attempts>,
    wanted: Arc<AtomicBool>,
    account_id: u32,
    source: Source,
) -> Result<(), String> {
    let result = match source {
        Source::Device(qr) => rpc.call::<_, ()>("get_backup", (account_id, qr)).await,
        Source::File(path) => {
            rpc.call::<_, ()>("import_backup", (account_id, path, Option::<String>::None))
                .await
        }
    }
    .map_err(|err| err.to_string());
    shared.release(account_id);
    if result.is_ok() && !wanted.load(Ordering::SeqCst) {
        discard(&rpc, account_id).await;
    }
    result
}

/// Whether the code read is one another device is offering a profile
/// with. Asked before the transfer is started on it: the core's own
/// answer to a code that is not one is about protocols, and the reader
/// is standing there with a camera.
async fn from_a_device(rpc: &RpcClient, account_id: u32, qr: &str) -> Result<(), &'static str> {
    let checked: serde_json::Value = rpc
        .call("check_qr", (account_id, qr))
        .await
        .map_err(|_| "not-a-backup")?;
    match json::str_at(&checked, "kind") {
        "backup2" => Ok(()),
        // The other device runs a newer Delta Chat than this core can
        // read a backup from, which is worth saying as such.
        "backupTooNew" => Err("too-new"),
        _ => Err("not-a-backup"),
    }
}

/// Remove a profile that was made after the reader gave up on it.
pub(crate) async fn discard(rpc: &RpcClient, account_id: u32) {
    let _ = rpc.call::<_, ()>("remove_account", (account_id,)).await;
}

/// Set the display name.
async fn set_display_name(
    rpc: &RpcClient,
    account_id: u32,
    display_name: String,
) -> Result<(), String> {
    rpc.call::<_, ()>(
        "set_config",
        (account_id, "displayname", Some(display_name)),
    )
    .await
    .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_attempt_is_the_one_that_counts() {
        let attempts = Arc::new(Attempts::default());
        let first = attempts.begin(DEADLINE);
        assert!(attempts.is_current(first.id()));
        let second = attempts.begin(DEADLINE);
        assert!(!attempts.is_current(first.id()));
        assert!(attempts.is_current(second.id()));
        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn a_cancel_leaves_no_attempt_current_and_names_what_is_held() {
        let attempts = Arc::new(Attempts::default());
        let attempt = attempts.begin(DEADLINE);
        attempts.busy().insert(7);
        attempts.busy().insert(3);
        assert_eq!(attempts.cancel(), vec![3, 7]);
        assert!(!attempts.is_current(attempt.id()));
        // Cancelling stops the waiting, not the holding: the core's
        // process is still running on those accounts.
        assert_eq!(attempts.busy().len(), 2);
        attempts.release(7);
        assert_eq!(attempts.busy().iter().copied().collect::<Vec<_>>(), vec![3]);
    }
}
