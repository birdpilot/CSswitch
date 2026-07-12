# CSNative architecture reference

Reference snapshot: `eust-w/CSNative` commit `64a68b1`. This is a responsibility-boundary comparison, not a code source.

CSNative's useful architectural lesson is that the wrapper selects configuration, prepares runtime resources, launches Science with a stable data-dir, and manages the Science process lifecycle. Upgrades reuse the data directory while the executable/runtime may change. Configuration and organization selection affect launch context; they do not require the wrapper to own every capability stored by Science.

CSNative does not need a separate Skill platform for persistence. Science's own organization and Skill directories live naturally under its reused data-dir. CSSwitch should absorb that ownership boundary while keeping its own provider conversion and Gateway implementation.

Do not copy CSNative implementation code. When updating this note, pin the reviewed commit and record only behavior supported by that snapshot.
