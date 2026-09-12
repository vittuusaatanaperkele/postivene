import QtQuick 2.0
import Sailfish.Silica 1.0

/*
 * Where the setup path begins: reached from the first screen, and from
 * the end of the walk through what Delta Chat is (IntroPage.qml). The
 * field of faces stays on the first screen, where it is the welcome; a
 * page asking a question wants nothing behind the question.
 *
 * Two ways from here, and both are built. Creating a profile goes
 * straight to the relay dialog. Having one already asks where it is --
 * on a device still in reach, or in a backup file -- and takes it over
 * from there (ExistingProfilePage.qml).
 */
Page {
    id: page

    allowedOrientations: Orientation.All

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
            onClicked: pageStack.push(Qt.resolvedUrl("ExistingProfilePage.qml"), {})
        }

        Button {
            objectName: "createProfileButton"
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("Create a profile")
            enabled: core.status === "ready"
            onClicked: pageStack.push(Qt.resolvedUrl("AddProfileDialog.qml"), {})
        }
    }
}
