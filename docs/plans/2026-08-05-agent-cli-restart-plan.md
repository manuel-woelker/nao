# How can agents request a running nao process to restart?

Agents need a CLI command that asks an already-running `nao` invocation to restart its current task graph. This should reuse the same behavior as `Ctrl+R`: cancel the in-flight run, let process cleanup happen through the PAL cancellation path, and start the same planned run again.

Use a deliberately small command surface:

```text
nao --restart
```

This command should not run tasks. It should only signal a restart request for the current workspace by touching a restart marker file.

# What restart signal should be used?

Use one workspace-local marker file:

```text
.nao/internal/restart
```

On startup, the interactive runner should ensure the file exists without truncating it when it already exists. It should then remember the file's current modification time. While the run is active, it should poll the file mtime every 1000ms. When the mtime changes, it should trigger the same restart path used by `Ctrl+R`.

The `nao --restart` command should resolve the normal `.nao` workspace path and touch `.nao/internal/restart`, creating parent directories and the file when needed.

# What are the MPMC tradeoffs?

This design is a many-producer, many-consumer signal, not a queue.

That is acceptable for restart because restart requests are naturally coalescing. If three agents touch the marker while a run is active, one restart is enough. Producers do not need identity, ordering, or acknowledgement for the initial feature.

Marker file mtime:

- Pros: very small implementation, easy for agents to trigger, debuggable with normal shell tools, workspace-scoped, no daemon, no PID discovery, no socket lifecycle.
- Cons: requests coalesce, there is no per-agent acknowledgement, mtime granularity must be sufficient, polling means restart latency can be up to 1000ms, and multiple running `nao` processes in the same workspace will all restart.

Filesystem queue:

- Pros: supports distinct requests, stronger delivery semantics, better future targeting if run ids are needed.
- Cons: more moving parts, stale cleanup, request claiming, and matching rules. This is overkill until restart needs acknowledgement or per-run targeting.

Unix signals:

- Pros: fast and Linux-friendly once a PID is known.
- Cons: still needs run discovery, repeated signals can coalesce, PID reuse is risky, and it is a poor workspace-level agent interface.

Unix domain sockets:

- Pros: low latency and supports request/response.
- Cons: significantly more lifecycle code and still needs a discovery story. Too much ceremony for a coalescing restart signal.

Recommendation: use the marker file now. Add a queue or socket only if future requirements need acknowledgement, per-run targeting, or lower latency.

# How should the implementation be structured?

Add a small restart marker module in the CLI crate. It should own:

- resolving `.nao/internal/restart`
- creating `.nao/internal`
- creating the marker file if missing without changing mtime unnecessarily during runner startup
- touching the marker file for `nao --restart`
- reading marker mtime for polling

The runner should combine keyboard and marker-file restart sources through one controller path. `Ctrl+R` should keep working exactly as it does today, while the marker poller should cancel the current run token and set the same restart-requested flag when it observes a changed mtime.

# What CLI and validation changes are needed?

Add an optional `--restart` flag.

Validation should reject incompatible combinations:

- `--restart` with task names
- `--restart` with `--ci`
- `--restart` with `--list`
- `--restart` with `--tui`
- `--restart` with `--init`
- `--restart` with `--version`

`--config` can remain valid if it is needed to resolve a non-default workspace recipe. If the current recipe path helpers cannot map `--config` cleanly to `.nao/internal/restart`, document the first implementation as default-workspace only and reject `--config`.

Help text should document:

- `nao --restart` requests restart of a currently running interactive `nao` process in the workspace
- it works by touching `.nao/internal/restart`
- a running process polls for that change roughly once per second
- the command does not start tasks by itself

# How should this be tested?

Use colocated tests for parser, validation, marker behavior, and runner behavior.

Mock PAL tests should cover:

- startup creates `.nao/internal/restart` when missing
- startup preserves the remembered mtime when the marker already exists
- `nao --restart` touches the marker file
- incompatible flag combinations are rejected
- the runner restarts when marker mtime changes
- repeated touches before the poller observes them coalesce into one restart
- keyboard `Ctrl+R` still works through the same controller path

Add a small real-filesystem PAL test only if mtime behavior cannot be represented well with `PalMock`.

# What is the implementation checklist?

- [ ] Add `--restart` to CLI parsing.
- [ ] Add validation for incompatible `--restart` flag combinations.
- [ ] Add restart marker path helpers for `.nao/internal/restart`.
- [ ] Implement startup marker creation without unnecessarily touching an existing marker.
- [ ] Implement `nao --restart` as a marker-file touch operation.
- [ ] Add a marker poller that checks mtime every 1000ms during interactive runs.
- [ ] Route marker changes and `Ctrl+R` through the same restart controller.
- [ ] Update help text with the marker-file behavior and one-second polling.
- [ ] Add colocated tests for parser, validation, marker touching, marker polling, and restart behavior.
- [ ] Run `./scripts/check-code.sh`.

# What assumptions need validation?

- Restart requests can be coalesced; the feature does not need per-agent acknowledgement.
- Restarting every active `nao` process in the same workspace is acceptable for the first version.
- Filesystem mtime resolution is sufficient for a one-second poll interval on the supported Linux filesystems.
- `.nao/internal` is acceptable as an implementation-owned directory and should not be edited by users except through `nao --restart`.

# What risks should be watched?

- Some filesystems have coarse mtime resolution. Tests should avoid relying on sub-second differences.
- Touching the file during runner startup would cause an immediate false restart if the mtime is captured too early.
- A restart request between process startup and remembered mtime capture can be missed unless startup ordering is precise.
- Direct-output tasks must keep their aligned output formatting after a restart status message is printed.
