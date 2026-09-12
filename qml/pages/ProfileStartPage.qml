import QtQuick 2.0
import Sailfish.Silica 1.0
import "../components"

/*
 * Where the setup path begins: reached from the first screen, and from
 * the end of the walk through what Delta Chat is (IntroPage.qml). The
 * screen is the first screen again -- the same field of faces in the
 * ambience's colours, the same box cleared for the words -- so that
 * going on feels like the same room rather than a new one.
 *
 * Two ways from here. Creating a profile is the one that works today and
 * goes straight to the relay dialog. Bringing one over from another
 * device is the one a reader with a phone in each hand wants, and it is
 * not built yet; it says so rather than pretending, because a button
 * that does nothing is worse than a button that explains itself.
 */
Page {
    id: page

    allowedOrientations: Orientation.All

    // Set by the button that has nothing behind it yet.
    property bool askedForExisting: false

    FaceField {
        objectName: "faceField"
        anchors.fill: parent
        source: page.width > page.height ? "../art/faces-landscape.png"
                                         : "../art/faces-portrait.png"
        clearX: words.x + words.width / 2
        clearY: words.y + words.height / 2
        clearWidth: words.width
        clearHeight: words.height
        clearRadius: Theme.paddingLarge
        clearFeather: Theme.itemSizeLarge
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
            text: qsTr("Alright, let's get you set up.")
            font.family: Theme.fontFamilyHeading
            font.pixelSize: Theme.fontSizeLarge
            color: Theme.highlightColor
        }

        Item { width: 1; height: Theme.paddingLarge }

        Button {
            objectName: "existingProfileButton"
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("I already have a profile")
            onClicked: page.askedForExisting = true
        }

        Button {
            objectName: "createProfileButton"
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("Create a profile")
            enabled: core.status === "ready"
            onClicked: pageStack.push(Qt.resolvedUrl("AddProfileDialog.qml"), {})
        }

        // What the first of those buttons has to say for itself.
        Label {
            objectName: "notYetHint"
            width: parent.width
            visible: page.askedForExisting
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            font.pixelSize: Theme.fontSizeSmall
            color: Theme.secondaryHighlightColor
            text: qsTr("Bringing a profile over from another device is not ready yet. It is being worked on.")
        }
    }
}
