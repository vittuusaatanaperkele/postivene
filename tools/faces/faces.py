#!/usr/bin/env python3
"""Paint the field of faces the first screen is drawn over.

The welcome page shows a new reader what the cover shows an old one: a
staggered grid of round avatars, in grey, with a few of them lit in the
ambience's own colour. Nobody is known yet, so the faces are made up --
drawn, not photographed, and drawn here, ahead of time, rather than on
the phone: a page of a hundred faces through the cover's own avatar
effects is a hundred shader passes with a texture each, which is not a
first impression, and a picture costs nothing to show and looks the same
on every phone.

NOT SHIPPED. `make faces` runs this and writes the masks to qml/art/,
which is what the app draws (qml/components/FaceField.qml).

# What a mask is

An RGB PNG, of which two channels mean something: RED is how much ink
each pixel of a grey face carries, GREEN the same for a lit one, and BLUE
is nothing. No colour at all: the app tints red with the theme's primary
colour and green with its highlight, so one file is right on every
ambience, a light one included, and there is nothing to regenerate when
Sailfish gains another. The room for the words in the middle is not cut
here either: it is geometry only the page knows, and the shader clears it.

Two masters, one per orientation, each at the largest size a Sailfish
phone has along its short side (1080), so a phone only ever scales them
down. The app crops rather than stretches, keeping each master's own
proportions, which is why one master serves every screen from 16:9 to
21:9 the same way up.

# The faces

Each is a bust in a disc, as the app's own avatars are: a head, hair of
one of nine kinds or none, shoulders in a shirt, and now and then
glasses, a beard, earrings, or a headscarf. One in five is the other
kind of avatar the app has, an initial on a disc, in a rounded stroke
that needs no font. Everything is a few grey levels apart from the disc,
so it reads in one tint. Which face goes where is fixed by a seed and a
generator of this file's own (`Dice`), so regenerating the masks changes
nothing unless this file does -- whichever Python does the painting.

# How it is drawn

A small scanline rasteriser over convex primitives -- ellipses, rounded
boxes, convex polygons, half-planes -- combined with union, intersection
and difference, and filled with four sub-rows per pixel and exact
horizontal coverage, so the edges are smooth without a supersampled
buffer. Pure standard library on purpose: this runs from `make faces`,
which a contributor may run on any machine, and a Pillow that has to be
installed first is a reason not to regenerate the art.
"""
import math
import pathlib
import struct
import sys
import zlib

REPO = pathlib.Path(__file__).resolve().parent.parent.parent
ART = REPO / "qml" / "art"

# The masters. `columns` is how many whole cells fit across, and it is
# what makes the field dense: the faces are as wide as the master is
# divided by it, so six across a phone's width -- and the same face size
# on its side -- is a crowd rather than a handful of large portraits,
# which is what a field is meant to read as.
MASTERS = (
    {"file": "faces-portrait.png", "width": 1080, "height": 2520, "columns": 6,
     "clear": (0.30, 0.22), "lit": 13, "seed": 11},
    {"file": "faces-landscape.png", "width": 2520, "height": 1080, "columns": 14,
     "clear": (0.24, 0.36), "lit": 13, "seed": 23},
)

# Ink, 0..1: how much of the tint a pixel gets. The disc is what the
# app's own avatar draws behind an initial, a quarter of the colour.
DISC = 0.25
SUBROWS = 4
# Coverage is quantised on the way out: the shapes are flat, so the
# levels only ever show at their edges, and thirty-two of them compress
# to well under half of what the full range does.
LEVELS = 32


# ----------------------------------------------------------------- dice

