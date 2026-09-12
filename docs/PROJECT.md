# Postivene

*A native SailfishOS client for [Delta Chat](https://delta.chat).*

## What this is

Delta Chat is a chatmail messenger with end-to-end encryption, no phone
number and no central operator.

The thesis is narrow: **do not build a messenger, build a SailfishOS UI on
top of one.** Protocol, cryptography and storage are the upstream core's.
Postivene contributes the presentation layer, the platform integration and
the packaging, and aspires to ships to Jolla's Harbour app store.

## What this isn't

- **Reimplementing any protocol logic** — no IMAP/SMTP/MIME, no Autocrypt,
  no encryption. If protocol code is being written, the core dependency is
  being misused. This is the most important boundary in the project.
- **Hand-written C FFI bindings.** The CFFI exists; JSON-RPC is the
  sanctioned, lower-maintenance path.
- **A push-notification service.** No Delta Chat push infrastructure is
  available to third-party clients.
- **Plain-email chats.**
- **Multi-protocol bridging**, a **desktop or web build**, and **running a
  chatmail server**. Single-purpose client only.
- **Old Sailfish releases.** One modern baseline, expanded only for future
  Jolla products. [Buy a Jolla Phone 2026](https://commerce.jolla.com/) and
  support European-made alternatives. 👊🇪🇺🔥
- **Shipping via OpenRepos/Chum.** We need to improve this platform and get
  it to a point where it is competitive. That means appealing to a broad
  majority of regular, non-technical people. That also means no developer mode,
  no SSH'ing to fix small things, no community repos. Most importantly, that
  means [dogfooding](https://en.wikipedia.org/wiki/Eating_your_own_dog_food).
  Lots and lots of dogfooding - and nagging Jolla about the things that the 
  platform is still missing. This is the way.

## Architecture

```
QML / Silica UI
        |  models / signals
Rust shim (qmetaobject-rs): JSON-RPC client, event loop -> Qt queued
signals, QAbstractListModel adapters for chats/messages/accounts
        |  JSON-RPC over stdio
deltachat-rpc-server (bundled binary, subprocess) = the entire core
```

- **The shim spawns the server as a subprocess.** This keeps the integration
  surface small and stable, and mirrors the desktop client's own migration
  away from CFFI. The OpenRPC spec is the interface contract.
- **Core events run off the main thread**, marshalled to the Qt main thread
  via queued signals.
- **A webxdc app is served, not unpacked.** All of this is behind one
  setting, off until it is asked for: "Enable webxdc apps
  (experimental)" on the settings page, `webxdc_enabled` in dconf. Off,
  the tray has no app entry, the store behind it is unreachable, and a
  `.xdc` somebody sent is drawn as the file it is and handed on by a tap
  -- the row a `.xdc` the core could not read as an app already lands on.
  The gate is three bindings on that one value rather than a check at
  each door: the pages read it and hand it down as a property, so the
  tray entry and the row simply are not there to tap, and nothing has to
  be kept in step. It is off by default because an app is somebody
  else's code and this is the newest part of the app; it is one switch
  rather than a build flag because the reader is the one who gets to
  decide that.
  An app somebody sent is a zip
  with an index.html in it, and the core reads the archive
  (`get_webxdc_blob`). So the shim puts one app on a loopback address of
  its own while it is open and answers every request out of the core
  (`webxdc_host.rs`); the `WebView` is pointed at that address and needs
  nothing else. The app sits at the root of it, as every other client
  serves one: an app built with a bundler's default settings asks for
  `/assets/index-1a2b.js`, so a host that keeps the app's files under a
  prefix draws a blank screen for every app that does not happen to use
  relative paths. What the token guards is the chat, not the files --
  the two API paths are under `/webxdc-api/<token>/`, and the files were
  sent to this reader anyway. The same host carries that API --
  `sendUpdate` is a POST, the updates from everyone else are a poll -- so
  the bridge is not a Gecko frame script, and no archive format is parsed
  here.
  An app can hand a file back the other way. The call is `sendToChat`
  and a chat is the only destination its name can carry, but the button
  an app draws over it is a *download* -- `sharer`'s is a download arrow
  -- and what a reader means by that is the file, on their phone. So the
  host writes what the app gives it into the cache and raises it on the
  page, and the page saves a copy into Downloads and says so. A chat is
  not a destination, and neither is a question: a chooser between
  opening and keeping was tried and was two taps in front of the one
  thing the reader had already asked for. Text with no file has nowhere
  to be saved and goes on the clipboard instead, which is still an
  answer.
  The file *is* the request body -- no base64, no JSON around it, its
  name and any words in the query -- and the host copies it from the
  socket into the cache a chunk at a time. It was JSON with the file
  base64 inside it at first, and that held the whole of it three times
  over between the two ends: what a file worth exporting is, is exactly
  the size that cannot afford it. There is no cap left at all. The one
  that outlived the base64 was a number standing in for a cleanup that
  did not exist -- nothing emptied the outbox, so every export left a
  second copy in the cache for good, and `deltachat-android` has the
  same leak in a worse place (its blobs go to the app's data directory,
  which the system will not reclaim). The cleanup is what was missing:
  the page deletes the cached copy once the file is saved, and starting
  an app empties its outbox, which is the one moment none of its own
  handovers can be in flight. What bounds a file now is the disk, and a
  write with no room for it is a `500` the app can show.
  Two other things about it are deliberate. The app's request is
  answered *before* the page is told, and only if that answer got out.
  And the page keeps serving an app it has opened a page over: stopping
  the app whenever the page deactivated stopped it in the middle of the
  very request that asked, so leaving is destruction, which is what a
  popped page and a replaced stack both are.
  A body this host will not take is read and dropped before it is
  refused. Answering and closing on a client still writing resets the
  connection, and a reset is not an answer: the app reported a host it
  could not reach and had no idea why. What has already come off the
  socket is counted, so a half-read body is not waited on twice -- that
  wait is for bytes the other end has already sent.
  Where a new app comes from is the store, a website
  (`WebxdcStorePage.qml`); a tap on a link to a `.xdc` is caught before
  the engine can download it and fetched through the core instead
  (`get_http_response`), which is deltachat-android's shape too. It
  catches the tap where the tap happens: `qml/webxdc/catch.js` is a frame
  script loaded into the engine's own world, and it stops the click and
  sends the address back. deltachat-android decides every navigation in
  `shouldOverrideUrlLoading`; this `WebView` has no such hook, and
  watching where the view goes is not one -- a `.xdc` is a download, and
  a download is not a navigation, which is why the first version of that
  page did nothing on a phone.
- **The app is in the phone's share sheet, and installs nothing to be
  in it.** A picture from the gallery or a link from the browser can be
  shared to Postivene because the desktop entry says so --
  `X-Share-Methods`, and a group per method saying what it is called and
  what it takes -- and because a `ShareProvider` of the same name is
  running in the app (`qml/share/ShareTarget.qml`). The older way, a
  transfer-engine plugin, is a `.so` in a system directory and is not
  open to a Harbour package; this is, and `tests/qml_syntax.rs` checks
  that the names in the two files still agree. What arrives goes to the
  window, which asks which chat it is for and opens that chat with the
  file on its attachment bar or the text in its field.
- **The `WebView`'s own bindings are left alone.** Silica's `WebView.qml`
  decides when the engine renders from the page's status and whether the
  app is in front. Overriding `active` cost a device build: the view was
  never activated by the page transition and drew as a grey rectangle.
  `tests/qml_syntax.rs` keeps it that way. Where the view is pointed is a
  plain `url:` binding for the same sort of reason -- the store page,
  which draws, has always had one, and the app page, which did not, was
  pointed from a signal handler instead.
- **A `WebView` that draws nothing says why.** Nothing about the browser
  engine can be tested off a phone, so a failure is put where the app
  would have been: the host answers a refused blob with the core's own
  reason rather than an empty body, and the page keeps that reason on
  the screen. The banner clears itself after a few seconds, which is
  right for something that happened and wrong for a view that never drew
  anything.
- **A bubble holds a remark; anything longer gets a page.** A message
  over a dozen lines is folded in the conversation, with View full
  message under it: drawn whole, somebody's to-do document fills the
  screen, pushes the chat out of it and leaves a row nobody can scroll
  past. The fold is a cap on the label's lines rather than a cut in the
  text, so nothing has to slice a rendering in half and leave a tag
  open. Opening one out in place was offered beside the page and is not
  any more: it was a second way to read the same words and the worse of
  the two, because what it made was exactly the row nobody can scroll
  past, and folding it again had to put the reader back where they had
  been by hand -- a timer, an index, and a view asked to show a row it
  still thought was tall. And past a length the core does not carry a
  message whole at all: it cuts the body and puts the rest in an HTML
  part, so the whole of such a message is only behind
  `get_message_html` -- read as words,
  never rendered as markup (`html.rs`), for the reason every label in
  the app is pinned to plain text. A newline in that part is *not* a
  line break: in HTML it is whitespace, and the core writes each line of
  the message as `line<br/>` with a newline after the tag, so a reader
  that counts both puts a blank line between every line. Whitespace
  between the markup is collapsed the way a browser collapses it, and
  the tag is the break; two `<br/>` in a row are still two, so a blank
  line the reader typed survives as one. The fake core's fixture is
  written in that shape for the same reason -- it was one unbroken line,
  and every long message reached the phone double-spaced with nothing in
  the suite to notice. The offer belongs to a body: an attachment with
  no caption has none to read on a page, and a message the core is still
  holding back has none of it here yet -- neither was excluded at first,
  and an attachment arriving in an open chat grew two words of chrome it
  had no use for. The same rule read from the other end is the notice
  above the field while a long message is being written, which is where
  parla puts its own.
  What the renderer emits for a line break is `<br>` and not a newline:
  in `Text.StyledText` a newline is whitespace, so a body joined with
  newlines is drawn as one running paragraph and nothing is ever long
  enough to fold. Both went unnoticed until a phone drew a to-do list as
  a sentence; `qml_message_lines.rs` now measures what Qt makes of each
  shape rather than trusting a reading of it.
- **A wait before something is destroyed belongs to the list, not the
  row.** `ListItem.remorseAction` is Silica's shortcut: it makes a
  `RemorseItem` in the row and hands it the action. Deleting a handful of
  messages one after another lost most of them on a phone, and moving the
  action off the row fixed it.
  Why it lost them is not established, and the obvious explanation is
  wrong: `RemorseItem` runs its callback rather than dropping it when the
  countdown is cut short, both when it is destroyed and when its page
  deactivates. So "the row went and took the countdown with it" is not
  the mechanism, whatever is. What is certain is that deleting out of a
  list destroys rows -- the first delete lands, the core says so, the row
  goes -- and that the chat list is churned harder still, since a message
  arriving reorders it, which is a remove and an insert. A wait that
  lives somewhere that volatile has to be proved every release; a wait
  that lives beside the list does not.
  So the two halves are kept apart. The *action* is a `PendingRemoval`
  beside each list, emptied as whoever holds it is left -- the
  conversation from its page's `Deactivating`, the other three from
  their own -- for the reason ConversationPage writes its draft there:
  leaving is exactly when a timer has not fired yet.
  Every id waiting carries its own deadline and goes on it, with the
  timer armed for whichever is soonest. One countdown shared between
  them was tried and was wrong on a phone: it had to be restarted
  whenever another delete was asked for, so the first message's drawn
  countdown ran out, the platform put the message back as though nothing
  had happened, and everything went together when the last one ended.
  One delete on its own looked right, which is why it took a phone to
  see it. The two clocks -- the drawn one and the deleting one -- are
  tied by `countdownFor(id)`, the only length a row may hand
  `RemorseItem.execute`, and `qml_syntax.rs` counts that every raised
  countdown asked for its length rather than choosing one.
  The *look* is still Silica's own
  `RemorseItem`, raised by the row over what is going and handed a
  callback that does nothing: the bar, the seconds, "Tap to cancel", and
  the fade over the row, all of it the platform's, because a reader
  already knows what a countdown looks like here. A hand-drawn stand-in
  was tried and was wrong twice over -- it hid the row's content, which
  collapsed the row, and even fixed it did not look like the phone.
  Raising it needs two things Silica's shortcut does for you: the row
  puts the countdown up again when it is rebuilt mid-wait
  (`PendingRemoval.remaining`), and nothing else may fade or hide what
  the countdown covers, since `RemorseItem` does that itself with an
  `opacity: 0.0` on the item it was handed. `qml_syntax.rs` holds every
  list to all of it, and the stub `ListItem` no longer has a
  `remorseAction` for anything to reach for. The profiles list keeps its
  in-place refresh (`core.rs`) as well, which is what stops every row
  flickering when one profile goes, but nothing depends on it any more.
- **A file is opened elsewhere or kept; reading belongs to messages.** A
  picture and a video have pages of their own and everything else is
  handed to the system. A page for a file was built and taken out again
  -- it named the file, showed it when it was text, and offered to open
  or save it -- because the reader whose problem it answered said there
  should be no such thing. What an attachment needs and a tap cannot
  give is a copy, and that is on the row's menu, beside Open.
- **A message of one's own can be reworded, and the phone can forget
  old ones.** Both are the core's: an edit is `send_edit_request`, which
  changes the text at every end and marks the message `isEdited`, and
  the footer says "Edited" as the reference clients' footers do. The
  menu offers Edit on what the core would take an edit of, which is the
  rule deltachat-android applies before offering it -- a message of
  one's own, not a notice, not a call, with text to change, not one the
  sending core cut -- and only in a chat that takes messages and is
  encrypted, which the conversation model asks the core about beside
  the chat's kind. Editing is a mode of the field: it holds the
  message's text, the bar above it says which message, the attach tray
  steps aside, and send means keep the change. The reader's unsent
  draft is put aside for the edit and put back afterwards; the
  reference clients throw it away, which is a loss with no reason
  behind it. Deleting old messages is the core's `delete_device_after`,
  one setting for every profile like the download limit, applied to
  every chat whatever that chat's own disappearing-messages timer says
  and never to "Saved messages". It deletes the moment it is set, so
  the settings page asks the core how many messages that is
  (`estimate_auto_deletion_count`) and puts the number to the reader on
  a page of its own, with a switch they have to turn before accept means
  anything -- the checkbox deltachat-android and deltachat-ios put on
  the same question. A picture or a video has a page of its own with
  Open and Save on its pull-down, so the message menu no longer offers
  either for those: two ways to the same two things was one too many.
- **A muted group stays quiet, except for a reply to the reader.** The
  chat list decides what is announced and never announces a muted chat;
  the one exception is what the reference clients call a mention, and
  it is on by default as they have it: a message in a muted *group*
  that quotes one of the account's own messages. The core does not say
  so on the event, and the quote names its message and nothing else
  about it, so the list reads the message and the one it quotes
  (`chatlist.rs`, `is_mention`). The answer lands after the refresh the
  event started has usually left the chat unannounced, so a mention
  starts a refresh of its own with the chat marked to pass the mute
  once -- the announcement then carries the row's preview like every
  other. A muted one-to-one chat is not a group: it was muted with the
  one person in it in mind.
- **What is made on the phone is made by the platform.** A picture or a
  video comes from QML's `Camera`; a voice message from `QAudioRecorder`,
  which QML on Qt 5.6 does not offer and the shim reaches through the
  tree's second `cpp!` block (`BUILDING.md`). Either waits in the app's
  cache directory until the core has copied it, and is sent as any other
  file -- a voice message with the core's `Voice` view type, the one kind
  the core has to be told.
- **What a chat holds besides words is listed by kind, off the core's own
  index.** The contact's and the group's page carry a row of tiles --
  Gallery, Audio, Files, and Apps where apps are on -- and each opens a
  page of that kind (`ChatMediaPage.qml`), which is where the reference
  clients keep theirs. The index is the core's `get_chat_media`: up to
  three view types in one call, and that limit is what shapes the four
  pages -- pictures, GIFs and videos; music and voice messages; files and
  shared contacts; apps. It answers oldest first and says not to re-sort
  it, so the model (`chat_media.rs`) turns the list round and does
  nothing else to its order. The rows are messages in the conversation's
  own shape, read in the conversation's own pages of fifty, but from the
  top down without waiting to be asked: every row stands as a
  placeholder from the moment the ids are in, and the first screen is
  filled before the rest have been read. What has been read is kept
  across a reload, so a picture arriving while the page is open is one
  row fetched and nothing moved. The gallery's tiles are the platform
  thumbnailer's -- what the gallery app scrolls through, drawn once to
  the cell's size and kept -- rather than a decode of every picture; the
  other three pages draw the conversation's own attachment rows, so a
  voice message plays where it sits and an app runs on a tap, and a
  long press offers a file what the chat's row menu offers it. The same
  press offers, on every kind, Show in chat and Delete. Deleting is the
  conversation's own arrangement -- the wait lives beside the views in
  a `PendingRemoval`, the platform's `RemorseItem` draws it, and the
  model's `delete_message` is the row menu's call -- so a run of
  deletes survives the rows it destroys. Show in chat walks the page
  stack down to the conversation this page was opened over
  (`previousPage` until a page has `showMessage`), tells it the
  message, and pops to it; the conversation keeps the ask until it is
  the page on screen and lands the message the way a search result
  lands, over the place it puts back on the way in.
- **The first screen is the cover with nobody on it yet, and it is a
  picture.** A new reader sees what an old one sees when the app is
  minimised: a field of faces in the ambience's colours, grey in its
  primary and a few lit in its highlight, filling the screen either way
  up, with the app's name, one line saying what it is, and the two ways
  on, in a box the field clears for them. Nobody is known yet, so the
  faces are made up
  -- busts in discs and initials on discs, the two kinds of avatar the
  app draws -- and they are painted ahead of time by `tools/faces/`
  (`make faces`) into two masks in `qml/art/`, one per orientation,
  rather than laid out on the phone: a screenful of the cover's avatars
  is a hundred masked, desaturated, tinted textures, and a first
  impression cannot afford a frame of that, while a picture is one
  texture and one pass. The masks carry no colour: red is a grey face's
  ink, green a lit one's, and one shader (`components/FaceField.qml`)
  tints them with the theme's own two colours, so one file is right on
  every ambience and the room for the words is cut where the words are.
  The painter is standard-library Python and deterministic, so the
  masks change only when it does, and a build needs neither it nor a
  display.
- **A newcomer is told what Delta Chat is before being asked to pick a
  server.** The first screen offers two ways on rather than one, because
  a reader who has never heard of Delta Chat and a reader who came for
  it want different next screens. "Tell me about Delta Chat" is five
  facts, one per screen, swiped through (`pages/IntroPage.qml`): a
  profile made on the device, no directory to be found in, encryption
  that is simply always on, groups without an owner, a relay that only
  carries messages. They follow delta.chat's own FAQ with the technical
  half left out, each over a drawing painted the way the faces are
  (`tools/faces/scenes.py`, `components/InkArt.qml`), and swiping past
  the last one goes on to the setup path rather than stopping. "Set up
  my profile" goes there directly: the same field, the same cleared box,
  and the choice between creating a profile -- the relay dialog, which
  is where a server is picked -- and bringing one over from another
  device, which is not built yet and says so.
  Adding a profile is the other half of that screen, and the relay is
  the part of it nobody here controls: a public relay is somebody's
  spare-time server, and one that is down holds the core's transport
  call for as long as its own connection attempts take, which is
  minutes. So an attempt is bounded (`signup.rs`): at thirty seconds the
  shim stops the process and tells the page the relay did not answer,
  and from the fourth second the page says under Cancel what a relay is
  and that another is worth trying. An attempt given up on is still
  running in the core, which allows one ongoing process per account and
  refused the retry that picked the same unconfigured account back up;
  the accounts an attempt still holds are remembered, a retry takes a
  fresh one, and a profile the first relay makes after all is removed
  rather than found on the next start.

## Platform baseline

- Toolchain floor **Rust 1.75.0, Qt 5.6.3** — what Sailfish ships.
- Built against the **5.2** SDK, the Jolla Phone's baseline. Anything older
  is out of scope: a binary from a newer SDK can call symbols an older
  phone lacks, and that is accepted rather than worked around. Harbour
  requires it too -- it rejects a binary that does not link
  `__libc_start_main@GLIBC_2.34`, which only a 5.x glibc provides.
- `aarch64` and `armv7hl` for devices; `i486`/`x86_64` for the emulator.
- Account storage is the core's own, pinned inside the sailjail grant at
  `$XDG_DATA_HOME/postivene/postivene/accounts` (`POSTIVENE_ACCOUNTS_DIR`
  overrides).

## What is missing

In order of what matters:

1. **Harbour-readiness.** Every rule a source tree can answer is now a
   mandatory CI gate (`ci/harbour-check.sh`, `HARBOUR.md`), and the real
   validator runs against each built RPM. One blocker remains, and it is not
   fixable here: the bundled `deltachat-rpc-server` is a second ELF
   executable, which Harbour permits nowhere.
2. **Blocking** outside a request; add-as-second-device and
   restore-from-backup.
3. **Message polish**: avatars on bubbles, and a way to react with an
   emoji the quick row does not offer.
4. **The rest of the webxdc API.** Apps are sent, shown and run
   (`webxdc.rs`, `WebxdcPage.qml`), and status updates go both ways. What
   is not offered is the newer calls -- `importFiles`, realtime channels
   -- which are absent rather than present and failing, so an app that
   feature-tests for one takes its own other path. Nor is
   an app's `source_code_url` shown anywhere: the page has no pulley to
   put it in (a WebView cannot sit in the flickable one needs), and a tap
   on the app's own name that opens a URL its sender chose is a worse
   answer than none.
5. **The store page loads itself.** The app a reader takes from the store
   is fetched by the core, but the store's own page is loaded by the
   engine straight off the web -- so that one page does not follow
   whatever the core has been told to reach the network through, and the
   site sees the device rather than the core. deltachat-android proxies
   every request through `get_http_response`; doing the same here means
   serving the site from the shim's own loopback host and rewriting the
   links in it, which is a page-shaped guess this repository cannot test
   against.
