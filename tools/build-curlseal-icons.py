"""Export the approved Curlseal system from committed contours and 50 palettes.

Run in the pinned shell with ImageMagick on PATH. No generation, external
session directories or tracing is needed to reproduce production assets.
"""
import colorsys
import html
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
PUBLIC = ROOT / 'web/public'
CONTOURS = json.loads((ROOT / 'assets/brand/cowboy-curlseal-contours.json').read_text())
PALETTES = json.loads((ROOT / 'assets/brand/cowboy-curlseal-palettes.json').read_text())
DEFAULT = 'curlseal-026'


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


def magick(*args):
    subprocess.run(['magick', *map(str, args)], check=True)


def mark(row, frame='0 0 1024 1024', tile=False, tab=False, classes=False):
    gradient = row.get('mode') == 'shared-gradient'
    gradient_id = 'curlseal-pigment'
    fill = f'url(#{gradient_id})' if gradient else row['solid']
    paths = ''.join(f'<path class="{"brand-" if classes else ""}{part}" fill="{fill}" d="{CONTOURS["paths"][part]}"/>' for part in ('crown', 'brim'))
    # Both parts share coordinates in the traced 10240-unit contour space.
    defs = f'<defs><linearGradient id="{gradient_id}" gradientUnits="userSpaceOnUse" x1="1600" y1="0" x2="8680" y2="0"><stop stop-color="{row["gradient"][0]}"/><stop offset="1" stop-color="{row["gradient"][1]}"/></linearGradient></defs>' if gradient else ''
    # Preserve approved pigments. A fine contrasting edge keeps pale/dark
    # variants legible on browser chrome without adding an opaque icon tile.
    style = '<style>path{stroke:#211B34;stroke-width:65;stroke-linejoin:round;paint-order:stroke fill}@media(prefers-color-scheme:dark){path{stroke:#eee7fa}}</style>' if tab else ''
    rect = f'<rect width="1024" height="1024" fill="{row["background"]}"/>' if tile else ''
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{frame}" width="1024" height="1024"' + (' class="brand-icon" focusable="false" aria-hidden="true"' if classes else '') + f'>{style}{rect}<g transform="{CONTOURS["transform"]}">{defs}{paths}</g></svg>\n'


def render(source, target, size, rgba=False):
    target.parent.mkdir(parents=True, exist_ok=True)
    # Supersampling is particularly important for the small hatband counter.
    magick('-background', 'none', '-density', '96', source, '-resize', f'{size}x{size}', '-strip', '-define', 'png:exclude-chunk=date,time', ('PNG32:' if rgba else 'PNG24:') + str(target))


def family(color):
    r, g, b = [int(color[i:i+2], 16)/255 for i in (1, 3, 5)]
    h, s, _ = colorsys.rgb_to_hsv(r, g, b)
    if s < .18: return 'neutral'
    h *= 360
    return 'red' if h < 20 or h >= 345 else 'orange' if h < 48 else 'gold' if h < 75 else 'blue' if h < 255 else 'purple' if h < 290 else 'pink'


rows = [r for r in json.loads((ROOT / 'web/src/appIconCatalog.json').read_text()) if r['collection'] != 'curlseal']
groups = [dict(id='flat', name='Flat', description='01–25 · One solid color across the whole mark', styles=[]),
          dict(id='flow', name='Flow', description='26–50 · One continuous gradient across both pieces', styles=[])]