class Dice:
    """A pseudorandom sequence of its own, fixed by a seed.

    Not `random.Random`: the standard library promises the same
    `random()` for a seed across versions, but not the same `choice`,
    `shuffle` or `uniform`, and a mask is meant to come out the same to
    the byte from whichever Python paints it. This is splitmix64, a
    dozen lines that never change. Nothing about it is secret: the seed
    picks which face goes where.
    """
    MASK = (1 << 64) - 1

    def __init__(self, seed):
        self.state = seed & self.MASK

    def roll(self):
        """The next 64 bits."""
        self.state = (self.state + 0x9E3779B97F4A7C15) & self.MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & self.MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & self.MASK
        return z ^ (z >> 31)

    def random(self):
        """A float in [0, 1)."""
        return (self.roll() >> 11) / float(1 << 53)

    def uniform(self, low, high):
        return low + (high - low) * self.random()

    def choice(self, options):
        return options[self.roll() % len(options)]

    def shuffle(self, items):
        """In place, as Fisher and Yates have it."""
        for i in range(len(items) - 1, 0, -1):
            j = self.roll() % (i + 1)
            items[i], items[j] = items[j], items[i]


# --------------------------------------------------------------- shapes

class Shape:
    """Something with a horizontal extent on every scanline."""

    def spans(self, y):
        """The `(x0, x1)` intervals covered at height `y`, in order."""
        raise NotImplementedError

    def __or__(self, other):
        return Union(self, other)

    def __and__(self, other):
        return Intersection(self, other)

    def __sub__(self, other):
        return Difference(self, other)


class Ellipse(Shape):
    def __init__(self, cx, cy, rx, ry=None):
        self.cx, self.cy, self.rx, self.ry = cx, cy, rx, rx if ry is None else ry

    def spans(self, y):
        t = (y - self.cy) / self.ry
        if t <= -1.0 or t >= 1.0:
            return []
        w = self.rx * math.sqrt(1.0 - t * t)
        return [(self.cx - w, self.cx + w)]


class Box(Shape):
    """An axis-aligned box with rounded corners."""

    def __init__(self, x0, y0, x1, y1, r=0.0):
        self.x0, self.y0, self.x1, self.y1 = x0, y0, x1, y1
        self.r = min(r, (x1 - x0) / 2, (y1 - y0) / 2)

    def spans(self, y):
        if y <= self.y0 or y >= self.y1:
            return []
        inset = 0.0
        if y < self.y0 + self.r:
            dy = self.y0 + self.r - y
            inset = self.r - math.sqrt(self.r * self.r - dy * dy)
        elif y > self.y1 - self.r:
            dy = y - (self.y1 - self.r)
            inset = self.r - math.sqrt(self.r * self.r - dy * dy)
        return [(self.x0 + inset, self.x1 - inset)]


class Polygon(Shape):
    """A convex polygon, given as its corners in order."""

    def __init__(self, points):
        self.points = points

    def spans(self, y):
        crossings = []
        n = len(self.points)
        for i in range(n):
            (ax, ay), (bx, by) = self.points[i], self.points[(i + 1) % n]
            if (ay <= y) == (by <= y):
                continue
            crossings.append(ax + (y - ay) * (bx - ax) / (by - ay))
        if len(crossings) < 2:
            return []
        return [(min(crossings), max(crossings))]


class HalfPlane(Shape):
    """Everything on one side of a line: `a*x + b*y >= c`."""

    def __init__(self, a, b, c):
        self.a, self.b, self.c = a, b, c

    def spans(self, y):
        if self.a == 0:
            return [(-1e9, 1e9)] if self.b * y >= self.c else []
        edge = (self.c - self.b * y) / self.a
        return [(edge, 1e9)] if self.a > 0 else [(-1e9, edge)]


def merge(spans):
    """Sorted, non-overlapping intervals from any list of them."""
    out = []
    for x0, x1 in sorted(spans):
        if out and x0 <= out[-1][1]:
            out[-1] = (out[-1][0], max(out[-1][1], x1))
        else:
            out.append((x0, x1))
    return out


class Union(Shape):
    def __init__(self, a, b):
        self.a, self.b = a, b

    def spans(self, y):
        return merge(self.a.spans(y) + self.b.spans(y))


