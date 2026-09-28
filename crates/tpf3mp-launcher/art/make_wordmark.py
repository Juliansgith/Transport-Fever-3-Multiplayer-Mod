# Usage: python make_wordmark.py <wordmark.png> <screenshot.png>
#
# The launcher's wordmark: Transport Fever 3's logo, lifted from a
# screenshot, on a black outline, with a black "MULTIPLAYER" band beneath,
# as tearded's TF2 launcher did it.
import sys
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter
from scipy import ndimage

# The game's title screen, its logo white on the scene behind it.
SRC = sys.argv[2]
# Worked at six times the screenshot's size, then halved: the logo in a
# screenshot is small, and its edges are rebuilt smooth at the large size.
SCALE = 6
OUTLINE = 44      # px at the working scale
CLOSE = 92
FONT = "C:/Windows/Fonts/ariblk.ttf"
SHEAR = 0.21

rgb = np.array(Image.open(SRC).convert("RGB")).astype(float)
lo, hi = rgb.min(2), rgb.max(2)
# White and colourless: the logo. Soft edges, from brightness.
alpha = np.clip((lo - 150) / 70, 0, 1) * np.clip(1 - (hi - lo - 25) / 30, 0, 1)
# Keep only the logo: marks inside its box, not the city's lights.
solid = alpha > 0.5
labels, n = ndimage.label(solid)
sizes = ndimage.sum(solid, labels, range(1, n + 1))
keep = np.zeros_like(solid)
for i, size in enumerate(sizes, start=1):
    ys, xs = np.nonzero(labels == i)
    if size >= 25 and 60 <= ys.min() and ys.max() <= 205 and 95 <= xs.min() and xs.max() <= 515:
        keep |= labels == i
keep = ndimage.binary_dilation(keep, iterations=2)
alpha = alpha * keep
ys, xs = np.nonzero(alpha > 0.05)
alpha = alpha[ys.min() - 2 : ys.max() + 3, xs.min() - 2 : xs.max() + 3]

logo = Image.fromarray((alpha * 255).astype("uint8"))
logo = logo.resize((logo.width * SCALE, logo.height * SCALE), Image.BICUBIC)
# Smooth the staircase of the small screenshot, then cut it back to a crisp
# edge a pixel or two wide: curves come out round, not stepped.
logo = logo.filter(ImageFilter.GaussianBlur(SCALE * 1.0))
logo = logo.point(lambda v: 0 if v < 110 else 255 if v > 146 else int((v - 110) * 255 / 36))
lw, lh = logo.size

def grow(mask, px):
    out = mask
    while px > 0:
        step = min(px, 10)
        out = out.filter(ImageFilter.MaxFilter(step * 2 + 1))
        px -= step
    return out

def shrink(mask, px):
    out = mask
    while px > 0:
        step = min(px, 10)
        out = out.filter(ImageFilter.MinFilter(step * 2 + 1))
        px -= step
    return out

PAD = 80
band_h = int(lh * 0.22)
gap = int(lh * 0.09)
W = lw + 2 * PAD
H = lh + gap + band_h + 2 * PAD
fill = Image.new("L", (W, H), 0)
fill.paste(logo, (PAD, PAD))
hard = fill.point(lambda v: 255 if v > 110 else 0)
black = shrink(grow(grow(hard, OUTLINE), CLOSE), CLOSE)

# The band: a black bar across the logo's width, "MULTIPLAYER" in wide
# heavy italics.
band = Image.new("L", (W, H), 0)
top = PAD + lh + gap
ImageDraw.Draw(band).rectangle([PAD - OUTLINE, top, PAD + lw + OUTLINE, top + band_h], fill=255)
text = Image.new("L", (W, H), 0)
font = ImageFont.truetype(FONT, int(band_h * 0.68))
word = "MULTIPLAYER"
track = band_h * 0.24
width = sum(font.getlength(c) for c in word) + track * (len(word) - 1)
x = PAD + (lw - width) / 2
y = top + (band_h - font.getbbox("M")[3]) / 2 - font.getbbox("M")[1] / 2
d = ImageDraw.Draw(text)
for c in word:
    d.text((x, y), c, font=font, fill=255)
    x += font.getlength(c) + track
text = text.transform(text.size, Image.AFFINE, (1, SHEAR, -SHEAR * (top + band_h / 2), 0, 1, 0), Image.BICUBIC)

out = Image.new("RGBA", (W, H), (0, 0, 0, 0))
out.paste((10, 10, 10, 255), mask=black)
out.paste((10, 10, 10, 255), mask=band)
out.paste((255, 255, 255, 255), mask=fill)
out.paste((255, 255, 255, 255), mask=text)
out = out.crop(out.getbbox())
# Halved, for an even, antialiased edge.
out = out.resize((out.width // 2, out.height // 2), Image.LANCZOS)
out.save(sys.argv[1])
back = Image.new("RGBA", out.size, (40, 60, 70, 255))
back.alpha_composite(out)
back.save(sys.argv[1].replace(".png", "-on-dark.png"))
print(out.size)