for p in PALETTES:
    number = int(p['id']); ident = f'curlseal-{number:03}'
    row = dict(id=ident, number=number, title=p['name'], collection='curlseal', crown=p['gradient'][0] if p['mode'] == 'shared-gradient' else p['solid'], brim=p['gradient'][1] if p['mode'] == 'shared-gradient' else p['solid'], solid=p['solid'], gradient=p['gradient'], mode=p['mode'], background=p['background'], family=family(p['solid']), source_sha256=CONTOURS['source_sha256'])
    brightness = sum(int(p['background'][i:i+2], 16)/255*w for i, w in zip((1, 3, 5), (.2126, .7152, .0722)))
    row['tone'] = 'light' if brightness > .72 else 'dark' if brightness < .25 else 'medium'
    rows.append(row)
    accent = p['solid'] if row['tone'] in ('dark', 'light') else p['background']
    groups[0 if number <= 25 else 1]['styles'].append(dict(id=ident, name=p['name'], themeColor=accent))
    directory = PUBLIC / 'app-icons/v10' / ident
    directory.mkdir(parents=True, exist_ok=True)
    source = directory / 'icon.svg'; source.write_text(mark(row, tile=True))
    (directory / 'favicon.svg').write_text(mark(row, frame='110 110 804 804', tab=True))
    for size in (96, 180, 192, 512): render(source, directory / f'icon-{size}.png', size)
    magick(directory/'icon-512.png', '-resize', '440x440', '-background', row['background'], '-gravity', 'center', '-extent', '512x512', '-strip', directory/'maskable-512.png')
    launch = f'/?app-icon={ident}'
    manifest = dict(name='Cowboy', short_name='Cowboy', id='/', scope='/', start_url=launch, display='standalone', display_override=['window-controls-overlay'], orientation='any', background_color=row['background'], theme_color=row['background'], icons=[dict(src=f'/app-icons/v10/{ident}/icon-{size}.png', sizes=f'{size}x{size}', type='image/png', purpose='any') for size in (180,192,512)] + [dict(src=f'/app-icons/v10/{ident}/maskable-512.png', sizes='512x512', type='image/png', purpose='maskable')])
    write(directory/'manifest.webmanifest', manifest)
    title = html.escape(row['title'])
    (directory/'install.html').write_text(f'<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="apple-mobile-web-app-capable" content="yes"><meta name="apple-mobile-web-app-title" content="Cowboy"><meta name="theme-color" content="{row["background"]}"><title>Cowboy · {title}</title><link rel="manifest" href="manifest.webmanifest"><link rel="apple-touch-icon" sizes="180x180" href="icon-180.png"><link rel="icon" href="favicon.svg"><style>body{{max-width:480px;margin:48px auto;padding:24px;background:#211B34;color:#f7f3ff;font:17px/1.6 system-ui}}img{{width:128px;border-radius:28px}}a{{color:#B99AF3}}</style><img src="icon-180.png" alt="Selected Cowboy icon"><h1>Cowboy · {title}</h1><p>In Safari, Share → Add to Home Screen. Existing Home Screen icons do not update automatically.</p><p>在 Safari 中使用“分享 → 添加到主屏幕”。旧主屏幕图标不会自动替换。</p><a href="{launch}">Open Cowboy / 打开 Cowboy</a><script>if(matchMedia("(display-mode: standalone)").matches||navigator.standalone)location.replace({json.dumps(launch)});</script></html>\n')
    asset = ROOT / f'apps/native-shell/apple/Assets.xcassets/Cowboy-{ident}.appiconset'
    render(source, asset/'icon.png', 1024)
    write(asset/'Contents.json', dict(images=[dict(filename='icon.png', idiom='universal', platform='ios', size='1024x1024')], info=dict(author='xcode',version=1)))

write(ROOT/'web/src/appIconCatalog.json', rows)
write(ROOT/'web/src/appIconStyles.json', dict(default=DEFAULT, groups=groups))
directory = PUBLIC/'app-icons/v10'/DEFAULT
source = directory/'icon.svg'
default = next(r for r in rows if r['id'] == DEFAULT)
manifest = json.loads((directory/'manifest.webmanifest').read_text());manifest['start_url'] = '/'
write(PUBLIC/'manifest.webmanifest', manifest)
for size in (180,192,512): render(source,PUBLIC/f'cowboy-app-icon-{size}-v10.png',size)
shutil.copyfile(directory/'maskable-512.png', PUBLIC/'cowboy-app-icon-maskable-512-v10.png')
for file in PUBLIC.glob('*.png'):
    if file.name.startswith('apple-touch-icon') or file.name in ('icon-192.png','icon-512.png','maskable-512.png'):
        size=152 if '152' in file.name else 167 if '167' in file.name else 192 if '192' in file.name else 512 if '512' in file.name else 180
        render(directory/'maskable-512.png' if 'maskable' in file.name else source,file,size)
