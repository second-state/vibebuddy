#!/usr/bin/env python3
"""Build the korekore website: fill index.template.html with inline SVG sprites drawn from the
same pixel data as the apps and the box (tools/korekore-pixel-art.py).

Usage: site/korekore/build.py   (writes site/korekore/index.html)
"""
import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location(
    'korekore_pixel_art', os.path.join(HERE, '..', '..', 'tools', 'korekore-pixel-art.py'))
gen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen)


def svg_from_rows(rows, pal, scale, label=None, gray=False, dot_gap=0.0, radius=0.0):
    h, w = len(rows), len(rows[0])
    parts = []
    for y, r in enumerate(rows):
        if dot_gap:
            for x, ch in enumerate(r):
                col = pal.get(ch)
                if col:
                    s = 1 - dot_gap
                    parts.append(f'<rect x="{x}" y="{y}" width="{s:.2f}" height="{s:.2f}" rx="{radius}" fill="{col}"/>')
            continue
        x = 0
        while x < w:
            col = pal.get(r[x])
            run = 1
            while x + run < w and pal.get(r[x + run]) == col:
                run += 1
            if col:
                parts.append(f'<rect x="{x}" y="{y}" width="{run}" height="1" fill="{col}"/>')
            x += run
    a11y = f'role="img" aria-label="{label}"' if label else 'aria-hidden="true"'
    style = ' style="filter:grayscale(1) brightness(.8)"' if gray else ''
    return (f'<svg class="px" width="{w * scale}" height="{h * scale}" viewBox="0 0 {w} {h}" '
            f'shape-rendering="crispEdges" {a11y}{style}>' + ''.join(parts) + '</svg>')


def kore(expr, scale, gray=False):
    pal = dict(gen.KORE_PAL)
    return svg_from_rows(gen.kore_rows(expr), pal, scale, label=f'Kore, {expr}', gray=gray)


def chibi(who, scale):
    rows, pal = gen.chibi(who)
    return svg_from_rows(rows, pal, scale, label=who.capitalize())


def icon(key, scale):
    rows, pal = gen.icon_rows(key)
    return svg_from_rows(rows, pal, scale, label=gen.ICONS[key][0])


def tag(dot, color, grain):
    rows = gen.tag_rows()
    pal = {'x': color}
    if grain == 'grain':
        return svg_from_rows(rows, pal, dot, label='your vibebuddy', dot_gap=0.25, radius=0.08)
    return svg_from_rows(rows, pal, dot, label='your vibebuddy')


def fill(text):
    def rep(m):
        kind, *args = m.group(1).split(':')
        if kind == 'kore':
            return kore(args[0], int(args[1]), gray=len(args) > 2 and args[2] == 'gray')
        if kind == 'chibi':
            return chibi(args[0], int(args[1]))
        if kind == 'icon':
            return icon(args[0], int(args[1]))
        if kind == 'tag':
            return tag(int(args[0]), args[1], args[2] if len(args) > 2 else '')
        raise ValueError(m.group(0))
    return re.sub(r'\{\{([a-z_]+:[^}]+)\}\}', rep, text)


if __name__ == '__main__':
    src = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, 'index.template.html')
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(HERE, 'index.html')
    html = fill(open(src, encoding='utf-8').read())
    assert '{{' not in html, 'unfilled placeholder'
    # The template is a page body with its <title>, <link> and <style> on top; wrap it into a standalone document.
    html = html.replace('<meta name="claude:viewport-fit-sides" content="cover">\n', '')
    head, body = html.split('</style>', 1)
    html = ('<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
            '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n'
            + head + '</style>\n</head>\n<body>' + body + '</body>\n</html>\n')
    open(out, 'w', encoding='utf-8').write(html)
    print(out, len(html), 'bytes')
