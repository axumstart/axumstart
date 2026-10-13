# Audit Fix Progress

## Overview

- **Total issues identified:** 25 actionable recommendations from `audit/review.md`.
- **Issues fixed and verified:** 21.
- **Issues fixed but not verified:** 1.
- **Issues in progress:** 0.
- **Issues blocked or deferred:** 3.
- **Issues confirmed already resolved or invalid:** 0.
- **Overall completion:** 84% (21/25 verified; unverified, blocked, and deferred items stay in the denominator).

Status is updated after each investigation or change. Findings that overlap are kept as separate entries when the audit gives them separate remediation work; cross-references are recorded in the relevant entries.

## P0-1 — MySQL repository SQL uses the wrong identifier quoting

**Status:** Verified
**Severity:** Critical

**Problem:** Repository query generation uses double-quoted identifiers throughout `dsl.rs` and `repository.rs`, while MySQL requires backticks in the supported default configuration.

**Expected Result:** Every generated SQL path uses the active backend's safe identifier quoting consistently.

**Before Edit:** `axumstart_db/macros/src/dsl.rs`, `repository.rs`, `insert.rs`, and `update.rs` contain mixed hard-coded identifier delimiters. `cargo check --features mysql` only proves compilation, not execution.

**Changes Made:** Added backend-aware identifier quoting in `axumstart_db/macros/src/dialect.rs` and runtime `quote_identifier` support in `axumstart_db/src/lib.rs`. Updated repository, relation, order, insert, and update SQL generation to use the renderer. Added SQLite runtime coverage and compiled the MySQL feature path.

**After Edit:** Generated repository SQL now uses quoted identifiers consistently for the selected backend. SQLite exercises reserved table names and MySQL code generation uses backticks.

**Verification:** `cargo test -p axumstart_db --features sqlite --offline` passed. Live disposable MySQL 8.4 and PostgreSQL 17 containers both passed `cargo test -p axumstart_db --no-default-features --features <backend> --test runtime --locked -- --ignored --nocapture` with explicit DATABASE_URL. Tested reads, ordered joins, updates, deletes, empty lists, cross-table writes/upserts, pagination, and rollback. No existing user database was used.

**Remaining Work:** None for the reported dialect bug.

## P0-2 — Runtime table-dependent SQL is cached in one `OnceLock`

**Status:** Verified
**Severity:** Critical

**Problem:** Generated insert/update helpers cache SQL that includes runtime `table` and `conflict_col` arguments. Reusing a values type across repositories can reuse the first table's SQL.

**Expected Result:** SQL cache keys include every SQL-shaping identifier, or generated SQL is fixed per repository method; no call can write to a previous call's table.

**Before Edit:** `axumstart_db/macros/src/insert.rs` and `update.rs` use function-local `OnceLock`s around `format!(..., table)`. The previous audit probe showed writes intended for table `b` reaching table `a`.

**Changes Made:** Removed `OnceLock` caching from generated insert/update helpers whenever SQL contains runtime table or conflict identifiers. SQL is built per call and wrapped with `sqlx::AssertSqlSafe` after explicit identifier quoting.

**After Edit:** Reusing one values type across multiple repository tables no longer reuses the first table's SQL.

**Verification:** SQLite regression `generated_queries_preserve_tables_and_filter_semantics` passed for inserts, updates, and upserts across `order` and `archive` tables.

**Verification update:** The same runtime suite passed on MySQL 8.4 and PostgreSQL 17. An initial implementation failed to compile because SQLx 0.9 requires `SqlSafeStr` for dynamic SQL; identifiers were audited/quoted before using SQLx's `AssertSqlSafe` wrapper. No leaked static allocations or global query caches were added.

**Remaining Work:** None for the reported cache bug.

## P1-1 — Relation SELECTs leave base columns unqualified

**Status:** Verified
**Severity:** High

**Problem:** Base filter columns remain unqualified when relation joins are present, so common names such as `id` become ambiguous.

**Expected Result:** All generated base-table and joined-table columns are qualified and correctly quoted when a query contains joins.

**Before Edit:** `axumstart_db/macros/src/dsl.rs::resolve_target` returns plain base column names. The prior SQLite probe reproduced `ambiguous column name: id`.

**Changes Made:** `resolve_target` now qualifies base and relation columns with dialect-quoted table/column names. Join and order rendering use the same quoting path.

**After Edit:** Joined queries with common columns such as `id` are unambiguous.

