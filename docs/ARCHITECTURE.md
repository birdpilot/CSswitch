# CSSwitch architecture

This file is the current architecture contract. Release notes and dated investigations are evidence, not replacements for it.

## Product boundary

CSSwitch is a provider switcher and launcher for Claude Science. It converts a selected provider profile into the Anthropic-compatible local endpoint Science expects, manages the CSSwitch Gateway, prepares the isolated local login state, and starts or reopens Science.

Science owns its product capabilities and data: projects, organizations, native Skills, Add Skill / GitHub import, runtime resources, and upgrades. CSSwitch must not make those features startup prerequisites. In the currently verified Science build, supported external-Skill authoring/import paths query the Anthropic account catalog and may fail in CSSwitch third-party mode; 0.4.4 neither emulates that catalog nor claims to fix external-Skill installation.

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
| Isolated Science runtime and user data | `~/.csswitch/sandbox/home/.claude-science` | Science |
| Native and imported Skills | Active organization under the Science data-dir | Science |
| Provider capability metadata | `catalog/capabilities.v1.json` | CSSwitch |
| Legacy Skill store/inventory from 0.4.2/0.4.3 | retained but unused | Neither runtime path |

CSSwitch reuses the persistent Science data-dir across launches and Science upgrades. It does not rebuild that directory, copy Skills into it, synchronize it in both directions, or delete user changes.

## Failure boundary

Provider configuration, Gateway startup, isolated-login preparation, port ownership, Science launch, and Science health/identity may fail one-click startup. Skill counts, legacy store conflicts, inventory corruption, missing Skill catalog data, and external `~/.claude/skills` must not fail or restart Science.

The Skill Manager source remains recoverable from the `v0.4.3` tag and protected development worktrees, but it is not compiled, registered, packaged, or executed in the focused runtime.
