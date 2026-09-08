"""Draws the MatterLess app icon.

A speech bubble with a bolt through it: the app is a chat client whose whole
argument is that it never makes you wait, and those are the two things worth
saying at 16 pixels.

Drawn at 4x and downsampled rather than drawn at size, because the tray icon is
16px on this machine and PIL's polygon fill has no antialiasing of its own -- a
bolt drawn directly at 16px is a staircase. Previews are written at the sizes
Windows actually asks for so the result can be looked at rather than assumed.

Run it, then `cd app && npx tauri icon ../tools/icon-source.png`.
"""

import os

from PIL import Image, ImageDraw, ImageEnhance

ROOT = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(ROOT)
SIZE = 1024
SCALE = 4
TRAY = 32
OUT = os.path.join(ROOT, "icon-source.png")

# Teal, and a darker teal beneath it: mid enough to hold its own against both a
# light and a dark taskbar, which is where this is mostly seen.
TOP = (23, 158, 186, 255)
BOTTOM = (11, 108, 132, 255)
BOLT = (255, 255, 255, 255)

# In 1024-space; everything is multiplied by SCALE when drawn.
#
# The bubble is deliberately tight and the bolt deliberately fat. At 16px an
# arm narrower than about 2px stops being a shape and becomes a grey smudge, so
# the padding that looks generous at 256 has to go: the first version had a
# bolt that vanished at tray size while reading perfectly at 48.
BODY = (60, 84, 964, 724)
BODY_RADIUS = 180
# Short and wide. A long thin tail is the other thing that turns to mush.
TAIL = [(268, 690), (268, 918), (506, 690)]
BOLT_SHAPE = [
    (650, 140),
    (306, 512),
    (476, 512),
    (378, 868),
    (722, 452),
    (552, 452),
]


def scaled(points):
    return [(x * SCALE, y * SCALE) for x, y in points]


def main():
    canvas = SIZE * SCALE
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))

    # The bubble, as a mask, so the gradient shows only inside it.
    mask = Image.new("L", (canvas, canvas), 0)
    pen = ImageDraw.Draw(mask)
    pen.rounded_rectangle(
        [coordinate * SCALE for coordinate in BODY],
        radius=BODY_RADIUS * SCALE,
        fill=255,
    )
    pen.polygon(scaled(TAIL), fill=255)

    gradient = Image.new("RGBA", (canvas, canvas))
    paint = ImageDraw.Draw(gradient)
    for row in range(canvas):
        along = row / (canvas - 1)
        paint.line(
            [(0, row), (canvas, row)],
            fill=tuple(
                round(TOP[band] + (BOTTOM[band] - TOP[band]) * along) for band in range(4)
            ),
        )
    image.paste(gradient, (0, 0), mask)

    bolt = ImageDraw.Draw(image)
    bolt.polygon(scaled(BOLT_SHAPE), fill=BOLT)

    icon = image.resize((SIZE, SIZE), Image.LANCZOS)
    icon.save(OUT)
    print("wrote", OUT)

    # The sizes Windows asks for: 16 at 100% DPI, then 125/150/200%.
    for size in (16, 20, 24, 32, 48):
        preview = icon.resize((size, size), Image.LANCZOS)
        path = os.path.join(ROOT, f"icon-preview-{size}.png")
        preview.save(path)
        print("wrote", path)

    # The tray gets its own, sharpened.
    #
    # It is drawn at 16px on a 96-DPI screen, and Lanczos softens exactly the
    # edges that carry a shape that small -- the same trap the taskbar badge
    # hit, where handing Windows a bigger bitmap for a small slot read blurrier
    # than drawing for the slot. 32px is the compromise: one clean halving to
    # 16, and still something to work with at 150% and 200%.
    tray = icon.resize((TRAY, TRAY), Image.LANCZOS)
    tray = ImageEnhance.Sharpness(tray).enhance(2.2)
    tray_path = os.path.join(REPO, "app", "src-tauri", "icons", "tray.png")
    tray.save(tray_path)
    print("wrote", tray_path)


if __name__ == "__main__":
    main()