class Intersection(Shape):
    def __init__(self, a, b):
        self.a, self.b = a, b

    def spans(self, y):
        out = []
        for a0, a1 in self.a.spans(y):
            for b0, b1 in self.b.spans(y):
                lo, hi = max(a0, b0), min(a1, b1)
                if lo < hi:
                    out.append((lo, hi))
        return merge(out)


class Difference(Shape):
    def __init__(self, a, b):
        self.a, self.b = a, b

    def spans(self, y):
        out = []
        holes = merge(self.b.spans(y))
        for a0, a1 in self.a.spans(y):
            cursor = a0
            for h0, h1 in holes:
                if h1 <= cursor or h0 >= a1:
                    continue
                if h0 > cursor:
                    out.append((cursor, h0))
                cursor = max(cursor, h1)
            if cursor < a1:
                out.append((cursor, a1))
        return out


def stroke(x0, y0, x1, y1, width):
    """A line from one point to another, with round ends."""
    dx, dy = x1 - x0, y1 - y0
    length = math.hypot(dx, dy) or 1.0
    nx, ny = -dy / length * width / 2, dx / length * width / 2
    body = Polygon([(x0 + nx, y0 + ny), (x1 + nx, y1 + ny),
                    (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)])
    return body | Ellipse(x0, y0, width / 2) | Ellipse(x1, y1, width / 2)


def ring(cx, cy, rx, ry, width):
    """An ellipse's outline."""
    return Ellipse(cx, cy, rx, ry) - Ellipse(cx, cy, rx - width, ry - width)


# ------------------------------------------------------------- a canvas

class Tile:
    """A square of ink levels, filled one shape at a time.

    Coordinates are the avatar's own: the disc is the unit circle, y
    down, and `scale` turns that into pixels. Everything drawn is clipped
    to the disc, so a shape can spill over its edge freely -- unless the
    tile is asked for the whole square, which is what the intro pictures
    (scenes.py) are drawn on.
    """

    def __init__(self, size, clip=True):
        self.size = size
        self.scale = size / 2.0
        self.rows = [[0.0] * size for _ in range(size)]
        self.clip = Ellipse(self.scale, self.scale, self.scale - 0.5) if clip else None

    def fill(self, shape, ink):
        shape = Scaled(shape, self.scale)
        if self.clip is not None:
            shape = shape & self.clip
        for py in range(self.size):
            diff, partial, lo, hi = self.coverage(shape, py)
            if lo <= hi:
                self.blend(py, lo, hi, diff, partial, ink)

    def coverage(self, shape, py):
        """One pixel row's coverage of a shape: the pixels wholly inside
        a span as a difference array, the part pixels at each span's
        ends apart, and the range of pixels touched."""
        size = self.size
        diff = [0.0] * (size + 1)
        partial = {}
        lo, hi = size, 0
        for k in range(SUBROWS):
            for x0, x1 in shape.spans(py + (k + 0.5) / SUBROWS):
                x0, x1 = max(0.0, x0), min(float(size), x1)
                if x1 > x0:
                    lo, hi = min(lo, int(x0)), max(hi, min(int(x1), size - 1))
                    add_span(diff, partial, x0, x1, size)
        return diff, partial, lo, hi

    def blend(self, py, lo, hi, diff, partial, ink):
        row = self.rows[py]
        run = 0.0
        for x in range(lo, hi + 1):
            run += diff[x]
            coverage = min(1.0, run + partial.get(x, 0.0))
            if coverage > 0.0:
                row[x] += (ink - row[x]) * coverage


def add_span(diff, partial, x0, x1, size):
    """One sub-row's span, a fraction of a pixel high: the pixels wholly
    inside it into `diff`, the two at its ends into `partial`."""
    weight = 1.0 / SUBROWS
    xa, xb = int(x0), int(x1)
    if xa == xb:
        partial[xa] = partial.get(xa, 0.0) + (x1 - x0) * weight
        return
    partial[xa] = partial.get(xa, 0.0) + (xa + 1 - x0) * weight
    if xb < size:
        partial[xb] = partial.get(xb, 0.0) + (x1 - xb) * weight
    diff[xa + 1] += weight
    diff[xb] -= weight


