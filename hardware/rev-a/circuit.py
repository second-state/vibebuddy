"""Rev A 原理图设计源；生产状态见 README 和设计审查记录。"""
from build_schematic import *

SOURCES = {
    'esp': 'https://www.espressif.com/sites/default/files/documentation/esp32-s3-wroom-1_wroom-1u_datasheet_en.pdf',
    'buck': 'https://www.diodes.com/assets/Datasheets/AP63200-AP63201-AP63203-AP63205.pdf',
    'lcd': 'https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/img/19.png',
}
RFP='Resistor_SMD:R_0603_1608Metric'
CFP='Capacitor_SMD:C_0603_1608Metric'
BF='Inductor_SMD:L_0603_1608Metric'
sheets=[]


def sheet(name,title,note):
    s=Sheet(name,title,note)
    s.text(note,20,18)
    sheets.append(s)
    return s


def r(s,n,v,a,b,x,y,mpn=''):
    s.component('Device:R','R'+str(n),v,x,y,{'1':a,'2':b},RFP,mpn)


def cap(s,n,v,a,b,x,y):
    size='Capacitor_SMD:C_0805_2012Metric' if any(k in v for k in ['10u','22u']) else CFP
    s.component('Device:C','C'+str(n),v,x,y,{'1':a,'2':b},size)


def bead(s,n,a,b,x,y):
    s.component('Device:FerriteBead','FB'+str(n),'120R@100MHz',x,y,{'1':a,'2':b},BF,'BLM18PG121SN1D')


def tp(s,n,net,x,y):
    s.component('Connector:TestPoint','TP'+str(n),net,x,y,{'1':net},'TestPoint:TestPoint_Pad_D1.5mm')


# 主控：模组内部存储器占用的 IO35/36/37 不对外使用。
s=sheet('controller','主控与恢复','ESP32-S3-WROOM-1-N16R8；GPIO 分配为本自研板专用。\nBOOT0 测试点拉低后按复位进入恢复下载；无外露下载键。')
n={str(i):None for i in range(1,42)}
n.update({'1':'GND','2':'3V3','3':'MCU_EN','4':'CC1_SENSE','5':'CC2_SENSE','6':'A_MCLK_SRC','7':'A_BCLK_SRC','8':'A_LRCK_SRC','9':'A_DOUT','10':'A_DIN','11':'LCD_BL_N','12':'LCD_RESET_N','13':'USB_DM_MCU','14':'USB_DP_MCU','17':'LCD_DC','18':'LCD_CS_N','19':'LCD_MOSI','20':'LCD_SCLK','23':'KEY1_N','27':'BOOT0','31':'KEY2_N','32':'KEY3_N','33':'PA_EN','36':'UART_RX','37':'UART_TX','38':'I2C_SCL','39':'I2C_SDA','40':'GND','41':'GND'})
s.component('RF_Module:ESP32-S3-WROOM-1','U1','ESP32-S3-WROOM-1-N16R8',95,100,n,mpn='ESP32-S3-WROOM-1-N16R8',source=SOURCES['esp'])
r(s,30,'10k','3V3','MCU_EN',195,55);cap(s,30,'1u/10V','MCU_EN','GND',245,55)
r(s,31,'10k','3V3','BOOT0',195,105)
cap(s,31,'10u/10V','3V3','GND',195,155);cap(s,32,'100n/16V','3V3','GND',245,155)
for i,(net,x,y) in enumerate([('BOOT0',195,215),('MCU_EN',245,215),('GND',295,215),('UART_TX',295,55),('UART_RX',345,55),('3V3',345,215)],1):tp(s,i,net,x,y)
r(s,32,'4.7k','3V3','I2C_SCL',295,105);r(s,33,'4.7k','3V3','I2C_SDA',345,105)
for i,net in enumerate(['MCLK','BCLK','LRCK']):r(s,34+i,'33R','A_'+net+'_SRC','A_'+net,195+i*65,260)

