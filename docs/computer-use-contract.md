# Computer Use integration contract

This contract fixes the interfaces for runtime registration (A), approvals (B), host settings (C), settings UI (D), and integration (E).
LCU owns desktop execution.
The Codex app-server owns the MCP connection.
Rust does not create another MCP client.

## Runtime registration

- Pin: [LCU 0.11.6](https://github.com/amontlabs/lcu/releases/tag/v0.11.6), source commit `19ec5797873fadeb6ff8b99eb94a091e2c7c1a14`.
- Darwin archive SHA-256: `0f512eb0d00a3576edfd4509fb5ff1b5917a0a069080e3a22ad196d70e7e5ccb`.
- Entry: `bridge::setup_computer_use() -> anyhow::Result<()>`.
- CLI entry: `codex-gpui --setup-computer-use`.
- Script: `tools/lcu-setup.sh`, embedded in the executable.
- Installation: `$CODEX_HOME/computer-use/lcu/current`; default `CODEX_HOME` is `~/.codex`.
- MCP server name: `lcu`.

Run setup outside the UI thread.
Show its error if registration fails.
Refresh host readiness and app-server configuration only after setup succeeds.
Reload the app-server with `config/mcpServer/reload`, then refresh `config/read`, after a Settings change.
Command-line setup still requires an app restart.
Do not show an enabled provider before that refresh succeeds.

Setup selects the same executable as `bridge::default_command()`.
Selection order is `CODEX_BIN`, the installed ChatGPT CLI, then `codex` on `PATH`.
Setup uses that exact CLI for hook discovery, trust, and `config/batchWrite`.
The original provider policy wins over conflicting existing LCU fields.
Other LCU fields, unrelated providers, and unrelated hooks remain unchanged.
Setup disables the existing `cua_repl` server and `unified-computer-use` plugins.
E must also remove any thread-specific original Computer Use override during cutover.
Keep unrelated browser and MCP providers unchanged.

The registration command uses the selected app's Node, the installed `adapters/codex.mjs`, and the installed `bin/lcu`.
No Chrome or audio flag is added.
The relay starts the original CUA REPL and keeps the original host-app approval guard.
The signed app and helper remain in their original locations.

Read `$CODEX_HOME/computer-use/lcu/current/installation.json` for `app`, `platform`, and `architecture`.
The adjacent `app` link must identify the same selected application.
For Darwin, the helper path below is relative to `installation.json.app`:

```text
Contents/Resources/cua_node/lib/node_modules/@oai/sky/Codex Computer Use.app
```

Read these paths without starting the helper.
Do not infer an OS permission grant from a file, process, socket, or successful setup.

## Tool and lifecycle traffic

Reuse `Bridge::request` and `Bridge::respond`.
Keep server-originated request IDs as opaque JSON values.
Retain real thread and turn IDs through queueing, navigation, cancellation, and responses.
Preserve unknown metadata fields.

The model tools are `js` and `js_reset`.
Use the runtime's tool schemas rather than defining another schema in Rust.
Host tools include `turn_ended` and `js_add_node_module_dir`.
They must not appear as model tools.
Each call carries the real identifiers in `_meta["x-codex-turn-metadata"]`.

Setup installs and trusts the original `Stop`, `Interrupt`, and `SubagentStop` hooks for server `lcu`.
Their `turn_ended` input retains `hook_event_name`, `session_id`, and `turn_id` substitutions.
E must not send a second cleanup call for those events.
Unsupported hooks fail setup before the provider becomes enabled.
The Any App switch changes the provider and these existing hooks in one `config/batchWrite`.
Hook enablement uses `hooks.state.<quoted opaque key>.enabled`; it does not change the trusted hash or create another hook.
Repeated native Stop actions share one pending interrupt for each thread and turn.

## Elicitation requests and responses

The app-server sends `mcpServer/elicitation/request` as a server-originated request.
Keep the enclosing `id`, `threadId`, and any supplied `turnId` unchanged.
Native app approval has this payload shape; retain all other supplied fields:

```json
{
  "mode": "form",
  "requestedSchema": {"type": "object", "properties": {}},
  "_meta": {
    "codex_approval_kind": "mcp_tool_call",
    "connector_id": "computer-use",
    "tool_params": {"app": "com.example.Fixture"},
    "persist": ["session", "always"]
  }
}
```

The `persist` array lists offered scopes, not granted scopes.
Offer only those scopes and require an explicit selection.
Answer the original request ID with exactly one result:

```json
{"action":"accept","content":{}}
```

```json
{"action":"accept","content":{},"_meta":{"persist":"session"}}
```

```json
{"action":"accept","content":{},"_meta":{"persist":"always"}}
```

```json
{"action":"decline"}
```

```json
{"action":"cancel"}
```

Use `cancel` for dismissal or withdrawal.
Never send command-approval `{ "decision": ... }` values for an elicitation request.
Never approve the agent's host application.
Keep the upstream relay guard active.
Other form schemas remain with the existing request-form implementation.

## Host readiness and settings

C owns `host::computer_use::ComputerUseSettings`.
Its `readiness() -> Result<Readiness>` returns these observations:

```text
Readiness {
    supported: bool,
    relay_installed: bool,
    provider_registered: bool,
    native_permissions: NativePermissions,
}
NativePermissions = Unknown | Unsupported
```

`relay_installed` means that the selected installation exists; it does not prove native control.
`provider_registered` means that the configured `lcu` provider is enabled.
`Unknown` is not a grant.
Use `Unsupported` on unsupported systems.
Show missing installation, missing registration, and read errors separately.
Do not add a synthetic `ready` or `granted` state.

The host settings contract is:

```text
get_app_approvals() -> Result<Option<AppApprovals>>
remove_app_approval(bundle_identifier: &str) -> Result<Option<AppApprovals>>
get_sound_mode() -> Result<Option<SoundMode>>
set_sound_mode(mode: SoundMode) -> Result<SoundMode>
AppApprovals { approved_apps, approved_bundle_identifiers }
AppApproval { bundle_identifier, display_name, icon_data_url: Option<String> }
SoundMode = ForegroundClicks | ForegroundAndBackgroundClicks | Off
```

JSON uses `approvedApps`, `approvedBundleIdentifiers`, `bundleIdentifier`, `displayName`, and `iconDataURL`.
Sound values are `foregroundClicks`, `foregroundAndBackgroundClicks`, and `off`.
A missing approvals store returns `None`; a corrupt store returns an error.
Approval removal is a trusted UI operation, not a model or generic fetch operation.
D owns UI data and action types without an app-crate dependency.
E translates C responses into D state and commits UI changes only after host operations succeed.

## Verified boundary

The runtime slice checks installation and registration without a model turn or native input.
An isolated Mac check used the installed ChatGPT `26.1013.40934` and CUA `0.0.30/20261014203459-0849368674e2`.
The app-server reported the three LCU lifecycle hooks as trusted.
This is registration proof, not acceptance of permissions, desktop input, browser control, Messages, or locked use.