class Scaled(Shape):
    """A shape in avatar coordinates, seen in pixels."""

    def __init__(self, inner, scale):
        self.inner, self.scale = inner, scale

    def spans(self, y):
        s = self.scale
        return [((x0 + 1.0) * s, (x1 + 1.0) * s)
                for x0, x1 in self.inner.spans(y / s - 1.0)]


# ------------------------------------------------------------- the faces

# Letters, as strokes from point to point on a unit disc; the rest are
# drawn with rings below.
LETTERS = {
    "A": [[(-.36, .45), (0, -.45), (.36, .45)], [(-.2, .15), (.2, .15)]],
    "E": [[(.3, -.45), (-.3, -.45), (-.3, .45), (.3, .45)], [(-.3, 0), (.2, 0)]],
    "F": [[(.3, -.45), (-.3, -.45), (-.3, .45)], [(-.3, 0), (.2, 0)]],
    "H": [[(-.3, -.45), (-.3, .45)], [(.3, -.45), (.3, .45)], [(-.3, 0), (.3, 0)]],
    "I": [[(0, -.45), (0, .45)]],
    "K": [[(-.3, -.45), (-.3, .45)], [(.3, -.45), (-.3, .05)], [(-.14, -.08), (.32, .45)]],
    "L": [[(-.3, -.45), (-.3, .45), (.3, .45)]],
    "M": [[(-.4, .45), (-.4, -.45), (0, .1), (.4, -.45), (.4, .45)]],
    "N": [[(-.32, .45), (-.32, -.45), (.32, .45), (.32, -.45)]],
    "T": [[(-.36, -.45), (.36, -.45)], [(0, -.45), (0, .45)]],
    "V": [[(-.36, -.45), (0, .45), (.36, -.45)]],
    "W": [[(-.44, -.45), (-.22, .45), (0, -.12), (.22, .45), (.44, -.45)]],
    "X": [[(-.32, -.45), (.32, .45)], [(.32, -.45), (-.32, .45)]],
    "Y": [[(-.32, -.45), (0, 0), (.32, -.45)], [(0, 0), (0, .45)]],
    "Z": [[(-.32, -.45), (.32, -.45), (-.32, .45), (.32, .45)]],
}
STROKE = 0.16


def round_letter(name):
    """The three capitals with a bowl, which strokes cannot draw."""
    if name == "U":
        bowl = ring(0, .12, .32, .34, STROKE) & HalfPlane(0, 1, .12)
        return bowl | stroke(-.32 + STROKE / 2, -.45, -.32 + STROKE / 2, .12, STROKE) \
            | stroke(.32 - STROKE / 2, -.45, .32 - STROKE / 2, .12, STROKE)
    whole = ring(0, 0, .38, .46, STROKE)
    return whole if name == "O" else whole - Polygon([(0, 0), (.8, -.5), (.8, .5)])


def letter(name):
    """A capital, centred on the disc."""
    if name in ROUND:
        return round_letter(name)
    shape = None
    for line in LETTERS[name]:
        for (x0, y0), (x1, y1) in zip(line, line[1:]):
            piece = stroke(x0, y0, x1, y1, STROKE)
            shape = piece if shape is None else shape | piece
    return shape


ROUND = "OCU"
ALPHABET = "".join(sorted(LETTERS)) + ROUND


def head_shape(kind):
    """The head: round, oval, or long."""
    rx, ry = {"round": (.42, .45), "oval": (.38, .47), "long": (.35, .49)}[kind]
    return Ellipse(0, -.16, rx, ry), rx, ry


def curls(rx, ry):
    """Nine curls around the top of the head, before the face is cut out."""
    shape = None
    for i in range(9):
        angle = math.pi + i * math.pi / 8
        curl = Ellipse(math.cos(angle) * rx * 1.02, -.16 + math.sin(angle) * ry * 1.02, .17)
        shape = curl if shape is None else shape | curl
    return shape


