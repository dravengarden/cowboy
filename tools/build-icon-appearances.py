"""Export the approved Neon light counterpart and native automatic appearances."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
LIGHT = ROOT / 'assets/brand/cowboy-neon-light-source.png'
DARK = ROOT / 'web/public/app-icons/v5/palette-103/icon-512.png'


def export(source, target, size):
    target.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['magick', str(source), '-resize', f'{size}x{size}', '-strip',
                    '-define', 'png:exclude-chunk=date,time', 'PNG24:' + str(target)], check=True)


for name in ('AppIcon', 'Cowboy-palette-103'):
    directory = ROOT / f'apps/native-shell/apple/Assets.xcassets/{name}.appiconset'
    export(LIGHT, directory / 'icon-light.png', 1024)
    export(DARK, directory / 'icon-dark.png', 1024)
    common = {'idiom': 'universal', 'platform': 'ios', 'size': '1024x1024'}
    images = [dict(common, filename='icon-light.png'),
              dict(common, filename='icon-dark.png', appearances=[{'appearance': 'luminosity', 'value': 'dark'}])]
    (directory / 'Contents.json').write_text(json.dumps({'images': images, 'info': {'author': 'xcode', 'version': 1}}, indent=2) + '\n')
for size in (96, 192, 512):
    export(LIGHT, ROOT / f'web/public/app-icons/v6/palette-103/icon-light-{size}.png', size)
