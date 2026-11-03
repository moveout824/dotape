"""Build DOTAPE's DOTRACK-family icon from its 16 x 16 vector pixel grid.

Run with Python + Pillow. Normal Cargo builds use the checked-in assets.
"""

from pathlib import Path
import struct

from PIL import Image


PALETTE = {
    ".": (0, 0, 0, 0),
    "o": (57, 67, 65, 255),
    "s": (29, 35, 37, 255),
    "a": (222, 173, 117, 255),
    "b": (23, 28, 31, 255),
    "c": (224, 218, 197, 255),
    "r": (224, 126, 119, 255),
}

# Shared stepped shell and warm frame; cassette + recording light inside.
PIXELS = (
    "..oooooooooooo..",
    ".osssssssssssso.",
    "ossaaaaaaaaaasso",
    "osabbbbbbbbbbaso",
    "osabbbbbbbbbbaso",
    "osabccccccccbaso",
    "osabcbbccbbcbaso",
    "osabcbbccbbcbaso",
    "osabccccccccbaso",
    "osabccbbbbccbaso",
    "osabbccccccrraso",
    "osabbbbbbbbrraso",
    "osabbbbbbbbbbaso",
    "ossaaaaaaaaaasso",
    ".osssssssssssso.",
    "..oooooooooooo..",
)


def main():
    assert len(PIXELS) == 16 and all(len(row) == 16 for row in PIXELS)
    destination = Path(__file__).resolve().parents[1] / "assets" / "icon"
    destination.mkdir(parents=True, exist_ok=True)
    base = Image.new("RGBA", (16, 16))
    base.putdata([PALETTE[pixel] for row in PIXELS for pixel in row])

    rectangles = []
    for y, row in enumerate(PIXELS):
        x = 0
        while x < 16:
            end = x + 1
            while end < 16 and row[end] == row[x]:
                end += 1
            red, green, blue, alpha = PALETTE[row[x]]
            if alpha:
                color = f"#{red:02x}{green:02x}{blue:02x}"
                rectangles.append(
                    f'  <rect x="{x}" y="{y}" width="{end - x}" height="1" fill="{color}"/>'
                )
            x = end
    svg = '\n'.join([
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" shape-rendering="crispEdges">',
        '  <title>DOTAPE cassette</title>',
        *rectangles,
        '</svg>',
        '',
    ])
    (destination / "dotape.svg").write_text(svg, encoding="utf-8")
    base.resize((256, 256), Image.Resampling.NEAREST).save(destination / "dotape.png")

    # Each Windows size is individually nearest-neighbour rasterized; never blurred.
    sizes = (16, 24, 32, 48, 64, 128, 256)
    payloads = []
    for size in sizes:
        from io import BytesIO

        stream = BytesIO()
        base.resize((size, size), Image.Resampling.NEAREST).save(stream, format="ICO", sizes=[(size, size)])
        ico = stream.getvalue()
        length, offset = struct.unpack_from("<II", ico, 14)
        payloads.append((size, ico[offset:offset + length]))
    offset = 6 + 16 * len(payloads)
    entries = []
    for size, data in payloads:
        entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
    (destination / "dotape.ico").write_bytes(
        struct.pack("<HHH", 0, 1, len(payloads)) + b"".join(entries) + b"".join(data for _, data in payloads)
    )
    print(destination)


if __name__ == "__main__":
    main()
