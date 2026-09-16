import zlib, struct, math

W = H = 512
bg = (22, 24, 28)        # 应用底色 #16181c
ring = (205, 210, 218)   # 浅灰环（端口意象）
inner = (40, 44, 52)     # 端口内孔

def px(x, y):
    cx, cy = W / 2.0, H / 2.0
    d = math.hypot(x - cx, y - cy)
    # 外环
    if abs(d - 165) < 20:
        return ring
    # 内孔
    if d < 70:
        return inner
    # 内孔到外环之间的细描边
    if d < 92:
        return bg
    return bg

raw = bytearray()
for y in range(H):
    raw.append(0)  # PNG filter type 0
    for x in range(W):
        r, g, b = px(x, y)
        raw += bytes((r, g, b))

def chunk(typ, data):
    c = typ + data
    return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xffffffff)

png = b"\x89PNG\r\n\x1a\n"
png += chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))  # 8-bit RGB
png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
png += chunk(b"IEND", b"")

with open("src-tauri/icons/source.png", "wb") as f:
    f.write(png)
print("wrote src-tauri/icons/source.png")
