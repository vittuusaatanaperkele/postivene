//! Guards QML rules that no host-Qt run can check: syntax newer than
//! Sailfish's Qt 5.6, and list rows whose height ignores their text.
//!
//! A text scan on purpose: host Qt 5.15 accepts the newer form and only
//! warns, while on device the handlers silently never fire.

use std::fs;
use std::path::{Path, PathBuf};

/// The lines of the element that declares `marker`, up to the blank line
/// that follows it. A whole-file `contains` cannot say which element a
/// string came from, and once two of them carry it the check stops being
/// able to fail.
fn block_of(text: &str, marker: &str) -> String {
    text.lines()
        .skip_while(|line| !line.contains(marker))
        .take_while(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn qml_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "qml") {
                out.push(path);
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    files
}

#[test]
fn qml_avoids_qt_5_15_only_signal_handler_syntax() {
    let files = qml_files();
    assert!(!files.is_empty(), "found no .qml files to check");

    let mut offenders = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read qml");
        for (number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("function on") {
                offenders.push(format!("{}:{}: {trimmed}", file.display(), number + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "`function onFoo() {{ ... }}` inside Connections is Qt 5.15+ syntax. \
         Sailfish runs Qt 5.6, where it is not an error but is never connected, \
         so the handler silently never runs. Use `onFoo: {{ ... }}` with the \
         shim's snake_case parameter names instead.\n  {}",
        offenders.join("\n  ")
    );
}

/// A list row holding a wrapping `Label` must take its height from that
/// label. With a constant `contentHeight` a long message -- a device
/// message runs to a dozen wrapped lines -- overlaps its neighbours and the
/// header.
///
/// A scan rather than a measurement: `tests/qml_conversation_list.rs` loads
/// the list, but a row collapsed to nothing would leave its assertions
/// about scrolling passing anyway.
#[test]
fn wrapping_list_rows_size_to_their_text() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/components/ConversationList.qml");
    let text = fs::read_to_string(&path).expect("read ConversationList.qml");

    // The whole binding, not the line it starts on. It wraps -- a row's
    // height is its message plus the day heading it may carry -- and
    // reading only the first line would fail a row that does follow its
    // delegate, or pass one that stopped.
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.trim_start().starts_with("contentHeight:"))
        .expect("ConversationList.qml states a row's contentHeight");
    let mut height = lines[start].trim().to_string();
    for line in &lines[start + 1..] {
        let trimmed = line.trim();
        if !trimmed.starts_with(['+', '-', '*', '/', '?', ':', '&', '|']) {
            break;
        }
        height.push(' ');
        height.push_str(trimmed);
    }
    assert!(
        height.contains("body.height"),
        "the message row's contentHeight does not follow the delegate it \
         holds; what the delegate itself measures is in \
         tests/qml_conversation.rs: {height:?}"
    );
}

/// Every `model.<role>` a delegate binds has to be a field the model
/// actually has: a rename on either side is otherwise a blank row that
/// nothing catches.
#[test]
fn delegates_bind_only_roles_their_models_have() {
    /// The `model.<role>` names a file reads.
    fn bound_roles(text: &str) -> Vec<String> {
        text.split("model.")
            .skip(1)
            .map(|tail| {
                tail.chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<String>()
            })
            .filter(|role| !role.is_empty())
            .collect()
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cases: [(&str, Vec<String>); 12] = [
        (
            "qml/components/ConversationList.qml",
            names_of::<postivene_shim::MessageListItem>(),
        ),
        // The media pages' rows are messages, in the conversation's own
        // shape.
        (
            "qml/pages/ChatMediaPage.qml",
            names_of::<postivene_shim::MessageListItem>(),
        ),
        (
            "qml/pages/ChatListPage.qml",
            names_of::<postivene_shim::ChatListItem>(),
        ),
        // One chat list per profile; the people themselves come out of
        // those lists as JSON and are bound as `modelData`, not roles.
        (
            "qml/cover/CoverPage.qml",
            names_of::<postivene_shim::AccountItem>(),
        ),
        (
            "qml/pages/ChatPickerPage.qml",
            names_of::<postivene_shim::ChatListItem>(),
        ),
        (
            "qml/pages/ProfilesPage.qml",
            names_of::<postivene_shim::AccountItem>(),
        ),
        (
            "qml/pages/NewChatPage.qml",
            names_of::<postivene_shim::ContactItem>(),
        ),
        (
            "qml/pages/NewGroupPage.qml",
            names_of::<postivene_shim::ContactItem>(),
        ),
        (
            "qml/pages/GroupPage.qml",
            names_of::<postivene_shim::ContactItem>(),
        ),
        (
            "qml/pages/AddMembersPage.qml",
            names_of::<postivene_shim::ContactItem>(),
        ),
        (
            "qml/pages/ContactPage.qml",
            names_of::<postivene_shim::ContactItem>(),
        ),
        (
            "qml/components/SearchResultsList.qml",
            names_of::<postivene_shim::SearchItem>(),
        ),
    ];

    for (file, roles) in cases {
        let text =
            fs::read_to_string(root.join(file)).unwrap_or_else(|err| panic!("read {file}: {err}"));
        let bound = bound_roles(&text);
        assert!(
            !bound.is_empty(),
            "{file} binds nothing from its model any more"
        );
        for role in &bound {
            assert!(
                roles.contains(role),
                "{file} binds model.{role}, which its model does not have: {roles:?}"
            );
        }
    }
}

/// A row's time label must position itself off the row, never off the
/// label whose width it decides.
///
/// `SearchResultRow` had the two referring to each other -- the title's
/// width subtracted the time label's width, and the time label anchored
/// to the title's top. On a device that cycle left the title at its
/// natural width and a long chat name ran off the right-hand edge; the
/// offscreen engine used by the tests resolves it and shows nothing, so
/// this is a scan rather than a measurement. `ChatListDelegate` is the
/// shape that works: its time label is placed off `root.width`, and the
/// name takes its width from that label's `x`.
#[test]
fn a_time_label_does_not_depend_on_the_label_it_sizes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/components");
    for (file, time_id, sized_id) in [
        ("SearchResultRow.qml", "id: timeLabel", "titleLabel"),
        ("ChatListDelegate.qml", "id: timeLabelItem", "nameLabel"),
    ] {
        let text =
            fs::read_to_string(root.join(file)).unwrap_or_else(|err| panic!("read {file}: {err}"));
        let block = block_of(&text, time_id);
        assert!(
            !block.is_empty(),
            "{file} has no {time_id} any more; this check is measuring nothing"
        );
        assert!(
            !block.contains(sized_id),
            "{file}'s time label positions itself off {sized_id}, whose width \
             is decided by this label -- that is the loop that put a long \
             title off the side of the screen. Place it off the row instead, \
             as ChatListDelegate does. Block was:\n{block}"
        );
    }
}

/// The role names a model row exposes to QML.
fn names_of<T: qmetaobject::listmodel::SimpleListItem>() -> Vec<String> {
    T::names()
        .iter()
        .map(std::string::ToString::to_string)
        .collect()
}

/// A list draws its delegates outside its own box unless told not to, and
/// both list pages sit above translucent things -- a message field, an
/// error banner -- that the content then shows through.
#[test]
fn list_pages_clip_and_leave_room_for_what_sits_below_them() {
    for (page, list) in [
        ("ConversationPage.qml", "../components/ConversationList.qml"),
        ("ChatListPage.qml", "ChatListPage.qml"),
    ] {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/pages");
        let text =
            fs::read_to_string(dir.join(page)).unwrap_or_else(|err| panic!("read {page}: {err}"));
        let list_text =
            fs::read_to_string(dir.join(list)).unwrap_or_else(|err| panic!("read {list}: {err}"));
        assert!(
            list_text.contains("clip: true"),
            "{list} does not clip, so its rows draw over what is below them"
        );
        // Scoped to the list's own block: the jump button anchors to the
        // same thing, so searching the whole file finds a match whatever
        // the list does.
        let list_block = block_of(&text, "id: listView");
        assert!(
            list_block.contains("bottom: banner.top"),
            "{page}'s list runs under the banner instead of stopping at it"
        );
        // Losing the core is a state, not an event: a page opened after it
        // went away never saw the transition that a handler would have
        // caught, so the banner has to read the status rather than wait for
        // it to change. Both halves, because a page that only knows
        // "stopped" says "restart Postivene" through a reconnection that is
        // already under way.
        for state in ["reconnecting", "stopped"] {
            assert!(
                text.contains(&format!("core.status === \"{state}\"")),
                "{page} does not read the core's {state} state, so it waits \
                 for a transition it may never see"
            );
        }
        assert!(
            text.contains("page.coreStatusMessage.length > 0"),
            "{page}'s banner does not prefer the core's own state over the \
             last error, so a dead core hides behind whatever failed first"
        );
    }
}

/// Copying a message says so, and the reply bar and jump button are the
/// components the tests measure rather than one-off items on the page.
#[test]
fn the_conversation_page_uses_the_pieces_that_are_tested() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/pages/ConversationPage.qml");
    let text = fs::read_to_string(&path).expect("read ConversationPage.qml");

    for piece in ["ReplyBar {", "JumpButton {", "Banner {"] {
        assert!(
            text.contains(piece),
            "the page does not use {piece} -- what a test measures is then not what runs"
        );
    }
    assert!(
        text.contains("Clipboard.text = body") && text.contains("notice.show("),
        "copying a message does not say that it happened"
    );
    // The field insets itself on the left; without the same on the right
    // the send button sits nearer the edge than the field does. Scoped to
    // the row: the jump button carries the same inset, so a whole-file
    // search cannot fail.
    assert!(
        block_of(&text, "id: inputRow").contains("rightMargin: Theme.horizontalPageMargin"),
        "the send button is flush against the edge of the screen"
    );
    // Two things the page has to do on its way out, both of them a timer
    // that has not fired yet: write the draft, and send the deletes the
    // reader asked for. The list's own end of the second is
    // `qml_delete_run.rs`; what cannot be tested by loading the list on
    // its own is that the page ever asks.
    assert!(
        block_of(&text, "PageStatus.Deactivating").contains("listView.flushDeletes()"),
        "leaving the chat does not send the deletes still waiting, so a \
         reader who asked for a message to go and then left keeps it"
    );
    // The return key is the reader's to give back to sending, and no test
    // can see it: every test loads the page with the `EnterKey.` lines
    // taken out (common::qml_tree_without_enter_key), which is also why
    // each has to be one line. So the shipped file is held to the shape
    // here -- the key drawn as the accept key while it sends, greyed with
    // nothing to send, and its click going to the function
    // `qml_enter_sends.rs` drives.
    for line in [
        "EnterKey.iconSource: page.enterSends ? \"image://theme/icon-m-enter-accept\"",
        "EnterKey.enabled: !page.enterSends || page.hasSomethingToSend",
        "EnterKey.onClicked: page.enterPressed()",
    ] {
        assert!(
            text.contains(line),
            "the message field does not carry `{line}`, so the return key \
             does not do what the settings page says it does"
        );
    }
}

/// Anything showing a string the other end chose has to say it is plain
/// text. The default is `Text.AutoText`, which sniffs the string and
/// switches to rich text when it looks like markup -- so a message body of
/// `<img src="https://tracker/p.gif">` fetches that image the moment its
/// row is drawn, from an app whose whole point is that the network cannot
/// watch. It also mismeasures: the hidden copies that size a bubble would
/// measure the rendered width, not the literal one.
///
/// Silica's `PageHeader`, `SectionHeader` and `ViewPlaceholder` draw their
/// text in labels of their own with no `textFormat` to set, so a remote
/// string reaching one of those is an offender outright: the conversation
/// page's header carried the chat's name that way, and this check only
/// looked at `Label` and `Text`. `ConversationHeader` is the replacement.
#[test]
fn text_from_the_other_end_is_pinned_to_plain() {
    // Bindings the core fills in from a message, a contact or a chat.
    // Anything reading one of these is showing remote input.
    const REMOTE: [&str; 29] = [
        "model.",
        "root.messageText",
        "root.quoteText",
        "root.quoteAuthor",
        "root.senderName",
        "root.chatName",
        "root.preview",
        "root.previewSender",
        "root.fileName",
        "root.filePath",
        "root.author",
        "root.body",
        "root.text",
        "root.title",
        "root.subtitle",
        "root.displayName",
        "root.address",
        "root.initial",
        "root.vcardName",
        "root.vcardAddr",
        "root.webxdcName",
        "root.webxdcDocument",
        "root.webxdcSummary",
        "root.genericText",
        "page.chatName",
        "page.fileName",
        "page.fileMime",
        "page.senderName",
        "page.myInvite",
    ];

    /// The element a line sits in, as the nearest `Foo {` above it.
    fn element_of(lines: &[&str], index: usize) -> String {
        lines[..index]
            .iter()
            .rev()
            .find_map(|line| {
                let trimmed = line.trim();
                let name = trimmed.strip_suffix('{')?.trim();
                (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '.'))
                    .then(|| name.to_string())
            })
            .unwrap_or_default()
    }

    let mut offenders = Vec::new();
    for file in qml_files() {
        let text = fs::read_to_string(&file).expect("read qml");
        let lines: Vec<&str> = text.lines().collect();
        for (number, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let is_text = trimmed.starts_with("text:");
            let is_title = trimmed.starts_with("title:");
            if !is_text && !is_title {
                continue;
            }
            // Only the things that render text themselves. A MenuItem's
            // label is a translated literal, whatever it switches on.
            let element = element_of(&lines, number);
            let renders = (element == "Label" || element == "Text") && is_text;
            let unpinnable = matches!(
                element.as_str(),
                "PageHeader" | "SectionHeader" | "ViewPlaceholder"
            );
            if !renders && !unpinnable {
                continue;
            }
            // The binding runs on while its lines stay further indented
            // than the `text:` that opened it.
            let indent = line.len() - trimmed.len();
            let mut value = trimmed.to_string();
            for next in &lines[number + 1..] {
                let next_indent = next.len() - next.trim_start().len();
                if next.trim().is_empty() || next_indent <= indent {
                    break;
                }
                value.push(' ');
                value.push_str(next.trim());
            }
            if !REMOTE.iter().any(|name| value.contains(name)) {
                continue;
            }
            if unpinnable {
                offenders.push(format!(
                    "{}:{}: {trimmed} ({element} cannot be told this is plain text; \
                     draw it in a Label of your own, as ConversationHeader does)",
                    file.display(),
                    number + 1
                ));
                continue;
            }
            // `textFormat` sits in the same element block, which starts at
            // the `{` found above.
            let start = number.saturating_sub(16);
            let block = lines[start..=number].join("\n");
            if !block.contains("textFormat:") {
                offenders.push(format!("{}:{}: {trimmed}", file.display(), number + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these show text the other end chose without saying it is plain, so \
         Qt decides for itself whether it is markup:\n  {}",
        offenders.join("\n  ")
    );
}

/// The dconf keys are named in `Settings.qml` and nowhere else.
///
/// Every page reads and writes the settings through that one object; a
/// second file naming a key would be a second definition of it, and the
/// two would drift the way the app and its page in the Settings app once
/// could.
#[test]
fn only_the_settings_object_names_the_dconf_keys() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml");
    let mut elsewhere = Vec::new();
    let mut keys = std::collections::BTreeSet::new();
    for file in qml_files() {
        let text = fs::read_to_string(&file).expect("read qml");
        let named: Vec<String> = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("key: \""))
            .filter_map(|rest| rest.split('"').next())
            .map(ToString::to_string)
            .collect();
        if named.is_empty() {
            continue;
        }
        if file == root.join("components/Settings.qml") {
            keys.extend(named);
        } else {
            elsewhere.push(file.display().to_string());
        }
    }
    assert!(
        elsewhere.is_empty(),
        "these name a dconf key outside components/Settings.qml, which is \
         the one place the keys are defined:\n  {}",
        elsewhere.join("\n  ")
    );
    assert!(
        keys.len() >= 5,
        "Settings.qml names fewer keys than the settings it exists for -- \
         Markdown, clean links, the download limit, notification detail, \
         and whether webxdc apps are offered: {keys:?}"
    );
    assert!(
        keys.iter()
            .all(|key| key.starts_with("/apps/harbour-postivene/")),
        "a key is outside the app's own dconf path: {keys:?}"
    );
}

/// No deletion hangs off the row it was asked for on.
///
/// `ListItem.remorseAction` is Silica's shortcut: it makes a
/// `RemorseItem` in the row and hands it the action. Deleting out of a
/// list is exactly what destroys rows, and a run of deletes lost most of
/// itself that way -- in the conversation, and open to the same thing in
/// the chat list, the profiles list and a group's members.
///
/// What replaced it keeps the two halves apart. The *action* is the
/// list's (components/PendingRemoval.qml), which outlives every row. The
/// *look* is still Silica's `RemorseItem`, raised by the row and handed
/// a callback that does nothing --
/// `every_pending_removal_is_drawn_by_the_platform` holds it to that. So
/// this bans the shortcut, not the widget.
#[test]
fn no_wait_before_a_deletion_lives_on_the_row_it_was_asked_on() {
    let mut offenders = Vec::new();
    for file in qml_files() {
        let code = code_only(&fs::read_to_string(&file).expect("read qml"));
        for (number, line) in code.lines().enumerate() {
            if line.contains("remorseAction(") {
                offenders.push(format!("{}:{}", file.display(), number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these put the wait before a deletion on the row it was asked \
         for on, which the deletion itself destroys; use a \
         PendingRemoval beside the list instead:\n  {}",
        offenders.join("\n  ")
    );
}

/// The wait is drawn by the platform, not by us.
///
/// Silica's `RemorseItem` is what a Sailfish reader already knows a
/// countdown to look like: the bar, the seconds, "Tap to cancel", the
/// fade over what is going. A hand-made stand-in was tried and looked
/// wrong on a phone, which is the whole reason this rule exists. Every
/// list that holds a wait has to raise the real one.
#[test]
fn every_pending_removal_is_drawn_by_the_platform() {
    let mut offenders = Vec::new();
    for file in qml_files() {
        let code = code_only(&fs::read_to_string(&file).expect("read qml"));
        if !code.contains("PendingRemoval {") {
            continue;
        }
        if !code.contains("RemorseItem {") || !code.contains(".execute(") {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these hold a wait before a deletion and draw it themselves; \
         raise Silica's own RemorseItem over the row instead, so it looks \
         like every other countdown on the phone:\n  {}",
        offenders.join("\n  ")
    );
}

/// The drawn countdown is given the list's own number, never its own.
///
/// A row raises Silica's `RemorseItem` over what is going, and the list
/// deletes when the wait is up. Those are two clocks, and they have to
/// end at the same moment: when they did not, the countdown ran out and
/// the platform put the message back while the list was still waiting,
/// and everything went together at the end instead. So the only value a
/// row may hand `execute` is `PendingRemoval.countdownFor(id)`, which
/// answers with what is left of that id's own wait.
///
/// Counted rather than matched line by line, because the call wraps: as
/// many `countdownFor(` as there are `.execute(`.
#[test]
fn the_drawn_countdown_is_asked_for_rather_than_chosen() {
    let mut offenders = Vec::new();
    for file in qml_files() {
        let code = code_only(&fs::read_to_string(&file).expect("read qml"));
        if !code.contains("PendingRemoval {") {
            continue;
        }
        let raised = code.matches(".execute(").count();
        let asked = code.matches("countdownFor(").count();
        if raised != asked {
            offenders.push(format!(
                "{}: {raised} countdown(s) raised, {asked} asked for",
                file.display()
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "these raise a countdown with a length of their own choosing; \
         pass PendingRemoval.countdownFor(id) instead, so what is drawn \
         and what is deleted end together:\n  {}",
        offenders.join("\n  ")
    );
}

/// A row waiting to go is faded, not hidden.
///
/// Hiding an item takes its children's `visible` with it, and the parts
/// of a row measure `visible ? implicitHeight : 0` -- so hiding the
/// content of a waiting row collapses the row to nothing. What replaces
/// the content is centred in that nothing and drawn across the rows
/// above and below: three messages deleted at once put three "Deleting"
/// on top of each other on a phone, which is how this was found.
/// `opacity` leaves the height alone, and `enabled` is what stops a tap
/// reaching controls that can no longer be seen.
///
/// One line at a time, so a binding wrapped across two escapes it. The
/// five files it applies to are counted by
/// `every_pending_removal_is_emptied_on_the_way_out`.
#[test]
fn a_row_waiting_to_go_is_faded_rather_than_hidden() {
    let mut offenders = Vec::new();
    for file in qml_files() {
        let code = code_only(&fs::read_to_string(&file).expect("read qml"));
        if !code.contains("PendingRemoval {") {
            continue;
        }
        for (number, line) in code.lines().enumerate() {
            let line = line.trim();
            let hides =
                line.contains('!') && (line.contains("doomed") || line.contains(".pending("));
            if line.starts_with("visible:") && hides {
                offenders.push(format!("{}:{}: {line}", file.display(), number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these hide a row's content while it waits to go, which collapses \
         the row and draws what replaces it over the neighbours; fade it \
         with `opacity` and take its taps with `enabled` \
         instead:\n  {}",
        offenders.join("\n  ")
    );
}

/// Every wait is emptied when what holds it is left.
///
/// A wait that nobody empties is a deletion the reader asked for and did
/// not get: they tapped Delete, then went somewhere else inside four
/// seconds. Each page that holds one flushes it as it deactivates, for
/// the reason `ConversationPage` writes its draft there. `ConversationList`
/// is not a page and offers `flushDeletes()` for its own page to call,
/// which `the_conversation_page_uses_the_pieces_that_are_tested` checks.
#[test]
fn every_pending_removal_is_emptied_on_the_way_out() {
    let pages = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/pages");
    let mut holders = 0;
    let mut offenders = Vec::new();
    for file in qml_files() {
        let text = fs::read_to_string(&file).expect("read qml");
        if !code_only(&text).contains("PendingRemoval {") {
            continue;
        }
        holders += 1;
        let leaving = if file.parent() == Some(pages.as_path()) {
            block_of(&text, "PageStatus.Deactivating")
        } else {
            text.clone()
        };
        if !leaving.contains(".flush()") {
            offenders.push(file.display().to_string());
        }
    }
    // Otherwise this passes by finding nothing to check, which is what it
    // would do if the five of them were quietly put back on their rows.
    assert!(
        holders >= 5,
        "only {holders} files hold a PendingRemoval; there should be five \
         -- the conversation, the chat list, the profiles, a group's \
         members and a chat's media"
    );
    assert!(
        offenders.is_empty(),
        "these hold a wait before a deletion and never empty it, so \
         leaving inside the wait keeps what the reader asked to \
         delete:\n  {}",
        offenders.join("\n  ")
    );
}

/// A path the other end named becomes a URL one segment at a time.
///
/// `encodeURI` leaves `#` and `?` alone -- to it they are URL syntax --
/// so an attachment called `a#b.png` made a URL whose path stopped at the
/// `a`, and the file never loaded. It also contradicted the comment above
/// it, which said it handled exactly that. `encodeURIComponent` on each
/// segment between the slashes encodes everything that is not a slash.
#[test]
fn file_urls_are_encoded_per_segment() {
    let mut offenders = Vec::new();
    for file in qml_files() {
        let code = code_only(&fs::read_to_string(&file).expect("read qml"));
        for (number, line) in code.lines().enumerate() {
            if line.contains("encodeURI(") {
                offenders.push(format!("{}:{}", file.display(), number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "encodeURI() keeps '#' and '?' as URL syntax, so a file named with \
         either points at a different URL. Build the path with \
         path.split('/').map(encodeURIComponent).join('/') instead, as \
         AttachmentPreview.qml does:\n  {}",
        offenders.join("\n  ")
    );
}

/// Only the picker pages name a `Sailfish.Pickers` type.
///
/// Those types resolve when the file that names them is loaded, so a type
/// that is not there on some future release takes that whole file down
/// with it. Kept to files that exist for nothing else -- pushed by URL and
/// connected to -- the cost is one button; in a page it is the page.
/// `SettingsPage` had its own `ImagePickerPage` for the profile picture,
/// and with it its own unguarded copy of the pick handler.
///
/// Named one by one rather than matched by prefix: three of them belong
/// to the attachment tray and one to taking a profile over from a backup
/// file, and adding to the list should be a decision rather than a side
/// effect of what a file is called.
#[test]
fn only_the_picker_pages_import_sailfish_pickers() {
    const ALLOWED: [&str; 4] = [
        "AttachAppPage.qml",
        "AttachLibraryPage.qml",
        "AttachPhotoPage.qml",
        "BackupFilePage.qml",
    ];
    let mut offenders = Vec::new();
    for file in qml_files() {
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let code = code_only(&fs::read_to_string(&file).expect("read qml"));
        let imports_pickers = code
            .lines()
            .any(|line| line.trim_start().starts_with("import Sailfish.Pickers"));
        if imports_pickers && !ALLOWED.contains(&name.as_str()) {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these import Sailfish.Pickers outside {ALLOWED:?}, so a picker type \
         that is missing takes the whole page down rather than one button; \
         push the picker page by URL and connect to its `picked` signal, as \
         SettingsPage.pickPicture does:\n  {}",
        offenders.join("\n  ")
    );
}

/// The two webxdc pages are the only files naming a `Sailfish.WebView`
/// type.
///
/// The same rule as the pickers above, for the same reason and a sharper
/// case: the browser engine is a separate package, and a release without
/// it -- or a device where it is not installed -- would take down every
/// file naming the type. Here that is the page that runs one app, pushed
/// by URL from the conversation, so a chat still opens and every other
/// attachment still works.
#[test]
fn only_the_webxdc_pages_import_sailfish_webview() {
    const ALLOWED: [&str; 2] = ["WebxdcPage.qml", "WebxdcStorePage.qml"];
    let mut offenders = Vec::new();
    for file in qml_files() {
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let code = code_only(&fs::read_to_string(&file).expect("read qml"));
        let imports_webview = code
            .lines()
            .any(|line| line.trim_start().starts_with("import Sailfish.WebView"));
        if imports_webview && !ALLOWED.contains(&name.as_str()) {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these import Sailfish.WebView outside {ALLOWED:?}, so a missing \
         browser engine takes the whole page down rather than the one app it \
         runs; push the page by URL instead, as ConversationPage.openApp \
         does:\n  {}",
        offenders.join("\n  ")
    );
}

/// Neither webxdc page may decide when its `WebView` is active.
///
/// `Sailfish.WebView`'s own `WebView.qml` binds `active` to the page's
/// status and to whether the app is in front -- which is what decides
/// when the engine renders and when it hands the GPU back. Binding it
/// here overrides that, and a view whose activation no longer follows the
/// page transition draws as a grey rectangle where the app should be.
/// That is what the first device build did.
#[test]
fn the_webxdc_pages_leave_the_views_activation_alone() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/pages");
    for name in ["WebxdcPage.qml", "WebxdcStorePage.qml"] {
        let code = code_only(&fs::read_to_string(root.join(name)).expect("read the page"));
        let offender = code
            .lines()
            .enumerate()
            .find(|(_, line)| line.trim_start().starts_with("active:"));
        assert!(
            offender.is_none(),
            "{name} sets `active` (line {}), which overrides the binding \
             Sailfish's own WebView.qml makes on it; the view then does not \
             follow the page's status and draws grey",
            offender.map_or(0, |(number, _)| number + 1)
        );
    }
}

/// The share methods the desktop file offers are the ones the app
/// answers.
///
/// An app is in the phone's share sheet because its desktop file says so
/// -- `X-Share-Methods`, and a group per method -- and what arrives goes
/// to a `ShareProvider` of the same name inside the app. A name in one
/// file and not the other is an entry in the sheet that does nothing, or
/// a provider nothing ever reaches; neither shows up anywhere but on a
/// phone.
///
/// The words are checked too: the sheet reads them from the desktop
/// file, which no catalogue reaches, so `ShareTarget.qml` carries the
/// same strings under `qsTr` for `lupdate` to collect. A description
/// that drifts from the one beside it is a share method translated
/// nowhere.
#[test]
fn the_share_methods_and_what_answers_them_agree() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let desktop =
        fs::read_to_string(root.join("harbour-postivene.desktop")).expect("read the desktop entry");
    let target =
        fs::read_to_string(root.join("qml/share/ShareTarget.qml")).expect("read the share target");

    // The desktop entry's side: the list, and a group for each name.
    let offered: Vec<String> = desktop
        .lines()
        .find_map(|line| line.trim().strip_prefix("X-Share-Methods="))
        .map(|value| {
            value
                .split(';')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !offered.is_empty(),
        "the desktop entry offers no share methods, so the app is not in \
         the phone's share sheet at all"
    );

    // The app's side: one provider per method.
    // Comments blanked and strings kept: the names are string literals,
    // which `code_only` blanks along with everything else quoted.
    let answered: Vec<String> = code_only_keeping_strings(&target)
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("method:")
                .map(|value| value.trim().trim_matches('"').to_string())
        })
        .collect();
    for name in &offered {
        assert!(
            answered.contains(name),
            "the desktop entry offers the share method {name:?} and no \
             ShareProvider answers it: {answered:?}"
        );
        let group = format!("[X-Share Method {name}]");
        let description = block_of(&desktop, &group)
            .lines()
            .find_map(|line| line.trim().strip_prefix("Description="))
            .map(str::to_string)
            .unwrap_or_default();
        assert!(
            !description.is_empty(),
            "the share method {name:?} has no {group} group with a \
             Description= in it, so the sheet has nothing to call it"
        );
        assert!(
            target.contains(&format!("qsTr(\"{description}\")")),
            "the share method {name:?} is called {description:?} in the \
             desktop entry and that string is not under qsTr in \
             ShareTarget.qml, so no catalogue carries it"
        );
    }
    for name in &answered {
        assert!(
            offered.contains(name),
            "a ShareProvider answers {name:?}, which the desktop entry does \
             not offer, so nothing will ever reach it: {offered:?}"
        );
    }
}

/// `Sailfish.Share` is named in the share target alone.
///
/// The same rule as the pickers and the webxdc pages, for the same
/// reason: the import resolves only on a release that ships the module,
/// and a file that names it is a file that fails without it. Here that
/// file is loaded by a `Loader` from the window, so a release without it
/// costs sharing rather than the app.
#[test]
fn only_the_share_target_imports_sailfish_share() {
    let mut offenders = Vec::new();
    for file in qml_files() {
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let code = code_only(&fs::read_to_string(&file).expect("read qml"));
        let imports = code
            .lines()
            .any(|line| line.trim_start().starts_with("import Sailfish.Share"));
        if imports && name != "ShareTarget.qml" {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "these import Sailfish.Share outside ShareTarget.qml, so a release \
         without the module takes them down with it; load the share target \
         from a Loader instead, as postivene.qml does:\n  {}",
        offenders.join("\n  ")
    );
}

/// The app's own name on the cover is not a string to be translated.
///
/// A name is the same word in every language, and a catalogue with this
/// string in it is an invitation to translate it -- one `qsTr` here and
/// some language ships a cover calling the app something else. It is a
/// literal, and this is what keeps it one.
#[test]
fn the_covers_name_is_the_apps_own_and_never_translated() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/cover/CoverPage.qml");
    let text = fs::read_to_string(&path).expect("read the cover");
    let brand = block_of(&text, "objectName: \"brand\"");
    assert!(
        brand.contains("text: \"Postivene\""),
        "the cover does not name the app in its heading:\n{brand}"
    );
    assert!(
        !brand.contains("qsTr"),
        "the app's name on the cover is run through a translation \
         catalogue:\n{brand}"
    );
}

/// Every frame script a page hands the browser engine is a file that
/// ships.
///
/// `loadFrameScript` takes a URL and says nothing about whether anything
/// is there: a renamed or unpackaged script is a store that quietly stops
/// catching taps on an app, on a phone and nowhere else. Nothing else can
/// notice -- the stub view records what it was asked to load without
/// looking -- so the path is checked here, against the tree the package
/// is built from.
#[test]
fn the_frame_scripts_the_pages_load_are_there() {
    let mut loaded = Vec::new();
    for file in qml_files() {
        let code = code_only_keeping_strings(&fs::read_to_string(&file).expect("read qml"));
        for line in code.lines() {
            let Some(rest) = line.split_once("loadFrameScript(") else {
                continue;
            };
            let Some(path) = rest.1.split('"').nth(1) else {
                panic!(
                    "{}: loadFrameScript with no path in it: {line}",
                    file.display()
                );
            };
            // `Qt.resolvedUrl` reads the path against the file that
            // names it, so this walks the same way: one directory up for
            // every `../`, then whatever is left.
            let mut here = file.parent().expect("a file is in a directory");
            let mut rest = path;
            while let Some(above) = rest.strip_prefix("../") {
                here = here.parent().expect("qml/ has a parent");
                rest = above;
            }
            let script = here.join(rest.trim_start_matches("./"));
            assert!(
                script.is_file(),
                "{} loads the frame script {path}, which is not in the tree \
                 at {}",
                file.display(),
                script.display()
            );
            loaded.push(path.to_string());
        }
    }
    assert!(
        loaded.contains(&"../webxdc/catch.js".to_string()),
        "the store no longer loads the frame script that catches a tap on \
         an app; if that moved, this list moves with it: {loaded:?}"
    );
}

/// A file an app hands over is kept, not put into a chat.
///
/// The webxdc call is `sendToChat` and a chat is the only destination its
/// name can carry, but the button an app draws over it is a download --
/// `sharer`'s is a download arrow -- and what the reader means by that is
/// the file, on their phone. So it is saved where the file manager looks,
/// and nothing is asked: a chooser between opening and keeping was tried
/// and was two taps in front of the one thing already asked for. Nothing
/// else can check this: the page names `Sailfish.WebView`, so it is never
/// loaded off a phone.
#[test]
fn the_webxdc_page_keeps_a_handed_over_file() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/pages/WebxdcPage.qml");
    let text = fs::read_to_string(&path).expect("read WebxdcPage.qml");
    let code = code_only(&text);
    assert!(
        code.contains("onHanded_over:"),
        "the page ignores an app handing a file over, so the call resolves \
         and nothing happens"
    );
    assert!(
        !code.contains("appWindow.shareInto("),
        "a file an app handed over still goes to the chat picker, which \
         is not what a download means"
    );
    let offer = block_of(&code, "function offer(");
    assert!(
        offer.contains("handoverSaver.save(") && offer.contains("StandardPaths.download"),
        "the file is not kept anywhere the reader can find it: {offer:?}"
    );
    assert!(
        !text.contains("HandoverDialog"),
        "the page still asks what to do with the file rather than keeping it"
    );
    // The cache copy is the page's to clean up: nothing else knows the
    // save has happened, and an export that leaves one behind costs the
    // phone twice over for every file an app ever makes.
    assert!(
        offer.contains("handoverSaver.handedOver ="),
        "the page does not keep the path it is saving from, so it has \
         nothing to delete afterwards: {offer:?}"
    );
    assert!(
        code.contains("app.discard("),
        "the copy in the cache is never deleted, so every file an app \
         hands over stays on the phone twice"
    );
}

/// Being covered by another page does not stop the app under it.
///
/// The dialog a handover opens is pushed *over* the app, so a page that
/// stops its app whenever it deactivates stops it in the middle of the
/// request that asked for it: the app's fetch is answered by a
/// closed socket and it reports that its host is gone. Leaving is
/// destruction -- a popped page is destroyed, and a replaced stack takes
/// its pages with it -- and that is where the app is stopped. The page
/// names `Sailfish.WebView`, so nothing else can check this.
#[test]
fn the_webxdc_page_keeps_serving_an_app_it_has_opened_a_page_over() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qml/pages/WebxdcPage.qml");
    let text = fs::read_to_string(&path).expect("read WebxdcPage.qml");
    let code = code_only(&text);
    assert!(
        !code.contains("PageStatus.Deactivating"),
        "the page stops its app when it deactivates, which is what being \
         covered by the handover dialog is: handing a file over opens \
         that dialog, so asking to export one kills the request that asked"
    );
    assert!(
        code.contains("Component.onDestruction:")
            && block_of(&code, "Component.onDestruction:").contains("app.stop()"),
        "nothing stops the app when the page goes, so an app the reader \
         has left carries on being served"
    );
}

/// Every page the app pushes is a page that exists.
///
/// A page is pushed by name, resolved at the moment of the tap, and a
/// name that is wrong fails there and nowhere earlier: the reader taps
/// and the app does nothing, which is exactly the bug the file page was
/// added to stop being. Nothing else checks these -- a page only some
/// device path reaches is never loaded by a test -- so the names are
/// checked here.
#[test]
fn the_pages_the_app_pushes_are_there() {
    let mut pushed = Vec::new();
    for file in qml_files() {
        let code = code_only_keeping_strings(&fs::read_to_string(&file).expect("read qml"));
        for line in code.lines() {
            let Some(rest) = line.split_once("Qt.resolvedUrl(") else {
                continue;
            };
            // Only the literal ones that name a file in the tree: a
            // path built from a variable is one this cannot follow, and
            // `Qt.resolvedUrl("file://" + path)` is a file on the phone
            // rather than a page of ours.
            let Some(path) = rest.1.split('"').nth(1) else {
                continue;
            };
            let kind = std::path::Path::new(path)
                .extension()
                .map(|kind| kind.to_string_lossy().into_owned())
                .unwrap_or_default();
            if kind != "qml" && kind != "js" {
                continue;
            }
            // `Qt.resolvedUrl` reads the path against the file that
            // names it; see the frame scripts above.
            let mut here = file.parent().expect("a file is in a directory");
            let mut rest = path;
            while let Some(above) = rest.strip_prefix("../") {
                here = here.parent().expect("qml/ has a parent");
                rest = above;
            }
            let target = here.join(rest.trim_start_matches("./"));
            assert!(
                target.is_file(),
                "{} pushes {path}, which is not in the tree at {}",
                file.display(),
                target.display()
            );
            pushed.push(path.to_string());
        }
    }
    assert!(
        pushed.iter().any(|path| path.ends_with("MessagePage.qml")),
        "nothing pushes MessagePage.qml any more, so a long message has \
         nowhere to be read whole: {pushed:?}"
    );
}

/// The file with its comments blanked out and its strings left alone.
///
/// `code_only` cannot be used where the string is the point: the paths
/// this scans for are string literals.
fn code_only_keeping_strings(text: &str) -> String {
    text.lines()
        .map(|line| match line.split_once("//") {
            // A `//` inside a string is not a comment: the only ones here
            // are in URLs, which are not what this scans for.
            Some((code, _)) if !code.contains('"') => code,
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The file with every comment and string body blanked out, newlines kept.
///
/// A scan that does not do this reads prose and translated text as code:
/// a URL in a string ends a line comment, and from there every quote
/// pairs up with the wrong one and whole blocks of real code disappear.
fn code_only(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let here = chars[i];
        let next = chars.get(i + 1).copied();
        if here == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if here == '/' && next == Some('*') {
            while i < chars.len() {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    out.push_str("  ");
                    i += 2;
                    break;
                }
                out.push(blank(chars[i]));
                i += 1;
            }
        } else if here == '"' || here == '\'' {
            out.push(' ');
            i += 1;
            while i < chars.len() && chars[i] != here {
                if chars[i] == '\\' {
                    out.push(' ');
                    i += 1;
                }
                if i < chars.len() {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
            if i < chars.len() {
                out.push(' ');
                i += 1;
            }
        } else {
            out.push(here);
            i += 1;
        }
    }
    out
}

/// Identifiers and single punctuation marks, in order.
fn tokens(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in code.chars() {
        if c.is_alphanumeric() || c == '_' || c == '$' {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        if !c.is_whitespace() {
            out.push(c.to_string());
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// Every name the file itself introduces: ids, properties and their
/// types, functions and their parameters, signal parameters, `var`s.
///
/// Deliberately greedy -- a type name landing in here alongside the
/// property it types costs nothing. Over-collecting only makes the check
/// weaker, while missing a declaration would make it wrong.
fn declared_names(code: &str) -> std::collections::HashSet<String> {
    fn is_name(token: &str) -> bool {
        token
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
    }

    let tokens = tokens(code);
    let mut names = std::collections::HashSet::new();
    let mut i = 0;
    while i < tokens.len() {
        // Everything after the keyword, up to the token that ends the
        // declaration.
        let (skip, stop): (usize, &[&str]) = match tokens[i].as_str() {
            "id" if tokens.get(i + 1).map(String::as_str) == Some(":") => (2, &[]),
            "property" => (1, &[":", "{", "}"]),
            "function" | "signal" => (1, &[")", "{"]),
            "var" => (1, &["="]),
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i + skip;
        // A bare `id:` or `var` names exactly one thing.
        let single = stop.is_empty() || tokens[i] == "var";
        while j < tokens.len() && !stop.contains(&tokens[j].as_str()) {
            if is_name(&tokens[j]) {
                names.insert(tokens[j].clone());
                if single {
                    break;
                }
            }
            j += 1;
        }
        i = j.max(i + 1);
    }
    names
}

/// Every `foo.` in the code, with the line it is on: a name being read
/// for something on it.
fn qualified_uses(code: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = code.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut out = Vec::new();
    for (at, c) in chars.iter().enumerate() {
        if *c != '.' {
            continue;
        }
        let mut end = at;
        while end > 0 && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        if start == end {
            continue;
        }
        // `a.b.c` reads `b` off `a`; only the head of the chain is a name
        // that has to exist on its own.
        if start > 0 && (chars[start - 1] == '.' || is_word(chars[start - 1])) {
            continue;
        }
        let name: String = chars[start..end].iter().collect();
        // Types, attached properties and enums are capitalised; numbers
        // are not names at all.
        if !name.starts_with(|c: char| c.is_lowercase() && c.is_alphabetic()) {
            continue;
        }
        let line = chars[..start].iter().filter(|c| **c == '\n').count() + 1;
        out.push((line, name));
    }
    out
}

/// A name read as `<name>.something` has to be one the file introduces or
/// one QML puts in scope.
///
/// An id that is not there any more is not a load error. The binding that
/// reads it throws at runtime, that one binding never runs, and whatever
/// it fed keeps its default -- everything else on the page carries on. A
/// patch deleted `SearchResultRow`'s `Avatar` and left the three
/// references to it behind; the row's height is measured off it, so every
/// search result collapsed to nothing and a search showed "Chats (1)"
/// with no chat under it. The file still parsed, the page still loaded,
/// and nothing in the suite could see it.
#[test]
fn qml_reads_no_name_that_is_not_there() {
    // What QML puts in scope without the file saying so.
    const IN_SCOPE: [&str; 20] = [
        // Grouped properties, and properties of the element being
        // configured read without qualifying them.
        "anchors",
        "font",
        "icon",
        "text",
        "parent",
        // A Rectangle's outline: the capture page's shutter wears one.
        "border",
        // Camera's grouped property for what goes into the file: the
        // capture page writes the orientation through it.
        "metaData",
        // An Item's own texture, and the effect drawn from it: Avatar
        // takes the colour out of a picture through one.
        "layer",
        // On Image, AnimatedImage and Nemo's Thumbnail: how big to decode.
        "sourceSize",
        // PinchArea's own grouped property, and the PinchEvent its
        // handlers are passed under the same name.
        "pinch",
        // The MouseEvent a MouseArea's handlers are passed.
        "mouse",
        // A delegate's scope.
        "model",
        "modelData",
        "section",
        "index",
        // Set from Rust in main.rs, and Silica's own.
        "core",
        "pageStack",
        // The root window's id in postivene.qml, in scope for every
        // page it loads. A page loaded on its own in a test has none,
        // which is why ChatListPage reads it behind a typeof check.
        "appWindow",
        // ContentPickerPage hands its answer to the handler under this
        // name; see AttachPhotoPage.
        "selectedContentProperties",
        // What a WebView's `recvAsyncMessage` carries beside the message
        // name: whatever the frame script sent. See WebxdcStorePage.
        "data",
    ];

    let files = qml_files();
    assert!(!files.is_empty(), "found no .qml files to check");

    let mut offenders = Vec::new();
    for file in &files {
        let code = code_only(&fs::read_to_string(file).expect("read qml"));
        let declared = declared_names(&code);
        let mut seen = Vec::new();
        for (line, name) in qualified_uses(&code) {
            if declared.contains(&name) || IN_SCOPE.contains(&name.as_str()) {
                continue;
            }
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            offenders.push(format!("{}:{line}: {name}.…", file.display()));
        }
    }

    assert!(
        offenders.is_empty(),
        "these read something off a name the file never declares. If it was \
         an id, the element is gone and the references were left behind: the \
         binding throws at load and silently keeps its default, which is how \
         a row ends up with no height. If it is something QML puts in scope, \
         add it to IN_SCOPE with a note saying where it comes from.\n  {}",
        offenders.join("\n  ")
    );
}
