#!/usr/bin/env python3
"""Export the korekore pixel art (Kore, Characters, agent and nav tiles, the "your vibebuddy" tag)
as exact data and nearest-neighbour PNG renders. This file is the source of truth for the pixels;
see docs/design/korekore/README.md.

Usage: tools/korekore-pixel-art.py [json_out_dir] [png_out_dir]
Defaults: docs/design/korekore/pixel and docs/design/korekore/images.
"""
import json
import os
import struct
import sys
import zlib

INK = "#1C1730"

# ---------- Kore (20 x 18) ----------
KORE_PAL = {".": None, "o": "#2B2140", "c": "#FFF3DD", "s": "#3B2F63", "e": "#FFFFFF", "p": "#FF9EB8",
            "m": "#FFB3C8", "r": "#F0587F", "y": "#FFD23F", "b": "#7CC8FF", "t": "#7CC8FF"}
KORE_FACES = {
    "normal":  ['ssssssssssss', 'ssssssssssss', 'sseesssseess', 'sseesssseess', 'ppsssssssspp', 'sssssmmsssss', 'ssssssssssss'],
    "working": ['ssssssssssss', 'ssssssssssss', 'ssseessssees', 'ssseessssees', 'ppsssssssspp', 'ssssssmmssss', 'ssssssssssss'],
    "input":   ['ssssssssssss', 'sseesssseess', 'sseesssseess', 'sseesssseess', 'ppsssssssspp', 'sssssmmsssss', 'sssssmmsssss'],
    "happy":   ['ssssssssssss', 'sssessssesss', 'ssesessesess', 'ppsssssssspp', 'ssssmmmmssss', 'sssssmmsssss', 'ssssssssssss'],
    "sad":     ['ssssssssssss', 'sseesssseess', 'sstsssssstss', 'ppsssssssspp', 'sssssmmsssss', 'ssssmssmssss', 'ssssssssssss'],
    "sleep":   ['ssssssssssss', 'ssssssssssss', 'ssssssssssss', 'seeesssseees', 'ppsssssssspp', 'sssssmssssss', 'ssssssssssss'],
}


def kore_rows(expr):
    face = KORE_FACES[expr]
    rows = ['....rr..yy..bb......', '..oooooooooooooooo..', '.o' + 'c' * 16 + 'o.', '.oc' + 'o' * 14 + 'co.']
    rows += ['.oco' + f + 'oco.' for f in face]
    rows += ['.oc' + 'o' * 14 + 'co.', 'ooc' + 'c' * 14 + 'coo', 'occ' + 'c' * 14 + 'cco', 'ooc' + 'c' * 14 + 'coo',
             '.o' + 'c' * 16 + 'o.', '..oooooooooooooooo..', '....oo........oo....']
    assert all(len(r) == 20 for r in rows), expr
    return rows


# ---------- Characters (20 x 25) ----------
CHIBI_BASE = [
    '......oooooooo......', '....oohhhhhhhhoo....', '...ohhhhhhhhhhhho...', '..ohhhhhhhhhhhhhho..',
    '..ohhhhhhhhhhhhhho..', '..ohhkkkkkkkkkkhho..', '..ohkkkkkkkkkkkkho..', '..ohkkkkkkkkkkkkho..',
    '..okkkwekkkkwekkko..', '..okkkeekkkkeekkko..', '..okkppkkkkkkppkko..', '...okkkkkmmkkkkko...',
    '....okkkkkkkkkko....', '.....oooooooooo.....', '.....ojjssssjjo.....', '....ojjjssssjjjo....',
    '...ojjjjssssjjjjo...', '...okjjjssssjjjko...', '...okojjssssjjoko...', '....oojjjjjjjjoo....',
    '.....onnnnnnnno.....', '.....onnnoonnno.....', '.....onno..onno.....', '....offfo..offfo....',
    '....ooooo..ooooo....']
CHIBI_PAL = {".": None, "o": INK, "e": INK, "w": "#FFFFFF", "p": "#FF9EB8", "m": "#E2586E"}


