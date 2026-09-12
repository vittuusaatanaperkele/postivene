import QtQuick 2.0
import Sailfish.Silica 1.0
import "../components"

/*
 * What Delta Chat is, for somebody who has never heard of it: five
 * facts, one per screen, swiped through the way a phone's own first run
 * is. The facts and their wording follow delta.chat/en/help -- a profile
 * made on the device, no directory to be found in, encryption that is
 * simply always on, groups without an owner, a relay that only carries
 * messages -- with the technical half of each left out. A reader who
 * wants that can read the FAQ; a reader here wants to know whether this
 * app is for them.
 *
 * Swiping past the last fact is the way on, to the setup path, because a
 * reader who has kept swiping is asking for what comes next. The reader
 * who has had enough swipes back instead, and the first screen still
 * offers both directions.
 *
 * The field of faces belongs to the first screen and stays there: here
 * the picture over each fact is the thing to look at, and a field of
 * faces behind it is one pattern too many. The pictures are the same
 * two-channel masks in the ambience's own colours, drawn by
 * components/InkArt.qml and painted by tools/faces/scenes.py.
 */
Page {
    id: page

    // Both ways up: the field has a master for each, and the facts are
    // a column either way.
    allowedOrientations: Orientation.All

    // Set once the reader has swiped past the end, so that the drag that
    // does it cannot ask for the next page twice.
    property bool leaving: false

    /// The facts, in order. A plain array rather than a ListModel:
    /// `ListElement` takes literals only, and every line here is a
    /// translated one.
    readonly property var facts: [
        {
            picture: "../art/intro-profile.png",
            title: qsTr("No sign-up, no phone number"),
            body: qsTr("Your profile is made here on your phone. No phone number, no account with a company, nothing to sign.")
        },
        {
            picture: "../art/intro-invite.png",
            title: qsTr("You choose who can reach you"),
            body: qsTr("There is no directory to be found in. Show a friend a code to scan, or send them a link, and the two of you can write.")
        },
        {
            picture: "../art/intro-lock.png",
            title: qsTr("Every message is encrypted"),
            body: qsTr("Messages are encrypted the whole way, always. Servers that transport them cannot read a word.")
        },
        {
            picture: "../art/intro-group.png",
            title: qsTr("Groups of equals"),
            body: qsTr("Everyone in a group has the same rights: anyone can add a friend, rename it or leave. Nobody is in charge.")
        },
        {
            picture: "../art/intro-relay.png",
            title: qsTr("The server only passes it on"),
            body: qsTr("A server holds a message until the other phone is online, and that is all it does. Your chats stay on your device.")
        }
    ]

    /// How far past the last fact a drag has to carry the view before it
    /// counts as asking for what comes next.
    readonly property real enough: Theme.itemSizeMedium

    // The setup path begins where the facts end. `replace` rather than
    // `push`: the walk is over, and a reader going back from there wants
    // the first screen, not the last fact again.
    function goOn() {
        if (page.leaving) {
            return
        }
        page.leaving = true
        pageStack.replace(Qt.resolvedUrl("ProfileStartPage.qml"), {})
    }

    // Called as the view moves: a reader pulling past the last fact is
    // already asking, so the page answers on the drag rather than
    // waiting for the rubber band to settle back.
    function advanceIfPastEnd() {
        if (slides.width <= 0 || slides.contentWidth <= 0) {
            return
        }
        if (slides.contentX - (slides.contentWidth - slides.width) > page.enough) {
            page.goOn()
        }
    }

    ListView {
        id: slides
        objectName: "slides"
        anchors.fill: parent
        orientation: ListView.Horizontal
        // One fact at a time, and one is always squarely on screen.
        snapMode: ListView.SnapOneItem
        highlightRangeMode: ListView.StrictlyEnforceRange
        // So that the last fact can be pulled past, which is what asks
        // for the next page.
        boundsBehavior: Flickable.DragOverBounds
        model: page.facts

        // Only a drag asks for the next page. Turning the phone changes
        // the view's width, and the content it has already scrolled is
        // measured against the old one for an instant: on a device that
        // read as a pull past the last fact and carried the reader off
        // the page with no finger on it.
        onContentXChanged: {
            if (slides.dragging) {
                page.advanceIfPastEnd()
            }
        }
        // And what the turn leaves behind is half of two facts, so the
        // one being read is put back squarely on screen.
        onWidthChanged: slides.positionViewAtIndex(slides.currentIndex,
                                                   ListView.Beginning)

        delegate: Item {
            width: slides.width
            height: slides.height

            Column {
                anchors {
                    horizontalCenter: parent.horizontalCenter
                    verticalCenter: parent.verticalCenter
                    // A little above the middle: the dots sit below.
                    verticalCenterOffset: -Theme.itemSizeSmall / 2
                }
                width: parent.width - 2 * Theme.horizontalPageMargin
                spacing: Theme.paddingLarge

                InkArt {
                    objectName: "slideArt" + index
                    anchors.horizontalCenter: parent.horizontalCenter
                    // Square, and small enough to leave the words room
                    // on a screen lying on its side.
                    width: Math.min(page.width * 0.42, page.height * 0.30)
                    height: width
                    source: modelData.picture
                }

                Label {
                    objectName: "slideTitle" + index
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                    text: modelData.title
                    font.family: Theme.fontFamilyHeading
                    font.pixelSize: Theme.fontSizeLarge
                    color: Theme.highlightColor
                }

                Label {
                    objectName: "slideBody" + index
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                    text: modelData.body
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.primaryColor
                }
            }
        }
    }

    // Where the reader is, and -- on the last fact -- what one more
    // swipe does. Both sit inside the cleared box.
    Column {
        anchors {
            horizontalCenter: parent.horizontalCenter
            top: parent.verticalCenter
            topMargin: page.height * 0.25
        }
        width: page.width - 2 * Theme.horizontalPageMargin
        spacing: Theme.paddingLarge

        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Theme.paddingMedium

            Repeater {
                model: page.facts.length

                Rectangle {
                    width: Theme.paddingSmall
                    height: width
                    radius: width / 2
                    color: index === slides.currentIndex ? Theme.highlightColor
                                                         : Theme.primaryColor
                    opacity: index === slides.currentIndex ? 1.0 : 0.3
                }
            }
        }

        Label {
            objectName: "swipeHint"
            width: parent.width
            visible: slides.currentIndex === page.facts.length - 1
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            font.pixelSize: Theme.fontSizeExtraSmall
            color: Theme.secondaryHighlightColor
            text: qsTr("Keep swiping to set up your profile.")

            // A tap on the line does what it describes. Nothing says so,
            // and nothing needs to: it is there for the thumb that finds
            // it rather than pulls once more.
            MouseArea {
                anchors.fill: parent
                onClicked: page.goOn()
            }
        }
    }
}
