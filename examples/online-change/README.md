# online-change — hot-swap walkthrough

A minimal project for exercising the v1 online-change path: a counter function
block with a retained total, a task configuration, and a test suite.

> `stc` is **not a safety PLC** and is not responsible for safety. It can
> control real machinery, but safety must be provided by a separate safety
> system and machine design as required by law and regulation. Online change
> is a deployment convenience, not a safety function.

## Build and test

```sh
stc check
stc build
stc build --release
stc test
stc fmt --check
```

## Hot-swap walkthrough

`stc run --watch` builds against the active runtime manifest store, launches
the host, watches sources, and deploys each successful rebuild at a cycle
boundary. A failed or incompatible candidate leaves the previous generation
running.

1. Start the host:

   ```sh
   stc run --watch
   ```

2. In another terminal, inspect the active manifest:

   ```sh
   stc online download --out active.stc-got.json
   stc online diff target/stc/debug/online-change.stc-got.json
   ```

3. Edit `src/main.st` and save. A compatible change is:

   - changing `INCREMENT` (for example `:= 2`), or
   - adding a new `VAR` field to `Counter` or `Main`.

   `stc run --watch` rebuilds and activates the new generation at the next
   cycle boundary. Retained values and compatible fields keep their state.

4. Make a change v1 cannot migrate, such as changing `Count : DINT` to
   `Count : LREAL` or resizing an array. The build/prepare step is rejected
   with the compatibility report and the old generation keeps running.

5. Recover with the documented cold download: stop the host, remove stale
   state if the policy requires it, and start again with a cold build:

   ```sh
   stc build --cold
   stc run --cycles 10
   ```

6. Ctrl-C stops the watch. The store under `target/stc/runtime` is preserved by
   `stc clean`; retained state lives in `state/` by this project's policy.

Debug and online change do not mix: every swap ends a debug session and clears
forces and breakpoints. Reconnect with `stc debug` after the new generation is
active.
