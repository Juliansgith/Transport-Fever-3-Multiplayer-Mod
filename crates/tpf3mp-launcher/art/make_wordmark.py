# Usage: python make_wordmark.py <wordmark.png> <logo_2x.png>
#
# The launcher's wordmark: Transport Fever 3's logo as Steam's library
# shows it, with a black "MULTIPLAYER" band beneath, as tearded's TF2
# launcher has one. The logo is the game's library logo, a white logo on
# transparency: the Steam client keeps it in
# <Steam>/appcache/librarycache/3493540/<hash>/logo.png, and the same
# address on Steam's image server has it at twice the size as logo_2x.png
# (1280x720), which this takes. The page shows the result at about a
# third of its width, so it stays sharp on any screen.
import sys

from PIL import Image, ImageDraw, ImageFont

FONT = "C:/Windows/Fonts/ariblk.ttf"
SHEAR = 0.21        # the band's italic slant
SUPERSAMPLE = 4     # the band's text is drawn this much larger, then reduced

logo = Image.open(sys.argv[2]).convert("RGBA")
logo = logo.crop(logo.getbbox())
lw, lh = logo.size

# The band, as wide as the logo.
band_h = round(lh * 0.22)
gap = round(lh * 0.07)
bw, bh = lw * SUPERSAMPLE, band_h * SUPERSAMPLE
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
out.alpha_composite(logo, (0, 0))
out.alpha_composite(band, (0, lh + gap))
out.save(sys.argv[1])
back = Image.new("RGBA", out.size, (40, 60, 70, 255))
back.alpha_composite(out)
back.save(sys.argv[1].replace(".png", "-on-dark.png"))
print(out.size)
