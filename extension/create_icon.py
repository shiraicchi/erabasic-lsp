from PIL import Image, ImageDraw, ImageFont

# Create 128x128 icon
img = Image.new('RGBA', (128, 128), (30, 30, 40, 255))
draw = ImageDraw.Draw(img)

# Draw background rounded rect
draw.rounded_rectangle([10, 10, 118, 118], radius=20, fill=(40, 120, 180, 255))

# Draw "E" text
try:
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 64)
except:
    font = ImageFont.load_default()

text = "E"
bbox = draw.textbbox((0, 0), text, font=font)
text_w = bbox[2] - bbox[0]
text_h = bbox[3] - bbox[1]
x = (128 - text_w) // 2
y = (128 - text_h) // 2 - 4
draw.text((x, y), text, fill=(255, 255, 255, 255), font=font)

# Save
img.save('icon.png')
print("Icon created")
