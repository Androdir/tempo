"""Generate a simple 1024x1024 source icon (app-icon.png) with no third-party
deps. Run `npx tauri icon app-icon.png` afterwards to produce the full set."""

import struct
import zlib

W = H = 1024
TOP = (79, 70, 229)    # indigo  #4f46e5
BOT = (124, 58, 237)   # violet  #7c3aed
WHITE = bytes((255, 255, 255))


def lerp(a, b, t):
    return int(a + (b - a) * t)


# Three ascending "bar chart" bars, centered.
bw, gap = 132, 64
total = 3 * bw + 2 * gap
x0 = (W - total) // 2
base = 772
heights = [196, 320, 452]
bars = []
for i, h in enumerate(heights):
    bx0 = x0 + i * (bw + gap)
    bars.append((bx0, bx0 + bw, base - h, base))

raw = bytearray()
for y in range(H):
    t = y / (H - 1)
    grad = bytes((lerp(TOP[0], BOT[0], t), lerp(TOP[1], BOT[1], t), lerp(TOP[2], BOT[2], t)))
    row = bytearray(grad * W)
    for (bx0, bx1, by0, by1) in bars:
        if by0 <= y < by1:
            row[bx0 * 3:bx1 * 3] = WHITE * (bx1 - bx0)
    raw.append(0)  # PNG filter type 0 for this scanline
    raw += row


def chunk(tag, data):
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))  # 8-bit RGB
    + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    + chunk(b"IEND", b"")
)

with open("app-icon.png", "wb") as f:
    f.write(png)

print(f"wrote app-icon.png ({len(png)} bytes)")
