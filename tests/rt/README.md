# rt-bench — WL-4 benchmark corpus

A deterministic ST project for the WL-4 measurement campaign. It has no I/O and no
dependencies; the base task interval is 1 ms.

## Layout

- `bench/` — base (medium workload) project; this is the online-change baseline.
- `workloads/light/` and `workloads/heavy/` — same program with a different `WORK`
  constant, built as separate cold-start launchers.
- `cases/code-only/` — compatible code-only change.
- `cases/field-add-remove/` — compatible field add and remove.
- `cases/incompatible/` — layout change, built cold; the runtime must report it as
  cold-required.
- `cases/trapping/` — compatible change with a no-interval trap task, so the trap fires
  on the first scan after activation and exercises one-cycle rollback.

The sources are built against the base manifest with `--got-manifest` where compatible.
The field add/remove case uses `--allow-untracked-removal`; the incompatible case uses
`--cold`.

## Building the bundle

The CI workflow `rt-build.yml` builds the Linux package and runs:

```sh
bash tools/rt/build-bundle.sh dist/stc-<version>-x86_64-unknown-linux-gnu dist/rt-bench
```

The bundle is transferred to the RT host and run there; the RT host never builds.
See [WL-4-Plan.md](../../WL-4-Plan.md) §9 for the runbook.
