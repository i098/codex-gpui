#!/usr/bin/env bash
# Install the pinned relay without copying or changing the signed application.
set -euo pipefail
version=0.11.6
sha256=0f512eb0d00a3576edfd4509fb5ff1b5917a0a069080e3a22ad196d70e7e5ccb
app=${CODEX_GPUI_LCU_APP:-/Applications/ChatGPT.app}
codex=${CODEX_BIN:-}
while (($#)); do
    case $1 in
        --codex) codex=$2; shift 2 ;;
        --existing-app) app=$2; shift 2 ;;
        --help) echo 'usage: lcu-setup.sh [--codex PATH] [--existing-app PATH]'; exit ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
done
: "${codex:=$app/Contents/Resources/codex-cli/bin/codex}"
[[ $(uname -s)-$(uname -m) == Darwin-arm64 ]] || { echo 'Computer use setup requires Apple Silicon macOS.' >&2; exit 1; }
[[ -n ${CODEX_HOME-$HOME/.codex} && ${CODEX_HOME-$HOME/.codex} == /* ]] || { echo 'CODEX_HOME must be an absolute, nonempty path.' >&2; exit 1; }
home=${CODEX_HOME-$HOME/.codex}
prefix=$home/computer-use/lcu
release=$prefix/current
if [[ ! -e $release ]] || [[ $("$release/bin/lcu" --version) != "lcu $version ("* ]]; then
    scratch=$(mktemp -d)
    trap 'rm -rf -- "$scratch"' EXIT
    archive=lcu-$version-darwin-arm64.tar.gz
    curl --fail --location --proto '=https' --tlsv1.2 \
        "https://github.com/amontlabs/lcu/releases/download/v$version/$archive" -o "$scratch/$archive"
    printf '%s  %s\n' "$sha256" "$scratch/$archive" | shasum -a 256 -c -
    tar -xzf "$scratch/$archive" -C "$scratch"
    "$scratch/${archive%.tar.gz}/scripts/install.sh" --prefix "$prefix" --existing-app "$app" --runtime-only --yes
fi
# LCU validates the original app, helper, architecture, and signature before registration.
"$release/bin/lcu" --version
node=$(cat "$release/node-path")
"$node" --input-type=module - "$release" "$codex" "$version" <<'JS'
import { readFileSync, realpathSync } from 'node:fs';
import { isAbsolute, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { isDeepStrictEqual } from 'node:util';
const [root, cli, version] = process.argv.slice(2);
const load = name => import(pathToFileURL(join(root, 'lcu', `${name}.mjs`)));
const manifest = JSON.parse(readFileSync(join(root, 'bundle.json'), 'utf8'));
if (manifest.version !== version) throw new Error(`Expected LCU ${version}; found ${manifest.version}.`);
const { installerEnvironment, hostPolicy } = await load('setup');
const { withLock } = await load('lock');
const { installHooks, selectedCodexHome } = await load('codex_hooks');
const { withAppServer } = await load('app_server');
const { mergeCodexPolicy } = await load('approval');
const { parse } = await load('toml');
const { paths } = await load('runtime');
const { resources } = paths(root);
const env = installerEnvironment(process.env.HOME, ['codex']);
const home = selectedCodexHome(env);
if (!isAbsolute(home)) throw new Error('CODEX_HOME must be absolute.');
const command = realpathSync(join(root, 'agent-tools/node/bin/node'));
const args = [join(root, 'adapters/codex.mjs'), join(root, 'bin/lcu')];
const policy = await hostPolicy(root);
const plugins = join(resources, 'plugins/openai-bundled');
const configPath = join(home, 'config.toml');
await withLock(join(home, 'computer-use/lcu-registration.lock'), async () => {
    // The exact selected CLI must discover and trust the original hooks before enabling the relay.
    await installHooks(cli, configPath, home, env, plugins);
    await withAppServer(cli, home, env, async server => {
        // config/read adds null defaults that the TOML writer cannot accept.
        const config = parse(readFileSync(configPath, 'utf8'));
        const previous = config.mcp_servers?.lcu ?? {};
        const provider = { ...mergeCodexPolicy(previous, policy), command, args, enabled: true };
        const edits = [];
        if (!isDeepStrictEqual(previous, provider)) {
            edits.push({ keyPath: 'mcp_servers.lcu', value: provider, mergeStrategy: 'replace' });
        }
        if (config.mcp_servers?.cua_repl?.enabled !== false && config.mcp_servers?.cua_repl) {
            edits.push({ keyPath: 'mcp_servers.cua_repl.enabled', value: false, mergeStrategy: 'replace' });
        }
        for (const [name, plugin] of Object.entries(config.plugins ?? {})) {
            if (name.split('@')[0] === 'unified-computer-use' && plugin.enabled !== false) {
                edits.push({ keyPath: `plugins.${JSON.stringify(name)}.enabled`, value: false, mergeStrategy: 'replace' });
            }
        }
        if (edits.length) await server.call('config/batchWrite', { edits, reloadUserConfig: true });
    });
});
console.log(`Registered LCU ${version} with ${cli} in ${home}. Restart Codex GPUI to use it.`);
console.log('Registration does not grant macOS permissions or prove native control.');
JS