**Verification:** SQLite relation and ordered-relation queries passed; DSL unit expectations were updated and pass.

**Remaining Work:** None for the reported ambiguity bug.

## P1-2 — `set_*_by_*` permits joined filters without emitting a JOIN

**Status:** Verified
**Severity:** High

**Problem:** `gen_set` accepts relation filters through `build_where` but generates an UPDATE without JOIN clauses.

**Expected Result:** Unsupported joined `set_*` filters fail at macro expansion with a clear diagnostic, or valid backend-specific join/update SQL is emitted.

**Before Edit:** `axumstart_db/macros/src/repository.rs::gen_set` emits `UPDATE ... WHERE "relation"...`; the prior SQLite probe reproduced `no such column: owner.name`.

**Changes Made:** `gen_set` now rejects filters requiring a relation JOIN with a compile-time error instead of emitting invalid UPDATE SQL. Added a trybuild compile-fail fixture.

**After Edit:** Joined `set_*_by_*` methods fail before code generation; FK-only filters remain supported.

**Verification:** The new `set_join.rs` fixture and the SQLite runtime suite passed.

**Remaining Work:** None for the chosen rejection behavior.

## P1-3 — Component cycle detection is shared across async tasks

**Status:** Verified
**Severity:** High

**Problem:** `ComponentProvider::resolving` is shared across cloned providers and remains populated while a factory awaits. Concurrent resolution of the same type is misclassified as a circular dependency.

**Expected Result:** Cycle detection follows one logical dependency chain; concurrent callers wait for the same cached initialization rather than panic.

**Before Edit:** `axumstart_components/src/lib.rs::resolve_by_id` uses an `Arc<Mutex<Vec<_>>>`. A prior focused probe reproduced `circular dependency detected: u32 -> u32` from two concurrent callers.

**Changes Made:** Replaced the shared mutable stack with immutable task-local dependency chains scoped around each factory future in `axumstart_components/src/lib.rs`. Provider identity distinguishes independent containers. Added a blocked-factory concurrency regression.

**After Edit:** Concurrent callers wait on the same `OnceCell`; nested cycles still panic. Spawned tasks start without an inherited dependency chain. Expected result achieved.

**Verification:** First attempt (provider-carried paths) failed the existing OnCreate self-spawn test. Replaced it with future-scoped Tokio task-local state; `cargo test -p axumstart_components --locked --offline` passed including concurrent initialization, genuine cycle detection, and all lifecycle tests. No test expectations were weakened.

**Remaining Work:** Cross-task cyclic waits where a factory awaits a task that in turn awaits the same component remain unsupported; the API cannot infer task wait dependencies.

## P1-4 — Missing `Inject<T>` returns HTTP 200 and leaks type names

**Status:** Verified
**Severity:** High

**Problem:** `Inject<T>` uses `String` as its rejection, which Axum treats as a successful response body. Missing dependencies return 200 and expose a Rust type name.

**Expected Result:** Missing dependencies produce a controlled 500 response with a generic body; diagnostic type details stay server-side.

**Before Edit:** `axumstart_components/src/axum.rs` sets `type Rejection = String`; the prior request probe returned `200 OK`.

**Changes Made:** Added `MissingComponent` rejection in `axumstart_components/src/axum.rs`, returning HTTP 500 with a generic body; internal type name is written to server stderr. Added `tests/extractor.rs`.

**After Edit:** Missing configuration is a server failure rather than a successful API response. Expected status result achieved.

**Verification:** `cargo test -p axumstart_components --locked --offline` passed the new rejection test. Full router body assertions are tracked under P2-6.

**Remaining Work:** None.

## P1-5 — Empty `_in` values generate `IN ()`

**Status:** Verified
**Severity:** High

**Problem:** `push_in_list` emits an empty parenthesized list for an empty vector, which is invalid or inconsistent across databases.

**Expected Result:** Empty `_in` values deterministically match no rows without malformed SQL or a database error.

**Before Edit:** `axumstart_db/src/lib.rs::push_in_list` always emits `(`, `)`, and no special empty case.

**Changes Made:** `push_in_list` now renders `IN (NULL)` for an empty iterator, giving a deterministic no-match predicate while preserving bound values for non-empty lists.

**After Edit:** Empty `_in` SELECT, UPDATE, and DELETE paths no longer emit `IN ()`.

**Verification:** SQLite runtime coverage exercises empty count, update, and delete paths and passed.

**Remaining Work:** None for the reported semantics; live PostgreSQL/MySQL execution also passed in the backend matrix.

