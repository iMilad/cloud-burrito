"""Static unsigned platform policy; never invokes the CLI, build, or network."""
import argparse
import json
from pathlib import Path
import re

from packaging_contract import ROOT, load_matrix

CLI_VERSION = '2.11.4'
SCHEMA_SHA256 = '923a41898c978c93616b9c1b6ae4d346f5586987459cfa877bacea21e7c85f1a'


def schema_accepts(value, schema, definitions):
    """Validate the bounded projected constraints used by these two overlays."""
    if '$ref' in schema:
        ref = schema['$ref']
        if not ref.startswith('#/definitions/'):
            raise ValueError('External schema references are forbidden')
        return schema_accepts(value, definitions[ref.rsplit('/', 1)[1]], definitions)
    for key in ('allOf', 'anyOf', 'oneOf'):
        if key in schema:
            matches = [schema_accepts(value, item, definitions) for item in schema[key]]
            if key == 'allOf' and not all(matches) or key == 'anyOf' and not any(matches) or key == 'oneOf' and sum(matches) != 1:
                return False
    if 'enum' in schema and value not in schema['enum']:
        return False
    if 'const' in schema and (type(value) is not type(schema['const']) or value != schema['const']):
        return False
    kinds = schema.get('type', [])
    kinds = [kinds] if isinstance(kinds, str) else kinds
    types = {'null': value is None, 'boolean': type(value) is bool, 'string': isinstance(value, str),
             'array': isinstance(value, list), 'object': isinstance(value, dict),
             'integer': type(value) is int, 'number': type(value) in (int, float)}
    if kinds and not any(types.get(kind, False) for kind in kinds):
        return False
    if isinstance(value, dict):
        props = schema.get('properties', {})
        if not all(key in value for key in schema.get('required', [])):
            return False
        if schema.get('additionalProperties') is False and set(value) - set(props):
            return False
        return all(schema_accepts(item, props[key], definitions) for key, item in value.items() if key in props)
    if isinstance(value, list) and 'items' in schema:
        return all(schema_accepts(item, schema['items'], definitions) for item in value)
    return True


def validate_config(config, row, root=ROOT):
    schema = json.loads((Path(root) / 'packaging/tauri-platform-schema.json').read_text(encoding='utf-8'))
    if schema['provenance']['tauri_cli'] != CLI_VERSION or schema['provenance']['source_sha256'] != SCHEMA_SHA256:
        raise ValueError('Platform schema provenance differs from the reviewed pin')
    pins = (Path(root) / 'scripts/tool-versions.env').read_text(encoding='utf-8')
    if not re.search(r'^TAURI_CLI_VERSION=2\.11\.4$', pins, re.M):
        raise ValueError('Review platform policy before changing the Tauri CLI pin')
    if not schema_accepts(config, schema, schema['definitions']):
        raise ValueError('Platform overlay uses an unsupported key or value shape')
    bundle = config.get('bundle', {})
    platform = row['platform']
    if set(bundle) != {'targets', 'createUpdaterArtifacts', 'useLocalToolsDir', platform}:
        raise ValueError('Platform overlay must contain only the declared bundle policy')
    if bundle['targets'] != row['bundle_targets']:
        raise ValueError('Bundle targets differ from the sole artifact matrix')
    if bundle['createUpdaterArtifacts'] is not False or bundle['useLocalToolsDir'] is not True:
        raise ValueError('Updater artifacts are disabled and the explicit local helper cache is required')
    if platform == 'windows':
        expected = {'allowDowngrades': False, 'certificateThumbprint': None, 'signCommand': None, 'timestampUrl': None,
                    'webviewInstallMode': {'type': 'downloadBootstrapper', 'silent': True},
                    'nsis': {'installMode': 'currentUser', 'installerIcon': 'icons/icon.ico',
                             'languages': ['English'], 'displayLanguageSelector': False}}
        if bundle[platform] != expected:
            raise ValueError('Windows policy requires unsigned per-user NSIS with the explicit online bootstrapper')
    elif platform == 'linux':
        expected = {'deb': {'section': 'utils', 'priority': 'optional'}, 'appimage': {'bundleMediaFramework': False}}
        if bundle[platform] != expected:
            raise ValueError('Linux policy requires generated runtime dependencies and no extra hooks or media payload')
    else:
        raise ValueError('This policy checker covers Windows and Linux only')
    return row


def check_platform(platform, root=ROOT):
    matrix = load_matrix(Path(root) / 'packaging/targets.json')
    rows = [row for row in matrix['targets'] if row['platform'] == platform]
    if len(rows) != 1:
        raise ValueError('Platform must resolve to exactly one matrix row')
    row = rows[0]
    config = json.loads((Path(root) / row['bundle_config']).read_text(encoding='utf-8'))
    return validate_config(config, row, root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', required=True, choices=['windows', 'linux'])
    args = parser.parse_args()
    try:
        row = check_platform(args.platform)
    except (ValueError, KeyError, TypeError, OSError, json.JSONDecodeError):
        parser.exit(1, 'Platform policy failed; inspect the matrix, pinned schema and platform overlay.\n')
    print(f"Platform policy ok: {row['id']}; static configuration only, native validation remains separate")


if __name__ == '__main__':
    main()