def chibi(who):
    g = [list(r) for r in CHIBI_BASE]

    def set_(r, c, ch):
        if 0 <= r < 25 and 0 <= c < 20:
            g[r][c] = ch

    def row(r, s):
        assert len(s) == 20, (who, r, s)
        g[r] = list(s)

    if who == 'mei':
        col = dict(h='#4A3036', k='#FFE0C7', j='#2BB3A3', s='#FFFFFF', n='#2B2B3A', f='#FFFFFF', a='#F0587F')
        for r in range(4, 10):
            set_(r, 17, 'h'); set_(r, 18, 'h'); set_(r, 19, 'o')
        for rc in [(3, 18), (3, 19), (10, 18), (10, 19)]:
            set_(*rc, 'o')
        set_(4, 17, 'a'); set_(5, 17, 'a')
    elif who == 'luna':
        col = dict(h='#7B5BC7', k='#FFE6D5', j='#B9A8FF', s='#EDE7FF', n='#3B2F63', f='#8A85A3', a='#F0587F')
        for r in range(8, 14):
            set_(r, 2, 'o'); set_(r, 3, 'h'); set_(r, 16, 'h'); set_(r, 17, 'o')
        for rc in [(12, 4), (12, 15), (13, 4), (13, 15)]:
            set_(*rc, 'h')
        for rc in [(14, 3), (14, 4), (14, 15), (14, 16)]:
            set_(*rc, 'o')
        for c in range(4, 16):
            set_(1, c, 'a')
        for r in range(6, 10):
            for c in (1, 2, 17, 18):
                set_(r, c, 'a')
    elif who == 'hank':
        col = dict(h='#C9C9D3', k='#F2C9A5', b='#C9C9D3', j='#3E6B48', s='#2B2B3A', n='#4A6FA5', f='#6B4A35')
        row(11, '...obbbbbmmbbbbbo...'); row(12, '....obbbbbbbbbbo....')
        set_(10, 4, 'b'); set_(10, 15, 'b')
    elif who == 'kai':
        col = dict(h='#FFD27A', k='#E8B48A', j='#FF7A7A', s='#FFFFFF', n='#D9C08C', f='#6B4A35', a='#FFFFFF', l='#E8B48A')
        row(0, '....o.oooooooo.o....'); row(1, '...oohhhhhhhhhhoo...')
        for rc in [(15, 5), (16, 14), (18, 6), (17, 13)]:
            set_(*rc, 'a')
        row(22, '.....ollo..ollo.....')
    elif who == 'ada':
        col = dict(h='#B5562E', k='#FFE3D3', j='#D9B98A', s='#2B3A67', n='#2B3A67', f=INK, a='#E0465A')
        row(5, '..ohhhhhhhhhhhhhho..')
        for r in range(8, 12):
            set_(r, 2, 'o'); set_(r, 3, 'h'); set_(r, 16, 'h'); set_(r, 17, 'o')
        set_(12, 3, 'o'); set_(12, 16, 'o')
        for c in range(7, 13):
            set_(14, c, 'a')
        for c in range(8, 12):
            set_(15, c, 'a')
        set_(16, 9, 'a'); set_(17, 9, 'a')
    elif who == 'dave':
        col = dict(h='#6B4A35', k='#F5CDB0', b='#6B4A35', j='#4A7BD0', s='#D9D3EA', n='#3B4F7A', f='#FFFFFF', g=INK)
        for c in list(range(5, 9)) + list(range(11, 15)):
            set_(7, c, 'g')
        for rc in [(8, 5), (9, 5), (8, 8), (9, 8), (8, 11), (9, 11), (8, 14), (9, 14), (8, 9), (8, 10)]:
            set_(*rc, 'g')
        row(11, '...okbbbbmmbbbbko...'); row(12, '....obbbbbbbbbbo....')
    elif who == 'xiaohe':
        col = dict(h='#3A2A33', k='#FFE4CF', j='#FF9EB8', s='#FFC2D4', n='#FF9EB8', f='#F0587F', a='#F0587F', l='#FFE4CF')
        row(0, '..ooo.oooooooo.ooo..'); row(1, '.ohhhohhhhhhhhhohhho'); row(2, '.ooohhhhhhhhhhhhooo.')
        set_(1, 3, 'a'); set_(1, 16, 'a')
        row(20, '....onnnnnnnnnno....'); row(21, '....onnnnnnnnnno....'); row(22, '.....ollo..ollo.....')
        for rc in [(15, 9), (16, 10), (17, 9)]:
            set_(*rc, 'a')
    else:
        raise ValueError(who)
    pal = dict(CHIBI_PAL)
    pal.update(col)
    return [''.join(r) for r in g], pal


