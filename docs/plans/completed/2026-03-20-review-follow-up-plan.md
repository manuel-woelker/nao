# What problem does this plan solve?

The repository review found a handful of issues that are small individually but bad in aggregate:

- failure summaries report the wrong elapsed time
- task selector wildcard rules can collide with literal task names
- the TUI rereads run artifacts more often than necessary
- CLI task listing always emits ANSI styling
- contributor guidance references a missing testing document

This plan describes how to fix those issues without turning the work into a giant refactor.

# What is the current status?

The review follow-up work described in this plan has landed.
This completed copy is retained as historical implementation context, not as a statement that the listed gaps still exist.

Before the follow-up work landed, the codebase had these correctness and maintainability gaps:

- `TaskFailure.elapsed_nanos` is currently populated using run-relative timing while some observer callbacks use task-relative timing
- selectors containing `_` are treated as wildcard patterns, but task names are not validated to keep that syntax unambiguous
- the TUI refresh loop reloads run detail and task logs from disk on every refresh tick when a run is open
- run history discovery loads full run detail for each run directory even when only summary data is needed
- `nao --list` renders bold ANSI escapes even when output is redirected
- `AGENTS.md` tells contributors to consult `docs/TESTING.md`, but that file does not exist

# What implementation approach should be used?

This work should be split into small, testable slices instead of trying to "clean up the whole project" in one pass.

The implementation should:

- fix correctness issues first
- tighten user-facing CLI behavior next
- reduce obvious TUI I/O inefficiencies without rewriting the TUI architecture
- repair repository documentation so contributor instructions match reality

The TUI work should stay incremental.
The goal is to stop unnecessary rereads and duplicate parsing, not to build a complex cache invalidation system on the first pass.

# Why should the timing bug be fixed first?

The timing bug is a user-visible correctness issue.
If a task waits on dependencies and then fails quickly, the CLI currently reports the total run time as if it were the task runtime.

That produces misleading failure summaries and makes the existing run-observer timing inconsistent with the final rendered CLI output.
This is the clearest bug in the review and should land first.

# How should selector ambiguity be addressed?

The current selector model uses `_` as a wildcard token because `*` would be shell-hostile.
That is fine, but the repository needs one unambiguous rule for literal task names.

The simplest safe fix is to reject task names containing `_` during recipe validation and document that restriction explicitly.
That preserves the current selector syntax and avoids adding escaping rules or dual parsing modes.

If the project later wants `_` in task names, that should be a separate design change with an explicit exact-match escape mechanism.

# How should the TUI refresh behavior be improved?

The current TUI does more disk I/O than needed:

- open run detail is reloaded every refresh tick
- selected task logs are reloaded every refresh tick
- run history summaries are built by parsing full run detail

The first improvement slice should:

- reload run detail only when an active run is open or when the user changes the selected run
- reload task logs only when the selected task changes, the open run changes, or an active run is still producing output
- load run history from summary files directly when available instead of reparsing full detail

This keeps the behavior simple while removing the most obvious waste.

# How should CLI output behavior be improved?

The CLI should render styled task names only for interactive terminals.
When output is redirected or piped, it should emit plain text.

This should apply at least to `--list`, since that mode is especially likely to be consumed by scripts or shell tools.

# How should the documentation gap be addressed?

The repository should either add `docs/TESTING.md` or stop referring to it.

The better fix is to add the document because testing guidance is already a stated project expectation.
The document should stay small and concrete:

- where tests should live
- when to use `PalMock`
- when snapshot tests are appropriate
- which repository-wide checks to run

# What implementation order is recommended?

The recommended order is:

1. Fix failure timing so task failure summaries use task-relative elapsed time consistently.
2. Add or update engine and CLI tests that would have caught the timing bug.
3. Reject `_` in task names during recipe validation and document selector naming rules.
4. Add parser tests for the task-name restriction and selector behavior.
5. Gate `--list` ANSI styling on interactive terminal detection.
6. Add runner tests for interactive versus non-interactive task listing.
7. Refactor TUI refresh paths so completed runs do not trigger full rereads every tick.
8. Teach run history discovery to use summary data directly when possible.
9. Add focused TUI/artifact-store tests around refresh behavior and summary loading.
10. Add `docs/TESTING.md` and update contributor-facing references if needed.
11. Run `./scripts/check-code.sh`.

# What concrete work items should be tracked?

- [x] Update task failure accounting so `elapsed_nanos` represents task runtime rather than run-relative time.
- [x] Keep failure rendering and observer callbacks aligned on elapsed-time semantics.
- [x] Add or update engine tests for dependency-delayed task failures so the reported duration matches the task runtime.
- [x] Reject task names containing `_` during recipe validation.
- [x] Document the selector wildcard rule and the task-name restriction in user-facing docs.
- [x] Add parser tests covering rejected `_` task names.
- [x] Change CLI task-list rendering to emit ANSI escapes only for interactive terminals.
- [x] Add runner tests for interactive and non-interactive `--list` output.
- [x] Avoid reloading run detail on every TUI refresh tick when no active run is changing.
- [x] Avoid reloading task logs on every TUI refresh tick when the selected task and run are unchanged.
- [x] Load run history summaries without parsing full run detail when summary data is available.
- [x] Add or update TUI/artifact-store tests covering the reduced reload behavior.
- [x] Add `docs/TESTING.md` with repository-specific testing guidance.
- [x] Update any stale documentation links or contributor instructions that mention testing guidance.
- [x] Run `./scripts/check-code.sh`.

# How should the work be verified?

This cleanup pass did re-run `./scripts/check-code.sh`, and it passed.
The checklist in this completed copy therefore matches both repository state and the verification rerun performed while finishing the follow-up work.

Verification should include:

- colocated engine tests for failure timing
- colocated parser tests for invalid task names containing `_`
- runner tests for ANSI versus plain-text task-list output
- TUI or artifact-store tests that assert reduced reload behavior through observable effects
- documentation review to ensure all referenced testing docs exist
- `./scripts/check-code.sh`

For the TUI work, prefer behavior-based tests over timing-based tests.
The important assertion is that unnecessary reload paths stop happening, not that a specific number of milliseconds elapses.

# What assumptions should remain explicit?

This plan assumes:

- keeping `_` as the wildcard token is preferable to redesigning selector syntax right now
- rejecting `_` in task names is acceptable as a short-term compatibility constraint
- the first TUI performance pass should optimize current behavior rather than introduce filesystem watching or a persistent cache
- summary-file-driven history loading is sufficient for the current scale of `.nao/runs`

# What risks or follow-up questions matter most?

The main risks are:

- changing task-name validation could reject recipes that already rely on `_`
- TUI refresh changes could accidentally stop live updates if reload conditions are too aggressive
- partial optimization of artifact loading could add branching that makes the TUI harder to reason about

The main follow-up questions are:

- whether task selector syntax should eventually support an explicit exact-match mode
- whether the TUI should later move to file watching instead of polling
- whether more CLI output modes should distinguish interactive and non-interactive rendering
