# Harbour

Postivene is packaged for [Jolla's Harbour store](https://harbour.jolla.com).
Harbour's rules are not advice: a validator failure is a guaranteed
rejection, and several of them constrain things — the package name, the
install paths, the linker flags — that are expensive to change once code
is built around them.

So they are a CI gate. `ci/harbour-check.sh` runs on every pull request
and is mandatory.

## The two checks

| | `ci/harbour-check.sh` | `sfdk check -s harbour` |
|---|---|---|
| Runs | every pull request (`ci.yml`) | when an RPM is built (`rpm.yml`) |
| Reads | the source tree | the built package |
| Needs | rpm, file, binutils, a host build | the Sailfish SDK, minutes of runner time |
| Authority | no | **yes** |

The second is Jolla's own `rpmvalidation.sh`, fetched and run against the
RPM the SDK produces by `ci/harbour-validate-rpm.sh`. It is the one that
decides. The first exists because the second cannot run on every push, and
because a rule broken in a pull request is cheaper to fix than one
discovered at intake.

It runs *after* the artifact upload, and the blockers below do not fail it:
a package Harbour would reject is still one worth putting on a phone, and a
workflow that is red for a reason nobody intends to fix this week is a
workflow people stop reading. Anything not in `ci/harbour/waivers.conf`
fails it.

They are kept honest against each other: `ci/harbour/` holds the
validator's own allow-lists, copied verbatim by
`scripts/update-harbour-rules.sh`, and the `rpm.yml` step warns if those
copies have fallen behind upstream. `ci/harbour-check.sh` reimplements the
logic around them, from `rpmvalidation.sh`.

## What the source check covers

Check IDs follow Jolla's own numbering.

**Naming** (1.1) — the `harbour-` prefix and lowercase package name;
Version digits and periods only; Release digits, underscores and periods,
*including the Release `rpm.yml` stamps onto each build*; only Harbour
architectures offered.

**Layout** (1.2) — every path in `%files`, for each device architecture,
against the four locations Harbour permits; the .desktop file and the
binary present; nothing under `/home`; no debug directories; no
world- or group-writable, setuid or setgid install modes.

**The .desktop file** (1.3) — a non-empty `Name=`; `Exec=` and `Icon=`
exactly the package name; `Type=Application`;
`X-Nemo-Application-Type=silica-qt5`; `[X-Sailjail]`, never `[Sailjail]`,
and never empty.

**Sailjail** (1.4) — only the four allowed keys; `OrganizationName` and
`ApplicationName` against their regexes and the reserved-name list; every
permission on the whitelist; `ExecDBus` agreeing with `Exec`.

**Icons** (1.5) — all four sizes present, real PNGs, pixel dimensions
matching their directory names.

**QML** (1.6) — every import against the allow-list and the blocked-prefix
patterns; no absolute-path imports; relative imports resolving inside the
installed tree; no ELF file outside the two places one may live.

**The binary** (1.6.1, 1.7) — every linked library against the allowed
list; that it links `__libc_start_main`; and that it **exports `main()` as
a dynamic symbol**. See below. The source check cannot see the *version* of
that symbol, which is what the SDK decides; only the built RPM shows it.

**RPM metadata** (1.8) — no `Vendor:`; no `Provides:`, `Obsoletes:`,
`Conflicts:`, `Recommends:`, `Suggests:`, `Supplements:` or `Enhances:`;
every `Requires:` unversioned and on the allowed list; no scriptlets or
triggers; `libsailfishapp-launcher` required if and only if the
`sailfish-qml` launcher is used; `qt5-qtdeclarative-import-xmllistmodel`
required if `QtQuick.XmlListModel` is imported.

**Runtime policy** (2.1, 2.5, 2.6) — no hardcoded `/home/nemo` or
`/home/defaultuser`; the data path the app builds spelled the same way as
the sandbox grant it depends on; nothing written to a path the package
installs.

`ci/harbour-check-selftest.sh` breaks each of these in a throwaway copy of
the tree and asserts the check names it. A gate that only ever prints
"ok" is indistinguishable from one that has stopped looking.

## What it cannot cover

Anything that needs the built package or a device. `rpm.yml`'s validator
step covers the first group; the rest is "Before submitting" below.

The sharpest example is the `__libc_start_main` version: it depends
entirely on which SDK built the binary, so no reading of the sources can
predict it, and it went unnoticed until the first real package went
through the real validator.

- The `Requires:` and `Provides:` **rpm generates** from the binary, as
  opposed to the ones the spec states.
- The RPATH (1.6.3), and the real file modes and ownership in the package.
  Linked libraries are checked, but against the *host* build — close
  enough for the Qt and C++ dependencies, which is what the rule is about.
- That the app works under Sailjail. Running it from a terminal or the
  IDE bypasses the sandbox entirely, so a missing permission does not
  surface until QA installs it. Force it: `sailjail /usr/bin/harbour-postivene`.
  Note what the sandbox is *for*: it confines user data, not the read-only
  system tree. A confined app still reads `/etc`, `/usr` and `/lib` -- it
  has to, for certificates, fonts and `/etc/hosts` -- so being able to
  attach `/etc/passwd` is expected and says nothing about whether
  confinement is on. What confinement covers is `$HOME`: the test that
  actually shows it working is failing to read *another app's* directory
  under `~/.local/share/`.
  Silica's pickers run inside the app's own process, so a file the grant
  does not cover is one the picker can offer and the app cannot open --
  which is why the attach button needs `UserDirs` and the profile picture
  needed `Pictures` *and* `MediaIndexing`.
- Whether the `Sailfish.Pickers`, `QtMultimedia`, `QtSensors`,
  `Nemo.Thumbnailer` and `Sailfish.WebView` types the conversation uses
  -- and the thumbnailer the gallery page fills its tiles from -- exist
  and behave on the target release.
  Harbour's own `allowed_qmlimports.conf` permits all five, which settles
  whether they may be used and says nothing about whether they work. The
  pickers are one page each so that a missing type costs one button
  (`qml/pages/Attach*Page.qml`), `Sailfish.Share` is named in
  `qml/share/ShareTarget.qml` alone -- loaded by a `Loader` from the
  window, so a release without it costs sharing rather than the app --
  and `Sailfish.WebView` is named in
  `WebxdcPage.qml` and `WebxdcStorePage.qml` alone for the same reason:
  the browser engine is a package of its own, and without it those pages
  break rather than the conversation. The store also hands the engine a
  frame script of its own (`qml/webxdc/catch.js`, installed with the QML
  by the spec), which is a file this package ships rather than anything
  Harbour has an opinion about. The media types are stubbed for tests in
  `tests/silica-stubs`, which proves what this app asks of them and
  nothing about what they answer.
- Everything in the quality bar QA applies by hand — no placeholder
  content, translated strings, `Theme` values rather than pixel counts,
  recoverable errors, a useful cover.

## The SDK version is a Harbour rule

Harbour requires the binary to link `__libc_start_main@GLIBC_2.34`, and the
version is the point: 2.34 is where glibc merged libpthread and libdl into
libc and re-versioned the symbol. A binary built against an older glibc
references `@GLIBC_2.17` on aarch64 and is rejected outright.

Only a 5.x SDK provides it. The first real validator run, against a package
built with 4.6.0.13, failed on exactly this while every other finding was
one of the two known blockers:

    FAIL /usr/bin/harbour-postivene -- Binary does not link to __libc_start_main@GLIBC_2.34.

`rpm.yml` therefore defaults to **5.2.0.15**, the Jolla Phone's baseline.
That is a deliberate floor, not a compromise: a binary from a newer SDK can
call symbols an older phone lacks, and this project does not support phones
older than the current one (`PROJECT.md`).

## Exporting `main()`

Harbour rejects a Silica app whose binary does not export `main()`: the
`silica-qt5` booster in mapplauncherd `dlopen()`s the binary and looks the
symbol up dynamically. C++ apps mark it `Q_DECL_EXPORT`.

Rust has no equivalent. `fn main` becomes an ordinary global symbol, which
lives only in `.symtab` — and rpmbuild strips `.symtab` on the way into
the package, so by the time Harbour looks there is nothing there.
`rust/postivene-app/build.rs` passes `--dynamic-list` at link time to put
`main` in `.dynsym`, where stripping cannot reach it.

`--dynamic-list` rather than `--export-dynamic-symbol`, which needs
binutils 2.35 and so may not exist in the SDK, or `--export-dynamic`,
which would export every symbol in the binary.

## Waivers

`ci/harbour/waivers.conf` records rules this package knowingly breaks, one
line each with a reason. Nothing belongs there that can be fixed: every
entry is a submission blocker.

A line is `<check id> <subject glob> <message glob>`. The source check
matches its findings on the id and the subject; the RPM check matches the
real validator's subject *and* message, so a waiver excuses one known
error about a path and never every error the validator might raise about
it -- a setuid bit or a dynamic link on the bundled server would still
fail.

A waiver that stops matching anything fails the check, so the file cannot
outlive what it excuses. Its entries cover the two blockers below --
removing the QtWidgets waiver is what the fix above had to do to land.

## QtWidgets, and the vendored qmetaobject

`qmetaobject-rs` builds its QML engine on `QApplication`, which comes from
QtWidgets — a library Harbour does not allow, since a Silica app is
expected to use QtGui's `QGuiApplication`. Upstream carries that
unconditionally, on the released crate and on master, with no feature to
turn it off.

Nothing here needs QtWidgets. The application object is used only for
`exec()` and `quit()`, both of which `QGuiApplication` provides. So
`third_party/qmetaobject` is upstream 0.2.10 plus
`third_party/qmetaobject.patch`: three lines, swapping the include, the
member type and the constructor.

`qttypes` separately passes `-lQt5Widgets` unconditionally, which would
record the dependency even with nothing using it. Rather than fork a
second crate for one line, `rust/postivene-app/build.rs` links with
`--as-needed`, which drops any library no symbol refers to. That only
works *because* the patch removed the last reference — with `QApplication`
still in use the library is genuinely needed and `--as-needed` keeps it.

Carrying someone else's crate in-tree is only safe while the difference is
visible, so `ci/vendor-check.sh` fetches the crates.io tarball, applies the
patch, and requires the result to match the vendored tree exactly. A stray
edit fails it; so does a patch that stops describing the tree.

The crate's own `tests/` are not vendored, and the check drops them from
both sides. Cargo never builds a dependency's tests, so they were 1,200
lines that could not run -- and CodeQL scanned them anyway and reported
seven high-severity findings in code this repository does not compile.

The same scanner reports the same kind of finding on our own code, and it
is worth knowing before spending an afternoon on it. **Every
`#[derive(QObject)]` a pull request adds draws a high-severity "Access of
invalid pointer" alert**, on the `#[derive]` line itself: the macro
generates the dispatcher and destructor Qt calls through raw pointers, and
CodeQL attributes expanded code to the macro. It follows the derive rather
than the file -- moving one from `tests/qml_media_pages.rs` into
`tests/common/mod.rs` moved the alert with it. The derives already
throughout `src/` do not report, only because CodeQL comments on lines a
pull request changed.

Nothing in the code answers it. Test scaffolding that stands in for a
`pageStack` has to be a QObject, because a page loaded on its own reads
`pageStack` from the QML context and nothing else can be put there.
Rewriting a test to avoid the derive means putting it somewhere it does
not belong, which costs more than the alert does. Dismiss it in the
Security tab, or -- for a lasting answer -- move the repository from
CodeQL's default setup to an advanced one whose configuration can exclude
`rust/postivene-shim/tests/`.

The real fix is upstream: a feature flag choosing between `QApplication`
and `QGuiApplication` would serve every Sailfish app built on qmetaobject,
all of which hit this. Until then the fork is three lines and rebases
cleanly.

## The open blockers

**Postivene cannot be submitted to Harbour today.** One rule is broken,
and it is structural: it cannot be fixed by editing anything in this
repository. (A second, an entry in the system's Settings app for the
app's own settings, was broken knowingly for a while; the entry never
made the page appear on a device, and the settings are back on a page
inside the app.)

### The bundled core

`deltachat-rpc-server` is an ELF executable, bundled at
`/usr/libexec/harbour-postivene/`, spawned as a subprocess and spoken to
over JSON-RPC on stdio. Harbour allows ELF files in exactly two places:
`/usr/bin/<NAME>`, and `*.so` under `/usr/share/<NAME>/lib/`. Neither fits
a second executable, so this is three validator errors — the path, the
binary in it, and its executable bit.

There are three ways to hold the core, and each is blocked differently:

| | Current API? | Buildable here? |
|---|---|---|
| `libdeltachat.so` (C interface) | **no — being deprecated** | yes: a C ABI spans the compiler gap |
| `deltachat-jsonrpc` crate, in-process | yes | **no**: needs Rust 1.89, the SDK ships 1.75, and Rust will not mix compiler versions |
| `deltachat-rpc-server` subprocess *(today)* | **yes — what upstream recommends** | yes |

Upstream is explicit that `libdeltachat` "is going to be deprecated and
only exists because Android, iOS and Ubuntu Touch are still using it", and
that new projects should use the JSON-RPC API. Migrating onto it to satisfy
a packaging rule would mean adopting a dying interface. The Rust-native
replacement cannot be compiled by the Rust the SDK ships, and Rust refuses
to link output from two compiler versions — this project has already met
that wall from the other side, where even the *same* 1.75 commit was
rejected for carrying a different release string (`E0514`, `BUILDING.md`).

So the remaining move is not technical. Harbour's one-executable rule
predates upstreams shipping self-contained helper binaries as the
recommended integration, and Delta Chat's is reproducibly built,
checksum-pinned (`scripts/fetch-rpc-server.sh`) and confined by the same
sandbox as the app. That is a case to put to Jolla, on the forum their
validator's README points at, rather than to engineer around.

Renaming the binary to `.so` would pass the validator. It is also
precisely what that README calls circumvention, and it says such apps are
removed from the store even after approval. Not an option.

## Before submitting

1. Build an RPM per architecture (`rpm.yml`); the validator step runs
   automatically.
2. Read every warning, not just the errors — several describe things that
   will be dropped in a future release.
3. Install on a real device and launch it as
   `sailjail /usr/bin/harbour-postivene`.
4. Exercise every permission-dependent path under the sandbox: the
   profile picture picker needs both `Pictures` and `MediaIndexing`, the
   attach tray's paper clip needs `UserDirs` for anything outside
   `~/Pictures`, playing a voice message needs `Audio`, recording one --
   and the sound on a video taken in the app -- needs `Microphone`, the
   camera page needs `Camera`, and running a webxdc app needs `WebView`.
   Send one of each kind -- photo, video, sound, document -- and open what
   arrives at the other end; take a picture, a video and a voice message
   in the app and send those too. The recorder picks a codec from what
   GStreamer offers (AAC in MP4 first); the microphone button is not shown
   at all when it finds none, which is the state the headless tests see.
   A long message is a device path too, because the keyboard is: write a
   paragraph with line breaks in it and check that the field grows to
   hold it and stops at a third of the screen rather than eating the
   chat, that the return key puts in a line break instead of sending,
   and that the notice appears once the draft passes about forty lines.
   Then turn on Settings > Messages > "Enter sends the message" and
   check the other half: the keyboard draws the key as the accept key,
   greyed while the field is empty, a press sends, and nothing of the
   line break the key used to put in is left behind in the field or in
   the message -- what Silica does with that break is Silica's, and the
   headless tests load the page with the `EnterKey` lines taken out.
   Send it, and check at the other end that its line breaks are line
   breaks, that the bubble folds it, and that View full message shows
   the whole of it -- including for a message another client sent long enough that
   the core had to cut it, which is the case the page cannot be tested
   for anywhere else.
   Deleting is a device path three times over: for the timing, for the
   look, and for the two agreeing. Delete four messages one after
   another, faster than the four seconds each waits, and check that all
   four go: the wait used to belong to the row, and deleting the message
   above a waiting row took its wait with it, so most of a run never
   went at all.
   Watch them go one at a time, in the order they were asked for, each
   as its own countdown ends. They shared one countdown once, restarted
   on every new delete, so a message's countdown would run out, the
   message would come back as though nothing had happened, and the lot
   would go together at the end. Deleting a single message looked right
   the whole time that was true, so delete two a second apart and watch
   the first one specifically.
   The look is Silica's own countdown and has to be indistinguishable
   from every other one on the phone -- the same bar, the same seconds,
   the same "Tap to cancel" -- because it *is* the platform's
   `RemorseItem`; only what it deletes is ours. Check that four of them
   at once each sit in their own row with nothing overlapping, that a
   tap on one puts that message back and leaves the others going, and
   that scrolling a waiting row out of the view and back brings the
   countdown back with the time it had left rather than a fresh one or
   none. Then delete one more and leave the chat before the wait is up
   -- it should be gone when you come back.
   The chat list, the profiles list and a group's member list all wait
   the same way and were all open to the same thing: delete two chats
   one after another, delete two profiles, remove two members, and check
   that both of each go and that each looks like the platform.
   Sharing *to* the app is a device path of its own, and the sandbox is
   half of it: share a picture from the gallery, a document from the file
   manager and a link from the browser, and check that Postivene is in
   the sheet under both of its entries, that picking a chat opens that
   chat with the file already on the attachment bar (or the text in the
   field), and that sending it works -- a file the sandbox will not let
   the app read fails here and nowhere else, which is what `UserDirs` and
   `Pictures` are for.
   A webxdc is the path nothing off-device can vouch for at all, and the
   first half of it is the setting: on a fresh install the tray has no app
   entry at all and a `.xdc` somebody sent is a paperclip row that saves
   like any other file, so check that before turning anything on. Then
   turn on Settings > Apps > "Enable webxdc apps (experimental)", go back
   to the chat without restarting -- the setting is dconf and every open
   page follows it -- and check that the entry has appeared and that the
   same `.xdc` is now the app's own card. Turn it off again and the row
   should go back to a paperclip. With it on: open the tray's app entry,
   take one from the store, send it, open it, and check
   that it draws, that a move reaches the other end and comes back, that
   its row shows what the app says about itself, and that leaving the page
   stops it -- `ss -ltn` should show no loopback port of ours afterwards.
   An app that hands a file back is the other direction: take one that
   exports (`sharer` has a download button on every file it holds), tap
   it, and check that a copy lands in Downloads where the file manager
   looks, with the notice saying so -- no chat picker, and nothing asked.
   The file is written into the cache first, so a sandbox that will not
   let the app write there fails here and nowhere else. Check the app
   itself too: it is told the file went out before anything else
   happens, so it should report success rather than an unreachable host.
   Try a big file as well as a small one -- a video, not a screenshot.
   The file goes from the socket to the disk a chunk at a time and
   nothing holds it, so there is no size limit to hit: what a big one
   costs is free storage, not memory. Check that it does not cost it
   twice -- after the notice says the copy is in Downloads, the cache
   copy under `~/.cache/postivene/postivene/webxdc/outbox/` should be
   gone, and the whole directory should be empty again next time the app
   is opened.
   An app that will not open says why rather than drawing grey: the host
   answers with the core's own reason, the engine's error page is left
   alone rather than turned back, and the reason stays on the screen
   after the banner has cleared itself.
   Keeping a copy of an attachment is the other half of `UserDirs`: save
   a picture, a video and a document from a message's menu and find all
   three where the platform's own folders are -- Pictures, Videos,
   Downloads -- and then open the document from the file manager, which
   is what the copy is for.
   The media pages behind a contact's or a group's tiles are the same
   two paths once more, on a grid: long-press a picture in the gallery
   and check that the menu opens under its row of cells (Silica's
   `GridItem`, which no other page uses), that Delete counts down over
   the cell and the picture goes when the count ends, and that Show in
   chat lands on the message in the conversation, lit, rather than at
   the newest one or wherever the chat was left.
   The first screen is a device path because the phone's own colours
   are: on a fresh install, before a profile, it is a field of faces
   filling the screen with the words in a cleared box in the middle,
   and the faces must be in the ambience's own colours -- grey in its
   primary, a few lit in its highlight, the way the cover draws whoever
   has written. Change the ambience with the app open and check the
   field follows it; try a light ambience, where the faces have to be
   dark on light rather than vanish. Turn the phone and check it fills
   the screen on its side too, with the middle still clear, and that
   nothing stutters on the way in: the field is one picture and one
   shader (`components/FaceField.qml`), and the headless tests can load
   it but cannot see it drawn.
   The two ways on from that screen are worth walking once each. "Tell
   me about Delta Chat" is five facts swiped through one at a time
   (`pages/IntroPage.qml`): check that each picture is in the
   ambience's colours as the field is, with its accents in the
   highlight, that the dots below follow the swipe, and that swiping
   past the last fact lands in the setup screen rather than rubber
   banding back. "Set up my profile" is that same setup screen, the
   field behind it unchanged; "I already have a profile" has nothing
   behind it yet and must say so rather than doing nothing.
   Adding a profile is the other half of that screen, and a relay that
   does not answer is the case worth trying, since a public relay is
   somebody's spare-time server. Type a custom server that does not
   exist and tap Create: the progress bar names it; after four seconds
   the line about volunteers' relays appears under Cancel; and after
   thirty the page gives up on its own, saying which relay did not
   answer and in how long, with the hint still there and Back under it.
   Then go back, pick a relay from the list, and check that the profile
   is made -- that retry used to fail with "There is already another
   ongoing process running", the core still being on the first relay,
   and now takes a fresh account (`signup.rs`). Cancel during a wait
   should go back at once, and a profile the first relay makes after
   all must not appear in the profiles list.
5. Delete the cache directory while the app runs; confirm nothing breaks.
6. Kill `deltachat-rpc-server` from a terminal while the app is open. The
   banner should say it is reconnecting and then clear itself, and messages
   should keep arriving afterwards -- the app starts a replacement and
   resumes IO on it (`PROJECT.md`). Nothing else in this list exercises
   that, and it is the failure a phone produces on its own by reclaiming
   memory.
7. Confirm **Version** was bumped, not just Release. Harbour refuses an
   update that does not sort higher than the one in the Store, and a
   Release-only bump is the most common avoidable resubmission.
8. Set "From OS version" to 4.5.0 on the submission form. The spec cannot
   say so — `sailfish-version` is not an allowed dependency, and a
   versioned one would be rejected twice over — but the `[X-Sailjail]`
   section needs it.
