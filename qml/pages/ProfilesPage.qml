import QtQuick 2.0
import Sailfish.Silica 1.0
import "../components"

/*
 * Which profile the chat list is showing, and the way to each one's own
 * page.
 *
 * The core keeps every configured account open at once, with IO running
 * on all of them, so switching is a matter of pointing the chat list at a
 * different one rather than starting anything up; the chat list tells the
 * core which it is on, and that is the profile the app opens on next
 * time. Picking the profile already shown does nothing. A row's
 * menu leads to the profile's page -- picture, name, address, the rest --
 * and to deleting it. Another profile is made from the plus under the
 * last row, where the group pages put "add members", and a profile this
 * reader already has on another device is taken over from the plus
 * under that one.
 *
 * Deleting counts down beside the list rather than on the row
 * (PendingRemoval): the row goes whenever the list reloads, and it used
 * to take the countdown with it, so the second of two profiles deleted
 * in the same breath was never deleted at all. The list is also
 * refreshed in place rather than rebuilt when a deletion lands
 * (core.rs), which is worth keeping for its own sake -- it is what stops
 * every row flickering -- but the countdown no longer depends on it.
 */
Page {
    id: page

    /// The profile the chat list is currently on.
    property int currentAccountId: 0

    /// True once a deletion has been asked for, so the empty list that
    /// follows is read as "the last profile is gone" rather than as the
    /// list simply not having loaded yet.
    property bool deleting: false

    Connections {
        target: core
        // Deleting the last profile leaves the app with nothing to show,
        // so it goes back to where a first profile is made. Replacing the
        // whole stack, not pushing: there is no chat list to return to.
        onAccounts_refreshed: {
            if (page.deleting && configured_count === 0) {
                page.deleting = false
                // The empty properties are passed rather than left
                // out: Silica's own replaceAbove takes them, and the
                // two-argument form errors out under a stack that
                // declares all three -- which is what the test harness
                // does, so the branch was never actually run there.
                pageStack.replaceAbove(null,
                                       Qt.resolvedUrl("WelcomePage.qml"),
                                       {})
            }
        }
        onAccount_error: page.errorMessage = message
    }

    property string errorMessage: ""

    /// How long a profile waits before it goes, in milliseconds.
    /// Nothing sets it; a test turns it down rather than waiting.
    property alias pendingDelay: doomedProfiles.delay

    /// The profiles the reader has asked to delete, waiting out the
    /// moment in which they can say they did not mean it.
    PendingRemoval {
        id: doomedProfiles
        onRemove: {
            page.deleting = true
            core.remove_account(id)
        }
    }

    // A profile page pushed over this one, or a swipe back to the chats:
    // either way, anything still waiting goes now. Leaving is exactly
    // when a timer has not fired yet.
    onStatusChanged: {
        if (page.status === PageStatus.Deactivating) {
            doomedProfiles.flush()
        }
    }

    SilicaListView {
        id: listView
        objectName: "profileList"
        anchors.fill: parent
        model: core.account_list

        header: PageHeader {
            title: qsTr("Profiles")
        }

        delegate: ListItem {
            id: profileDelegate
            // Named per profile, so a test can find the one it means.
            objectName: "profileRow" + model.account_id
            contentHeight: body.height

            /// This profile is on its way out.
            readonly property bool doomed: doomedProfiles.pending(model.account_id)

            /// Silica's own countdown, drawn over the profile. The
            /// deletion is not its business -- that belongs to
            /// `doomedProfiles`, because a remorse item lives in the row
            /// it covers and the row goes whenever the list reloads. So
            /// it draws, and reports the tap.
            function raiseRemorse() {
                //: What Silica's countdown says it is doing, over a
                //: profile the reader has asked to delete.
                remorse.execute(
                    body, qsTr("Deleting profile"), function() {},
                    doomedProfiles.countdownFor(model.account_id))
            }

            RemorseItem {
                id: remorse
                objectName: "profileRemorse"
                onCanceled: doomedProfiles.spare(model.account_id)
            }

            // A row rebuilt mid-wait comes back with no countdown on it.
            Component.onCompleted: {
                if (profileDelegate.doomed) {
                    profileDelegate.raiseRemorse()
                }
            }

            menu: ContextMenu {
                MenuItem {
                    objectName: "profileSettingsItem"
                    text: qsTr("Profile settings")
                    onClicked: pageStack.push(Qt.resolvedUrl("ProfilePage.qml"),
                                              { accountId: model.account_id })
                }
                MenuItem {
                    objectName: "deleteProfileItem"
                    text: qsTr("Delete profile")
                    // The page is told, not this row: the row is
                    // destroyed whenever the list reloads, and a wait
                    // living on it would go too. Same as the chat list
                    // and the conversation.
                    onClicked: {
                        doomedProfiles.ask(model.account_id)
                        profileDelegate.raiseRemorse()
                    }
                }
            }

            ContactRow {
                id: body
                // The remorse covering this does the fading, with its
                // own `opacity: 0.0` on what it was handed.
                enabled: !profileDelegate.doomed
                width: parent.width
                displayName: model.display_name.length > 0
                             ? model.display_name : model.addr
                address: model.addr
                // The profile's own picture and colour, as the core lists
                // them with the account: the same row the chat list draws
                // for everyone else, drawn for oneself.
                ownColor: model.color
                picturePath: model.avatar_path
                // The reader's own, and what tells two profiles apart.
                showAddress: true
                isKeyContact: true
                // Room for the badge and the mark, so a long name fades
                // before them rather than running under them.
                trailingSpace: marks.width + Theme.paddingMedium
            }

            Row {
                id: marks
                // Beside the row rather than in it, so the remorse does
                // not cover it: faded to match.
                opacity: profileDelegate.doomed ? 0 : 1
                anchors {
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: body.verticalCenter
                }
                spacing: Theme.paddingMedium

                // Waiting to be read in this profile: the badge the chat
                // list draws on a chat, drawn on the profile. The core's
                // own count, which leaves muted chats out.
                Rectangle {
                    objectName: "profileUnreadBadge"
                    anchors.verticalCenter: parent.verticalCenter
                    visible: model.unread_count > 0
                    width: visible
                           ? Math.max(height, unreadLabel.implicitWidth + Theme.paddingMedium)
                           : 0
                    height: unreadLabel.implicitHeight + Theme.paddingSmall
                    radius: height / 2
                    color: Theme.highlightColor

                    Label {
                        id: unreadLabel
                        objectName: "profileUnreadLabel"
                        anchors.centerIn: parent
                        font.pixelSize: Theme.fontSizeExtraSmall
                        color: Theme.primaryColor
                        // A number, but pinned like everything read off
                        // a model.
                        textFormat: Text.PlainText
                        text: model.unread_count > 99 ? "99+" : model.unread_count
                    }
                }

                // The one being shown, marked the way a chosen group
                // member is.
                Label {
                    objectName: "currentMark"
                    anchors.verticalCenter: parent.verticalCenter
                    visible: model.account_id === page.currentAccountId
                    text: "✓"
                    color: Theme.highlightColor
                    font.pixelSize: Theme.fontSizeLarge
                }
            }

            // A profile waiting to go is covered by the remorse, which
            // takes the tap itself and calls the deletion off.
            onClicked: {
                if (model.account_id !== page.currentAccountId) {
                    // The whole stack, not just this page. `replace`
                    // swapped out the accounts page and left the previous
                    // account's chat list underneath it -- one swipe back
                    // into the profile just left. A null target replaces
                    // everything, which is what the onboarding pages do
                    // when they hand over to the chat list.
                    pageStack.replaceAbove(null,
                                           Qt.resolvedUrl("ChatListPage.qml"),
                                           { accountId: model.account_id })
                } else {
                    pageStack.pop()
                }
            }
        }

        // The ways to another profile, where the next one would be
        // listed: rows shaped like a profile's, with a plus for a
        // picture, as the group pages offer another member. Under the
        // last row rather than in the pulley, which is where a reader
        // who has just read the list is already looking.
        //
        // The first makes one, through the welcome page's own flow,
        // which replaces the stack with the new profile's chat list once
        // the core has it. The second takes one over from a device that
        // has it already -- the same transfer the first screen offers a
        // reader with no profile at all, which is exactly what somebody
        // holding their old phone wants from here.
        footer: Column {
            width: listView.width

            ListItem {
                id: addProfileRow
                objectName: "addProfileButton"
                width: parent.width
                contentHeight: Theme.itemSizeSmall + 2 * Theme.paddingMedium

                PlusMark {
                    id: plus
                    x: Theme.horizontalPageMargin
                    y: Theme.paddingMedium
                }

                Label {
                    x: plus.x + plus.width + Theme.paddingMedium
                    width: parent.width - x - Theme.horizontalPageMargin
                    anchors.verticalCenter: plus.verticalCenter
                    wrapMode: Text.Wrap
                    color: addProfileRow.highlighted ? Theme.highlightColor
                                                     : Theme.primaryColor
                    text: qsTr("Add profile")
                }

                onClicked: pageStack.push(Qt.resolvedUrl("AddProfileDialog.qml"), {})
            }

            ListItem {
                id: secondDeviceRow
                objectName: "secondDeviceButton"
                width: parent.width
                contentHeight: Theme.itemSizeSmall + 2 * Theme.paddingMedium

                PlusMark {
                    id: secondPlus
                    x: Theme.horizontalPageMargin
                    y: Theme.paddingMedium
                }

                Label {
                    x: secondPlus.x + secondPlus.width + Theme.paddingMedium
                    width: parent.width - x - Theme.horizontalPageMargin
                    anchors.verticalCenter: secondPlus.verticalCenter
                    wrapMode: Text.Wrap
                    color: secondDeviceRow.highlighted ? Theme.highlightColor
                                                       : Theme.primaryColor
                    text: qsTr("Add as second device")
                }

                onClicked: pageStack.push(Qt.resolvedUrl("RestoreProfilePage.qml"),
                                          { from: "device" })
            }
        }

        // Counted off the model, not off what is drawn: the plus is the
        // view's own row rather than a profile, so a list with nothing
        // in it still has a row on it.
        ViewPlaceholder {
            enabled: listView.count === 0
            text: qsTr("No profiles")
        }
    }

    Banner {
        objectName: "errorBanner"
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }
        text: page.errorMessage
        onDismissed: page.errorMessage = ""
    }
}
