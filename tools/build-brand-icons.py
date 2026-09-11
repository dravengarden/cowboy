"""Export Cowboy's approved artwork. Run from the root: nix develop -c python3 tools/build-brand-icons.py.

Import existing generated batches once with --import-batch original=/path or
--import-batch palette=/path. No image generation or contour reconstruction.
The committed 512px PNGs and catalog are the reproducible export inputs.
"""
import argparse
import base64
import colorsys
import hashlib
import html
import json
from pathlib import Path
import shutil
import struct
import subprocess

ROOT = Path(__file__).resolve().parent.parent
PUBLIC = ROOT / 'web/public/app-icons/v5'
CATALOG = ROOT / 'web/src/appIconCatalog.json'
DEFAULT = 'palette-054'

def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')

def magick(*args):
    subprocess.run(['magick', *map(str, args)], check=True)

def export(source, destination, size):
    destination.parent.mkdir(parents=True, exist_ok=True)
    magick(source, '-resize', f'{size}x{size}', '-strip', '-define', 'png:exclude-chunk=date,time', 'PNG24:' + str(destination))

def family(color):
    r,g,b = [int(color[i:i+2],16)/255 for i in (1,3,5)]
    h,s,v = colorsys.rgb_to_hsv(r,g,b)
    if s < .18: return 'neutral'
    h *= 360
    if h < 20 or h >= 345: return 'red'
    if h < 48: return 'orange'
    if h < 75: return 'gold'
    if h < 255: return 'blue'
    if h < 290: return 'purple'
    return 'pink'

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--import-batch', action='append', default=[])
parser.add_argument('--defaults-only', action='store_true')
parser.add_argument('--only-imported', action='store_true')
args = parser.parse_args()
rows = json.loads(CATALOG.read_text()) if CATALOG.exists() else []
by_id = {p['id']:p for p in rows}
imported = set()
for batch in args.import_batch:
    collection, directory = batch.split('=',1)
    assert collection in ('original','palette')
    for meta in sorted(Path(directory).glob('*.json')):
        if not meta.stem.isdigit(): continue
        p = json.loads(meta.read_text())
        n = p['number']
        ident = f'{collection}-{n:03}'
        source = Path(directory) / f'cowboy-{n:02}.png'
        row = {k:p[k] for k in ('number','title','crown','brim','background')}
        row.update(id=ident, collection=collection, family=family(p['crown']), source_sha256=hashlib.sha256(source.read_bytes()).hexdigest())
        r,g,b = [int(p['background'][i:i+2],16)/255 for i in (1,3,5)]
        brightness = .2126*r+.7152*g+.0722*b
        row['tone'] = 'light' if brightness > .72 else 'dark' if brightness < .25 else 'medium'
        by_id[ident] = row
        imported.add(ident)
        export(source, PUBLIC / ident / 'icon-512.png', 512)
rows = sorted(by_id.values(), key=lambda p:(p['collection'] != 'palette',p['number']))
assert DEFAULT in by_id and len(rows) >= 100
write(CATALOG, rows)

