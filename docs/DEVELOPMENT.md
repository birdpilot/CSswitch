# Development rules

## Scope first

CSSwitch is a switcher and launcher, not an alternate Science product. Before implementing a Science-adjacent feature, follow this order:

1. verify the upstream runtime fact in an isolated environment;
2. assign ownership and the source of truth;
3. prototype the shortest path without new storage or state machines;
4. run one real end-to-end case and separate each evidence stage;
5. only then decide whether CSSwitch needs UI, catalog, cache, or storage.

If Science already owns the capability, CSSwitch may expose a link or diagnostics, but must not duplicate its installer, validation, directory ownership, or lifecycle.

## Safe runtime verification

- Use a temporary outer `HOME`, persistent temporary Science data-dir, dynamic ports, and fake `security`.
- Never inspect OAuth tokens, API keys, Keychain contents, SSH material, or account databases.
- Never use the real user Science data-dir, port `8765`, or `/Applications/CSSwitch.app` as a test target.
- State source tests, built-artifact tests, installed-copy tests, installed-runtime tests, live-provider tests, signing/notarization, and public release as separate evidence layers.
- A test that copied files does not prove Science discovery; discovery does not prove triggering or functional execution.

## Dirty worktrees

Treat uncommitted work as user data. Do not reset, clean, overwrite, or remove a dirty worktree. Use a focused worktree from the intended base for isolated changes. Remove old worktrees only after verifying they are clean and no branch or artifact is still needed; do not delete remote branches automatically.

## Documentation maintenance

- `docs/ARCHITECTURE.md` is the only current architecture contract.
- Update it and `docs/SCIENCE_RUNTIME.md` when ownership, paths, or verified upstream facts change.
- Update README for real user-visible behavior, CHANGELOG only for shipped versions, and `docs/RELEASE.md` when gates change.
- Put dated, read-only evidence in `docs/investigations/` only when it is worth retaining; it is never normative.
- Archive superseded designs rather than leaving them beside current specifications without a warning.
- After every release or important runtime finding, reconcile the architecture, capability status, known limitations, and release evidence before starting the next feature.
