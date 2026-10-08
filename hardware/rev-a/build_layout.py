"""KiCad pcbnew 生成工程布局；未布线，不输出生产包。"""
from pathlib import Path
import json, math, xml.etree.ElementTree as ET
import wx
app=wx.App(False)
import pcbnew as p
ROOT=Path(__file__).resolve().parent
LIB=Path('/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints')
board=p.BOARD(); board.SetCopperLayerCount(4)
mm=p.FromMM
vec=lambda x,y:p.VECTOR2I(mm(x),mm(y))
xml=ET.parse(ROOT/'layout-input.xml').getroot()
manifest={c['ref']:c for c in json.loads((ROOT/'circuit-manifest.json').read_text())}
nets={}; node_net={}
for n in xml.findall('nets/net'):
 name=n.attrib['name']; net=p.NETINFO_ITEM(board,name);board.Add(net);nets[name]=net
 for node in n.findall('node'):node_net[(node.attrib['ref'],node.attrib['pin'])]=net
fps={}; bounds={}
for c in xml.findall('components/comp'):
 ref=c.attrib['ref']; libid=c.findtext('footprint'); lib,name=libid.split(':',1)
 fp=p.FootprintLoad(str(ROOT/'VB.pretty' if lib=='VB' else LIB/(lib+'.pretty')),name)
 assert fp,libid
 fp.SetReference(ref);fp.SetValue(c.findtext('value'));fp.SetFPID(p.LIB_ID(lib,name))
 fp.Reference().SetTextSize(vec(.8,.8));fp.Reference().SetTextThickness(mm(.12));fp.Value().SetVisible(False)
 if ref.startswith(('R','C','FB','TP')) or ref in ('U5','U6'): fp.Reference().SetLayer(p.F_Fab)
 stamp=c.findtext('tstamps').split()[0]; path=c.find('sheetpath').attrib['tstamps']+stamp
 try:fp.SetPath(p.KIID_PATH(path))
 except TypeError:pass
 pads={pad.GetNumber() for pad in fp.Pads() if pad.GetNumber()}
 expected=set(manifest[ref]['nets'])
 assert expected <= pads,(ref,expected-pads)
 for pad in fp.Pads():
  net=node_net.get((ref,pad.GetNumber()))
  if net:pad.SetNet(net)
 board.Add(fp);fps[ref]=fp
 # 仅用实体庭院和焊盘计算占用，不把丝印文字作为机械轮廓。
 items=[i for i in fp.GraphicalItems() if i.GetLayer()==p.F_CrtYd] or list(fp.Pads())
 boxes=[i.GetBoundingBox() for i in items]
 bounds[ref]=(min(b.GetLeft() for b in boxes)/1e6,min(b.GetTop() for b in boxes)/1e6,max(b.GetRight() for b in boxes)/1e6,max(b.GetBottom() for b in boxes)/1e6)
fixed={'U1':(72,60),'J1':(76,126),'J2':(110,66),'J3':(132,125),'SW1':(105,55),'SW2':(116,55),'SW3':(127,55),'SW4':(57,91),'MK1':(132,74),'U2':(76,116),'U3':(61,112),'U4':(95,89),'U5':(117,82),'U6':(117,104),'U7':(131,113),'U8':(64,122),'L1':(64,105)}
occupied=[]; positions={}
def box(ref,x,y):
 a,b,c,d=bounds[ref];return (x+a-.25,y+b-.25,x+c+.25,y+d+.25)
def overlap(a,b):return a[0]<b[2] and b[0]<a[2] and a[1]<b[3] and b[1]<a[3]
def put(ref,x,y):
 fp=fps[ref];fp.SetPosition(vec(x,y));positions[ref]=(x,y);occupied.append((ref,box(ref,x,y)))
