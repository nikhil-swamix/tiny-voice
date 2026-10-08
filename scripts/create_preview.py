from pathlib import Path
from PIL import Image, ImageDraw, ImageFont, ImageFilter

OUT = Path(__file__).resolve().parents[1] / "screenshots" / "product-preview.png"
S = 2
W, H = 1600, 1000
im = Image.new("RGB", (W*S, H*S), "#0a1020")
d = ImageDraw.Draw(im)
def box(coords, fill, radius=18, outline=None, width=1):
    d.rounded_rectangle(tuple(int(v*S) for v in coords), radius=radius*S, fill=fill, outline=outline, width=width*S)
def txt(x, y, value, size, color="#eef5ff", bold=False):
    path = "C:/Windows/Fonts/seguisb.ttf" if bold else "C:/Windows/Fonts/segoeui.ttf"
    d.text((x*S, y*S), value, font=ImageFont.truetype(path, size*S), fill=color)

# Soft teal and blue light behind the product card.
glow = Image.new("RGBA", im.size, (0, 0, 0, 0))
gd = ImageDraw.Draw(glow)
gd.ellipse((900*S, 40*S, 1580*S, 800*S), fill=(35, 210, 187, 42))
gd.ellipse((80*S, 500*S, 800*S, 1120*S), fill=(70, 108, 255, 24))
glow = glow.filter(ImageFilter.GaussianBlur(115*S))
im = Image.alpha_composite(im.convert("RGBA"), glow)
d = ImageDraw.Draw(im)

# Brand and campaign copy.
box((96, 92, 132, 128), "#58dfc7", 12)
txt(104, 96, "t", 24, "#08202a", True)
txt(148, 94, "TINY VOICE", 15, "#b9c8dc", True)
txt(98, 209, "Speak.", 76, "#f2f6ff", True)
txt(98, 300, "Structure.", 76, "#63e4cf", True)
txt(98, 391, "Paste.", 76, "#f2f6ff", True)
txt(102, 510, "A small voice widget for clear,", 24, "#bdc9da")
txt(102, 545, "ready-to-use task notes.", 24, "#bdc9da")

features = [
    ("01", "Live transcription", "See words arrive as you speak"),
    ("02", "Smart structure", "Separate tasks and stages"),
    ("03", "One-step paste", "Return notes to your workflow"),
]
for i, (number, title, subtitle) in enumerate(features):
    y = 640 + i*84
    box((100, y, 144, y+42), "#15293b", 12, "#254456")
    txt(111, y+9, number, 14, "#67e1ce", True)
    txt(163, y-1, title, 18, "#edf4ff", True)
    txt(163, y+27, subtitle, 14, "#8fa3ba")
box((101, 911, 365, 956), "#112437", 22, "#234456")
txt(121, 923, "CTRL  +  SHIFT  +  SPACE", 13, "#a8bdcf", True)

# Product card, based on the current expanded widget.
box((818, 86, 1505, 912), "#101c2b", 26, "#30485d", 1)
box((843, 111, 1480, 888), "#0c1724", 20, "#263f53", 1)
box((869, 139, 907, 177), "#152b3b", 19, "#58dfc7", 2)
d.ellipse((882*S, 152*S, 894*S, 164*S), fill="#58dfc7")
txt(924, 136, "Tiny Voice", 20, "#eef5ff", True)
txt(924, 165, "Ctrl + Shift + Space", 12, "#98adc2")
box((1384, 142, 1420, 178), "#172b3e", 10)
txt(1394, 145, "-", 24, "#dce8f6", True)
box((1428, 142, 1464, 178), "#172b3e", 10)
txt(1438, 148, "x", 18, "#dce8f6", True)

box((869, 205, 1043, 238), "#11372f", 16, "#1c5a50")
d.ellipse((883*S, 216*S, 891*S, 224*S), fill="#61e2bd")
txt(901, 211, "TRANSCRIPT READY", 11, "#a8f6df", True)

box((869, 257, 1454, 792), "#0a1420", 14, "#2d465a")
txt(897, 280, "Task 1: Prepare the launch update", 18, "#70e4d1", True)
txt(917, 321, "-  Confirm release details", 16, "#e6eef8")
txt(917, 354, "-  Share the announcement with the team", 16, "#e6eef8")
txt(897, 414, "Task 2: Collect feedback", 18, "#70e4d1", True)
txt(917, 455, "-  Ask early users for their notes", 16, "#e6eef8")
txt(943, 488, "-  Group replies by priority", 15, "#b3c5d7")
box((892, 557, 1430, 663), "#101f2d", 12, "#263d50")
txt(915, 574, "- [hint: Review feedback early, prioritize", 14, "#a9bed0")
txt(937, 601, "patterns, and make focused improvements", 14, "#a9bed0")
txt(937, 628, "before sharing the final update.]", 14, "#a9bed0")

box((869, 817, 1454, 862), "#162c3d", 12)
d.ellipse((891*S, 832*S, 913*S, 854*S), fill="#58dfc7")
txt(928, 826, "Speak naturally. Get notes ready to use.", 14, "#bdd0df")

im = im.convert("RGB").resize((W, H), Image.Resampling.LANCZOS)
OUT.parent.mkdir(parents=True, exist_ok=True)
im.save(OUT, optimize=True)
im.crop((818, 86, 1505, 912)).save(OUT.with_name("widget-crop.png"), optimize=True)
print(OUT)
