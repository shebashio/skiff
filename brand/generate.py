#!/usr/bin/env python3
"""Build Skiff vector marks and platform icons; requires rsvg-convert.

The generated master is retained, so builds do not need an AI service.
"""
import base64
from pathlib import Path
import struct
import subprocess
import sys

OUT = Path(sys.argv[1] if len(sys.argv) > 1 else 'brand')
INK, TEAL, WHITE = '#071525', '#24E6C8', '#ECFFFC'


def mark(mono=None):
    colors = [mono] * 4 if mono else ['#087B98', TEAL, '#08BAA6', WHITE]
    return ''.join(f'<path fill="{color}" d="{path}"/>' for color, path in zip(colors, [
        'M6 46 61 17 47 43 5 50Z M26 47 35 45 29 53 45 52 11 57 25 53Z',
        'M19 40 61 17 49 29Z M8 34 25 25 39 26 18 40Z',
        'M8 34 18 36 18 40 8 44Z M18 36 39 26 18 40Z',
        'M19 40 61 17 49 28 60 18Z',
    ]))


def svg(body, width=64, height=64, title='Skiff'):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" role="img"><title>{title}</title>{body}</svg>\n'


def write(name, body):
    path = OUT / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)


for theme, ink, mono in [('light', INK, None), ('dark', WHITE, None), ('mono', INK, INK), ('mono-white', WHITE, WHITE)]:
    shape = mark(mono)
    write(f'logo/skiff-mark-{theme}.svg', svg(shape))
    word = f'<text x="76" y="48" fill="{ink}" font-family="Inter,Arial,sans-serif" font-size="49" font-weight="800" letter-spacing="-2">skiff</text>'
    write(f'logo/skiff-horizontal-{theme}.svg', svg(shape + word, 190, 68))
    word = f'<text x="64" y="100" text-anchor="middle" fill="{ink}" font-family="Inter,Arial,sans-serif" font-size="36" font-weight="800" letter-spacing="-1.5">skiff</text>'
    write(f'logo/skiff-stacked-{theme}.svg', svg('<g transform="translate(32 0)">' + shape + '</g>' + word, 128, 112))

tile = f'<rect x="3" y="3" width="58" height="58" rx="13" fill="{INK}"/>'
write('logo/skiff-favicon.svg', svg(tile + mark()))
write('icon/skiff-app-icon.svg', svg(tile + mark(), title='Skiff app icon'))
write('icon/skiff-app-icon-windows.svg', svg(tile + mark(), title='Skiff app icon'))
write('icon/skiff-menubar-template.svg', svg(mark('#000000')))
master = OUT / 'icon/skiff-master.png'
encoded = base64.b64encode(master.read_bytes()).decode()
raster = f'<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1024" height="1024"><image width="1024" height="1024" xlink:href="data:image/png;base64,{encoded}"/></svg>'


def render(source, size, output):
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['rsvg-convert', '-w', str(size), '-h', str(size), '-o', str(output)], input=source.encode(), check=True)


for size in [16, 32, 64, 128, 256, 512, 1024]:
    render(raster, size, OUT / f'icon/png/skiff-app-icon-{size}.png')
render(svg(mark('#000000')), 44, OUT / 'icon/png/skiff-menubar-template@2x.png')
sizes = [16, 20, 24, 32, 40, 48, 64, 256]
pngs = []
for size in sizes:
    path = OUT / f'icon/windows/skiff-{size}.png'
    render(raster, size, path)
    pngs.append(path.read_bytes())
offset = 6 + 16 * len(sizes)
entries = []
for size, png in zip(sizes, pngs):
    entries.append(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(png), offset))
    offset += len(png)
(OUT / 'icon/windows/skiff.ico').write_bytes(struct.pack('<HHH', 0, 1, len(sizes)) + b''.join(entries) + b''.join(pngs))
print('Generated Skiff vector marks, PNG sizes, menu-bar template and Windows ICO.')
