# Usage: python make_logo.py <logo.png>. Needs Pillow and numpy.
#
# The TF3MP logo, in the style of tearded's TF2 MP launcher logo: heavy
# italic capitals in white, a thick black outline around the word with a
# black line between letters, and a thin white rim around the whole.
import os, sys
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter

FONT = os.environ.get("LOGO_FONT", "C:/Windows/Fonts/ariblk.ttf")
VARIATION = os.environ.get("LOGO_VARIATION")
W, H = 1900, 1200
SHEAR = 0.21     # italic slant
OUTLINE = 30     # black around the word
GAP = 16         # black between letters
RIM = 9          # white around everything
MARGIN = 300     # room for the slant
CLOSE = 55       # notches narrower than this fill black, as in tearded's

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

def shear(img):
    return img.transform(img.size, Image.AFFINE, (1, SHEAR, -SHEAR * H / 2, 0, 1, 0), Image.BICUBIC)

def glyph(ch, fs, x, y):
    font = ImageFont.truetype(FONT, fs)
    if VARIATION:
        font.set_variation_by_name(VARIATION)
    mask = Image.new("L", (W, H), 0)
    ImageDraw.Draw(mask).text((x, y), ch, font=font, fill=255)
    return shear(mask)

def placed(ch, fs, y, left_of):
    """The glyph moved so it sits GAP px right of `left_of`, row by row."""
    # Drawn with a margin: the slant moves low rows left.
    m = glyph(ch, fs, MARGIN, y)
    if left_of is None:
        return m
    a = np.array(left_of) > 127
    b = np.array(m) > 127
    rows = [r for r in range(H) if a[r].any() and b[r].any()]
    if not rows:
        return m
    need = max(np.nonzero(a[r])[0].max() - np.nonzero(b[r])[0].min() for r in rows) + GAP
    return m.transform(m.size, Image.AFFINE, (1, 0, -need, 0, 1, 0))

def word(text, fs, y, x0):
    masks, prev = [], None
    for ch in text:
        m = placed(ch, fs, y, prev)
        if prev is None:
            m = m.transform(m.size, Image.AFFINE, (1, 0, -(x0 - MARGIN), 0, 1, 0))
        masks.append(m)
        prev = m if prev is None else Image.fromarray(np.maximum(np.array(prev), np.array(m)))
    return masks

masks = word("TF3", 470, 150, 120) + word("MP", 280, 610, 540)
union = Image.new("L", (W, H), 0)
for m in masks:
    union = Image.fromarray(np.maximum(np.array(union), np.array(m)))
black = shrink(grow(grow(union, OUTLINE), CLOSE), CLOSE)
rim = grow(black, RIM)

logo = Image.new("RGBA", (W, H), (0, 0, 0, 0))
logo.paste((255, 255, 255, 255), mask=rim)
logo.paste((10, 10, 10, 255), mask=black)
for m in masks:
    logo.paste((255, 255, 255, 255), mask=m)

logo = logo.crop(logo.getbbox())
side = max(logo.size) + 48
square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
square.paste(logo, ((side - logo.size[0]) // 2, (side - logo.size[1]) // 2))
square = square.resize((1024, 1024), Image.LANCZOS)
square.save(sys.argv[1])
back = Image.new("RGBA", square.size, (40, 60, 70, 255))
back.alpha_composite(square)
back.save(sys.argv[1].replace(".png", "-on-dark.png"))
print("saved", sys.argv[1])
