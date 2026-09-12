import QtQuick 2.0
import QtMultimedia 5.6
import Sailfish.Silica 1.0
import Postivene 1.0

/*
 * Point the camera at a QR code -- or, from the button under the
 * viewfinder, type or paste the link the code would carry. What either
 * says is handed out on `scanned` and the view waits; what to do with it
 * is the host's -- an invite is followed -- and the core is asked what
 * it is first, the same as for a pasted link.
 *
 * A component of its own, loaded by URL by the pages that want it -- the
 * QR page for an invite, the take-over page for the code a device shows
 * while offering its profile: this is the only file that names a Camera,
 * so a device without one costs the scanner rather than the page around
 * it. Both give it the rest of the page, which is what a code needs to
 * land in enough pixels to read.
 *
 * What it is looking for is the host's to say (`hintText`), and so is
 * whether the link behind the code can be typed instead (`offerLink`).
 *
 * Frames come through `grabToImage`, which is the one way QML on Qt 5.6
 * hands a viewfinder's pixels to anything, and go to the scanner as a
 * file. The next frame is grabbed only once the last has been decoded, so
 * a slow decode never queues frames behind it.
 *
 * Once a code is read the view stays, with the camera off and a busy
 * indicator on, until the host takes it down: Silica drops any stack
 * operation asked for while a transition is running, so a page that
 * popped itself here would have the host's own navigation -- opening the
 * chat the invite led to -- land during the pop and be lost. The typed
 * link is a panel over the viewfinder rather than a dialog for the same
 * reason: a dialog's own pop would be the transition the host's
 * navigation lands in.
 *
 * Focus is asked for three ways, because a camera left to itself gave a
 * viewfinder too soft for any code to register: continuous autofocus in
 * the video pipeline, where the platform's cameras run it; a focus search
 * every couple of seconds while nothing has been read, for a camera that
 * only focuses when told; and a tap on the viewfinder, which focuses on
 * the point under the finger.
 */