# ---------- Icons (12 x 12 tile, 10 x 10 glyph) ----------
ICONS = {
    'cc': ('Claude Code', '#F2A27E', ['..........', '.xxxxxxxx.', '.x......x.', '.x.x....x.', '.x..x...x.', '.x.x.xx.x.', '.x......x.', '.xxxxxxxx.', '..xx......', '..x.......']),
    'cx': ('Codex', '#9DB4FF', ['..........', '.xxxxxxxx.', '.xwwwwwwx.', '.xxxxxxxx.', '.x......x.', '.x.x..x.x.', '.xx....xx.', '.x.x..x.x.', '.x......x.', '.xxxxxxxx.']),
    'ci': ('GitHub Actions', '#D9D3EA', ['....xx....', '.x.xxxx.x.', '..xxxxxx..', '.xxx..xxx.', 'xxx....xxx', 'xxx....xxx', '.xxx..xxx.', '..xxxxxx..', '.x.xxxx.x.', '....xx....']),
    'oc': ('OpenCode', '#8FE3C4', ['..........', '..xx..xx..', '.x......x.', '.x......x.', 'x...xx...x', '.x..xx..x.', '.x......x.', '.x......x.', '..xx..xx..', '..........']),
    'gh': ('GitHub Copilot CLI', '#A8D8FF', ['..........', '....xx....', '....xx....', 'xxxxxxxxxx', '.xxxxxxxx.', '....xx....', '....xx....', '...xxxx...', '..........', '..........']),
    'pi': ('Pi', '#FFC59E', ['..........', '.xxxxxxxx.', '.xxxxxxxx.', '..x...x...', '..x...x...', '..x...x...', '..x...xx..', '.xx....xx.', '..........', '..........']),
    'api': ('Your own events', '#FFE58A', ['.....xx...', '....xx....', '...xx.....', '..xxxxxx..', '.....xx...', '....xx....', '...xx.....', '..xx......', '.x........', '..........']),
    'nav_char': ('Character', '#FFB3C8', ['..xxxxxx..', '.x......x.', 'x..x..x..x', 'x..x..x..x', 'x........x', 'x.x....x.x', 'x..xxxx..x', '.x......x.', '..xxxxxx..', '..........']),
    'nav_agents': ('Agents', '#9DB4FF', ['xxxx..xxxx', 'x..x..x..x', 'x..x..x..x', 'xxxx..xxxx', '..........', '..........', 'xxxx..xxxx', 'x..x..x..x', 'x..x..x..x', 'xxxx..xxxx']),
    'nav_box': ('Device', '#FFF3DD', ['..x.x.x...', '.xxxxxxxx.', '.x......x.', '.x.xxxx.x.', '.x.x..x.x.', '.x.xxxx.x.', '.x......x.', '.xxxxxxxx.', '..x....x..', '..........']),
    'nav_chat': ('Conversation', '#FFE58A', ['..........', '.xxxxxxxx.', 'x........x', 'x.x.x.x..x', 'x........x', '.xxxxxxxx.', '..xx......', '..x.......', '..........', '..........']),
    'nav_gen': ('General', '#D9D3EA', ['.xx.......', 'xxxxxxxxxx', '.xx.......', '..........', '......xx..', 'xxxxxxxxxx', '......xx..', '..........', '...xx.....', 'xxxxxxxxxx']),
    'nav_adv': ('Advanced', '#D9D3EA', ['..........', 'xxxxxxxxxx', 'x........x', 'x.x......x', 'x..x.....x', 'x.x..xxx.x', 'x........x', 'xxxxxxxxxx', '..........', '..........']),
}


def icon_rows(key):
    """12 x 12 tile: 1-cell ink border, corners cut, glyph inside."""
    _, bg, glyph = ICONS[key]
    rows = []
    for y in range(12):
        r = ''
        for x in range(12):
            corner = (x in (0, 11) and y in (0, 11))
            if corner:
                r += '.'
            elif x in (0, 11) or y in (0, 11):
                r += 'o'
            elif (x, y) in ((1, 1), (10, 1), (1, 10), (10, 10)):
                r += 'o'
            else:
                ch = glyph[y - 1][x - 1]
                r += {'x': 'x', 'w': 'w'}.get(ch, 'g')
        rows.append(r)
    return rows, {'.': None, 'o': INK, 'x': INK, 'w': '#FFFFFF', 'g': bg}


# ---------- Pixel tag font (9 rows, baseline under row 6) ----------
TAG_FONT = {
    'a': ['....', '....', '.xx.', '...x', '.xxx', 'x..x', '.xxx', '....', '....'],
    'b': ['x...', 'x...', 'xxx.', 'x..x', 'x..x', 'x..x', 'xxx.', '....', '....'],
    'd': ['...x', '...x', '.xxx', 'x..x', 'x..x', 'x..x', '.xxx', '....', '....'],
    'e': ['....', '....', '.xx.', 'x..x', 'xxxx', 'x...', '.xxx', '....', '....'],
    'i': ['x', '.', 'x', 'x', 'x', 'x', 'x', '.', '.'],
    'o': ['....', '....', '.xx.', 'x..x', 'x..x', 'x..x', '.xx.', '....', '....'],
    'r': ['...', '...', 'x.x', 'xx.', 'x..', 'x..', 'x..', '...', '...'],
    'u': ['....', '....', 'x..x', 'x..x', 'x..x', 'x..x', '.xxx', '....', '....'],
    'v': ['.....', '.....', 'x...x', 'x...x', 'x...x', '.x.x.', '..x..', '.....', '.....'],
    'y': ['....', '....', 'x..x', 'x..x', 'x..x', 'x..x', '.xxx', '...x', '.xx.'],
    ' ': ['..', '..', '..', '..', '..', '..', '..', '..', '..'],
}