def hair_shape(style, rx, ry, rng):
    """Hair of one of the kinds, over a head of the given size, or None."""
    top = Ellipse(0, -.19, rx * 1.06, ry * 1.06)
    crown = top & HalfPlane(0, -1, .28)
    styles = {
        "short": lambda: crown,
        "swept": lambda: top & HalfPlane(rng.choice((-1, 1)) * 0.55, -1, .30),
        "long": lambda: (top & HalfPlane(0, -1, .24))
        | (Box(-rx - .12, -.45, rx + .12, .62, .22) - Ellipse(0, -.10, rx * .92, ry * .96)),
        "bun": lambda: crown | Ellipse(0, -.72, .15),
        "curly": lambda: curls(rx, ry) - Ellipse(0, -.10, rx * .90, ry * .92),
        "beanie": lambda: (Ellipse(0, -.42, rx + .04, .36) & HalfPlane(0, -1, .34))
        | Box(-rx - .06, -.40, rx + .06, -.28, .04),
    }
    return styles[style]() if style in styles else None


def scarf_shape(rx, ry):
    """A headscarf: around the head, over the ears, down to the shoulders."""
    around = Ellipse(0, -.12, rx + .16, ry + .18) | Box(-rx - .2, -.1, rx + .2, .7, .3)
    return around - Ellipse(0, -.12, rx * .82, ry * .88)


def draw_bust(tile, rng):
    """One person: shoulders, head, hair, and whatever else they wear."""
    skin = rng.uniform(.56, .70)
    shirt = rng.uniform(.38, .50)
    hair_ink = rng.choice((.86, .86, .80, .42))
    kind, rx, ry = head_shape(rng.choice(("round", "oval", "long")))
    style = rng.choice(("short", "short", "swept", "swept", "long", "long",
                        "bun", "curly", "curly", "beanie", "bald", "scarf"))

    tile.fill(Ellipse(0, 0, 1), DISC)
    if style == "long":
        tile.fill(Box(-rx - .12, -.3, rx + .12, .9, .22), hair_ink)
    tile.fill(Ellipse(0, 1.08, .92, .78), shirt)
    tile.fill(Box(-.15, .05, .15, .5, .04), skin)
    if style == "scarf":
        tile.fill(scarf_shape(rx, ry), rng.uniform(.30, .48))
    elif style != "long":
        tile.fill(Ellipse(-rx - .02, -.13, .09, .11) | Ellipse(rx + .02, -.13, .09, .11), skin)
    tile.fill(kind, skin)
    hair = hair_shape(style, rx, ry, rng)
    if hair is not None:
        tile.fill(hair, .46 if style == "beanie" else hair_ink)
    if style in ("short", "swept", "bald", "curly") and rng.random() < .3:
        tile.fill(Ellipse(0, -.14, rx * .98, ry * 1.03) & HalfPlane(0, 1, .16), hair_ink)
    if rng.random() < .2:
        glasses = ring(-.17, -.12, .13, .12, .035) | ring(.17, -.12, .13, .12, .035) \
            | Box(-.06, -.14, .06, -.11)
        tile.fill(glasses, .9)
    if style in ("long", "bun", "scarf", "curly") and rng.random() < .4:
        tile.fill(Ellipse(-rx - .03, .02, .035) | Ellipse(rx + .03, .02, .035), .9)


def draw_initial(tile, rng):
    """The other avatar: a letter on the disc."""
    tile.fill(Ellipse(0, 0, 1), DISC)
    tile.fill(letter(rng.choice(ALPHABET)), .9)


def draw_avatar(size, seed):
    """A tile with one avatar on it, fixed by its seed."""
    rng = Dice(seed)
    tile = Tile(size)
    if rng.random() < .2:
        draw_initial(tile, rng)
    else:
        draw_bust(tile, rng)
    return tile


