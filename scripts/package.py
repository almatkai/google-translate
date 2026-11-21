#!/usr/bin/env python3
"""Build the root-layout target ZIP, exact SHA-256 inventory and sidecars. No uploads."""
import argparse, hashlib, json, os, re, shutil, stat, subprocess, sys, tempfile, zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {'aarch64-apple-darwin': False, 'x86_64-pc-windows-msvc': True}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_sdk():
    text = (ROOT / 'Cargo.toml').read_text()
    match = re.search(r'^tezbar-extension-sdk\s*=\s*\{([^}\n]*)\}', text, re.M)
    fields = match.group(1) if match else ''
    revision = re.search(r'\brev\s*=\s*"([0-9a-f]{40})"', fields)
    if not revision or re.search(r'\b(path|branch|tag)\s*=', fields) or not re.search(r'\bgit\s*=\s*"https://github.com/[^" ]+"', fields):
        raise SystemExit('Release blocked: the SDK must be pinned to an immutable git revision.')
    lock = (ROOT / 'Cargo.lock').read_text()
    entry = re.search(r'\[\[package\]\]\nname = "tezbar-extension-sdk"(.*?)(?=\[\[package\]\]|\Z)', lock, re.S)
    if not entry or not re.search(r'source = "git\+[^"\n]+#' + revision.group(1) + r'"', entry.group(1)):
        raise SystemExit('Release blocked: Cargo.lock must contain the same immutable SDK git revision.')


def check_target(binary, target):
    with binary.open('rb') as stream:
        header = stream.read(64)
        if target == 'x86_64-pc-windows-msvc':
            if len(header) != 64 or header[:2] != b'MZ':
                raise ValueError('Windows package requires a PE32+ x64 executable')
            offset = int.from_bytes(header[60:64], 'little')
            if offset < 64 or offset > binary.stat().st_size - 26:
                raise ValueError('Invalid PE header offset')
            stream.seek(offset)
            pe = stream.read(26)
            if pe[:4] != b'PE\0\0' or pe[4:6] != b'\x64\x86' or pe[24:26] != b'\x0b\x02':
                raise ValueError('Windows package requires a PE32+ x64 executable')
        elif header[:8] != b'\xcf\xfa\xed\xfe\x0c\x00\x00\x01':
            raise ValueError('Mac package requires a native ARM64 Mach-O executable')


def build(target, binary, output):
    check_sdk()
    check_target(binary, target)
    subprocess.check_call([sys.executable, str(ROOT / 'scripts/vendor-licenses.py'), '--check'])
    manifest = json.loads((ROOT / 'extension.json').read_text())
    slug = manifest['slug']
    windows = TARGETS[target]
    executable = f'bin/tezbar-{slug}' + ('.exe' if windows else '')
    info = json.loads(subprocess.check_output([str(binary.resolve()), '--protocol-info'], timeout=10))
    if (info['id'], info['version'], info['protocol']) != (manifest['id'], manifest['version'], 1):
        raise ValueError('Binary identity mismatch')
    platform = 'Windows' if windows else 'macOS'
    expected = [c['name'] for c in manifest['commands'] if c['runtime'] == 'rust' and platform in c['platforms']]
    if set(info['commands']) != set(expected) or len(info['commands']) != len(expected):
        raise ValueError('Binary/manifest platform coverage mismatch')
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temp:
        stage = Path(temp)
        (stage / 'bin').mkdir()
        shutil.copy2(binary, stage / executable)
        for name in ['resources', 'licenses']:
            shutil.copytree(ROOT / name, stage / name)
        for name in ['LICENSE', 'NOTICE']:
            shutil.copy2(ROOT / name, stage / name)
        if not (stage / manifest['icon']).is_file():
            raise ValueError('Manifest icon is not packaged')
        files = {}
        for path in sorted(stage.rglob('*')):
            if path.is_symlink():
                raise ValueError('Symlinks cannot be packaged')
            if path.is_file():
                files[path.relative_to(stage).as_posix()] = digest(path)
        manifest['files'] = files
        # Published archive URL/digests are pinned by the trusted host catalog, not here.
        manifest['artifacts'] = {}
        (stage / 'extension.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
        archive = output / f'{slug}-{target}.zip'
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as z:
            for path in sorted(stage.rglob('*')):
                if path.is_file():
                    name = path.relative_to(stage).as_posix()
                    entry = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
                    entry.create_system = 3
                    entry.external_attr = (stat.S_IFREG | (0o755 if name == executable else 0o644)) << 16
                    entry.compress_type = zipfile.ZIP_DEFLATED
                    z.writestr(entry, path.read_bytes())
        with zipfile.ZipFile(archive) as z:
            packaged = json.loads(z.read('extension.json'))
            if set(z.namelist()) != set(files) | {'extension.json'} or len(z.namelist()) != len(files) + 1:
                raise ValueError('Archive inventory mismatch')
            for name, sha in packaged['files'].items():
                if hashlib.sha256(z.read(name)).hexdigest() != sha:
                    raise ValueError('Archive file hash mismatch: ' + name)
        artifact = {'target': target, 'sha256': digest(archive), 'executable': executable, 'executableSha256': files[executable]}
        (output / f'{slug}-{target}.artifact.json').write_text(json.dumps(artifact, indent=2) + '\n')
        (output / f'{archive.name}.sha256').write_text(f"{artifact['sha256']}  {archive.name}\n")
        (output / f'{slug}-{target}.executable.sha256').write_text(f"{artifact['executableSha256']}  {executable}\n")
        (output / f'{slug}-{target}.files.json').write_text(json.dumps(files, indent=2) + '\n')
        print(archive)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', required=True, choices=TARGETS)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    args = parser.parse_args()
    build(args.target, args.binary, args.output)