for ref,pos in fixed.items():put(ref,*pos)
# 模组天线铜/元件禁布区由官方模组封装携带；额外限制自动摆放。
occupied.append(('antenna',(48,30,96,53.25)))
# 固定孔与周围螺帽预留。
for x,y in [(54,80),(136,55),(54,126),(136,92)]:occupied.append(('mount',(x-3,y-3,x+3,y+3)))
anchors={'controller':(83,80),'power-usb':(78,111),'regulators':(64,111),'buttons':(113,61),'display':(108,72),'audio':(118,83),'audio-power':(104,91),'audio-playback':(119,106)}
# 优先放电源输入、反馈和本地去耦；余件按电路分区就近放置。
near={'C13':(61,110),'C14':(58,112),'C15':(64,100),'C16':(68,104),'C17':(92,88),'C18':(98,89),'C19':(66,122),'C10':(69,123),'C31':(64,72),'C32':(64,70),'C96':(131,109),'C97':(134,111),'FB5':(128,109),'R12':(83,113),'R13':(83,115)}
refs=[r for r in fps if r not in fixed]
refs.sort(key=lambda r:(r not in near, -((bounds[r][2]-bounds[r][0])*(bounds[r][3]-bounds[r][1]))))
for ref in refs:
 ax,ay=near.get(ref,anchors[manifest[ref]['sheet']])
 candidates=[(x*.5,y*.5) for x in range(106,276) for y in range(112,254)]
 candidates.sort(key=lambda q:(q[0]-ax)**2+(q[1]-ay)**2)
 for x,y in candidates:
  b=box(ref,x,y)
  if b[0]<52 or b[2]>138 or b[1]<52 or b[3]>128:continue
  if any(overlap(b,o) for _,o in occupied):continue
  put(ref,x,y);break
 else:raise RuntimeError('无法放置 '+ref)
# 工程样机 90 x 80 mm，屏幕装配尺寸未冻结。
for start,end in [((50,50),(140,50)),((140,50),(140,130)),((140,130),(50,130)),((50,130),(50,50))]:
 shape=p.PCB_SHAPE();shape.SetShape(p.SHAPE_T_SEGMENT);shape.SetStart(vec(*start));shape.SetEnd(vec(*end));shape.SetLayer(p.Edge_Cuts);shape.SetWidth(mm(.05));board.Add(shape)
for i,(x,y) in enumerate([(54,80),(136,55),(54,126),(136,92)],1):
 fp=p.FootprintLoad(str(LIB/'MountingHole.pretty'),'MountingHole_2.2mm_M2');fp.SetReference('H'+str(i));fp.SetValue('M2');fp.SetPosition(vec(x,y));fp.Value().SetVisible(False);board.Add(fp)
for text,x,y,size in [('VIBE BUDDY REV A / LAYOUT ONLY',96,133,1),('NOT FOR FABRICATION - UNROUTED',96,136,1),('SCREEN MECHANICAL FIT UNVERIFIED',110,70,.65)]:
 t=p.PCB_TEXT(board);t.SetText(text);t.SetPosition(vec(x,y));t.SetTextSize(vec(size,size));t.SetTextThickness(mm(.12));t.SetLayer(p.Dwgs_User);board.Add(t)
board.GetDesignSettings().m_MinThroughDrill=mm(0.2)
board.BuildConnectivity()
p.SaveBoard(str(ROOT/'vibe-buddy-rev-a.kicad_pcb'),board)
(ROOT/'layout-placement.json').write_text(json.dumps(positions,indent=2)+'\n')
# 封装针号检查来自 KiCad 导出网表，与 PCB 焊盘比较；空白安装焊盘不强行接信号。
report={'components':len(fps),'mounting_holes':4,'nets':len(nets),'tracks':len(board.GetTracks()),'board_mm':[90,80],'pin_number_check':'通过','fixed_courtyard_overlaps':[(a,b) for i,(a,ra) in enumerate(occupied) for b,rb in occupied[i+1:] if a in fixed and b in fixed and overlap(ra,rb)]}
(ROOT/'layout-check.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n');print(report)
