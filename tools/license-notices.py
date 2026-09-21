"""Collect notices from installed frontend dependencies and host Rust dependencies.
Run after dependency changes; review emitted MISSING entries before distributing.
"""
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]
sections = ["XmlRows — dependency license notices\n\nCopyright in the dependencies remains with their respective authors.\nThese notices include runtime and Rust build dependencies for the build host.\nOther target platforms require regenerating this file on that target.\n\nThe local xmlcore crate is separately designated MIT in its Cargo.toml.\nXmlRows and xmlcore are MIT-licensed; dependencies retain their own licenses.\n"]
missing = []
count = 0

def add(name, version, license_name, folder, explicit=None):
    global count
    count += 1
    paths = {p for p in folder.iterdir() if p.is_file() and p.name.lower().startswith(("license", "licence", "copying", "notice", "copyright", "unlicense"))}
    for sub in (folder / 'licenses', folder / 'LICENSES'):
        if sub.is_dir():
            paths.update(p for p in sub.rglob('*') if p.is_file())
    if explicit and (folder / explicit).is_file():
        paths.add(folder / explicit)
    text = f"\n{'=' * 72}\n{name} {version}\nLicense identifier: {license_name or 'not declared'}\n"
    override = root / 'legal/upstream' / f'{name}-{version}'
    if not paths and override.is_dir():
        folder = override
        paths = {p for p in folder.iterdir() if p.is_file()}
    if not paths:
        missing.append(name)
        text += 'MISSING: no license text found in installed package.\n'
    for p in sorted(paths):
        text += f"\n--- {p.relative_to(folder)} ---\n{p.read_text(errors='replace')}\n"
    sections.append(text)

# Follow only the frontend production dependency closure, excluding test tools.
seen = set()
def visit(name, base=root):
    folder = base / 'node_modules' / name
    if not folder.exists():
        folder = root / 'node_modules' / name
    if folder in seen:
        return
    seen.add(folder)
    package = json.loads((folder / 'package.json').read_text())
    add(package['name'], package['version'], package.get('license'), folder)
    for dep in package.get('dependencies', {}):
        visit(dep, folder)
for dep in json.loads((root / 'package.json').read_text())['dependencies']:
    visit(dep)

host = next(l.split(': ', 1)[1] for l in subprocess.check_output(['rustc','-vV'],text=True).splitlines() if l.startswith('host:'))
metadata = json.loads(subprocess.check_output(['cargo','metadata','--offline','--format-version','1','--filter-platform',host,'--manifest-path',str(root/'src-tauri/Cargo.toml')],text=True))
reachable = set()
nodes = {n['id']:n for n in metadata['resolve']['nodes']}
def walk(package):
    if package in reachable: return
    reachable.add(package)
    for dep in nodes[package]['deps']:
        walk(dep['pkg'])
walk(metadata['resolve']['root'])
for package in sorted(metadata['packages'],key=lambda p:(p['name'],p['version'])):
    if package['id'] not in reachable or package['source'] is None: continue
    sections.append(f"Source for {package['name']} {package['version']} (unmodified): https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download\n")
    add(package['name'],package['version'],package['license'],Path(package['manifest_path']).parent,package.get('license_file'))
add('xmlcore', '0.1.0', 'MIT', root / 'crates/xmlcore')
(root/'legal/THIRD-PARTY-NOTICES.txt').write_text('\n'.join(sections))
print(f'Generated {count} dependency notices.')
if missing:
    print('MISSING license texts: '+', '.join(missing))
    raise SystemExit(1)