## P1-6 — SQLite `_this_week` is wrong on Mondays

**Status:** Verified
**Severity:** High

**Problem:** SQLite `WEEK_START_SQL` uses `weekday 1` followed by `-7 days`, which resolves Monday to the prior week.

**Expected Result:** The expression returns the current Monday for every day of the week under the documented timestamp convention.

**Before Edit:** `axumstart_db/src/lib.rs:41`; the prior probe evaluated Monday `2026-10-12` as `2026-10-05`.

**Changes Made:** Replaced SQLite `WEEK_START_SQL` with a Monday-offset expression that does not subtract an extra week on Monday.

**After Edit:** Monday returns the current Monday and Sunday returns the preceding Monday under the documented timestamp convention.

**Verification:** SQLite boundary assertions for Monday, Sunday, and a preceding Sunday passed.

**Remaining Work:** None for the reported boundary bug.

## P1-7 — `_this_week` is appended without grouping

**Status:** Verified
**Severity:** High

**Problem:** Flat OR/AND filters receive the week condition after the filter, allowing SQL precedence to bypass the week constraint for the first OR branch.

**Expected Result:** `_this_week` constrains the complete user filter, e.g. `(a OR b) AND week_boundary`.

**Before Edit:** `axumstart_db/macros/src/repository.rs::gen_select_all` appends the date predicate directly; the prior probe returned an old row matching the first OR branch.

**Changes Made:** Parenthesized the complete user filter before appending `_this_week` in both static and QueryBuilder paths.

**After Edit:** `_this_week` now constrains the full OR/AND filter rather than only the final branch.

**Verification:** SQLite regression inserted an old first-branch row and a current second-branch row; only the current row was returned. Static and `_in` paths passed.

**Remaining Work:** None for the reported grouping bug.

## P1-8 — MySQL row-returning helpers rely on unsafe select-back assumptions

**Status:** Verified
**Severity:** High

**Problem:** MySQL row-returning insert helpers use `last_insert_id` and auto-increment ranges even for custom/non-auto-increment keys.

**Expected Result:** Returned rows are selected by known key values when possible, and unsupported shapes fail clearly rather than returning incorrect data.

**Before Edit:** `axumstart_db/macros/src/insert.rs::mysql_impl` uses `last_insert_id` for single insert and a contiguous range for batch insert. The limitation is documented but not enforced.

**Changes Made:** Added key-field detection to MySQL insert generation. Explicit numeric/string keys are selected back using bound values. Missing-key single inserts check `information_schema.columns` for an auto-increment key before writing, then use last_insert_id. Removed guessed batch ID ranges: missing-key row-returning batches return a protocol error before writing; void batches keep the one-statement path. Added `axumstart_db/tests/mysql_keys.rs`. Deliberate cost: one schema check for generated-key row-returning single inserts.

**After Edit:** Actual key values determine select-back. Unsupported batches fail without modifying the database. Expected result achieved within the documented two-statement/select-back semantics; this does not promise atomic write-plus-read operations through a pool.

**Verification:** Live MySQL `cargo test -p axumstart_db --no-default-features --features mysql --test mysql_keys --locked -- --ignored --nocapture` passed for custom string keys, sparse batch keys, generated single IDs, and no-write rejection paths. The first run caught a missing pre-write schema guard (row count 2 instead of 1); implemented it and reran successfully. Existing cross-backend runtime tests also passed.

**Remaining Work:** None for reported key/range assumptions. Documented compatibility change: row-returning generated-key batches now error instead of guessing IDs.

## P1-9 — Rust CI does not test the library

**Status:** Verified
**Severity:** High

**Problem:** The only workflow deploys docs and does not run Rust tests, formatting, Clippy, or backend checks.

**Expected Result:** Pull requests affecting Rust code run the relevant quality gates; docs deployment remains separate.

**Before Edit:** `.github/workflows/deploy-docs.yml` is the only workflow and is path-filtered to `docs/**`.

**Changes Made:** Added `.github/workflows/ci.yml` for locked workspace tests, Clippy, formatting, backend compilation, and SQLite runtime checks.

**After Edit:** Pull requests and pushes to `main` now run formatting, strict Clippy, locked workspace tests, backend feature checks, and SQLite runtime tests.

**Verification:** Local equivalents passed: `cargo fmt --all --check`, strict workspace Clippy, locked workspace tests, SQLite runtime tests, and all three backend feature checks. GitHub-hosted execution remains environment-dependent.