# ------------------------------------------------------------- the grid

def cells(width, height, columns):
    """The cover's grid: `columns` across, every other row shifted half a
    cell and holding one more, the rows nested at nine tenths of a cell,
    cut off at the edges. Yields `(x, y, size, whole)` per cell."""
    cell = width // columns
    step = max(1, round(cell * 0.9))
    rows = -(-height // step)
    for row in range(rows):
        across = columns + 1 if row % 2 == 0 else columns
        for col in range(across):
            x = col * cell - (cell // 2 if row % 2 == 0 else 0)
            y = row * step
            whole = x >= 0 and x + cell <= width and y + cell <= height
            yield x, y, cell, whole


def choose_lit(grid, master, rng):
    """Which cells are lit: some of the whole ones, spread out, and none
    under the words in the middle -- the page clears that room, so a
    face lit there would be a face lit for nothing."""
    width, height = master["width"], master["height"]
    keep_x, keep_y = master["clear"]
    candidates = [
        (x, y) for x, y, size, whole in grid
        if whole and (abs(x + size / 2 - width / 2) > keep_x * width
                      or abs(y + size / 2 - height / 2) > keep_y * height)
    ]
    rng.shuffle(candidates)
    lit = []
    spacing = 1.7 * (width // master["columns"])
    for x, y in candidates:
        if len(lit) == master["lit"]:
            break
        if all(math.hypot(x - lx, y - ly) >= spacing for lx, ly in lit):
            lit.append((x, y))
    return set(lit)


def paint(master):
    """Both channels of one master."""
    width, height = master["width"], master["height"]
    red = bytearray(width * height)
    green = bytearray(width * height)
    grid = list(cells(width, height, master["columns"]))
    rng = Dice(master["seed"])
    lit = choose_lit(grid, master, rng)
    gap = max(2, round(grid[0][2] * 0.025))
    for index, (x, y, size, _) in enumerate(grid):
        tile = draw_avatar(size - gap, master["seed"] * 1000 + index)
        stamp(green if (x, y) in lit else red, width, height, tile, x, y)
    return red, green


def stamp(channel, width, height, tile, x, y):
    """Copy a tile onto a channel, clipped to the master."""
    step = 255.0 / (LEVELS - 1)
    for ty, row in enumerate(tile.rows):
        py = y + ty
        if py < 0 or py >= height:
            continue
        base = py * width
        for tx, ink in enumerate(row):
            px = x + tx
            if 0 <= px < width and ink > 0.0:
                channel[base + px] = int(round(ink * 255.0 / step) * step)


# ---------------------------------------------------------------- output

def png_chunk(kind, body):
    return (struct.pack(">I", len(body)) + kind + body
            + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF))


def write_png(path, width, height, red, green):
    """An 8-bit RGB PNG with the two channels in it. Filtered with `Up`
    throughout: the faces are stacked rows of near-identical scanlines,
    which is what that filter is for."""
    raw = bytearray()
    previous = bytes(width * 3)
    for y in range(height):
        base = y * width
        line = bytes(v for pair in zip(red[base:base + width], green[base:base + width])
                     for v in (pair[0], pair[1], 0))
        raw.append(2)
        raw += bytes((a - b) & 0xFF for a, b in zip(line, previous))
        previous = line
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n"
                     + png_chunk(b"IHDR", header)
                     + png_chunk(b"IDAT", zlib.compress(bytes(raw), 9))
                     + png_chunk(b"IEND", b""))


def main(argv):
    wanted = set(argv[1:])
    ART.mkdir(parents=True, exist_ok=True)
    for master in MASTERS:
        if wanted and master["file"] not in wanted:
            continue
        red, green = paint(master)
        target = ART / master["file"]
        write_png(target, master["width"], master["height"], red, green)
        print("%s  %dx%d  %d KiB" % (target.relative_to(REPO), master["width"],
                                     master["height"], target.stat().st_size // 1024))


if __name__ == "__main__":
    main(sys.argv)
