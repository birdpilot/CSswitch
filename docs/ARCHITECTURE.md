# CSSwitch architecture

This file is the current architecture contract. Release notes and dated investigations are evidence, not replacements for it.

## Product boundary

CSSwitch is a provider switcher and launcher for Claude Science. It converts a selected provider profile into the Anthropic-compatible local endpoint Science expects, manages the CSSwitch Gateway, prepares the isolated local login state, and starts or reopens Science.

Science owns its product capabilities and data: projects, organizations, native Skills, runtime resources, and upgrades. CSSwitch must not make those features startup prerequisites. In the currently verified Science build, built-in Add Skill / GitHub import paths may query the Anthropic account catalog and fail in third-party mode. CSSwitch does not emulate that catalog. Its external-Skill bridge accepts a public GitHub repository/collection/Skill URL or a host-selected local ZIP. The CSSwitch host downloads one archive, validates it, atomically commits either one Skill or one Nature-like Skill collection, and binds `OPERON` through a bounded loopback nonce/CSRF control plane. Bundle support preserves sibling Skills and support paths such as `_shared`; it does not install Plugin hooks, MCP, agents, or external runtime dependencies. The design does not enable Skill Manager, store, inventory, catalog, or SQLite.

## Runtime flow

```text
CSSwitch provider profile
  -> CSSwitch Gateway
  -> isolated local login state
  -> persistent Science data-dir
  -> start/reuse Science
  -> open Science UI
```

The one-click path must not pass through an external Skill directory, CSSwitch Skill store, inventory, Skill catalog, reconcile, or deploy step.

## Sources of truth and ownership

| Data | Source of truth | Owner |
| --- | --- | --- |
| Provider profiles and CSSwitch settings | `~/.csswitch/` configuration | CSSwitch |
| Gateway lifecycle and local routing | CSSwitch runtime state | CSSwitch |
| Installed Science executable | `/Applications/Claude Science.app/.../claude-science` | User / Science installer |
| Persistent Science state | `~/.csswitch/sandbox/home/.claude-science` | Science |
| Version-specific runtime resources | `<data-dir>/runtime/<version>/` | Science internal implementation |
| Native and imported Skills | `<data-dir>/orgs/<active-org>/skills/<name>/` | Science organization |
| Provider capability metadata | `catalog/capabilities.v1.json` | CSSwitch |
| Dormant Skill Manager/store/inventory source | retained but unused | Neither runtime path |

CSSwitch reuses the persistent Science data-dir across launches and Science upgrades. The normal launch path does not rebuild that directory, synchronize it in either direction, or delete user changes. The external-Skill bridge may atomically ensure its exact fixed route, install one explicitly requested package into the active organization, bind it to `OPERON`, and quarantine only content carrying a valid CSSwitch marker. Bundle ownership and path hashes live under `sandbox/skill-bundles/<org>/`; they are not Science database records. Same-name foreign content or a changed installed payload is never overwritten.

The App executable, persistent data-dir, version-specific runtime resources, and organization Skill directories are different facts. The data-dir provides state continuity; it is not an executable-version pin. CSSwitch normally selects the currently installed Claude Science App. A valid absolute non-symlink `SCIENCE_BIN` remains an explicit development override and fails closed when invalid.

CSSwitch never copies `bin`, `conda`, `runtime`, or `seed-assets` from the real `~/.claude-science` during initialization. Existing historical cache files are retained but are not migrated, deleted, or selected automatically. If the App is absent, a cached binary is eligible only when its version can be read and the user authorizes “use cached once” in the preflight UI. This choice exists only in memory for that launch. When the App is present it always wins.

After launch, CSSwitch records the selected binary path, source, and readable version in memory. URL, status, and stop operations use that identity. After a CSSwitch restart, recovery asks candidate binaries to confirm the same data-dir daemon; it never treats port occupancy alone as identity proof.

Science's `@` output/artifact references are request attachments, not registered Skills. They can provide file or prompt context but do not replace the Skill registry, Agent binding, scripts/resources, or persistent natural-language triggering.

The executable and data directory have different ownership. For a new launch CSSwitch prefers the binary inside the locally installed official `/Applications/Claude Science.app`, while the persistent sandbox directory remains the Science-owned data source of truth. CSSwitch never reads or clones runtime assets from the user's real `~/.claude-science`. A previously retained sandbox binary may be used only after a one-launch user authorization when the App is unavailable; CSSwitch does not download Science, invoke `claude-science update`, overwrite that fallback, persist the choice, or force-restart an already healthy daemon to apply version drift.

## Network exposure boundary

The CSSwitch Gateway binds loopback, and isolated Science is launched with an explicit `--host 127.0.0.1`; CSSwitch does not provide a `0.0.0.0` switch. CSSwitch assigns Science's preview listener explicitly on the port immediately after the UI port; configuration and launch preflight reject overflow, reserved, Gateway, or occupied preview ports. Raw `serve` output is discarded because the official CLI may print a data-dir or Web UI URL; CSSwitch logs only a generic result.

System SSH configuration reuse is a separate opt-in compatibility bridge for Science's native SSH functionality. When disabled, isolated Science does not see the real SSH config. When enabled, CSSwitch prepends a narrow `ssh` wrapper that invokes `/usr/bin/ssh -F <real-home>/.ssh/config`; it does not copy or link `.ssh`, expose private-key content to the UI, start `sshd`, enable Remote Login, change the firewall, or open a public listener. This grant is behavioral rather than file-count based: OpenSSH `Include`, `IdentityFile`, `IdentityAgent`, `ProxyCommand`, and `Match exec` rules can reach other files or commands under their normal semantics.

The local Rust backend obtains fresh one-time Science URLs for route reconciliation, Skill control preflight, each attach, and browser opening. It keeps them in backend memory and never serializes them into Tauri status. Attach subprocesses run with a cleared environment and a verified Science binary fingerprint. Closing the settings window only hides it and keeps the managed local chain running. Explicitly quitting CSSwitch stops the isolated Science daemon before the Gateway.

## Failure boundary

Provider configuration, Gateway startup, isolated-login preparation, runtime preflight, port ownership, Science launch, and Science health/identity may fail one-click startup. A missing App without an explicitly authorized readable cache is a runtime preflight result, not a provider or Skill error. Skill counts, legacy store conflicts, inventory corruption, missing Skill catalog data, external `~/.claude/skills`, and route/MCP registration failures must not fail or restart Science.

Science version discovery is fail-open with respect to an existing healthy daemon. A missing or non-runnable official App candidate makes a readable retained sandbox binary eligible for explicit one-launch authorization; it is never selected implicitly. Once a newer binary has actually attempted to open the persistent data-dir, CSSwitch must not blindly start an older binary against a potentially migrated directory.

The Skill Manager source remains in the repository for possible future work, but the two supported installation entries do not call its store, inventory, deployment, catalog, or database modules.