**Remaining Work:** None for the workflow scope; GitHub Actions execution will provide the final hosted confirmation.

## P1-10 — Deployment SSH host verification trusts live `ssh-keyscan`

**Status:** Fixed (unverified)
**Severity:** High

**Problem:** Deployment dynamically accepts the host key returned by `ssh-keyscan`, enabling trust-on-first-use substitution.

**Expected Result:** Deployment verifies a reviewed, pinned host key before rsync.

**Before Edit:** `.github/workflows/deploy-docs.yml:41` appends `ssh-keyscan` output to `~/.ssh/known_hosts`.

**Changes Made:** Replaced live key scanning with `SSH_KNOWN_HOSTS` secret content. No host keys or credentials were guessed. Secret provisioning and end-to-end deployment cannot be verified locally.

**After Edit:** Deployment writes a reviewed `SSH_KNOWN_HOSTS` value and uses strict host-key checking; it no longer trusts live `ssh-keyscan` output.

**Verification:** The workflow content and local YAML/script behavior were reviewed; actual secret configuration cannot be verified locally.

**Remaining Work:** Provision and review `SSH_KNOWN_HOSTS` in the repository settings before deployment.

## P1-11 — Component construction failures are represented as panics

**Status:** Deferred
**Severity:** High

**Problem:** Component factories and config parsing have no fallible build channel; missing/invalid configuration and factory failures commonly panic.

**Expected Result:** Applications can build components through a structured fallible API and propagate startup errors from `serve`.

**Before Edit:** `ComponentBlueprint::new` and `OnCreate` are infallible traits; `ComponentProvider::build` returns `Components`; `ComponentConfig` emits panics.

**Changes Made:** Existing infallible APIs remain unchanged. This item is deliberately deferred because a fallible component lifecycle requires a public API design rather than a local implementation patch.

**After Edit:** Current panic behavior remains explicit and compatible; the requested fallible startup channel is documented as a future API design.

**Verification:** Deferred by design; current panic behavior remains covered by existing tests.

**Remaining Work:** Design a separately versioned fallible registration/build API if startup error propagation becomes a release requirement.

## P2-1 — Facade runtime dependencies are not feature-aligned

**Status:** Verified
**Severity:** Medium

**Problem:** Root `axum`, `tokio`, and `dotenvy` dependencies are unconditional even though the facade uses them only for the router/serve feature.

**Expected Result:** `default-features = false` consumers do not pull router-only root dependencies.

**Before Edit:** Root `Cargo.toml` lists those dependencies unconditionally and `router` only enables subcrate dependencies.

**Changes Made:** Made root axum/tokio/dotenvy optional and enabled them through `router`; default features retain existing behavior.

**After Edit:** `axum`, `tokio`, and `dotenvy` are optional and enabled by the `router` feature. A no-default-features dependency tree is free of the router facade dependencies.

**Verification:** `cargo tree -p axumstart --no-default-features -e features`, no-default compilation, router compilation, and all backend feature checks passed.

**Remaining Work:** None.

## P2-2 — `Page` accepts invalid values and unchecked multiplication

**Status:** Verified
**Severity:** Medium

**Problem:** `Page::new` accepts negative values and `Page::number` can overflow `page * size`.

**Expected Result:** Callers have a fallible, checked constructor for external pagination values without breaking existing API compatibility.

**Before Edit:** `axumstart_db/src/lib.rs::Page::new` and `Page::number` are unchecked.

**Changes Made:** Added `PageError`, `try_new`, `try_number`, and `validate`. Generated paginated queries validate even struct-literal/legacy Page values before sending SQL. `number` retains its signature but panics explicitly for invalid arithmetic instead of release-mode wrapping.

**After Edit:** External inputs can be rejected fallibly and all generated query paths enforce nonnegative pagination. Expected result achieved.

**Verification:** `cargo test -p axumstart_db --features sqlite --offline` passed constructor boundary tests plus generated query rejection for negative limits. Static/QueryBuilder pagination paths compile in the trybuild suite.

**Remaining Work:** None; public constructor compatibility retained except invalid inputs now fail deterministically.

## P2-3 — Relation cardinality is not enforced

**Status:** Verified
**Severity:** Medium

**Problem:** `has_one` relies on an unverified schema uniqueness constraint, and duplicate relation target names are accepted. The quickstart profile migration lacks a unique constraint.

**Expected Result:** Example schema matches declared cardinality and duplicate relation target declarations fail clearly.

