# What problem does this plan solve?

[`crates/cli/src/main.rs`](/data/projects/nao/crates/cli/src/main.rs) is now doing too many jobs in one file.
It defines CLI flags, owns top-level execution, validates flag combinations, handles `--init`, renders help text, formats version metadata, and carries a large mixed test module.

That makes the CLI harder to change safely.
Even small feature work now means editing a 900-line file with unrelated responsibilities bundled together.

# What is the current status?

The current file is 902 lines long and contains these distinct concerns:

- CLI flag schema generation via `xflags`
- top-level program entry and dispatch
- mode-selection logic such as defaulting to the TUI
- request validation for `--tui`, `--ci`, `--version`, and `--init`
- recipe initialization and starter recipe text
- version metadata loading and formatting
- long-form help rendering
- tests for all of the above

The file already has one sibling module, [`runner.rs`](/data/projects/nao/crates/cli/src/runner.rs), so the next cleanup step should continue that direction instead of keeping `main.rs` as a grab bag.

# What implementation approach should be used?

The split should be responsibility-driven, not just a cosmetic shuffle.
The goal is to make each module small, easy to test, and named after one coherent concern.

The first pass should keep the public CLI behavior exactly the same while moving logic into focused modules.
That means no flag redesign, no output changes, and no behavioral cleanup mixed into the refactor unless needed to preserve compile-time correctness.

# How should the file be split?

The recommended target structure is:

- `main.rs`
  - keep only module declarations, `shadow!`, the `xflags!` definition, and the thin top-level entrypoint
- `command_dispatch.rs`
  - own `run()`, dispatch sequencing, and the decision about whether to call TUI, init, version, or the runner
- `request_validation.rs`
  - own `should_run_tui()` and all flag-combination validation helpers
- `recipe_init.rs`
  - own `initialize_recipe_file()` and `starter_recipe()`
- `version_metadata.rs`
  - own `VersionMetadata`, `load_version_metadata()`, `render_version()`, and normalization helpers
- `help_text.rs`
  - own `render_help()` and `indent_block()`

This keeps each file small enough to reason about and fits the repository guideline that structs, functions, and related logic should be organized into focused files instead of one oversized root module.

# Why should `main.rs` stay thin?

`main.rs` should answer one question: how does the binary start?

If it also owns feature logic, help text, and validation, every CLI change pulls the reader through too much context.
A thin `main.rs` also makes it easier to spot top-level wiring mistakes because the entrypoint is not buried inside unrelated code.

# How should tests be reorganized?

Tests should move with the code they cover rather than remaining in one giant `main.rs` test module.

Recommended test placement:

- dispatch tests near `command_dispatch.rs`
- validation tests near `request_validation.rs`
- init tests near `recipe_init.rs`
- version and normalization tests near `version_metadata.rs`
- help-rendering tests near `help_text.rs`

This matters because the current test module is doing too much.
It hides ownership boundaries and makes refactoring noisy.

# What should stay in `main.rs` even after the split?

The `xflags!` definition should probably stay in `main.rs` for the first pass because it defines the binary surface and the generated `Nao` type is central to module wiring.

That is a reasonable compromise:

- keep binary shape in `main.rs`
- move almost all behavior into helpers

If the project later wants a fully separate flag-definition module, that can be a second-step cleanup.

# How should module boundaries be kept clean?

The split should avoid creating a new ball of mud spread across multiple files.

Some concrete rules:

- prefer passing `&Nao` or simple arguments rather than introducing a large service object
- keep version formatting independent from PAL concerns
- keep help rendering independent from dispatch logic except where version text is injected
- keep init logic independent from flag validation
- avoid circular module dependencies by letting dispatch depend on the smaller helper modules

# What implementation order is recommended?

1. Extract version metadata logic into its own module.
2. Extract recipe initialization and starter recipe text.
3. Extract request validation and TUI-defaulting logic.
4. Extract help rendering.
5. Extract dispatch wiring so `main.rs` becomes a thin entrypoint.
6. Move tests alongside the extracted modules.
7. Run `./scripts/check-code.sh`.

# What concrete work items should be tracked?

- [x] Add focused CLI modules for dispatch, request validation, recipe initialization, version metadata, and help rendering.
- [x] Reduce [`main.rs`](/data/projects/nao/crates/cli/src/main.rs) to a thin entrypoint plus flag definition.
- [x] Preserve the existing CLI behavior for `--init`, `--version`, `--tui`, `--ci`, task execution, and help output.
- [x] Move validation tests out of the large `main.rs` test module and colocate them with the validation code.
- [x] Move init and starter-recipe tests next to the init module.
- [x] Move version formatting and normalization tests next to the version module.
- [x] Move help-rendering tests next to the help module.
- [x] Keep runner-related behavior in [`runner.rs`](/data/projects/nao/crates/cli/src/runner.rs) rather than expanding the new dispatch module.
- [x] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should rely on the existing CLI test coverage rather than introducing a second layer of integration-only checks.

The refactor should be considered complete when:

- the extracted module tests pass
- the existing command-dispatch behavior is still covered
- help rendering remains byte-for-byte stable where current tests assert on it
- `./scripts/check-code.sh` passes

# What assumptions should remain explicit?

This plan assumes:

- the current CLI surface is already acceptable and should not change during the split
- keeping `xflags!` in `main.rs` is an acceptable first-pass compromise
- a multi-file refactor is worthwhile even without adding new features because the current file size is already a maintenance problem

# What risks or follow-up questions matter most?

The main risks are:

- accidentally changing CLI behavior while moving code around
- introducing awkward module dependencies because the generated `Nao` type still lives in `main.rs`
- leaving tests in worse shape if they are only partially moved

The main follow-up questions are:

- whether the flag-definition macro should eventually move into a dedicated `flags.rs`
- whether the CLI should later gain a proper command/request type instead of passing raw flags through dispatch
- whether the long help text should eventually be generated from structured data instead of one large formatted string
