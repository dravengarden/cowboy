"""Export transparent browser favicons from the approved hat contours.

The paths were traced once from Neon 103's crown/brim, without moving either
part. Keep native/PWA icon artwork separate from these small browser marks.
"""
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
contours = json.loads((ROOT / 'assets/brand/cowboy-tab-contours.json').read_text())
rows = json.loads((ROOT / 'web/src/appIconCatalog.json').read_text())


def readable(color):
    # Transparent light-tab artwork needs stronger ink than the dark app tile.
    channels = [int(color[i:i+2], 16) for i in (1, 3, 5)]
    def lum(rgb):
        c = [v / 255 for v in rgb]
        c = [v / 12.92 if v <= .04045 else ((v + .055) / 1.055) ** 2.4 for v in c]
        return sum(a*b for a,b in zip(c, (.2126,.7152,.0722)))
    for step in range(101):
        values = [round(v*(1-step/100)) for v in channels]
        if 1.05/(lum(values)+.05) >= 3.2:
            return '#' + ''.join(f'{v:02x}' for v in values)


def svg(row):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="{contours['viewBox']}">
<style>.crown{{fill:{readable(row['crown'])}}}.brim{{fill:{readable(row['brim'])}}}@media(prefers-color-scheme:dark){{.crown{{fill:{row['crown']}}}.brim{{fill:{row['brim']}}}}}</style>
<g transform="{contours['transform']}"><path class="crown" d="{contours['paths']['crown']}"/><path class="brim" d="{contours['paths']['brim']}"/></g></svg>
'''


for row in rows:
    if row['collection'] != 'palette':
        continue
    path = ROOT / f"web/public/app-icons/v7/{row['id']}/favicon.svg"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(svg(row))
source = ROOT / 'web/public/app-icons/v7/palette-103/favicon.svg'
for target in ('web/public/cowboy-favicon-v7.svg', 'web/public/favicon.svg', 'site/assets/cowboy-tab-icon-v7.svg'):
    shutil.copyfile(source, ROOT / target)
for size in (16,32,48):
    target = ROOT / f'site/assets/cowboy-tab-icon-v7-{size}.png'
    subprocess.run(['magick','-background','none',str(source),'-resize',f'{size}x{size}','-strip','PNG32:'+str(target)],check=True)
# Explicit frames avoid a browser downsampling a desktop-sized app tile.
ico = ROOT / 'web/public/cowboy-favicon-v7.ico'
subprocess.run(['magick',*[str(ROOT/f'site/assets/cowboy-tab-icon-v7-{s}.png') for s in (16,32,48)],str(ico)],check=True)
for target in ('web/public/favicon.ico','site/assets/cowboy-tab-icon-v7.ico'):
    shutil.copyfile(ico, ROOT / target)
