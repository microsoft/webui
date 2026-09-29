# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Dependency-free, bounded PNG pixel check for the real WK fixture."""

import collections
import pathlib
import struct
import sys
import zlib

ORANGE = (255, 154, 0)
GREEN = (18, 238, 53)
BLUE = (18, 52, 171)
BLACK = (0, 0, 0)


def raster(path):
    png = path.read_bytes()
    if len(png) > 12 * 1024 * 1024 or not png.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("PNG missing or exceeds native 12 MiB limit")
    offset = 8
    compressed = []
    width = height = 0
    while offset < len(png):
        length = struct.unpack_from(">I", png, offset)[0]
        kind = png[offset + 4 : offset + 8]
        if length > 12 * 1024 * 1024 or offset + 12 + length > len(png):
            raise ValueError("invalid PNG chunk")
        payload = png[offset + 8 : offset + 8 + length]
        offset += 12 + length
        if kind == b"IHDR":
            width, height, depth, color, _, _, interlace = struct.unpack(
                ">IIBBBBB", payload
            )
            if (depth, color, interlace) != (8, 6, 0):
                raise ValueError("expected native non-interlaced RGBA image")
        elif kind == b"IDAT":
            compressed.append(payload)
        elif kind == b"IEND":
            break
    if width < 1 or height < 1 or width > 1600 or height > 1200:
        raise ValueError(f"invalid raster dimensions {width}x{height}")
    stride = width * 4
    expected = (stride + 1) * height
    inflater = zlib.decompressobj()
    raw = inflater.decompress(b"".join(compressed), expected + 1)
    if len(raw) != expected or not inflater.eof:
        raise ValueError("incomplete or oversized native raster")
    previous = bytearray(stride)
    cursor = 0
    for y in range(height):
        filter_type = raw[cursor]
        row = bytearray(raw[cursor + 1 : cursor + 1 + stride])
        cursor += stride + 1
        if filter_type > 4:
            raise ValueError("invalid scanline filter")
        for x in range(stride):
            left = row[x - 4] if x >= 4 else 0
            above = previous[x]
            upper_left = previous[x - 4] if x >= 4 else 0
            if filter_type == 1:
                predictor = left
            elif filter_type == 2:
                predictor = above
            elif filter_type == 3:
                predictor = (left + above) // 2
            elif filter_type == 4:
                p = left + above - upper_left
                differences = (abs(p - left), abs(p - above), abs(p - upper_left))
                predictor = (left, above, upper_left)[differences.index(min(differences))]
            else:
                predictor = 0
            row[x] = (row[x] + predictor) & 255
        yield width, height, y, row
        previous = row


def check(path, viewport, scroll):
    if viewport not in {"desktop", "narrow"}:
        raise ValueError("invalid fixture viewport")
    expected_css_width = 800 if viewport == "desktop" else 390
    expected_css_height = 600 if viewport == "desktop" else 700
    counts = collections.Counter()
    black = 0
    dimensions = None
    for width, height, y, row in raster(path):
        dimensions = (width, height)
        scale_x = width / expected_css_width
        scale_y = height / expected_css_height
        if abs(scale_x - scale_y) > 0.02:
            raise ValueError("WK returned a clipped or distorted viewport")
        top = int((120 - (72 if scroll else 0)) * scale_y)
        bottom = int((240 - (72 if scroll else 0)) * scale_y)
        right = int(160 * scale_x)
        for x in range(width):
            color = tuple(row[x * 4 : x * 4 + 3])
            if color == BLACK:
                black += 1
            if 0 <= x < right and top <= y < bottom:
                counts[color] += 1
    if black != 0:
        raise ValueError(f"offscreen black content leaked into viewport: {black} pixels")
    if not scroll and (counts[ORANGE] == 0 or counts[GREEN] == 0):
        raise ValueError(f"allowed preview pixels missing: {counts.most_common(4)}")
    if not scroll and counts[BLUE] != 0:
        raise ValueError(f"parent blue pixels replaced loaded preview: {counts[BLUE]}")
    print(
        f"PIXELS_OK file={path.name} dimensions={dimensions} "
        f"orange={counts[ORANGE]} green={counts[GREEN]} "
        f"parent_blue={counts[BLUE]} other="
        f"{sum(counts.values()) - counts[ORANGE] - counts[GREEN] - counts[BLUE]} "
        f"offscreen_black={black}"
    )


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit("usage: native-capture-pixels.py PNG desktop|narrow 0|1")
    check(pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3] == "1")