**Before Edit:** `examples/quickstart/migrations/0002_create_profile.sql` lacks `UNIQUE(user_id)`; `collect_relations` does not reject duplicate target tables.

**Changes Made:** Added duplicate relation-target validation and a compile-fail fixture. Added an additive profile uniqueness migration to preserve existing migration checksums.

**After Edit:** `has_one` example schema enforces uniqueness, and duplicate relation target tables fail at macro expansion.

**Verification:** The additive migration was reviewed, duplicate-relation trybuild coverage passed, and the complete database macro test suite passed.

**Verification update:** New duplicate-relation snapshot initially failed due to an incorrectly hand-counted span; corrected the new snapshot to underline the actual `has_one` attribute identifier (7 characters). The full 13 unit + 24 trybuild fixture suite then passed. Existing snapshots were unchanged.

**Remaining Work:** None for the reported example/schema and macro validation gaps.

## P2-4 — Server bootstrap is not configurable

**Status:** Verified
**Severity:** Medium

**Problem:** `serve(port)` hard-codes bind address and owns router construction, preventing straightforward global layers, custom routes, and shutdown integration.

**Expected Result:** Existing `serve(port)` remains available while callers can build the application/router and customize binding/layers.

**Before Edit:** Root `src/lib.rs::serve` performs all bootstrap steps in one function.

**Changes Made:** Added `build_app().await`, which loads the environment, resolves registered components, and returns the stateful auto-registered router. Existing `serve(port)` delegates to it; callers can merge routes and configure Axum serving/shutdown themselves.

**After Edit:** Bootstrap is customizable without breaking the existing `serve(port)` API.

**Verification:** `build_app` and `serve` doctests passed; router runtime coverage passed route registration, layers, injection, and missing-component behavior.

**Remaining Work:** None for the requested customization scope.

## P2-5 — Macro-supplied identifiers need safe quoting/validation

**Status:** Verified
**Severity:** Medium

**Problem:** Identifier strings are interpolated into SQL without one central escaping/quoting policy, especially in public low-level helper arguments.

**Expected Result:** Compile-time and runtime identifiers are safely quoted for the active backend; values remain bound.

**Before Edit:** SQL generation uses hard-coded quote characters and `format!` for table/column identifiers across `dsl.rs`, `repository.rs`, `insert.rs`, and `update.rs`.

**Changes Made:** Centralized compile-time and runtime identifier quoting for the active backend, including escaped quote handling. Values remain bound parameters; dynamic SQL is wrapped only after identifier validation/quoting.

**After Edit:** Reserved identifiers and quote characters render safely across generated SELECT, JOIN, order, insert, update, and delete SQL.

**Verification:** Identifier escaping unit tests, reserved-name SQLite runtime coverage, and live PostgreSQL/MySQL runtime suites passed.

**Remaining Work:** None for the reported injection/portability gap.

## P2-6 — Runtime SQL and router tests are missing

**Status:** Verified
**Severity:** Medium

**Problem:** Trybuild tests prove macro expansion but not generated SQL execution or actual Axum requests.

**Expected Result:** SQLite regression tests cover generated query paths and router/component tests exercise HTTP behavior; backend-specific tests cover dialects where services are available.

**Before Edit:** `axumstart_db/macros/tests/codegen.rs` and router macro codegen tests contain compile-only fixtures; quickstart DB code runs only manually.

**Changes Made:** Added SQLite generated-query runtime coverage, live PostgreSQL/MySQL database coverage, MySQL key behavior coverage, router HTTP coverage, component extractor coverage, and CI wiring.

**After Edit:** Macro expansion, generated SQL execution, backend behavior, dependency injection, route layers, missing injection, and cfg-disabled routes have regression coverage.

**Verification:** Workspace tests, SQLite runtime tests, router runtime tests, live PostgreSQL 17/MySQL 8.4 suites, and MySQL key tests passed.

**Remaining Work:** None for the identified runtime paths.

## P2-7 — Formatting and lint policy is not enforced

**Status:** Verified
**Severity:** Medium

**Problem:** `cargo fmt --all --check` and strict Clippy fail locally, and CI does not enforce either.

**Expected Result:** The workspace has one checked-in formatting policy and passes formatting/lint gates.

**Before Edit:** `rustfmt.toml` sets two-space indentation while several files are four-space formatted; Clippy reports `collapsible_if` in component macros.