for p in ([] if args.defaults_only else rows):
    if args.only_imported and p['id'] not in imported: continue
    directory = PUBLIC / p['id']
    source = directory / 'icon-512.png'
    for size in (96,180,192): export(source, directory / f'icon-{size}.png',size)
    magick(source, '-resize','450x450','-background',p['background'],'-gravity','center','-extent','512x512','-strip',directory / 'maskable-512.png')
    manifest = {'name':'Cowboy','short_name':'Cowboy','description':'Your coding agents, anywhere.', 'id':'/', 'scope':'/', 'start_url':f'/?app-icon={p["id"]}', 'display':'standalone','display_override':['window-controls-overlay'],'orientation':'any','background_color':p['background'],'theme_color':p['background'],'icons':[{'src':f'/app-icons/v5/{p["id"]}/icon-{s}.png','sizes':f'{s}x{s}','type':'image/png','purpose':'any'} for s in (180,192,512)]+[{'src':f'/app-icons/v5/{p["id"]}/maskable-512.png','sizes':'512x512','type':'image/png','purpose':'maskable'}]}
    write(directory / 'manifest.webmanifest',manifest)
    title = html.escape(p['title'])
    launch = f'/?app-icon={p["id"]}'
    page = f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="theme-color" content="{p['background']}"><meta name="apple-mobile-web-app-capable" content="yes"><meta name="apple-mobile-web-app-title" content="Cowboy"><title>Cowboy · {title}</title><link rel="manifest" href="manifest.webmanifest"><link rel="apple-touch-icon" sizes="180x180" href="icon-180.png"><link rel="icon" href="icon-192.png"><style>body{{margin:0;padding:48px 24px;background:#232831;color:#f3f4f7;font:17px/1.6 system-ui;max-width:520px;margin-inline:auto}}img{{width:144px;border-radius:28px}}a{{color:#bdd2ed}}h1{{font-size:28px}}p{{color:#c7ccd6}}</style><img src="icon-180.png" alt="Selected Cowboy icon"><h1>Cowboy · {title}</h1><p>On iPhone or iPad, open this page in Safari, then Share → Add to Home Screen. The new icon will open Cowboy. An existing Home Screen icon is not replaced automatically.</p><p>iPhone / iPad：在 Safari 打开本页，使用“分享 → 添加到主屏幕”。旧图标不会自动替换。</p><p>On other browsers, use Install app or Add to Home Screen. An installed app may ask you to review an icon update.</p><a href="{launch}">Open Cowboy / 打开 Cowboy</a><script>if(matchMedia('(display-mode: standalone)').matches||navigator.standalone)location.replace({json.dumps(launch)});</script></html>'''
    (directory / 'install.html').write_text(page)
    # Xcode compiles only bundled, allowlisted icon sets. A new Web catalog alone
    # never claims support from an older installed native binary.
    if p['id'] != DEFAULT:
        asset = ROOT / 'apps/native-shell/apple/Assets.xcassets' / f'Cowboy-{p["id"]}.appiconset'
        export(source,asset/'icon.png',1024)
        write(asset/'Contents.json',{'images':[{'filename':'icon.png','idiom':'universal','platform':'ios','size':'1024x1024'}],'info':{'author':'xcode','version':1}})

source = PUBLIC / DEFAULT / 'icon-512.png'
encoded = base64.b64encode(source.read_bytes()).decode('ascii')
(ROOT/'web/public/favicon.svg').write_text(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><image width="512" height="512" href="data:image/png;base64,{encoded}"/></svg>\n')
export(source,ROOT/'assets/brand/cowboy-logo.png',1024)
for size in (180,192,512):
    export(source,ROOT/f'web/public/cowboy-app-icon-{size}-v5.png',size)
shutil.copyfile(PUBLIC/DEFAULT/'maskable-512.png',ROOT/'web/public/cowboy-app-icon-maskable-512-v5.png')
# Compatibility aliases have real current bytes; fresh HTML uses versioned URLs.
for file in (ROOT/'web/public').glob('*.png'):
    if 'v5' in file.name: continue
    name=file.name
    if name.startswith('cowboy-app-icon-') or name.startswith('apple-touch-icon') or name in ('icon-192.png','icon-512.png','maskable-512.png'):
        size=152 if '152' in name else 167 if '167' in name else 192 if '192' in name else 512 if '512' in name else 180
        export(PUBLIC/DEFAULT/'maskable-512.png' if 'maskable' in name else source,file,size)
favicon=ROOT/'web/public/cowboy-favicon-v5.ico'
magick(source,'-define','icon:auto-resize=48,32,16',favicon)
for file in (ROOT/'web/public').glob('*.ico'): shutil.copyfile(favicon,file) if file != favicon else None
manifest=json.loads((PUBLIC/DEFAULT/'manifest.webmanifest').read_text())
manifest['start_url']='/'
write(ROOT/'web/public/manifest.webmanifest',manifest)

native=ROOT/'apps/native-shell/tauri/icons'
(native/'android/values/ic_launcher_background.xml').write_text(
    '<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
    '  <color name="ic_launcher_background">#232831</color>\n</resources>\n')
for file in native.glob('*.png'):
    size=struct.unpack('>I',file.read_bytes()[16:20])[0]
    export(source,file,size)
shutil.copyfile(favicon,native/'icon.ico')
for base in (native/'ios',ROOT/'apps/native-shell/apple/Assets.xcassets/AppIcon.appiconset'):
    for file in base.glob('*.png'):
        data=file.read_bytes()
        size=struct.unpack('>I',data[16:20])[0]
        export(source,file,size)
for file in (native/'android').rglob('*.png'):
    size=struct.unpack('>I',file.read_bytes()[16:20])[0]
    export(source,file,size)
# Modern ICNS chunks embed PNGs, so this portable export needs no macOS tool.
chunks=[]
for kind,size in ((b'icp4',16),(b'icp5',32),(b'icp6',64),(b'ic07',128),(b'ic08',256),(b'ic09',512),(b'ic10',1024)):
    data=subprocess.check_output(['magick',str(source),'-resize',f'{size}x{size}','-strip','PNG32:-'])
    chunks.append(kind+struct.pack('>I',len(data)+8)+data)
body=b''.join(chunks)
(native/'icon.icns').write_bytes(b'icns'+struct.pack('>I',len(body)+8)+body)
shutil.copyfile(native/'icon.icns',ROOT/'apps/macos-installer/Resources/Cowboy.icns')
for size in (16,32): export(source,ROOT/f'site/assets/cowboy-tab-icon-v5-{size}.png',size)
shutil.copyfile(favicon,ROOT/'site/assets/cowboy-tab-icon-v5.ico')
export(source,ROOT/'site/assets/cowboy-brand-icon-v5.png',256)
export(source,ROOT/'site/assets/cowboy-readme-icon-v5.png',256)
export(source,ROOT/'apps/native-shell/loader/cowboy-icon.png',180)
print(json.dumps({'icons':len(rows),'default':DEFAULT,'catalog':str(CATALOG)}))