# USB 输入和主电源；电源模式需要固件配合，供电预算及审核见 power-footprint-review.md。
s=sheet('power-usb','USB 与主电源','USB-C 设备端；两路 CC 各自 5.1k 下拉。\n输入浪涌、USB 枚举前功耗和限流策略仍属发布前待验证项。')
usb={str(first(p,'number')[1]):None for p in pins(load('Connector:USB_C_Receptacle_USB2.0_16P'))}
for p in ['A1','A12','B1','B12','SH']:usb[p]='GND'
for p in ['A4','A9','B4','B9']:usb[p]='VBUS'
for p in ['A6','B6']:usb[p]='USB_DP'
for p in ['A7','B7']:usb[p]='USB_DM'
usb.update(A5='CC1',B5='CC2')
s.component('Connector:USB_C_Receptacle_USB2.0_16P','J1','USB4105-GF-A',55,85,usb,'Connector_USB:USB_C_Receptacle_GCT_USB4105-xx-A_16P_TopMnt_Horizontal','USB4105-GF-A')
r(s,10,'5.1k 1%','CC1','GND',135,50);r(s,11,'5.1k 1%','CC2','GND',180,50)
s.component('Power_Protection:USBLC6-2SC6','U2','USBLC6-2SC6',280,70,{'1':'USB_DM','2':'GND','3':'USB_DP','4':'USB_DP_PROT','5':'VBUS','6':'USB_DM_PROT'},mpn='USBLC6-2SC6')
r(s,12,'22R','USB_DM_PROT','USB_DM_MCU',235,145);r(s,13,'22R','USB_DP_PROT','USB_DP_MCU',300,145)
for i in [1,2]:
 x=60+(i-1)*90
 r(s,14+(i-1)*2,'100k 1%',f'CC{i}',f'CC{i}_SENSE',x,195)
 r(s,15+(i-1)*2,'100k 1%',f'CC{i}_SENSE','GND',x,245)
 cap(s,10+i,'100n/16V',f'CC{i}_SENSE','GND',x+42,245)
cap(s,10,'4.7u/10V','VBUS','GND',135,105)
tp(s,7,'VBUS',350,230)
s=sheet('regulators','稳压与电源分配','3V3 开关稳压供主控、屏幕与功放；模拟电路使用独立 LDO。\n2A 是稳压器能力，不代表 USB 主机允许取用 2A。')
custom('TPS22918', [('1','VIN','power_in'),('2','GND','power_in'),('3','ON','input'),('4','CT','passive'),('5','QOD','passive'),('6','VOUT','power_out')], 'Package_TO_SOT_SMD:SOT-23-6')
s.component('VB:TPS22918','U8','TPS22918DBVR',340,65,{'1':'VBUS','2':'GND','3':'VBUS','4':'SOFT_CT','5':None,'6':'VBUS_SW'},mpn='TPS22918DBVR',source='https://www.ti.com/lit/ds/symlink/tps22918.pdf')
cap(s,19,'10n/16V','SOFT_CT','GND',365,130)
s.component('Regulator_Switching:AP63203WU','U3','AP63203WU-7',95,75,{'1':'3V3','2':'VBUS_SW','3':'VBUS_SW','4':'GND','5':'BUCK_SW','6':'BUCK_BST'},mpn='AP63203WU-7',source=SOURCES['buck'])
cap(s,13,'100n/16V','BUCK_BST','BUCK_SW',170,50)
s.component('Device:L','L1','4.7uH/4.6A',235,65,{'1':'BUCK_SW','2':'3V3'},'Inductor_SMD:L_Bourns_SRP5030T',mpn='SRP5030T-4R7M',source='https://www.bourns.com/docs/product-datasheets/srp5030t.pdf')
cap(s,14,'10u/10V','VBUS_SW','GND',40,155)
cap(s,15,'22u/10V','3V3','GND',110,155);cap(s,16,'22u/10V','3V3','GND',175,155)
s.component('Regulator_Linear:AP2112K-3.3','U4','AP2112K-3.3TRG1',285,155,{'1':'VBUS_SW','2':'GND','3':'VBUS_SW','4':None,'5':'A3V3'},mpn='AP2112K-3.3TRG1')
cap(s,17,'1u/10V','VBUS_SW','GND',250,215);cap(s,18,'10u/10V','A3V3','GND',315,215)