for target in ('web/public/favicon.svg','web/public/cowboy-favicon-v10.svg','site/assets/cowboy-tab-icon-v10.svg'):
    shutil.copyfile(directory/'favicon.svg', ROOT/target)
for size in (16,32,48): render(directory/'favicon.svg',ROOT/f'site/assets/cowboy-tab-icon-v10-{size}.png',size,True)
magick(*[ROOT/f'site/assets/cowboy-tab-icon-v10-{s}.png' for s in (16,32,48)],PUBLIC/'cowboy-favicon-v10.ico')
for target in ('web/public/favicon.ico','site/assets/cowboy-tab-icon-v10.ico','apps/native-shell/tauri/icons/icon.ico'):
    shutil.copyfile(PUBLIC/'cowboy-favicon-v10.ico',ROOT/target)
render(source,ROOT/'assets/brand/cowboy-logo.png',1024)
render(source,ROOT/'site/assets/cowboy-brand-icon-v10.png',512)
render(source,ROOT/'apps/native-shell/loader/cowboy-icon.png',180)
wordmark = mark(default, frame='140 205 744 620', classes=True)
(ROOT/'site/assets/cowboy-wordmark-v2.svg').write_text(wordmark)
readme = ROOT/'site/assets/cowboy-readme-mark-v10.svg'
readme.write_text(mark(default, frame='110 160 804 704'))
native = ROOT/'apps/native-shell/tauri/icons'
for file in native.glob('*.png'):
    size=struct.unpack('>I',file.read_bytes()[16:20])[0];render(source,file,size,True)
for base in (native/'ios',ROOT/'apps/native-shell/apple/Assets.xcassets/AppIcon.appiconset'):
    for file in base.glob('*.png'):
        size=struct.unpack('>I',file.read_bytes()[16:20])[0];render(source,file,size)
primary = ROOT/'apps/native-shell/apple/Assets.xcassets/AppIcon.appiconset'
render(source,primary/'icon.png',1024)
write(primary/'Contents.json',dict(images=[dict(filename='icon.png',idiom='universal',platform='ios',size='1024x1024')], info=dict(author='xcode',version=1)))
for file in (native/'android').rglob('*.png'):
    size=struct.unpack('>I',file.read_bytes()[16:20])[0]
    render(directory/'maskable-512.png' if 'foreground' in file.name else source,file,size)
(native/'android/values/ic_launcher_background.xml').write_text('<?xml version="1.0" encoding="utf-8"?>\n<resources><color name="ic_launcher_background">#211B34</color></resources>\n')
chunks=[]
with tempfile.TemporaryDirectory() as tmp:
    for kind,size in ((b'icp4',16),(b'icp5',32),(b'icp6',64),(b'ic07',128),(b'ic08',256),(b'ic09',512),(b'ic10',1024)):
        file=Path(tmp)/f'{size}.png';render(source,file,size,True);data=file.read_bytes();chunks.append(kind+struct.pack('>I',len(data)+8)+data)
body=b''.join(chunks);(native/'icon.icns').write_bytes(b'icns'+struct.pack('>I',len(body)+8)+body)
shutil.copyfile(native/'icon.icns',ROOT/'apps/macos-installer/Resources/Cowboy.icns')
print('Exported Lilac Flow default and 50 Curlseal variants; retained legacy catalog and assets.')

# Compact public palette sampler; all artwork comes from the same vector master.
parts = []
for index, number in enumerate((26, 30, 33, 35, 37, 41, 46, 50)):
    row = next(r for r in rows if r['id'] == f'curlseal-{number:03}')
    svg = mark(row, tile=True).replace('curlseal-pigment', f'sampler-{number}')
    svg = svg.replace('width="1024" height="1024"', 'width="104" height="104"', 1)
    parts.append(f'<g transform="translate({index*120},0)"><clipPath id="clip-{number}"><rect width="104" height="104" rx="24"/></clipPath><g clip-path="url(#clip-{number})">{svg}</g></g>')
(ROOT/'site/assets/cowboy-colorways-v10.svg').write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 944 104" role="img" aria-label="Eight of the fifty Cowboy colorways">' + ''.join(parts) + '</svg>\n')