def tag_rows(text='your vibebuddy', bold=True):
    rows = [''] * 9
    for k, ch in enumerate(text):
        g = TAG_FONT[ch]
        for r in range(9):
            line = g[r]
            if bold and ch != ' ':
                line = ''.join('x' if (x < len(line) and line[x] == 'x') or (x > 0 and line[x - 1] == 'x') else '.'
                               for x in range(len(line) + 1))
            rows[r] += line + ('.' if k < len(text) - 1 else '')
    return rows


# ---------- PNG writer ----------
def hex_rgba(h):
    if h is None:
        return (0, 0, 0, 0)
    h = h.lstrip('#')
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), 255)


def write_png(path, rows, pal, scale):
    w, h = len(rows[0]) * scale, len(rows) * scale
    raw = bytearray()
    for r in rows:
        line = bytearray()
        for ch in r:
            line += bytes(hex_rgba(pal.get(ch))) * scale
        for _ in range(scale):
            raw += b'\x00' + line
    def chunk(t, d):
        c = struct.pack('>I', len(d)) + t + d
        return c + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
    png += chunk(b'IDAT', zlib.compress(bytes(raw), 9)) + chunk(b'IEND', b'')
    with open(path, 'wb') as f:
        f.write(png)


def main():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'docs', 'design', 'korekore')
    jdir = sys.argv[1] if len(sys.argv) > 1 else os.path.join(root, 'pixel')
    pdir = sys.argv[2] if len(sys.argv) > 2 else os.path.join(root, 'images')
    os.makedirs(jdir, exist_ok=True)
    os.makedirs(pdir, exist_ok=True)
    kore = {'size': [20, 18], 'palette': {k: v for k, v in KORE_PAL.items() if v}, 'transparent': '.',
            'expressions': {e: kore_rows(e) for e in KORE_FACES}}
    for e in KORE_FACES:
        write_png(os.path.join(pdir, f'kore-{e}.png'), kore_rows(e), KORE_PAL, 8)
    chars = {}
    for who in ['mei', 'luna', 'hank', 'kai', 'ada', 'dave', 'xiaohe']:
        rows, pal = chibi(who)
        chars[who] = {'rows': rows, 'palette': {k: v for k, v in pal.items() if v}}
        write_png(os.path.join(pdir, f'character-{who}.png'), rows, pal, 8)
    icons = {}
    for key, (label, bg, glyph) in ICONS.items():
        rows, pal = icon_rows(key)
        icons[key] = {'label': label, 'tile': bg, 'glyph10': glyph, 'tile12': rows}
        write_png(os.path.join(pdir, f'icon-{key.replace("_", "-")}.png'), rows, pal, 8)
    tag = tag_rows()
    write_png(os.path.join(pdir, 'tag-your-vibebuddy.png'), tag, {'.': None, 'x': '#E2466F'}, 4)
    with open(os.path.join(jdir, 'kore.json'), 'w') as f:
        json.dump(kore, f, indent=1)
    with open(os.path.join(jdir, 'characters.json'), 'w') as f:
        json.dump({'size': [20, 25], 'transparent': '.', 'note': 'Style drafts. Final looks: 48x64, 16 colours, 4 frames (docs/characters.md).', 'characters': chars}, f, indent=1)
    with open(os.path.join(jdir, 'icons.json'), 'w') as f:
        json.dump({'tile': [12, 12], 'glyph': [10, 10], 'ink': INK, 'tileBorder': '1 cell ink, corners cut', 'icons': icons}, f, indent=1)
    with open(os.path.join(jdir, 'pixel-tag-font.json'), 'w') as f:
        json.dump({'rows': 9, 'baselineBelowRow': 6, 'letterSpacing': 1, 'bold': 'OR each glyph with itself shifted one dot right (not spaces)',
                   'glyphs': TAG_FONT, 'rendered': {'your vibebuddy': tag}}, f, indent=1)
    print('ok', sorted(os.listdir(pdir)))


if __name__ == '__main__':
    main()
