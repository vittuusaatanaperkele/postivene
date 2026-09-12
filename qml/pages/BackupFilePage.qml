import QtQuick 2.0
import Sailfish.Pickers 1.0

/*
 * A backup off the phone: the platform's file browser, showing only the
 * files a Delta Chat backup can be in.
 *
 * The library picker beside it cannot serve here, for the reason the
 * webxdc one cannot: it lists what the media index knows about, and a
 * backup copied over from another device is a file the index has no
 * opinion on.
 *
 * A page of its own, pushed by URL, for the reason every picker page is
 * one: `Sailfish.Pickers` types resolve when the file naming them is
 * loaded, so a type that is not there costs this button rather than the
 * page that offers it.
 */
FilePickerPage {
    id: picker

    /// The absolute path of the chosen backup.
    signal picked(string path)

    // What the core writes: `export_backup` makes a .tar, and that is
    // what every Delta Chat has written since the format changed.
    nameFilters: ["*.tar"]

    onSelectedContentPropertiesChanged: {
        if (selectedContentProperties.filePath) {
            picker.picked(selectedContentProperties.filePath)
        }
    }
}
