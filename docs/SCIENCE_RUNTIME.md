# Science runtime facts used by CSSwitch

Last focused verification: 2026-07-12, Claude Science `0.1.18-dev.20260709.t211149.shab3f5130-release` (`b3f5130a`). Reverify these facts when the upstream binary changes.

## Confirmed facts

- CSSwitch launches Science with a fixed, persistent data-dir: `~/.csswitch/sandbox/home/.claude-science`.
- The same directory contains runtime assets, active-organization state, projects, and organization-owned Skills. Restarting Science with the same directory preserves them.
- Science's native Settings > Skills UI provides `Add skill` and `Import from GitHub`. The UI states that it accepts plugin-marketplace repositories or repositories with `skills/` directories.
- A fresh isolated Science data-dir initialized standard multi-file Skill directories under `orgs/<org-id>/skills/<skill>/`, including `SKILL.md` plus optional `scripts`, `references`, and other resources. Science displayed those Skills without CSSwitch scanning or deploying them.
- Science upgrades should reuse this data-dir. CSSwitch may select a newer Science executable, but must not treat the application bundle as the user-data source of truth.
- The Agent-facing `host.skills` SDK exposes `list`, `read`, `edit`, `publish`, and `delete`, but no local `install` or `import` method. The UI GitHub importer uses a separate marketplace API.

## What was not proved

The 2026-07-12 isolated GitHub preview initially stayed at `Fetching...` for both configurations below:

1. `ANTHROPIC_BASE_URL` plus process-wide `HTTPS_PROXY` through CSSwitch Gateway.
2. The same `ANTHROPIC_BASE_URL` with all process-wide proxy variables removed.

The later real-machine attempt with `https://github.com/anthropics/skills/tree/main/skills/pdf` produced an invalid GitHub API request ending in `/commits/main/skills/pdf` and HTTP 422. A conversation request to install the same Skill downloaded its files, then misrouted into the authoring flow `host.skills.edit`; Science refused the new draft because its account-backed Skill catalog was degraded.

The matching Science log showed repeated account fetch HTTP 401 responses followed by `[skillCatalog] provider list() degraded`. This proves that the currently supported UI import and Agent authoring paths are not usable without that catalog in the tested third-party session. It does not prove that every standard Skill directory intrinsically requires OAuth, nor that copied content was discovered, triggered, or executed.

## Evidence vocabulary

Never collapse these into “installed successfully”:

1. repository/content fetched;
2. standard Skill directory created;
3. Science discovered and displayed it;
4. the Skill was selected or triggered;
5. its actual function completed;
6. the data survived a Science restart.

CSSwitch must remain fail-open with respect to all Skill stages. A future upstream verification may document reload behavior, but it must not add a startup gate.
