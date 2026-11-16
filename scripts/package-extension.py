"""Build a Chrome Web Store ZIP: extension files only, manifest at the archive root."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import zipfile

ROOT = Path(__file__).resolve().parents[1]
FILES = (
    'manifest.json', 'background.js', 'content.js', 'options.html', 'options.js',
    'icons/icon-16.png', 'icons/icon-32.png', 'icons/icon-48.png', 'icons/icon-128.png',
)


def package(output):
    folder = ROOT / 'browser/extension'
    manifest = json.loads((folder / 'manifest.json').read_text(encoding='utf-8'))
    version = manifest['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError('Expected a stable X.Y.Z version')
    if manifest['manifest_version'] != 3 or len(manifest['description']) > 132:
        raise ValueError('Invalid MV3 metadata')
    if set(manifest['permissions']) != {'activeTab', 'scripting', 'nativeMessaging'}:
        raise ValueError('Review permission changes before submitting to the store')
    if manifest.get('host_permissions') or manifest.get('externally_connectable'):
        raise ValueError('Unexpected persistent host or external messaging access')
    for size in (16, 32, 48, 128):
        name = manifest['icons'][str(size)]
        if name != f'icons/icon-{size}.png':
            raise ValueError('Unexpected icon path')
        data = (folder / name).read_bytes()
        if data[:8] != b'\x89PNG\r\n\x1a\n' or struct.unpack('>II', data[16:24]) != (size, size):
            raise ValueError('Invalid icon dimensions')
    payload = {name: (folder / name).read_bytes() for name in FILES}
    payload['LICENSE'] = (ROOT / 'LICENSE').read_bytes()
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f'termvault-extension-{version}.zip'
    sums = archive.with_suffix('.zip.sha256')
    if archive.exists() or sums.exists():
        raise FileExistsError('Choose a fresh output directory; existing artifacts are never overwritten')
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as zipped:
        for name, data in sorted(payload.items()):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            zipped.writestr(info, data)
    with zipfile.ZipFile(archive) as zipped:
        assert set(zipped.namelist()) == set(payload) and zipped.testzip() is None
    sums.write_text(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n', encoding='ascii')
    print(f'Chrome Web Store upload ZIP: {archive.resolve()}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    package(parser.parse_args().output)
