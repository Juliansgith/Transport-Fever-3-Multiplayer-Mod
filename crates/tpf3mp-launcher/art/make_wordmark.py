# Usage: python make_wordmark.py <wordmark.png> <screenshot.png>
#
# The launcher's wordmark: Transport Fever 3's logo, lifted white from a
# screenshot of the game's title (the scene behind it removed), with a
# black "MULTIPLAYER" band beneath, as tearded's TF2 launcher has one. The
# page gives it a soft shadow to stand on the city behind.
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont
from scipy import ndimage

FONT = "C:/Windows/Fonts/ariblk.ttf"
SCALE = 2           # output at twice the screenshot's size, for sharp HiDPI
WORK = 8            # edges are rebuilt at this many times the screenshot
SHEAR = 0.21        # the band's italic slant
PAD = 6             # clear space around the logo, at the screenshot's size

rgb = np.array(Image.open(sys.argv[2]).convert("RGB")).astype(float)
lo, hi = rgb.min(2), rgb.max(2)
# White and colourless is the logo; everything else is the scene.
alpha = np.clip((lo - 150) / 70, 0, 1) * np.clip(1 - (hi - lo - 25) / 30, 0, 1)
# Only the logo's own marks, not the city's lights: the large ones, and
# inside the logo's box the ® (its ring and its R, 20 pixels and more at
# the title screen's size); the lights showing through are smaller.
solid = alpha > 0.5
labels, n = ndimage.label(solid)
sizes = ndimage.sum(solid, labels, range(1, n + 1))
big = [i + 1 for i, size in enumerate(sizes) if size >= 60]
ys, xs = np.nonzero(np.isin(labels, big))
top, bottom, left, right = ys.min(), ys.max(), xs.min(), xs.max()
keep = np.zeros_like(solid)
for i, size in enumerate(sizes, start=1):
    ly, lx = np.nonzero(labels == i)
    inside = top <= ly.min() and ly.max() <= bottom and left <= lx.min() and lx.max() <= right
    if size >= 60 or (size >= 20 and inside):
        keep |= labels == i
keep = ndimage.binary_dilation(keep, iterations=2)
alpha = (alpha * keep)[top - PAD : bottom + PAD + 1, left - PAD : right + PAD + 1]
# Specks of the scene inside a letter: a gap in the white that nothing
# outside reaches and that is smaller than any real counter (the inside of
# an O or an R) is filled.
gaps, count = ndimage.label(alpha < 0.5)
for i in range(1, count + 1):
    where = gaps == i
    gy, gx = np.nonzero(where)
    touches = gy.min() == 0 or gx.min() == 0 or gy.max() == alpha.shape[0] - 1 or gx.max() == alpha.shape[1] - 1
    if not touches and where.sum() < 12:
        alpha[where] = 1.0

# Rebuild the edges large: smooth the screenshot's pixel steps, cut back to
# a crisp edge, then reduce to the output size, which antialiases it.
logo = Image.fromarray((alpha * 255).astype("uint8"))
w, h = logo.size
logo = logo.resize((w * WORK, h * WORK), Image.BICUBIC)
logo = logo.filter(ImageFilter.GaussianBlur(WORK * 0.45))
logo = logo.point(lambda v: 0 if v < 105 else 255 if v > 150 else int((v - 105) * 255 / 45))
logo = logo.resize((w * SCALE, h * SCALE), Image.LANCZOS)
lw, lh = logo.size

# The band, as wide as the logo, drawn large and reduced.
band_h = round(lh * 0.24)
gap = round(lh * 0.06)
bw, bh = lw * 4, band_h * 4
band = Image.new("RGBA", (bw, bh), (10, 10, 10, 255))
text = Image.new("L", (bw, bh), 0)
font = ImageFont.truetype(FONT, int(bh * 0.6))
word = "MULTIPLAYER"
track = bh * 0.32
width = sum(font.getlength(c) for c in word) + track * (len(word) - 1)
m_top, m_bottom = font.getbbox("M")[1], font.getbbox("M")[3]
x = (bw - width) / 2
y = (bh - (m_bottom - m_top)) / 2 - m_top
draw = ImageDraw.Draw(text)
for c in word:
    draw.text((x, y), c, font=font, fill=255)
    x += font.getlength(c) + track
text = text.transform(text.size, Image.AFFINE, (1, SHEAR, -SHEAR * bh / 2, 0, 1, 0), Image.BICUBIC)
band.paste((255, 255, 255, 255), mask=text)
band = band.resize((lw, band_h), Image.LANCZOS)

out = Image.new("RGBA", (lw, lh + gap + band_h), (0, 0, 0, 0))
white = Image.new("RGBA", logo.size, (255, 255, 255, 255))
white.putalpha(logo)
out.alpha_composite(white, (0, 0))
out.alpha_composite(band, (0, lh + gap))
out.save(sys.argv[1])
back = Image.new("RGBA", out.size, (40, 60, 70, 255))
back.alpha_composite(out)
back.save(sys.argv[1].replace(".png", "-on-dark.png"))
print(out.size)
