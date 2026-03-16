# What problem does this plan solve?

`nao` needs a concrete recipe file format so users can define tasks, dependencies, execution modes, environment variables, and artifacts in a structured way. [`docs/RECIPES.md`](../../RECIPES.md) establishes KDL as the intended format, and this plan covered the first implementation slice of that format.

This plan describes how to turn that format direction into a working parser and typed recipe model in Rust.

# What is the current status?

This plan is complete.

The repository now has:

- a `nao-recipe` crate with typed recipe domain types
- KDL parsing and semantic validation for the initial documented subset
- colocated parser tests, including snapshot-style error assertions
- a minimal CLI code path that loads and validates a recipe file

Follow-up work can extend the schema, improve source-location diagnostics, and add broader execution integration.

# What implementation approach should be used?

The implementation should be split into two phases:

1. Parse KDL into a parser-specific document model.
2. Convert that parsed document into `nao-recipe` domain types with explicit semantic validation.

This keeps the public recipe API independent from the parser library and makes future schema evolution easier.

The initial implementation should stay narrow and focus on a small, end-to-end vertical slice rather than trying to support every documented feature at once.

# What data model should `nao-recipe` expose?

`nao-recipe` should expose typed Rust structures for the stable concepts in the recipe format instead of leaking raw KDL nodes into the rest of the codebase.

The initial domain model should likely include:

- `Recipe`
- `Task`
- `TaskName`
- `DependencyName`
- `RunSpec`
- `ArtifactSpec`
- `EnvironmentSpec`
- `RecipeError`

The error model should distinguish between:

- Syntax errors in the KDL input
- Semantic validation errors in an otherwise parseable recipe

# What subset should be implemented first?

The implemented subset supports:

- One top-level recipe
- `task` nodes with unique names
- Repeated `depends-on` entries
- A `run` node with exactly one execution form
- `shell`, `script`, and `container` execution forms
- Optional `env` entries
- Optional `artifact` entries with a name and path

The implemented slice still defers:

- Advanced reproducibility settings
- Per-task failure policy
- Includes or imports
- Parameterization or templating
- Execution-time artifact transport semantics

# What validation should be implemented?

The conversion and validation layer should check:

- missing required nodes or properties
- duplicate task names
- unknown dependency references
- invalid or conflicting `run` definitions
- malformed artifact declarations
- malformed environment declarations

Validation currently produces task-aware diagnostics and integrates with `nao-base` error rendering. Richer source-location reporting inside KDL files remains follow-up work.

# How should the work be ordered?

The recommended implementation order is:

1. Add a KDL dependency to `nao-recipe`.
2. Replace the placeholder recipe crate with a real module structure and domain types.
3. Implement parsing for the minimal task/dependency/run subset.
4. Add semantic validation and integrate errors with `nao-base`.
5. Add tests for success cases and invalid configurations.
6. Add a minimal CLI path that loads and validates a recipe file.
7. Extend the parser to cover `env`, `artifact`, and container arguments from the documented example.
8. Reconcile [`docs/RECIPES.md`](../../RECIPES.md) with the implemented schema if the implementation uncovers better naming or structure.

# What concrete work items should be tracked?

- [x] Add a KDL parsing dependency to `crates/recipe/Cargo.toml`.
- [x] Replace the placeholder `crates/recipe/src/lib.rs` with module declarations only.
- [x] Add small focused domain model files in `crates/recipe/src/`.
- [x] Implement a public recipe parsing entrypoint in `nao-recipe`.
- [x] Parse task names and `depends-on` entries.
- [x] Parse `run` nodes for `shell`, `script`, and `container`.
- [x] Parse optional `env` entries.
- [x] Parse optional `artifact` entries.
- [x] Detect duplicate task names.
- [x] Detect unknown dependency references.
- [x] Detect invalid or conflicting execution definitions.
- [x] Integrate recipe errors with `nao-base` error reporting.
- [x] Add colocated tests for successful parsing.
- [x] Add colocated tests for invalid recipe diagnostics.
- [x] Add snapshot tests where rendered diagnostics are part of the contract.
- [x] Add a minimal CLI code path to load and validate a recipe file.
- [x] Run `./scripts/check-code.sh`.

# How should the work be verified?

Verification should include:

- colocated unit tests for the recipe domain model and parser
- black-box tests for recipe parsing entrypoints
- snapshot tests for user-facing parse and validation errors
- at least one realistic KDL fixture that matches the documented recipe shape
- running `./scripts/check-code.sh`

If the implementation introduces fixture-based verification or a broader verification workflow later, the plan should be updated to reference that explicitly.

# What assumptions should remain explicit?

This plan assumes:

- the recipe format will remain KDL-based
- the documented example in [`docs/RECIPES.md`](../../RECIPES.md) is directionally correct, but not yet frozen as a final schema
- the first implementation should optimize for clarity and diagnostics rather than breadth
- the CLI only needs a minimal load-and-validate integration in the first milestone

# What risks or open questions matter most?

The main risks are:

- choosing KDL node names or property names too early
- exposing parser-library details in the public API
- producing weak diagnostics for user-authored configuration errors
- allowing the documented format and implemented format to drift apart

The main open design question is how much of the documented example should be treated as stable contract in the first implementation versus provisional design guidance.
