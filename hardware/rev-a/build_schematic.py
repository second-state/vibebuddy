"""生成 Rev A 电路草稿；未确认的器件不赋予生产封装。"""
from pathlib import Path
import copy
import json
import re
import uuid
from functools import lru_cache

ROOT = Path(__file__).resolve().parent
LIB = Path('/Applications/KiCad/KiCad.app/Contents/SharedSupport/symbols')
PROJECT = 'vibe-buddy-rev-a'


class Quoted(str):
    pass


def parse(text):
    tokens = re.findall(r'"(?:\\.|[^"\\])*"|[()]|[^\s()]+', text)
    stack, root = [], None
    for t in tokens:
        if t == '(':
            node = []
            if stack:
                stack[-1].append(node)
            else:
                root = node
            stack.append(node)
        elif t == ')':
            stack.pop()
        else:
            stack[-1].append(Quoted(json.loads(t)) if t.startswith('"') else t)
    assert not stack
    return root


def dump(n):
    if isinstance(n, list):
        return '(' + ' '.join(dump(x) for x in n) + ')'
    return json.dumps(n, ensure_ascii=False) if isinstance(n, Quoted) else str(n)


def children(n, key):
    return [x for x in n if isinstance(x, list) and x[0] == key]


def first(n, key):
    return children(n, key)[0]


def ident(key):
    return str(uuid.uuid5(uuid.NAMESPACE_URL, 'vibebuddy/rev-a/' + key))


@lru_cache(maxsize=None)
def load(libid):
    lib, name = libid.split(':')
    tree = parse((LIB / (lib + '.kicad_sym')).read_text())
    symbols = {str(x[1]): x for x in children(tree, 'symbol')}

    def resolve(name):
        n = copy.deepcopy(symbols[name])
        ext = children(n, 'extends')
        if ext:
            parent = resolve(str(ext[0][1]))
            existing = {x[0] for x in n if isinstance(x, list)}
            prop_names = {x[1] for x in children(n, 'property')}
            for field in children(parent, 'property'):
                if field[1] not in prop_names:
                    n.append(copy.deepcopy(field))
            for field in parent[2:]:
                if isinstance(field, list) and field[0] not in existing and field[0] != 'symbol':
                    n.append(copy.deepcopy(field))
            for section in children(parent, 'symbol'):
                section[1] = Quoted(str(section[1]).replace(str(parent[1]), name, 1))
                n.append(section)
            n = [x for x in n if not (isinstance(x, list) and x[0] == 'extends')]
        return n
    n = resolve(name)
    n[1] = Quoted(libid)
    return n


def pins(symbol):
    return [p for sub in children(symbol, 'symbol') for p in children(sub, 'pin')]


def fx(size=1.27, justify=''):
    return f'(effects (font (size {size} {size})){" (justify " + justify + ")" if justify else ""})'


def q(s):
    return json.dumps(str(s), ensure_ascii=False)


CUSTOM = {}


def custom(name, pinmap, footprint='', width=25.4):
    """专用符号；pinmap=(编号、名称、电气类型)，按实际原厂编号输入。"""
    half = (len(pinmap) + 1) // 2
    height = (half + 1) * 2.54
    s = f'(symbol "VB:{name}" (pin_names (offset 0.5)) (in_bom yes) (on_board yes)'
    s += f'(property "Reference" "U" (at 0 {height/2+2.54} 0) {fx()})'
    s += f'(property "Value" "{name}" (at 0 {-height/2-2.54} 0) {fx()})'
    s += f'(property "Footprint" {q(footprint)} (at 0 0 0) (effects (font (size 1.27 1.27)) hide))'
    s += f'(symbol "{name}_0_1" (rectangle (start {-width/2} {height/2}) (end {width/2} {-height/2}) (stroke (width 0.254) (type default)) (fill (type background))))'
    s += f'(symbol "{name}_1_1"'
    for i, (number, label, kind) in enumerate(pinmap):
        side = i < half
        x = -width/2-5.08 if side else width/2+5.08
        y = height/2 - 2.54 * ((i if side else i-half)+1)
        angle = 0 if side else 180
        s += f'(pin {kind} line (at {x} {y} {angle}) (length 5.08) (name {q(label)} {fx()}) (number {q(number)} {fx()}))'
    s += '))'
    CUSTOM['VB:'+name] = parse(s)


