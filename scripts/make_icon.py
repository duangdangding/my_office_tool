from pathlib import Path
from PIL import Image, ImageDraw

root = Path(__file__).resolve().parents[1]
out = root / "src-tauri" / "icons"
out.mkdir(parents=True, exist_ok=True)
size = 256
image = Image.new("RGBA", (size, size), (22, 36, 31, 255))
draw = ImageDraw.Draw(image)
draw.rounded_rectangle((10, 10, 246, 246), radius=55, fill=(22, 36, 31, 255), outline=(57, 87, 73, 255), width=3)
colors = [(131, 211, 173, 255), (60, 155, 118, 255), (60, 155, 118, 255), (131, 211, 173, 255)]
for index, (x, y) in enumerate(((53, 53), (139, 53), (53, 139), (139, 139))):
    draw.rounded_rectangle((x, y, x + 64, y + 64), radius=14, fill=colors[index])
image.save(out / "icon.png")
image.save(out / "icon.ico", sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
mac_image = image.resize((1024, 1024), Image.Resampling.LANCZOS)
mac_image.save(out / "icon.icns", sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
print(out / "icon.ico")
print(out / "icon.icns")