s=sheet('buttons','三个业务按键与复位','业务键低电平有效，固件消抖。复位 EN 的 RC 网络位于主控页。\nTL3305AF160QG 上按式工程候选；按帽高度与装配待验证。')
for i,x in enumerate([60,155,250],1):
 s.component('Switch:SW_Push','SW'+str(i),'业务键 '+str(i),x,90,{'1':f'KEY{i}_N','2':'GND'},'Button_Switch_SMD:SW_SPST_TL3305A',mpn='TL3305AF160QG')
 r(s,i,'10k','3V3',f'KEY{i}_N',x,150)
s.component('Switch:SW_Push','SW4','复位',60,225,{'1':'MCU_EN','2':'GND'},'Button_Switch_SMD:SW_SPST_TL3305A',mpn='TL3305AF160QG')

s=sheet('display','BOX3 裸屏参考接口','参考 TFT024B579-B0：30P / 0.5mm / 下接 FPC，4 线 SPI。\n不是 ATK-MD0240 的 29 针裸屏。屏幕采购、排线厚度和机械图仍需确认。\n不使用触摸；背光电流按原厂屏幕规格复核后才能冻结。')
ln={str(i):'GND' for i in range(1,31)}
ln.update({'1':'LCD_LEDA','4':'3V3','5':'3V3','6':'3V3','7':'3V3','9':'LCD_RESET_N','10':'LCD_CS_N','11':'LCD_SCLK','12':'LCD_DC','14':'LCD_MOSI'})
s.component('Connector_Generic:Conn_01x30','J2','TFT024B579-B0 接口（待核实）',75,115,ln,'Connector_FFC-FPC:Hirose_FH12-30S-0.5SH_1x30-1MP_P0.50mm_Horizontal',mpn='FH12-30S-0.5SH(55)',source=SOURCES['lcd'])
s.component('Transistor_BJT:MMBT3906','Q1','MMBT3906',240,90,{'1':'LCD_BASE','2':'LCD_EMITTER','3':'LCD_LEDA'},mpn='MMBT3906')
r(s,40,'10R/0.125W','3V3','LCD_EMITTER',170,60)
r(s,41,'4.7k','LCD_EMITTER','LCD_BASE',170,125)
r(s,42,'1k','LCD_BL_N','LCD_BASE',310,125)
r(s,43,'10k','3V3','LCD_BL_N',310,60)
cap(s,40,'10u/10V','3V3','GND',170,200);cap(s,41,'100n/16V','3V3','GND',240,200)