Item {
    id: root

    /// The view is the one on screen. The camera runs only then.
    property bool active: false

    /// A code was read, or a link typed, and this is its text.
    signal scanned(string text)
    /// The scanner could not read a frame at all. The message is its own.
    signal failed(string message)

    /// Set once a code is read, so a second frame in flight cannot
    /// report the same code twice.
    property bool done: false
    /// The link panel is open. The camera keeps running behind it: a
    /// reader who opened the panel and then found the code can still
    /// hold the phone up to it.
    property bool typing: false

    /// The line under the viewfinder saying what to point it at. The
    /// host's words, because what a code means is the host's: an invite
    /// on the QR page, the other phone's offer of a profile on the
    /// take-over page.
    property string hintText: qsTr("Point the camera at someone's invite code")

    /// Whether the link behind the code can be typed instead. An invite
    /// is a link someone can send, so it can be; the code a device shows
    /// while offering a profile carries an address and a one-time
    /// secret, which nobody reads off a screen and types.
    property bool offerLink: true

    /// What was read or typed, once it is worth acting on.
    function found(text) {
        if (root.done) {
            return
        }
        root.done = true
        camera.stop()
        root.scanned(text)
    }

    /// Open the link panel, with the clipboard's contents in it when
    /// they look like an invite: pasting is what the panel is for, and a
    /// link copied a moment ago is the likely reason it was opened.
    function typeLink() {
        root.typing = true
        var clip = Clipboard.text
        if (linkField.text.length === 0 && clip.length > 0 && root.looksLikeInvite(clip)) {
            linkField.text = clip
        }
    }

    /// Whether some text is one of the things a code carries, so the
    /// clipboard is not pasted into the field when it holds a shopping
    /// list. Only the prefixes: what the payload is is the core's call.
    function looksLikeInvite(text) {
        var trimmed = text.trim().toLowerCase()
        return trimmed.indexOf("https://i.delta.chat/") === 0
            || trimmed.indexOf("openpgp4fpr:") === 0
            || trimmed.indexOf("dcaccount:") === 0
            || trimmed.indexOf("dclogin:") === 0
    }

    function useLink() {
        var text = linkField.text.trim()
        if (text.length === 0) {
            return
        }
        root.found(text)
    }

    QrScanner {
        id: scanner
        objectName: "scanner"
        onFound: root.found(text)
        onError: root.failed(message)
    }

    Camera {
        id: camera
        objectName: "camera"
        // The video pipeline is where continuous autofocus runs.
        captureMode: Camera.CaptureVideo
        focus {
            focusMode: Camera.FocusContinuous
            focusPointMode: Camera.FocusPointAuto
        }
    }

    // An autofocus run, for a camera that does not run one on its own.
    // Unlocked first: a search that finds focus locks it, and a locked
    // focus is a fixed one.
    function refocus() {
        camera.unlock()
        camera.searchAndLock()
    }

    // Every couple of seconds while nothing has been read.
    Timer {
        id: refocusTimer
        objectName: "refocus"
        interval: 2500
        repeat: true
        triggeredOnStart: true
        running: root.active && !root.done
        onTriggered: root.refocus()
    }

    // The camera runs only while this view is the one on screen.
    onActiveChanged: {
        if (root.active && !root.done) {
            camera.start()
        } else {
            camera.stop()
        }
    }

    /// The long side of a grabbed frame, in pixels.
    ///
    /// Grabbed small because a code fills a good part of the frame when
    /// someone is holding a phone up to it, and a third of the pixels
    /// decode in a ninth of the time.
    readonly property int grabLongSide: 640

    /// That long side, in the viewfinder's own shape. A grab to a fixed
    /// square stretches the modules by whatever the viewfinder is not
    /// square by, and a decoder reading a square symbol as an oblong one
    /// has that much less to work with.
    function grabSize() {
        var longest = Math.max(viewfinder.width, viewfinder.height)
        if (longest <= 0) {
            return Qt.size(root.grabLongSide, root.grabLongSide)
        }
        // Never upscale: a small viewfinder has no more detail to give.
        var ratio = Math.min(1, root.grabLongSide / longest)
        return Qt.size(Math.max(1, Math.round(viewfinder.width * ratio)),
                       Math.max(1, Math.round(viewfinder.height * ratio)))
    }

    Timer {
        id: grabber
        objectName: "grabber"
        interval: 300
        repeat: true
        running: root.active && !scanner.busy && !root.done
        onTriggered: {
            var path = scanner.frame_path()
            if (path.length === 0) {
                return
            }
            viewfinder.grabToImage(function(result) {
                if (result.saveToFile(path)) {
                    scanner.decode(path)
                }
            }, root.grabSize())
        }
    }

    VideoOutput {
        id: viewfinder
        objectName: "viewfinder"
        anchors.fill: parent
        source: camera

        // Tap to focus on what is under the finger.
        MouseArea {
            objectName: "focusTap"
            anchors.fill: parent
            onClicked: {
                camera.focus.focusPointMode = Camera.FocusPointCustom
                camera.focus.customFocusPoint = Qt.point(mouse.x / width, mouse.y / height)
                root.refocus()
            }
        }
    }

    // The code was read and the host is acting on it.
    BusyIndicator {
        objectName: "acting"
        anchors.centerIn: parent
        running: root.done
        size: BusyIndicatorSize.Large
    }

    // The other way in, where a thumb finds it: a button under the
    // viewfinder that opens the panel for the link the code would carry,
    // typed or pasted. Over the viewfinder rather than instead of it.
    Button {
        id: typeLinkButton
        objectName: "typeLinkButton"
        visible: root.offerLink && !root.typing && !root.done
        anchors {
            horizontalCenter: parent.horizontalCenter
            bottom: hint.top
            bottomMargin: Theme.paddingMedium
        }
        text: qsTr("Enter invite link")
        onClicked: root.typeLink()
    }

    Column {
        id: linkPanel
        objectName: "linkPanel"
        visible: root.offerLink && root.typing && !root.done
        anchors {
            left: parent.left
            right: parent.right
            bottom: hint.top
        }
        spacing: Theme.paddingSmall

        Rectangle {
            width: parent.width
            height: linkField.height + followButton.height + 2 * Theme.paddingMedium
            color: Theme.rgba(Theme.highlightDimmerColor, 0.8)

            TextField {
                id: linkField
                objectName: "linkField"
                anchors {
                    left: parent.left
                    right: parent.right
                    top: parent.top
                    topMargin: Theme.paddingMedium
                }
                label: qsTr("Invite link")
                placeholderText: "https://i.delta.chat/..."
                inputMethodHints: Qt.ImhNoAutoUppercase | Qt.ImhNoPredictiveText
            }

            Button {
                id: followButton
                objectName: "followButton"
                anchors {
                    horizontalCenter: parent.horizontalCenter
                    top: linkField.bottom
                }
                text: qsTr("Connect")
                enabled: linkField.text.trim().length > 0
                onClicked: root.useLink()
            }
        }
    }

    Label {
        id: hint
        objectName: "hint"
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
            margins: Theme.horizontalPageMargin
        }
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        color: Theme.secondaryHighlightColor
        visible: !root.done
        text: root.typing
              ? qsTr("Or point the camera at the code")
              : root.hintText
    }
}
