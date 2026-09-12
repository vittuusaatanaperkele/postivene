import QtQuick 2.0
import Sailfish.Silica 1.0
import "../components"

/*
 * A profile that exists already, brought onto this phone. Both ways the
 * other Delta Chat apps offer, on one page because everything after the
 * first step is the same:
 *
 * - `from: "device"` is their "add second device". The device holding
 *   the profile offers it over the local network and shows a code; this
 *   one reads the code and the two transfer the profile between them
 *   (`get_backup`). Both devices end up with it, which is the point.
 * - `from: "file"` is their "restore from backup": a file the other
 *   device wrote, copied here and read back (`import_backup`).
 *
 * The transfer is the core's, and it reports itself in progress events
 * the shim passes on, so this page is a bar and a way to stop. What it
 * cannot do is take a profile over twice at once: the shim counts a
 * transfer as an attempt, the same as making a profile, so Cancel is
 * `cancel_ongoing` and a second attempt makes the first one nobody's.
 *
 * A code that is not a device offering a profile is this app's own
 * finding rather than the core's, so the shim names the reason and this
 * page puts it into the reader's language -- the core's own words for it
 * are about protocols, and the reader is holding a camera.
 */
Page {
    id: page

    allowedOrientations: Orientation.All

    /// Where the profile is coming from: `device` or `file`.
    property string from: "device"
    readonly property bool fromDevice: page.from !== "file"

    /// True while the core is transferring.
    property bool busy: false
    /// How far it has got, in permille, as the core counts it.
    property int permille: 0
    property string errorMessage: ""

    // Nothing to go back to mid-transfer: leaving would drop the page
    // that is listening for the answer, and the core would carry on.
    backNavigation: !page.busy

    /// Start the transfer with whatever the first step produced: a code
    /// from the camera, or a path from the file browser.
    function begin(text) {
        page.errorMessage = ""
        page.permille = 0
        page.busy = true
        if (page.fromDevice) {
            core.restore_from_device(text)
        } else {
            core.restore_from_file(text)
        }
    }

    /// The file browser, pushed by URL and connected to, the way the
    /// conversation pushes its own pickers.
    function chooseFile() {
        var picker = pageStack.push(Qt.resolvedUrl("BackupFilePage.qml"), {})
        if (picker) {
            picker.picked.connect(page.begin)
        }
    }

    /// What the shim's reason for refusing means, in words for a reader.
    function reasonText(reason) {
        if (reason === "too-new") {
            return qsTr("The other device runs a newer Delta Chat than this app can take a profile from.")
        }
        if (reason === "stalled") {
            return qsTr("The transfer stopped before it finished. Both devices have to stay on the same network, with this page open.")
        }
        return qsTr("That is not the code a device shows while it is offering its profile.")
    }

    // Qt 5.6 handler syntax; see WelcomePage.qml.
    Connections {
        target: core

        onRestore_progress: {
            if (page.busy) {
                page.permille = permille
            }
        }

        // The profile is here, with IO started on it: the app opens on
        // it, above nothing, because there is no earlier screen worth
        // going back to once a profile exists.
        onProfile_restored: {
            if (!page.busy) {
                return
            }
            page.busy = false
            pageStack.replaceAbove(null, Qt.resolvedUrl("ChatListPage.qml"),
                                   { accountId: account_id })
        }

        onRestore_refused: {
            if (!page.busy) {
                return
            }
            page.busy = false
            page.errorMessage = page.reasonText(reason)
        }

        onRestore_failed: {
            if (!page.busy) {
                return
            }
            page.busy = false
            page.errorMessage = message
        }
    }

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: Math.max(height, column.height + Theme.paddingLarge)

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingLarge

            PageHeader {
                objectName: "header"
                title: page.fromDevice ? qsTr("Add as second device")
                                       : qsTr("Restore from a backup")
            }

            Label {
                objectName: "instructions"
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.secondaryHighlightColor
                text: page.fromDevice
                      ? qsTr("On the device that has your profile, open Settings and choose to add a second device. Hold this phone up to the code it shows. Both devices have to be on the same network.")
                      : qsTr("On the device that has your profile, make a backup and copy the file onto this phone. Then choose it here.")
            }

            // The file half of the first step. The camera half is below,
            // outside the column, because it takes the rest of the page.
            Button {
                objectName: "chooseFileButton"
                anchors.horizontalCenter: parent.horizontalCenter
                visible: !page.fromDevice && !page.busy
                text: qsTr("Choose a backup file")
                onClicked: page.chooseFile()
            }

            ProgressBar {
                objectName: "transferProgress"
                width: parent.width
                visible: page.busy
                minimumValue: 0
                maximumValue: 1000
                value: page.permille
                label: qsTr("Taking the profile over...")
            }

            Button {
                objectName: "cancelButton"
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.busy
                text: qsTr("Cancel")
                onClicked: {
                    page.busy = false
                    core.cancel_ongoing()
                    pageStack.pop()
                }
            }

            Banner {
                objectName: "errorBanner"
                width: parent.width
                text: page.errorMessage
                onDismissed: page.errorMessage = ""
            }

            // The camera, when that is the way the profile is coming.
            // Loaded by URL, as the QR page loads it: this is the only
            // other place a Camera is named, and a phone without one
            // should lose the scanner rather than the page.
            Item {
                id: scanArea
                objectName: "scanArea"
                visible: page.fromDevice && !page.busy
                width: parent.width
                height: visible ? Math.max(Theme.itemSizeLarge, page.height * 0.5) : 0

                Loader {
                    id: scanLoader
                    objectName: "scanLoader"
                    anchors.fill: parent
                    active: page.fromDevice
                    source: Qt.resolvedUrl("../components/ScanView.qml")
                    onLoaded: {
                        scanLoader.item.scanned.connect(page.begin)
                        scanLoader.item.failed.connect(function(message) {
                            page.errorMessage = message
                        })
                    }
                }

                // The camera runs while this page is on screen and there
                // is nothing to read any more once a transfer is under
                // way.
                Binding {
                    target: scanLoader.item
                    property: "active"
                    value: page.status === PageStatus.Active && page.fromDevice
                           && !page.busy
                }

                Label {
                    objectName: "noCamera"
                    visible: scanLoader.status === Loader.Error
                    anchors.centerIn: parent
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    color: Theme.secondaryHighlightColor
                    text: qsTr("The camera is not available on this device. A backup file works without one.")
                }
            }
        }
    }
}
