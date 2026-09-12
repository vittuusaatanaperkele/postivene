#!/usr/bin/env python3
"""Paint the pictures the Delta Chat introduction is drawn with.

The first screen offers a new reader a walk through what Delta Chat is
(qml/pages/IntroPage.qml), one fact per screen, and each fact gets a
picture. They are drawn here, with the same rasteriser and in the same
hand as the field of faces (faces.py): flat shapes a few levels apart,
nothing photographic, nothing that has to be redrawn for a new ambience.

NOT SHIPPED. `make faces` runs this after the field and writes the
pictures to qml/art/, which is what the app draws
(qml/components/InkArt.qml).

# What a picture is

The same two-channel mask as the field: RED is what the theme's primary
colour draws, GREEN what its highlight draws, BLUE nothing. Where a
picture wants an accent -- the plus on a new profile, the shackle of the
lock, the corners of the code -- it goes in green and the app paints it
in the ambience's own highlight, so the pictures belong to whatever
colours the phone is wearing.

The two channels are added by the shader, so ink in both at one pixel
would draw twice as bright; `Plate` takes the grey back out from under
the lit, which is what lets a lit badge sit on a grey disc.

Unlike a face, a picture is the whole square rather than a disc, so the
tiles here are asked not to clip (`Tile(size, clip=False)`).
"""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

# pylint: disable=wrong-import-position
from faces import (ART, DISC, REPO, Box, Dice, Ellipse, HalfPlane, Polygon,
                   Tile, ring, stamp, stroke, write_png)

# Each picture is square and shown at about a third of a phone's width,
# so this is generous; a phone only ever scales it down.
SIZE = 480

# Ink, 0..1, as in faces.py: the levels a picture is drawn in.
LINE = 0.85
SOFT = 0.55
FULL = 1.0


class Plate:
    """One picture, in the two channels the shader reads."""

    def __init__(self, size):
        self.size = size
        self.grey = Tile(size, clip=False)
        self.lit = Tile(size, clip=False)

    def channels(self):
        """The red and green planes, with the grey taken out from under
        the lit: the shader adds the two, and a pixel carrying both would
        come out twice as bright as the picture is meant to be."""
        red = bytearray(self.size * self.size)
        green = bytearray(self.size * self.size)
        stamp(red, self.size, self.size, self.grey, 0, 0)
        stamp(green, self.size, self.size, self.lit, 0, 0)
        for index, value in enumerate(green):
            if value:
                red[index] = int(red[index] * (1.0 - value / 255.0))
        return red, green


def bust(cx, cy, r):
    """The app's own avatar at any size: the disc, and the person on it."""
    disc = Ellipse(cx, cy, r)
    head = Ellipse(cx, cy - 0.30 * r, 0.38 * r, 0.42 * r)
    shoulders = Ellipse(cx, cy + 0.92 * r, 0.86 * r, 0.62 * r)
    return disc, (head | shoulders) & disc


def draw_profile(plate):
    """A profile of one's own: an avatar with a plus on it."""
    disc, person = bust(-0.10, -0.10, 0.72)
    plate.grey.fill(disc, DISC)
    plate.grey.fill(person, LINE)
    at = 0.56
    badge = Ellipse(at, at, 0.36)
    plus = (Box(at - 0.19, at - 0.06, at + 0.19, at + 0.06, 0.02)
            | Box(at - 0.06, at - 0.19, at + 0.06, at + 0.19, 0.02))
    plate.lit.fill(badge - plus, FULL)


# A scannable code the size a small one really is: twenty-one modules
# across, with the three corner marks, the separators around them and the
# timing runs between them where a reader of codes expects to find them.
# Only the data modules are made up.
MODULES = 21
SPAN = 1.80
MODULE = SPAN / MODULES
# The corner marks, by the module their seven-by-seven block starts at.
MARKS = ((0, 0), (MODULES - 7, 0), (0, MODULES - 7))


def module(column, row):
    """One module of the code. A hair wider than its cell on every side,
    so that neighbours meet rather than leave a seam where the edge
    pixel is shared."""
    x = -SPAN / 2 + column * MODULE
    y = -SPAN / 2 + row * MODULE
    bleed = MODULE * 0.02
    return Box(x - bleed, y - bleed, x + MODULE + bleed, y + MODULE + bleed)