class Sheet:
    def __init__(self, name, title, note):
        self.name, self.title, self.note = name, title, note
        self.symbols, self.body, self.components = {}, [], []

    def text(self, t, x, y):
        self.body.append(f'(text {q(t)} (at {x} {y} 0) {fx(1.27,"left top")} (uuid "{ident(self.name+str(x)+str(y)+t)}"))')

    def component(self, libid, ref, value, x, y, nets, footprint=None, mpn='', source=''):
        x, y = round(round(x / 1.27) * 1.27, 4), round(round((y * 0.8) / 1.27) * 1.27, 4)
        s = copy.deepcopy(CUSTOM[libid] if libid in CUSTOM else load(libid))
        self.symbols[libid] = s
        props = {str(p[1]): str(p[2]) for p in children(s, 'property')}
        footprint = props.get('Footprint', '') if footprint is None else footprint
        pp = pins(s)
        numbers = {str(first(p, 'number')[1]) for p in pp}
        assert numbers == set(nets), (ref, numbers - set(nets), set(nets)-numbers)
        cid = ident(ref)
        sy = [f'(symbol (lib_id {q(libid)}) (at {x} {y} 0) (unit 1) (in_bom yes) (on_board yes) (dnp no) (uuid "{cid}")']
        # 外围注释位于符号上方，避免压住器件名称或端口标签。
        pin_y = [float(first(p,'at')[2]) for p in pp]
        top = y-max(pin_y, default=0)-17.78
        prop_x = x + 12.7 if len(pp) <= 3 else x
        if len(pp) <= 3: top = y - 2.54
        for key,val,yy in [('Reference',ref,top),('Value',value,top+2.54)]:
            sy.append(f'(property "{key}" {q(val)} (at {prop_x} {yy} 0) {fx()})')
        for key,val in [('Footprint',footprint),('MPN',mpn),('Source',source)]:
            sy.append(f'(property "{key}" {q(val)} (at {x} {y} 0) (effects (font (size 1.27 1.27)) hide))')
        for num in numbers:
            sy.append(f'(pin {q(num)} (uuid "{ident(ref+"/"+num)}"))')
        sy.append(f'(instances (project "{PROJECT}" (path "/{ident("root")}/{ident(self.name)}" (reference "{ref}") (unit 1)))))')
        self.body.append(''.join(sy))
        seen = set()
        for i,p in enumerate(pp):
            num = str(first(p,'number')[1]); net = nets[num]
            at = first(p,'at');px=x+float(at[1]);py=y-float(at[2]);ang=int(at[3])
            point = (round(px,4), round(py,4), net)
            if point in seen: continue
            seen.add(point)
            key=ref+'/'+num+'/'+str(i)
            if net is None:
                self.body.append(f'(no_connect (at {px} {py}) (uuid "{ident(key+"nc")}"))')
                continue
            dx,dy={0:(-7.62,0),180:(7.62,0),90:(0,7.62),270:(0,-7.62)}[ang]
            ex,ey=round(px+dx,4),round(py+dy,4)
            self.body.append(f'(wire (pts (xy {px} {py}) (xy {ex} {ey})) (stroke (width 0) (type default)) (uuid "{ident(key+"wire")}"))')
            # 全局标签使模块页之间真正电气连接，未接端口由 ERC 保留告警。
            rot = {0:180,180:0,90:270,270:90}[ang]
            self.body.append(f'(global_label {q(net)} (shape passive) (at {ex} {ey} {rot}) {fx(1.016,"right" if ang in (0,90) else "left")} (uuid "{ident(key+"net")}") (property "Intersheetrefs" "${{INTERSHEET_REFS}}" (at {ex} {ey} {rot}) (effects (font (size 1.016 1.016)) hide)))')
        self.components.append(dict(ref=ref,value=value,libid=libid,footprint=footprint,mpn=mpn,source=source,nets=nets))

    def save(self):
        s=f'(kicad_sch (version 20250114) (generator "vibebuddy") (uuid "{ident(self.name+"page")}") (paper "A3") (title_block (title {q(self.title)}) (rev "A-工程草稿") (comment 1 "尚未完成生产审核，禁止直接下单"))'
        s+='(lib_symbols '+''.join(dump(x) for x in self.symbols.values())+')'
        s+='\n'.join(self.body)+')\n'
        (ROOT/(self.name+'.kicad_sch')).write_text(s)


if __name__ == '__main__' and '--pins' in __import__('sys').argv:
    for n in __import__('sys').argv[2:]:
        s=load(n)
        print(n,[(str(first(p,'number')[1]),str(first(p,'name')[1])) for p in pins(s)])