# 原厂 Rev22 / BOX3 原理图交叉一致引脚；供电和参考脚明确列出。
names7210=['AD0','AD1','CDATA','CCLK','MCLK','VDDP','VDDD','GNDD','SCLK','LRCK','SDOUT1','SDOUT2','INT','DMIC_CLK','MIC1N','MIC1P','REFP12','REFQ12','MIC2P','MIC2N','GNDA','VDDA','VDDM','MICBIAS12','REFQM','MICBIAS34','MIC4N','MIC4P','REFP34','REFQ34','MIC3P','MIC3N','EP']
types7210={6:'power_in',7:'power_in',8:'power_in',21:'power_in',22:'power_in',23:'power_in',33:'power_in',3:'bidirectional',11:'output',12:'output',13:'output',14:'output',24:'output',26:'output'}
custom('ES7210',[(str(i),name,types7210.get(i,'input')) for i,name in enumerate(names7210,1)],'')
names8311=['CCLK','MCLK','PVDD','DVDD','DGND','SCLK','ASDOUT','LRCK','DSDIN','AGND','AVDD','OUTP','OUTN','DACVREF','ADCVREF','VMID','MIC1N','MIC1P','CDATA','CE','EP']
types8311={3:'power_in',4:'power_in',5:'power_in',10:'power_in',11:'power_in',21:'power_in',7:'output',12:'output',13:'output',19:'bidirectional'}
custom('ES8311',[(str(i),name,types8311.get(i,'input')) for i,name in enumerate(names8311,1)],'')
custom('NS4150B',[(str(i),name,kind) for i,(name,kind) in enumerate([('CTRL','input'),('BYPASS','passive'),('INP','input'),('INN','input'),('VoN','output'),('VCC','power_in'),('GND','power_in'),('VoP','output')],1)],'Package_SO:MSOP-8_3x3mm_P0.65mm')
s=sheet('audio','音频采集：ES7210','单模拟麦克风 + 功放前播放参考。原理图引脚按 ES7210 Rev22。\n参考通道初始增益 0dB，避免与麦克风高增益混用；固件需配置双通道时序。')
an=['GND','GND','I2C_SDA','I2C_SCL','A_MCLK','ADC_D3V3','ADC_D3V3','GND','A_BCLK','A_LRCK','A_DIN',None,None,None,'MIC1N','MIC1P','REFP12','REFQ12','MIC2P','MIC2N','GND','ADC_A3V3','ADC_A3V3','MICBIAS12','REFQM','MICBIAS34','MIC4N','MIC4P','REFP34','REFQ34','REF_INP','REF_INN','GND']
s.component('VB:ES7210','U5','ES7210',100,95,{str(i):n for i,n in enumerate(an,1)},'Package_DFN_QFN:VQFN-32-1EP_4x4mm_P0.4mm_EP2.8x2.8mm',mpn='ES7210',source='ES7210 Rev22 / audio-component-evidence.md')
for i,net in enumerate(['REFP12','REFQ12','MIC2P','MIC2N','REFQM','MICBIAS34','MIC4N','MIC4P','REFP34','REFQ34','MICBIAS12']):
 cap(s,50+i,'2.2u/10V' if net=='MICBIAS12' else '1u/10V',net,'GND',200+(i%4)*55,55+(i//4)*65)
cap(s,61,'1u/10V','DAC_OUTP','REF_INP',55,200);cap(s,62,'1u/10V','DAC_OUTN','REF_INN',125,200)
custom('CMC-4015-40P',[('1','MIC+','passive'),('2','CASE/MIC-','passive')])
s.component('VB:CMC-4015-40P','MK1','CMC-4015-40P',95,255,{'1':'MIC_SIGNAL','2':'GND'},'VB:CMC-4015-40P',mpn='CMC-4015-40P',source='https://www.sameskydevices.com/product/resource/cmc-4015-40p.pdf')
r(s,60,'2.2k','MICBIAS12','MIC_SIGNAL',220,255)
cap(s,63,'1u/10V','MIC_SIGNAL','MIC1P',285,255);cap(s,64,'1u/10V','GND','MIC1N',350,255)

s=sheet('audio-power','音频电源与去耦','独立 A3V3 经磁珠分域；保持完整地平面，不将地切成孤岛。\n每个音频电源脚就近 100nF；各域另有 10uF。封装与布局复核后发布。')
domains=[('ADC_D3V3',2),('ADC_A3V3',2),('DAC_D3V3',2),('DAC_A3V3',1)]
ci=70
for i,(net,count) in enumerate(domains):
 x=55+i*90
 bead(s,i+1,'A3V3',net,x,60)
 cap(s,ci,'10u/10V',net,'GND',x,115);ci+=1
 for j in range(count):cap(s,ci,'100n/16V',net,'GND',x,170+j*50);ci+=1

s=sheet('audio-playback','音频播放与功放','ES8311 输出分支：送 ES7210 采集播放参考，另经耦合电容和电阻到功放。\n功放采用稳压 3V3，不直接连接 USB VBUS；PA_EN 下拉，默认关闭。\n扬声器两端都是驱动输出，负端不能接地。')
dn=['I2C_SCL','A_MCLK','DAC_D3V3','DAC_D3V3','GND','A_BCLK',None,'A_LRCK','A_DOUT','GND','DAC_A3V3','DAC_OUTP','DAC_OUTN','DACVREF','ADCVREF','VMID',None,None,'I2C_SDA','GND','GND']
s.component('VB:ES8311','U6','ES8311',85,90,{str(i):n for i,n in enumerate(dn,1)},'Package_DFN_QFN:VQFN-20-1EP_3x3mm_P0.4mm_EP1.7x1.7mm',mpn='ES8311',source='ES8311 Rev5 / audio-component-evidence.md')
for i,net in enumerate(['DACVREF','ADCVREF','VMID']):cap(s,90+i,'2.2u/10V',net,'GND',40+i*60,170)
cap(s,93,'2.2u/10V','DAC_OUTP','PA_ACP',220,55);cap(s,94,'2.2u/10V','DAC_OUTN','PA_ACN',285,55)
r(s,61,'30k','PA_ACP','PA_INP',220,120);r(s,62,'30k','PA_ACN','PA_INN',285,120)
s.component('VB:NS4150B','U7','NS4150B',285,190,{'1':'PA_EN','2':'PA_BYPASS','3':'PA_INP','4':'PA_INN','5':'SPK_N','6':'PA_3V3','7':'GND','8':'SPK_P'},mpn='NS4150B',source='NS4150B March2021 V1.1')
r(s,63,'100k','PA_EN','GND',190,235)
cap(s,95,'1u/10V','PA_BYPASS','GND',250,260)
cap(s,96,'10u/10V','PA_3V3','GND',310,260);cap(s,97,'100n/16V','PA_3V3','GND',370,260)
bead(s,5,'3V3','PA_3V3',370,170)
s.component('Connector_Generic:Conn_01x02','J3','扬声器 4R/2W',375,95,{'1':'SPK_P','2':'SPK_N'},'Connector_JST:JST_PH_B2B-PH-K_1x02_P2.00mm_Vertical','B2B-PH-K-S')
s.text('外接扬声器候选 CES-28266-24-L200；配线束/端子另确认。',20,240)


# 外部 USB 输入及经过电感/磁珠的电源网络：ERC 不会穿越无源器件推导电源。
for i, (page, net) in enumerate([('power-usb','VBUS'),('power-usb','GND'),('regulators','3V3'),('audio-power','ADC_D3V3'),('audio-power','ADC_A3V3'),('audio-power','DAC_D3V3'),('audio-power','DAC_A3V3'),('audio-playback','PA_3V3')]):
    target = next(x for x in sheets if x.name == page)
    target.component('power:PWR_FLAG', f'#FLG{i+1:03}', 'PWR_FLAG', 25+i*35, 280, {'1':net})


def save():
    from build_schematic import CUSTOM, copy, Quoted, dump
    symbols=[]
    for libid, symbol in CUSTOM.items():
        symbol=copy.deepcopy(symbol)
        symbol[1]=Quoted(libid.split(':',1)[1])
        symbols.append(dump(symbol))
    (ROOT/'VB.kicad_sym').write_text('(kicad_symbol_lib (version 20241209) (generator "vibebuddy") '+''.join(symbols)+')\n')
    (ROOT/'sym-lib-table').write_text('(sym_lib_table (version 7) (lib (name "VB") (type "KiCad") (uri "${KIPRJMOD}/VB.kicad_sym") (options "") (descr "Vibe Buddy 原厂引脚符号")))\n')
    for s in sheets:s.save()
    root=f'(kicad_sch (version 20250114) (generator "vibebuddy") (uuid "{ident("root")}") (paper "A3") (title_block (title "Vibe Buddy Rev A — 原理图草稿") (rev "A-工程草稿") (comment 1 "尚未完成生产审核，禁止直接下单")) (lib_symbols)'
    root+=f'(text "所有页面使用同名全局网络连接。已有未布线工程布局；机械配合与电源预算尚待验证，禁止投产。" (at 20 20 0) {fx(1.27,"left top")} (uuid "{ident("root-note")}"))'
    for i,s in enumerate(sheets):
        x=25+(i%3)*125;y=50+(i//3)*65
        root+=f'(sheet (at {x} {y}) (size 105 35) (stroke (width 0.1524) (type default)) (fill (color 0 0 0 0)) (uuid "{ident(s.name)}") (property "Sheetname" {q(s.title)} (at {x} {y-1} 0) {fx(1.27,"left bottom")}) (property "Sheetfile" "{s.name}.kicad_sch" (at {x} {y+36} 0) {fx(1.27,"left top")}) (instances (project "{PROJECT}" (path "/{ident("root")}" (page "{i+2}")))))'
    root+='(sheet_instances (path "/" (page "1"))))'
    (ROOT/(PROJECT+'.kicad_sch')).write_text(root)
    (ROOT/'circuit-manifest.json').write_text(json.dumps([dict(sheet=s.name,**c) for s in sheets for c in s.components],ensure_ascii=False,indent=2)+'\n')


if __name__=='__main__':save()
