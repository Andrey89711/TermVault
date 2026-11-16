"""Generate the project's simple lock mark using only Python's standard library."""
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]


def rounded(x, y, left, top, right, bottom, radius):
    near_x = min(max(x, left + radius), right - radius)
    near_y = min(max(y, top + radius), bottom - radius)
    return (x - near_x) ** 2 + (y - near_y) ** 2 <= radius ** 2


def color(x, y):
    blue = (37, 80, 145, 255)
    if not rounded(x, y, .04, .04, .96, .96, .16):
        return (0, 0, 0, 0)
    distance = ((x - .5) ** 2 + (y - .43) ** 2) ** .5
    shackle = (y <= .43 and .12 <= distance <= .19) or (
        .43 <= y <= .53 and (.31 <= x <= .38 or .62 <= x <= .69)
    )
    body = rounded(x, y, .25, .48, .75, .83, .05)
    hole = (x - .5) ** 2 + (y - .62) ** 2 <= .045 ** 2 or (.48 <= x <= .52 and .62 <= y <= .73)
    return (245, 248, 255, 255) if (shackle or body) and not hole else blue


def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))


def icon(size):
    data = bytearray()
    for y in range(size):
        data.append(0)  # PNG filter: none
        for x in range(size):
            samples = [color((x + (sx + .5) / 4) / size, (y + (sy + .5) / 4) / size)
                       for sy in range(4) for sx in range(4)]
            data.extend(round(sum(pixel[channel] for pixel in samples) / 16) for channel in range(4))
    header = struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header) + chunk(b'IDAT', zlib.compress(data, 9)) + chunk(b'IEND', b'')


if __name__ == '__main__':
    folder = ROOT / 'browser/extension/icons'
    folder.mkdir(exist_ok=True)
    for size in (16, 32, 48, 128):
        path = folder / f'icon-{size}.png'
        path.write_bytes(icon(size))
        print(path)