**Changes Made:** Applied the repository's two-space rustfmt policy, fixed strict Clippy findings, and added CI gates for formatting and linting.

**After Edit:** The workspace has a consistent checked-in format and strict Clippy is clean.

**Verification:** `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --offline -- -D warnings` passed.

**Remaining Work:** None.

## P2-8 — Contributor/release workflow is incomplete

**Status:** Verified
**Severity:** Medium

**Problem:** There is no root README, contribution/release workflow, MSRV declaration, or dependency audit automation; docs deployment instructions do not match the SSH workflow.

**Expected Result:** A new contributor can build, test, document, and understand release/deployment expectations from repository docs.

**Before Edit:** No root `README.md`; `docs/README.md` describes GitHub Pages while `.github/workflows/deploy-docs.yml` uses SSH/rsync.

**Changes Made:** Added a root README covering quickstart, features, development commands, MSRV/release context, dependency trade-offs, and generated SQL troubleshooting. Reconciled docs deployment instructions with the SSH/rsync workflow.

**After Edit:** Contributors can find the build/test/docs commands and the deployment secret contract in repository documentation.

**Verification:** README/workflow content review passed; the Docusaurus `npm ci && npm run build` completed successfully.

**Remaining Work:** None for the documented workflow scope.

## P2-9 — Database macro generator is too large and cross-coupled

**Status:** Deferred
**Severity:** Medium

**Problem:** `repository.rs` combines parsing, planning, type inspection, diagnostics, and token generation in roughly 1,409 lines; insert/update dialect branches duplicate large blocks.

**Expected Result:** Future dialect/query changes can be made in focused modules without changing public behavior.

**Before Edit:** `axumstart_db/macros/src/repository.rs`, `insert.rs`, and `update.rs` contain the described responsibilities and duplicated rendering.

**Changes Made:** Deferred broad extraction until the runtime behavior is protected by regression coverage.

**After Edit:** Correctness changes remain in the existing generator structure; this maintainability refactor is intentionally separate.

**Verification:** Runtime and macro regression coverage is now present, but the refactor itself remains intentionally unimplemented.

**Remaining Work:** Split the generator in a separate refactor when maintainability work justifies its compatibility and review cost.

## P3-1 — `parking_lot` may be unnecessary

**Status:** Deferred
**Severity:** Low

**Problem:** Short synchronous component locks use `parking_lot::Mutex`, adding a dependency whose measurable benefit is unknown here.

**Expected Result:** Dependency choice is evidence-based and does not add complexity without benefit.

**Before Edit:** `axumstart_components/src/lib.rs` uses `parking_lot::Mutex`.

**Changes Made:** Kept `parking_lot::Mutex`; the locks are short synchronous critical sections and no benchmark currently justifies changing lock semantics.

**After Edit:** The dependency remains an intentional implementation choice pending evidence that the standard mutex is better for this workload.

**Verification:** Dependency usage was reviewed; no performance regression or portability issue was identified.

**Remaining Work:** Benchmark under a demonstrated contention workload before replacing the dependency.

## P3-2 — Document `async-trait` and `inventory` trade-offs

**Status:** Verified
**Severity:** Low

**Problem:** The architectural reason for these dependencies is spread across code rather than summarized for maintainers.

**Expected Result:** Documentation explains why object-safe repository traits and automatic registration currently require them.

**Before Edit:** No root architecture/contributor document covers these trade-offs.

**Changes Made:** Documented that `inventory` provides automatic registration and `async-trait` keeps generated async repository traits usable as object-safe `Arc<dyn Repository>` values.

**After Edit:** Maintainers can see why both dependencies are currently part of the public architecture.

**Verification:** Root README architecture notes reviewed.

**Remaining Work:** None.

## P3-3 — Generated SQL diagnostics are limited

**Status:** Verified
**Severity:** Low

**Problem:** Users must inspect macro expansion or database errors to understand generated SQL.

**Expected Result:** The project documents a safe development-time way to inspect generated SQL without logging bound secrets by default.

**Before Edit:** No generated-SQL troubleshooting workflow is documented.

**Changes Made:** Documented `cargo expand` as the development-time way to inspect macro-generated SQL while avoiding bound-value logging in production.

**After Edit:** Generated SQL can be inspected during development without adding a runtime diagnostics feature or exposing bound secrets by default.

**Verification:** Root README troubleshooting guidance reviewed.

**Remaining Work:** None; a dedicated diagnostics feature remains unnecessary unless users need structured runtime SQL tracing.