def in_mark(column, row):
    """Whether a module belongs to a corner mark or its separator, which
    is what keeps the made-up half out of the half a scanner reads."""
    return any(mx - 1 <= column <= mx + 7 and my - 1 <= row <= my + 7
               for mx, my in MARKS)


def mark_module(column, row):
    """Whether a module of a corner mark is drawn: the seven-by-seven
    ring, and the three-by-three block inside it."""
    for mx, my in MARKS:
        if mx <= column <= mx + 6 and my <= row <= my + 6:
            dx, dy = abs(column - (mx + 3)), abs(row - (my + 3))
            return max(dx, dy) == 3 or max(dx, dy) <= 1
    return False


def draw_invite(plate):
    """The code a friend scans, or the link they are sent."""
    dice = Dice(7)
    for row in range(MODULES):
        for column in range(MODULES):
            if in_mark(column, row):
                if mark_module(column, row):
                    plate.lit.fill(module(column, row), FULL)
            elif column == 6 or row == 6:
                # The timing runs: every other module, starting filled.
                if (column + row) % 2 == 0:
                    plate.grey.fill(module(column, row), LINE)
            elif dice.random() < 0.45:
                plate.grey.fill(module(column, row), LINE)


def draw_lock(plate):
    """A padlock, shut: the body in the page's own colour, the shackle
    in the ambience's."""
    body = Box(-0.58, -0.04, 0.58, 0.74, 0.16)
    keyhole = (Ellipse(0, 0.26, 0.13)
               | Polygon([(-0.07, 0.26), (0.07, 0.26), (0.12, 0.58), (-0.12, 0.58)]))
    plate.grey.fill(body - keyhole, LINE)
    plate.lit.fill(ring(0, -0.06, 0.34, 0.40, 0.13) & HalfPlane(0, -1, 0.04), FULL)


# Three people, the last one in front; the second is the one who wrote,
# as a lit face is on the cover.
GROUP = ((-0.40, -0.32, 0.44), (0.40, -0.32, 0.44), (0.00, 0.46, 0.44))


def draw_group(plate):
    """A group of equals: three avatars, none of them in charge."""
    for index, (cx, cy, r) in enumerate(GROUP):
        halo = Ellipse(cx, cy, r + 0.05)
        plate.grey.fill(halo, 0.0)
        plate.lit.fill(halo, 0.0)
        disc, person = bust(cx, cy, r)
        tile = plate.lit if index == 1 else plate.grey
        tile.fill(disc, 0.35 if index == 1 else DISC)
        tile.fill(person, FULL if index == 1 else LINE)


def draw_relay(plate):
    """A message handed on: an envelope, and the hop it takes over it."""
    body = Box(-0.62, -0.06, 0.62, 0.62, 0.08)
    plate.grey.fill(body, SOFT)
    flap = (stroke(-0.62, -0.06, 0.0, 0.34, 0.06)
            | stroke(0.62, -0.06, 0.0, 0.34, 0.06)) & body
    plate.grey.fill(flap, LINE)
    plate.lit.fill(ring(0, 0.02, 0.86, 0.76, 0.07) & HalfPlane(0, -1, 0.32), FULL)
    # The head, at the right-hand end of the arc and pointing the way the
    # arc is going -- down towards the envelope, not back along itself.
    plate.lit.fill(Polygon([(0.88, -0.10), (0.60, -0.28), (0.87, -0.45)]), FULL)


SCENES = (
    ("intro-profile.png", draw_profile),
    ("intro-invite.png", draw_invite),
    ("intro-lock.png", draw_lock),
    ("intro-group.png", draw_group),
    ("intro-relay.png", draw_relay),
)


def main(argv):
    wanted = set(argv[1:])
    ART.mkdir(parents=True, exist_ok=True)
    for name, draw in SCENES:
        if wanted and name not in wanted:
            continue
        plate = Plate(SIZE)
        draw(plate)
        red, green = plate.channels()
        target = ART / name
        write_png(target, SIZE, SIZE, red, green)
        print("%s  %dx%d  %d bytes" % (target.relative_to(REPO), SIZE, SIZE,
                                       target.stat().st_size))


if __name__ == "__main__":
    main(sys.argv)
