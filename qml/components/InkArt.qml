// 2.3 for `Image.mipmap`; Harbour allows up to 2.6 (ci/harbour/).
import QtQuick 2.3
import Sailfish.Silica 1.0

/*
 * A picture in the ambience's colours: one of the drawings the
 * introduction (pages/IntroPage.qml) puts above each fact, painted ahead
 * of time by tools/faces/scenes.py into qml/art/.
 *
 * The same two channels as the field of faces, and for the same reason:
 * what ships has no colour in it, the red channel is what the theme's
 * primary colour draws and the green what its highlight draws, so one
 * file is right on every ambience -- a light one included -- and nothing
 * has to be redrawn when Sailfish gains another. FaceField does this too,
 * over a whole screen, with a crop and a hole cut in it; a picture needs
 * none of that, so it is its own small thing rather than a fifth mode of
 * that one.
 *
 * The pictures are square and this does not letterbox: give it a square.
 */
Item {
    id: art

    /// The picture, as a URL relative to the file that sets it.
    property url source

    /// The colours: the drawing in the first, its accents in the second.
    property color colour: Theme.primaryColor
    property color litColour: Theme.highlightColor
    /// How much of each. A picture is what the reader is looking at, so
    /// it is drawn nearly full strength -- unlike the field behind it.
    property real ink: 0.9
    property real litInk: 1.0

    /// True once there is something to draw.
    readonly property bool ready: mask.status === Image.Ready

    // Loaded off the main thread and never drawn itself, only sampled.
    // Mipmapped: every phone scales the master down, and a drawing scaled
    // down without them crawls at its edges.
    Image {
        id: mask
        source: art.source
        visible: false
        asynchronous: true
        mipmap: true
    }

    ShaderEffect {
        anchors.fill: parent
        // A shader over a texture that is not there yet draws a block of
        // colour.
        visible: art.ready

        property variant source: mask
        property color tint: art.colour
        property color litTint: art.litColour
        property real ink: art.ink
        property real litInk: art.litInk

        // Fixed text; the colours arrive premultiplied and opaque, and
        // what goes out is premultiplied too, which is what the scene
        // graph composites.
        fragmentShader: "
            varying highp vec2 qt_TexCoord0;
            uniform sampler2D source;
            uniform lowp vec4 tint;
            uniform lowp vec4 litTint;
            uniform lowp float ink;
            uniform lowp float litInk;
            uniform lowp float qt_Opacity;

            void main() {
                lowp vec4 drawn = texture2D(source, qt_TexCoord0);
                lowp float body = drawn.r * ink * qt_Opacity;
                lowp float accent = drawn.g * litInk * qt_Opacity;
                gl_FragColor = vec4(tint.rgb, 1.0) * body
                             + vec4(litTint.rgb, 1.0) * accent;
            }"
    }
}
