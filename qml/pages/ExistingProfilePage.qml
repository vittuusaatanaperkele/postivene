import QtQuick 2.0
import Sailfish.Silica 1.0

/*
 * The reader has a profile already, somewhere else. Where it is decides
 * how it gets here, and there are two answers -- the same two the other
 * Delta Chat apps offer:
 *
 * - It is on a device they still have, which can offer it over the local
 *   network. Both devices end up with it; nothing is moved.
 * - It is in a backup file that device wrote, copied onto this phone.
 *
 * Both go to RestoreProfilePage, which does the transfer; this page is
 * the question, because the two need different first steps -- a camera
 * for one, the file browser for the other -- and a reader who has to
 * find that out by trying is a reader who gave up.
 */
Page {
    id: page

    allowedOrientations: Orientation.All

    function takeOver(from) {
        pageStack.push(Qt.resolvedUrl("RestoreProfilePage.qml"), { from: from })
    }

    Column {
        id: words
        anchors {
            horizontalCenter: parent.horizontalCenter
            verticalCenter: parent.verticalCenter
            verticalCenterOffset: -page.height * 0.04
        }
        width: Math.min(parent.width - 2 * Theme.horizontalPageMargin,
                        Screen.width - 2 * Theme.horizontalPageMargin)
        spacing: Theme.paddingMedium

        Label {
            objectName: "lead"
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            text: qsTr("Where is your profile now?")
            font.family: Theme.fontFamilyHeading
            font.pixelSize: Theme.fontSizeLarge
            color: Theme.highlightColor
        }

        Item { width: 1; height: Theme.paddingLarge }

        Button {
            objectName: "secondDeviceButton"
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("Add as second device")
            onClicked: page.takeOver("device")
        }

        Label {
            objectName: "secondDeviceHint"
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryHighlightColor
            text: qsTr("The device that has it keeps it. This one joins, over the same network.")
        }

        Item { width: 1; height: Theme.paddingLarge }

        Button {
            objectName: "backupFileButton"
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("Restore from a backup")
            onClicked: page.takeOver("file")
        }

        Label {
            objectName: "backupFileHint"
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryHighlightColor
            text: qsTr("A backup file the other device wrote, copied onto this phone.")
        }
    }
}
