# axumstart Repository Audit

**Reviewed:** 2026-10-12
**Repository state:** `0.1.4`, commit `71d2a2e`
**Scope:** Rust workspace, proc macros, example application, documentation site, dependencies, tests, and GitHub workflow.

## Review method and verification

I inspected all tracked source, test, configuration, documentation, migration, and workflow files. I also traced the generated-code paths instead of limiting the review to public runtime modules.

Checks performed:

- `cargo test --workspace --locked --offline` — passed, including unit tests, doctests, and trybuild UI tests.
- `cargo check -p axumstart --no-default-features --features mysql --locked` — passed.
- `cargo check -p axumstart --no-default-features --features sqlite --locked` — passed after fetching the SQLite build dependencies.
- `cargo fmt --all --check` — failed because the workspace is not formatted consistently with the checked-in `rustfmt.toml`.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` — failed at `axumstart_components/macros/src/lib.rs:317` on `clippy::collapsible_if`.
- The documentation build was not run because `docs/node_modules` is not checked out; the workflow does run `npm ci` before building.
- A temporary external SQLite probe exercised generated SQL, concurrent DI resolution, missing injection, relation filters, `_this_week`, and empty `_in` lists. It confirmed the runtime issues called out below. The probe was outside the repository and made no repository changes.

## Executive summary

The repository has a coherent split between runtime crates and proc-macro crates, and the compile-time repository DSL is well tested for syntax and type-shape errors. The immutable `Components` snapshot is a good fit for Axum state, and the three-backend feature model is clear.

The highest-risk problems are in generated database code rather than ordinary Rust code:

1. MySQL-generated repository reads, deletes, updates, and joins use PostgreSQL/SQLite-style double-quoted identifiers even though MySQL write helpers use backticks. The advertised MySQL backend is therefore not runtime-portable.
2. Several generated SQL helpers cache SQL containing a runtime `table` or `conflict_col` in a function-local `OnceLock`. Reusing one `SqlxInsert`/`SqlxUpdate` values type across repositories can send later operations to the first table used, causing wrong-table writes.
3. Relation code can produce ambiguous SELECTs and invalid `set_*_by_*` UPDATEs because base columns are not qualified and UPDATE generation does not emit the required JOIN.
4. Component cycle detection uses a provider-global mutable resolution stack across async tasks. Concurrent resolution of the same component is reported as a circular dependency even when no cycle exists.
5. The testing strategy proves macro expansion but rarely executes generated SQL or routes, so these failures currently escape CI.

There is no evidence of a checked-in production secret. SQL values are generally bound correctly. The deployment workflow does, however, accept a fresh `ssh-keyscan` result instead of pinning the deployment host key.

# 1. Current architecture

## Workspace/package layout

| Package | Responsibility |
|---|---|
| `axumstart` | Facade crate, Cargo feature selection, `.env` loading, DI/router bootstrap, and re-exports. |
| `axumstart_components` | Runtime DI provider, immutable component snapshot, `Inject<T>`, inventory collection, lifecycle hook. |
| `axumstart_components/macros` | `Component`, `ComponentConfig`, and `#[component]` expansion. |
| `axumstart_router` | Route registration record, route folding, Axum re-exports, layer factory support. |
| `axumstart_router/macros` | `#[get]`, `#[post]`, other HTTP method attributes, and `#[layer]`. |
| `axumstart_db` | Backend aliases, `Page`, repository runtime traits, dialect-specific runtime helpers. |
| `axumstart_db/macros` | `#[repository]`, `SqlxInsert`, `SqlxUpdate`, method-name DSL parsing, SQL generation, compile-time probes. |
| `examples/quickstart` | PostgreSQL example, migrations, Compose service, and a small integration-style compile/test consumer. |
| `docs` | Docusaurus documentation site. |

## Startup and request flow

`axumstart::serve` in `src/lib.rs` performs the following sequence:

1. Loads `.env` using `dotenvy`.
2. Creates a `ComponentProvider` and calls `register_all()` over global `inventory` registrations.
3. Eagerly resolves every registered blueprint into an immutable `Components` snapshot.
4. Folds every inventory-registered route onto a new `axum::Router<Components>`.
5. Attaches the snapshot as Axum state.
6. Binds `0.0.0.0:{port}` and calls `axum::serve`.

Handlers use ordinary Axum extractors. `Inject<T>` looks up a component from the immutable snapshot, while `#[layer]` factories can receive that snapshot during route construction.

## Component architecture

`ComponentProvider` stores:

- `TypeId -> InjectableFactory` blueprints;
- `TypeId -> OnceCell<Arc<dyn Any + Send + Sync>>` instances;
- a shared resolution stack for cycle detection;
- type names for diagnostics.

Factories are type-erased at runtime, but the public registration and lookup APIs remain generic. `Components` is clone-cheap and immutable after `build()`, which makes it appropriate as application state.

The derive and attribute macros submit registrations through `inventory`. `RegisterProbe` and `FactoryProbe` use stable autoderef-based specialization to select `OnCreate` and trait-object registration paths.

## Router architecture

Each route macro emits the original handler plus an `inventory::submit!` record. `register_routes` folds those records over a router in inventory order. The route macro deliberately does not own application state; it relies on `Components` as Axum state and leaves handler validation to Axum/Rust.

## Database architecture

The database API is compile-time backend-selected. `#[repository(table = "...")]` parses a trait and generates:

- a `Db{Trait}` struct containing `DbPool`;
- an implementation whose method names select query generators;
- optional transactional `_tx` methods;
- optional component registration.

The DSL is split reasonably into pure string/data planning in `dsl.rs` and `syn`-based macro expansion in `repository.rs`. `SqlxInsert` and `SqlxUpdate` derive macros add executor-generic helper methods. Postgres, MySQL, and SQLite branches are generated separately where SQL semantics differ.

## What is already well-designed

- Runtime and proc-macro responsibilities are separated into distinct crates.
- `proc-macro-crate` path resolution supports both direct subcrate consumers and the `axumstart` facade.
- Compile-time arity checks, return-shape checks, column probes, and trybuild diagnostics are valuable safety features.
- Values are bound through `sqlx`; the method-name DSL does not interpolate filter values.
- Backend exclusivity is enforced with compile errors in both runtime and proc-macro database crates.
- The relation fan-out rule is explicit and is covered by a compile-fail test.
- `Components` avoids per-request dependency construction and gives the router a stable state object.
- The documentation is unusually detailed for the macro DSL and records several known backend caveats.

# 2. Findings and recommendations

## P0 — Critical / fix before relying on the generated DB API

### P0-1 — MySQL repository SQL uses the wrong identifier quoting

- **Problem:** `axumstart_db/macros/src/repository.rs` and `dsl.rs` generate `FROM "table"`, `UPDATE "table"`, `DELETE FROM "table"`, and joined identifiers using double quotes. MySQL-specific code in `insert.rs` and `update.rs` uses backticks, which exposes the inconsistency.
- **Why it matters:** MySQL does not treat double quotes as identifier quotes in its default SQL mode. Normal repository reads, deletes, updates, and relation queries can fail at runtime even though `cargo check --features mysql` succeeds. This affects a core advertised backend.
- **Suggested solution:** Centralize identifier rendering in the dialect module, with backend-specific quoting and escaping. Use that helper for base tables, relation tables, pivot tables, columns, order clauses, and generated write SQL. Add live MySQL integration tests for every query family before changing more SQL generation.
- **Files/modules likely affected:** `axumstart_db/macros/src/dialect.rs`, `axumstart_db/macros/src/dsl.rs`, `axumstart_db/macros/src/repository.rs`, `axumstart_db/macros/src/insert.rs`, `axumstart_db/macros/src/update.rs`, backend integration tests.
- **Estimated complexity:** Medium.

### P0-2 — `OnceLock` caches SQL using runtime table/conflict arguments

- **Problem:** Generated helpers such as `SqlxInsert::__sqlx_insert_into`, `__sqlx_upsert_into`, and `SqlxUpdate::__sqlx_update_in` cache SQL in function-local `OnceLock`s, but the SQL is constructed from the `table` and sometimes `conflict_col` arguments. Those functions are shared by every repository that uses the derived values type.
- **Why it matters:** The first call determines the cached table. A temporary SQLite probe using the same `Values` type for repositories `a` and `b` inserted both rows into `a`; the same pattern caused the second update to update `a` while selecting it back. This can silently corrupt data when a values type is reused.
- **Suggested solution:** Prefer generating fixed SQL per repository method, where the table is known at macro expansion time. If the public helper API must remain table-parameterized, remove the static cache or key it by all SQL-shaping inputs. Do not cache a query containing a runtime identifier or conflict target in a single `OnceLock`.
- **Files/modules likely affected:** `axumstart_db/macros/src/insert.rs`, `axumstart_db/macros/src/update.rs`, `axumstart_db/macros/src/repository.rs`, `axumstart_db/macros/tests`.
- **Estimated complexity:** Medium.

## P1 — High-value correctness, reliability, security, and workflow improvements

### P1-1 — Relation SELECTs leave base columns unqualified

- **Problem:** `dsl::resolve_target` returns plain base columns unchanged. Once a relation JOIN exists, a method such as `find_all_by_owner_name_and_id` emits a predicate equivalent to `WHERE "owner".name = ? AND id = ?`.
- **Why it matters:** If both tables have a common column such as `id`, the query is ambiguous and fails at runtime. The temporary SQLite probe reproduced `ambiguous column name: id`. This makes otherwise valid relation methods sensitive to ordinary schema columns.
- **Suggested solution:** When a JOIN is present, qualify every base-table target as `"base_table"."column"`; qualify relation targets through the relation path. Preserve the belongs-to FK shortcut but qualify it against the base table. Add tests for `id`, `created_at`, and multiple relation predicates.
- **Files/modules likely affected:** `axumstart_db/macros/src/dsl.rs`, `axumstart_db/macros/src/repository.rs`, `axumstart_db/macros/tests/ui`, runtime SQL integration tests.
- **Estimated complexity:** Medium.

### P1-2 — `set_*_by_*` permits joined filters but emits no JOIN

- **Problem:** `gen_set` calls `build_where`, so belongs-to/has-one relation columns can pass parsing and fan-out validation. It then emits `UPDATE ... WHERE "owner".name = ?` without adding the JOIN clauses.
- **Why it matters:** The generated UPDATE fails at runtime. The temporary SQLite probe reproduced `no such column: owner.name`. The compile-time API currently promises more than the SQL generator implements.
- **Suggested solution:** Either reject all joined filters in `set_*_by_*` with a clear compile-time error, or generate backend-correct update-through-join SQL. Rejection is the smaller and safer change; callers can write an explicit method using a subquery.
- **Files/modules likely affected:** `axumstart_db/macros/src/repository.rs`, `axumstart_db/macros/src/dsl.rs`, `axumstart_db/macros/tests/ui/fail`, `docs/docs/axumstart/db/dsl/set_by.md`.
- **Estimated complexity:** Low if rejected; High if supported across all backends.

### P1-3 — Component cycle detection is shared across async tasks

- **Problem:** `ComponentProvider::resolving` is an `Arc<Mutex<Vec<...>>>` shared by all cloned providers. `resolve_by_id` holds a type on that global stack while awaiting its factory. A second task resolving the same type sees the first task’s entry and panics with `circular dependency detected: u32 -> u32`.
- **Why it matters:** Concurrent startup, background initialization, or a factory that resolves a component from a spawned task can fail nondeterministically even when the graph is acyclic. The cleanup guard also removes every stack entry for a type, so concurrent frames can interfere with one another.
- **Suggested solution:** Make cycle state resolution-local rather than provider-global. A task-local stack or an internal resolution context passed through nested factory calls is appropriate; spawned tasks should start a fresh graph context. Add concurrent resolution tests for the same type and for two independent components.
- **Files/modules likely affected:** `axumstart_components/src/lib.rs`, component tests in `axumstart_components/tests/on_create.rs`.
- **Estimated complexity:** Medium.

### P1-4 — Missing `Inject<T>` returns HTTP 200 and exposes an internal type name

- **Problem:** `axumstart_components/src/axum.rs` uses `String` as the extractor rejection. Axum converts that string into a successful response status by default. A missing component therefore returned `200 OK` in a focused request probe, with a body naming the missing Rust type.
- **Why it matters:** A server misconfiguration can look like a successful API response, breaking clients and monitoring. The type name also leaks implementation details to callers.
- **Suggested solution:** Define a small custom rejection that implements `IntoResponse`, returns `500 Internal Server Error`, logs the type name internally, and sends a generic response body. Keep the missing-component condition distinct from user/input errors.
- **Files/modules likely affected:** `axumstart_components/src/axum.rs`, component extractor tests, documentation for `Inject<T>`.
- **Estimated complexity:** Low.

### P1-5 — Empty `_in` values generate invalid or backend-dependent SQL

- **Problem:** `push_in_list` always renders parentheses. An empty vector produces `IN ()`.
- **Why it matters:** PostgreSQL rejects the syntax, while other backends may treat it differently. A normal empty filter can become a database error rather than a predictable no-match result.
- **Suggested solution:** Handle empty lists in the generated query path and emit a constant false predicate, or make the helper render a backend-safe false expression. Add tests for empty values on every supported backend.
- **Files/modules likely affected:** `axumstart_db/src/lib.rs`, `axumstart_db/macros/src/repository.rs`, `axumstart_db/macros/tests`, backend integration tests.
- **Estimated complexity:** Low.

### P1-6 — SQLite `_this_week` is wrong on Mondays

- **Problem:** `axumstart_db/src/lib.rs:41` defines SQLite’s boundary as `date(CURRENT_TIMESTAMP, 'weekday 1', '-7 days')`. On a Monday, the expression resolves to the previous Monday.
- **Why it matters:** A weekly query silently excludes the current Monday-through-now data at the exact start of the week. A focused SQLite probe evaluated the boundary for Monday `2026-10-12` as `2026-10-05`.
- **Suggested solution:** Replace the expression with a tested Monday-start calculation that handles Monday and Sunday explicitly. Add boundary tests for Monday, Sunday, UTC/local-time assumptions, and TEXT versus INTEGER timestamp storage.
- **Files/modules likely affected:** `axumstart_db/src/lib.rs`, `axumstart_db/macros/tests`, `docs/docs/axumstart/db/dsl/find_all.md`, `axumstart_db/README.md`.
- **Estimated complexity:** Low to Medium.

### P1-7 — `_this_week` is appended without grouping

- **Problem:** `dsl::parse_conditions` intentionally emits a flat connector chain. `gen_select_all` then appends the week predicate. A method like `find_all_by_id_or_name_this_week` becomes effectively `id = ? OR name = ? AND created_at >= week_start`.
- **Why it matters:** SQL precedence makes this `(id = ?) OR (name = ? AND week condition)`, so an old row matching `id` is returned. The focused SQLite probe reproduced that behavior. The documentation describes the flat connector rule, but the modifier still creates an easy-to-miss semantic trap.
- **Suggested solution:** Parenthesize the complete user filter before applying `_this_week`, or reject `_this_week` combined with `_or_` until the DSL has explicit grouping. Add a regression test that inserts an old row matching the first OR branch.
- **Files/modules likely affected:** `axumstart_db/macros/src/repository.rs`, `axumstart_db/macros/src/dsl.rs`, `axumstart_db/macros/tests`, DSL documentation.
- **Estimated complexity:** Low if parentheses are the chosen semantics; Medium if the grammar changes.

### P1-8 — MySQL row-returning helpers have unsafe select-back assumptions

- **Problem:** MySQL has no `RETURNING`, so generated code performs a second SELECT. `insert_all` selects by an auto-increment range, and single-row insert uses `last_insert_id()` even when `#[key(...)]` names a custom key. The docs acknowledge the batch assumption, but the generated API still returns ordinary row types as if the operation were general.
- **Why it matters:** A non-auto-increment key or a schema that does not guarantee contiguous allocation can return the wrong rows or no rows. This is a data-integrity problem, not merely a performance difference.
- **Suggested solution:** Restrict these helpers at compile time to the documented auto-increment shape, or capture/select the actual key values before the write when the values struct contains them. Make the limitation part of the type/API contract rather than a runtime caveat.
- **Files/modules likely affected:** `axumstart_db/macros/src/insert.rs`, `axumstart_db/macros/src/update.rs`, insert/upsert docs, MySQL integration tests.
- **Estimated complexity:** Medium to High.

### P1-9 — Rust CI does not test the library

- **Problem:** `.github/workflows/deploy-docs.yml` is the only workflow. It only runs for documentation changes and has no Rust formatting, lint, test, backend, or dependency checks.
- **Why it matters:** Changes to the core crates can merge without compiling, while trybuild-only tests leave runtime SQL and router behavior untested. The current local checks already show formatting and `-D warnings` failures that CI would not catch.
- **Suggested solution:** Add a Rust workflow for `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --locked`. Add a backend matrix that at minimum compiles all features and runs SQLite integration tests; use service containers for PostgreSQL/MySQL query execution where practical.
- **Files/modules likely affected:** `.github/workflows/`, new integration test crates or `axumstart_db/tests`, workspace Cargo configuration.
- **Estimated complexity:** Medium.

### P1-10 — Deployment SSH host verification is vulnerable to key substitution

- **Problem:** `.github/workflows/deploy-docs.yml:41` runs `ssh-keyscan` during the deployment job and appends whatever key is returned to `known_hosts`.
- **Why it matters:** A network attacker able to interfere with that lookup can cause the workflow to trust an attacker-controlled host key and send the deployment over the wrong SSH endpoint.
- **Suggested solution:** Store a reviewed, pinned `known_hosts` entry or host-key fingerprint as a protected secret, write it to `~/.ssh/known_hosts`, and use strict host-key checking. Keep the existing private-key secret handling and avoid dynamic trust-on-first-use.
- **Files/modules likely affected:** `.github/workflows/deploy-docs.yml`, repository deployment secrets/documentation.
- **Estimated complexity:** Low.

### P1-11 — Component construction failures are represented as panics

- **Problem:** `ComponentBlueprint::new`, `OnCreate`, and `ComponentProvider::build` have no error channel. `ComponentConfig` panics on missing/invalid environment values, and factory/database initialization errors are commonly expressed with `expect` or panic. `serve` only returns listener/serve I/O errors.
- **Why it matters:** Invalid production configuration produces an uncontrolled panic instead of a startup error with a stable diagnostic path. It is harder to test, observe, and integrate with supervisors.
- **Suggested solution:** Preserve the existing infallible API for compatibility but add a fallible registration/build path, for example `build_result() -> Result<Components, ComponentError>` and fallible factory/config traits. Update `serve` to propagate initialization failures. Avoid printing secret values in parse diagnostics.
- **Files/modules likely affected:** `axumstart_components/src/lib.rs`, component macros, `src/lib.rs`, config/component docs, tests.
- **Estimated complexity:** High because it expands public traits; do after runtime regressions are covered.

## P2 — Useful improvements

### P2-1 — Make `axumstart`’s direct runtime dependencies match its features

- **Problem:** The facade declares `axum`, `tokio`, and `dotenvy` as unconditional dependencies even though they are only used by the router/`serve` path. `default-features = false` still pulls them into a consumer that only wants a narrower feature set.
- **Why it matters:** It increases compile time and dependency surface, undermining the documented “pick the pieces you need” feature model.
- **Suggested solution:** Make the direct dependencies optional and include them in the `router` feature. Keep the subcrate dependencies unchanged where their public APIs require Axum/Tokio. Verify `cargo tree -e features` for `default-features = false` after the change.
- **Files/modules likely affected:** root `Cargo.toml`, root `src/lib.rs`, `Cargo.lock`.
- **Estimated complexity:** Low.

### P2-2 — Validate `Page` inputs and arithmetic

- **Problem:** `Page::new` accepts negative limits/offsets, and `Page::number` computes `page * size` without overflow checking.
- **Why it matters:** Bad pagination reaches the database as an error or backend-specific behavior; large values can panic in debug builds or wrap in release builds.
- **Suggested solution:** Add a fallible constructor or validation method using checked multiplication and non-negative bounds. Keep the current constructor only if compatibility requires it, and document that generated queries should not receive unchecked external pagination values.
- **Files/modules likely affected:** `axumstart_db/src/lib.rs`, repository DSL tests/docs.
- **Estimated complexity:** Low.

### P2-3 — Enforce relation invariants instead of trusting annotations

- **Problem:** `has_one` is treated as never fan-out, but the macro cannot verify a unique constraint on the target foreign key. The quickstart `profile.user_id` migration is not unique despite the repository declaring `has_one`. Duplicate relation attributes targeting the same table are also accepted and resolved by first-match order.
- **Why it matters:** `first_by`, `count_by`, and `exists_by` can return arbitrary or inflated results when the schema does not match the annotation.
- **Suggested solution:** Add a migration constraint for the example and document that `has_one` requires a database uniqueness constraint. Reject duplicate relation target names at macro expansion. Consider requiring an explicit `#[unchecked_relation]` escape hatch if schema introspection is never added.
- **Files/modules likely affected:** `examples/quickstart/migrations/0002_create_profile.sql`, `axumstart_db/macros/src/repository.rs`, relation docs and UI tests.
- **Estimated complexity:** Low to Medium.

### P2-4 — Make the server bootstrap configurable

- **Problem:** `serve(port)` hard-codes `0.0.0.0`, creates only the auto-registered router, and offers no graceful-shutdown hook or global middleware hook.
- **Why it matters:** Real deployments often need a configured bind address, a manually merged route, global tracing/CORS/timeout layers, and coordinated shutdown of background tasks. The docs currently require users to duplicate bootstrap code for this.
- **Suggested solution:** Add a small `build_router()`/`build_app()` API and a configurable server/builder entry point while retaining `serve(port)` as the convenience wrapper. Add a shutdown hook only if `OnCreate`-spawned tasks are expected to be managed by the framework.
- **Files/modules likely affected:** `src/lib.rs`, `axumstart_router/src/lib.rs`, router docs, quickstart.
- **Estimated complexity:** Medium.

### P2-5 — Qualify and validate all macro-supplied identifiers

- **Problem:** Table, relation, column, order, and pivot names are interpolated into SQL without identifier escaping. They normally originate from source code, but the generated public helpers still accept table/conflict strings, and unusual identifiers/reserved words are not handled consistently.
- **Why it matters:** It creates an avoidable injection/API-misuse boundary and makes portability depend on backend SQL modes. Values are safely bound, but identifiers cannot be bound and need separate handling.
- **Suggested solution:** Restrict generated low-level helpers to compile-time constants where possible; otherwise validate identifiers against a conservative grammar and quote them through the dialect helper. Add compile-fail tests for invalid macro identifiers and runtime tests for reserved names such as `user` and `order`.
- **Files/modules likely affected:** `axumstart_db/macros/src/dialect.rs`, `dsl.rs`, `repository.rs`, `insert.rs`, `update.rs`.
- **Estimated complexity:** Medium.

### P2-6 — Add runtime router and generated-SQL tests, not only expansion tests

- **Problem:** The database trybuild suite verifies that code compiles or fails with expected diagnostics, but does not execute generated SQL. The router suite similarly checks macro expansion but does not call `register_routes` or send a request.
- **Why it matters:** Identifier quoting, static SQL caching, empty lists, JOIN shape, HTTP status, and route collisions are runtime behaviors that the current suite cannot observe.
- **Suggested solution:** Add a small SQLite integration suite for SELECT/INSERT/UPDATE/DELETE, relations, pagination, `_in`, `_this_week`, and generated write helpers. Add Axum `oneshot` tests for registered routes, layers, missing injection, duplicate paths, and `cfg`-gated handlers. Add PostgreSQL/MySQL service-backed tests for dialect-specific behavior.
- **Files/modules likely affected:** `axumstart_db/tests/`, `axumstart_router/tests/`, `axumstart_components/tests/`, `.github/workflows/`.
- **Estimated complexity:** Medium to High.

### P2-7 — Establish one formatting/lint policy

- **Problem:** `cargo fmt --all --check` currently reports broad differences, and strict Clippy fails on `collapsible_if` in the component macro crate. The repository has a custom `rustfmt.toml` but no CI gate.
- **Why it matters:** Contributors cannot tell which style is canonical, and warnings can accumulate until a future toolchain makes them build blockers.
- **Suggested solution:** Decide whether the project wants two-space indentation or standard rustfmt output, format the workspace once, fix the current Clippy lint, and add both checks to CI. Avoid mixing manually formatted proc-macro files with a different workspace policy.
- **Files/modules likely affected:** `rustfmt.toml`, all Rust sources after the one-time formatting pass, `axumstart_components/macros/src/lib.rs`, CI workflow.
- **Estimated complexity:** Low to Medium.

### P2-8 — Improve the contributor and release workflow

- **Problem:** There is no root README, CONTRIBUTING guide, changelog/release procedure, Rust toolchain/MSRV declaration, or automated dependency/security check. The docs README still describes GitHub Pages deployment while the workflow deploys to an SSH server.
- **Why it matters:** New users must infer the correct commands from comments and subcrate docs, and maintainers have no recorded compatibility target or repeatable release checklist.
- **Suggested solution:** Add a short root README with the facade quick start, feature/backend matrix, test commands, and link to docs. Declare a supported Rust version and test it in CI. Update deployment documentation to match the actual workflow. Add a periodic dependency/security audit as a maintenance job rather than a build-time runtime dependency.
- **Files/modules likely affected:** new `README.md`, new `CONTRIBUTING.md` or release docs, workspace `Cargo.toml`, `docs/README.md`, `.github/workflows/`.
- **Estimated complexity:** Low to Medium.

### P2-9 — Split the database macro generator only along behavior boundaries

- **Problem:** `axumstart_db/macros/src/repository.rs` is about 1,409 lines and owns attribute parsing, DSL dispatch, return-type analysis, probes, and SQL generation. `insert.rs` and `update.rs` duplicate large backend-specific generated blocks.
- **Why it matters:** The current structure makes it easy to fix one dialect/path while missing a sibling path, as shown by the quoting and cache inconsistencies. It also raises the cost of adding a supported backend.
- **Suggested solution:** Do not rewrite the DSL. First extract small, testable layers: parsed repository metadata, query plan, dialect rendering, and token emission. Share common insert/update plan logic while retaining separate dialect renderers. Keep the public generated API stable.
- **Files/modules likely affected:** `axumstart_db/macros/src/repository.rs`, `dsl.rs`, `dialect.rs`, `insert.rs`, `update.rs`.
- **Estimated complexity:** High; schedule only after P0/P1 regression coverage exists.

## P3 — Optional / nice-to-have improvements

### P3-1 — Reconsider `parking_lot` for short synchronous locks

- **Problem:** `axumstart_components` uses `parking_lot::Mutex` only for short, non-awaiting critical sections. The standard library mutex may be sufficient.
- **Why it matters:** Removing it slightly reduces the dependency surface and aligns with the library’s small-runtime goal, but it is not a measurable application bottleneck.
- **Suggested solution:** Benchmark or simply trial `std::sync::Mutex`; keep `parking_lot` if initialization contention or poisoning semantics are a deliberate requirement. Do not prioritize this ahead of correctness and integration tests.
- **Files/modules likely affected:** `axumstart_components/src/lib.rs`, component tests, workspace dependency declarations.
- **Estimated complexity:** Low.

### P3-2 — Document the deliberate tradeoffs of `async-trait` and inventory

- **Problem:** `async-trait` and `inventory` add compile-time/runtime machinery. They are currently justified by object-safe generated repository traits and automatic registration, but the tradeoff is not summarized in one architectural document.
- **Why it matters:** Future maintainers may attempt an unsafe or breaking removal without understanding why they exist.
- **Suggested solution:** Add a short architecture/FAQ section explaining that native async traits do not replace `async-trait` for the current `Arc<dyn Repository>` API, and that `inventory` is what enables zero-boilerplate registration. Revisit only as a deliberate API-version decision.
- **Files/modules likely affected:** root README/docs, `axumstart_components`, `axumstart_db`, `axumstart_router` documentation.
- **Estimated complexity:** Low.

### P3-3 — Offer optional query diagnostics for generated repositories

- **Problem:** Generated SQL is hidden in macro output and database errors are the primary feedback when a query shape is wrong.
- **Why it matters:** Maintainers and users would benefit from seeing the generated SQL and method metadata during development, especially for relation and dialect issues.
- **Suggested solution:** Consider a compile-time opt-in expansion/debug mode or a documented `cargo expand` workflow. If runtime query tracing is added, integrate with `tracing` without logging bound secrets by default.
- **Files/modules likely affected:** database macro docs, `repository.rs`, optional diagnostics support.
- **Estimated complexity:** Medium.

# 3. Code quality, separation of concerns, and maintainability

## Main architectural concerns

1. **The facade does too much at startup.** `serve` loads environment, builds every component, constructs the router, binds the socket, and owns the server loop. It is convenient, but it makes initialization errors hard to return and customization require copying the sequence.
2. **The DI provider mixes registration, graph resolution, cache ownership, and concurrency diagnostics.** The first three are coherent; the shared resolution stack is the unsafe coupling point.
3. **The database macro mixes a compile-time fixed concept with runtime parameters.** Repository table names are known to `#[repository]`, but generated value helpers accept a runtime table name and cache it globally per generated function. This is the direct cause of P0-2.
4. **Relation planning and SQL rendering are partially separated, but not fully.** `dsl.rs` plans relation targets while `repository.rs` decides when to emit joins. `gen_set` bypasses the join emission path, which creates the P1-2 failure.
5. **Global inventory is convenient but hidden.** Registration collisions, duplicate route paths, and test interactions are discovered late. The existing manual registration APIs are useful and should remain available as an explicit mode.

## Code quality observations

- `repository.rs` has a clear top-level dispatch (`gen_body`) and useful helper names, but it is too large for safe cross-dialect maintenance.
- The `RegisterProbe`/`FactoryProbe` machinery is complex, but it is purposeful: stable Rust cannot directly inspect whether a user type implements optional lifecycle/trait-object bounds during macro expansion. This is not a good candidate for a cosmetic rewrite.
- `itertools_unzip3` is a local helper for a single call site. It is harmless, but could be replaced with three pushes in the existing loop if the macro file is already being simplified.
- `ComponentProvider::build` and generated component factories use panics for missing registrations and bad configuration. This is consistent but limits library composability.
- Error messages generally name the method and expected DSL, which is good. Runtime errors need the same care: internal type names should not be sent as HTTP bodies.
- The quickstart is useful and documents behavior well, but it is a scratchpad rather than a deterministic live integration test. Its migration schema should match its declared relation cardinalities.
- There is no obvious large dead-code subsystem. The duplicated `krate.rs` files are small and understandable; extracting a shared internal crate would add more workspace machinery than value unless macro path handling changes often.

# 4. Performance review

## Reasonable current choices

- Components are constructed once and stored in `OnceCell`; request handlers retrieve `Arc`s from an immutable snapshot.
- Generated scalar queries use static SQL strings or compile-time SQL assembly in most paths, avoiding repeated query-shape construction.
- Batch writes use `QueryBuilder::push_values` rather than issuing one request per row.
- `_in` uses bound values rather than interpolating list contents.

## Performance concerns worth acting on

- MySQL row-returning writes necessarily use a second SELECT because the backend lacks `RETURNING`; this is documented and should remain an explicit tradeoff until a better backend-specific strategy exists.
- `find_random_by_*` uses `ORDER BY RANDOM()`/`RAND()`, which can scan and sort a large candidate set. This is acceptable for a convenience method but should be documented as unsuitable for high-volume tables; callers needing scale should write a domain-specific random-key query.
- `build()` eagerly resolves every inventory component sequentially. That is a good fail-fast startup default for a small starter, but large applications may eventually benefit from an explicit lazy/selected build mode. This is not worth adding before correctness work.
- SQL string caching currently optimizes allocations at the cost of correctness. Correct table selection must take precedence; a database round trip dominates the small formatting cost if the fixed per-repository strategy is not available.

# 5. Testing review

## Existing coverage

- `axumstart_components` covers basic resolution, derived components, defaults, inventory registration, `OnCreate`, and a single-threaded cycle panic.
- `axumstart_db/macros` has 13 pure DSL unit tests plus trybuild pass/fail cases for method dispatch, arity, columns, relations, pagination, operations, and diagnostics.
- `axumstart_router/macros` has one pass fixture and one compile-fail fixture.
- The quickstart verifies facade macro resolution and component registration but does not execute database operations under `cargo test`.
- Doctests and workspace tests pass locally.

## Important missing tests

### Database runtime/regression tests

- MySQL identifier quoting for `find_all`, `first_by`, `delete_by`, `set_by`, and relation joins.
- Reuse of one `SqlxInsert`/`SqlxUpdate` values type across two repository table names.
- Different `#[unique(...)]` conflict columns using one values type.
- Empty `_in` for SELECT, DELETE, and UPDATE paths.
- Base `id` plus a joined relation predicate.
- `set_*_by_*` relation filters, which should currently fail at macro expansion if support is not implemented.
- `_this_week` at Monday and Sunday on all timestamp storage formats supported by the docs.
- OR filters combined with `_this_week`.
- MySQL custom keys and batch row-returning insert assumptions.
- `Page` negative and overflow inputs.

### Component tests

- Concurrent `get::<T>()` calls while a factory is blocked.
- Concurrent independent component graphs.
- Duplicate registrations for the same `TypeId` and deterministic behavior.
- Factory/config failure propagation once a fallible API exists.

### Router/HTTP tests

- Call `register_routes` and send requests through `Router::oneshot`.
- Verify each method attribute and per-route layer.
- Verify `Inject<T>` missing-component status and response body.
- Verify `cfg`-disabled handlers do not leave an inventory record referencing a removed function.
- Verify duplicate path/method behavior is intentional and produces a controlled startup error.

# 6. Feature roadmap

## Essential features

1. **Backend-correct generated SQL with executable integration coverage** — required for the documented PostgreSQL/MySQL/SQLite promise.
2. **Reliable component initialization errors** — a fallible build path and a production-safe injection rejection improve startup and request reliability.
3. **Configurable application bootstrap** — bind address, router composition, global layers, and shutdown integration are the natural next step beyond the convenience `serve` function.
4. **Regression-tested pagination and filter semantics** — especially empty lists, relation filters, `_this_week`, and connector grouping.
5. **CI for Rust crates and backend feature combinations** — the project needs a gate that runs whenever library code changes.

## Optional / nice-to-have features

1. **Explicit component scopes or qualifiers** for applications needing multiple instances of the same concrete type.
2. **Registration collision diagnostics** for duplicate component types and route paths.
3. **Generated SQL/debug expansion support** for macro users troubleshooting a query.
4. **Lifecycle shutdown support** for `OnCreate` background workers.
5. **A documented scalable random-selection pattern** for tables where `ORDER BY RANDOM()` is too expensive.

# 7. Developer experience and dependencies

## Current workflow assessment

The Rust workspace can be built and tested from the root, and the quickstart includes a Compose database plus migrations. The docs have a reproducible `npm ci`/`npm run build` workflow in deployment. The missing pieces are discoverability and automated gates:

- No root README explains the facade, feature combinations, or the shortest test command.
- No Rust CI runs on Rust changes.
- Formatting and strict Clippy currently fail locally.
- No declared MSRV/toolchain policy.
- Docs instructions describe both GitHub Pages deployment and the actual SSH deployment workflow without reconciling them.

## Dependency assessment

| Dependency | Assessment |
|---|---|
| `axum`, `tokio`, `dotenvy` in root facade | Needed by `serve`, but should be optional behind `router`. |
| `axumstart_components`, `axumstart_router`, `axumstart_db` | Correctly optional facade dependencies. |
| `sqlx` | Heavy but central to the DB crate; backend feature selection is appropriate. |
| `inventory` | Core to automatic component and route registration; not redundant. |
| `proc-macro-crate`, `syn`, `quote`, `proc-macro2` | Appropriate proc-macro dependencies. |
| `trybuild` | Appropriate dev dependency for compile-time API diagnostics. |
| `async-trait` | Still justified for the current object-safe generated repository traits; do not remove as a cosmetic optimization. |
| `parking_lot` | Probably replaceable with `std::sync::Mutex`, but the benefit is small and not a priority. |
| Docusaurus/React packages | Appropriate for the documentation site; no obvious redundant application dependency from static inspection. |

No dependency appeared obviously abandoned or gratuitous from the checked-in manifests and lockfile alone. The more meaningful dependency simplification is feature-aligning the facade’s unconditional runtime dependencies.

# 8. Security and reliability summary

## Positive findings

- `.env` is ignored by Git.
- The tracked Compose password is clearly a local development value, not a production secret.
- SQL filter values are bound through SQLx rather than interpolated.
- Compile-time SQL identifiers come from macro source, not ordinary request input, so the normal DSL path is not directly request-parameter SQL injection.
- No credential or private key was found in tracked files.

## Risks

- Dynamic identifier interpolation in public generated helper methods is an API-hardening concern; see P2-5.
- The deployment workflow trusts a live `ssh-keyscan`; see P1-10.
- Missing DI dependencies produce successful HTTP responses; see P1-4.
- Initialization failures panic rather than return structured errors; see P1-11.
- MySQL select-back assumptions can return incorrect rows; see P1-8.
- Hard-coded `0.0.0.0` exposes the listener on all interfaces; this is documented and may be intentional for a starter, but a configurable bind address should be available.

# Current Architecture

`axumstart` is a facade over three independently usable subsystems. Components and routes are registered at link time through `inventory`. At startup, the provider resolves all component blueprints into an immutable `Components` map. Auto-registered Axum handlers share that map as router state and use `Inject<T>` for dependencies. The database crate uses proc macros to turn repository trait method names into SQLx implementations selected for exactly one compile-time backend. Docusaurus documents the public macros and a PostgreSQL quickstart demonstrates the consumer-facing facade.

# Key Problems

1. The generated database SQL is not consistently dialect-aware; MySQL uses incompatible identifier quoting in the repository path.
2. Generated write helpers cache table-dependent SQL globally per generated function, allowing wrong-table operations when values types are reused.
3. Relation SQL planning is incomplete for qualified base columns and `set_*` joins.
4. DI cycle detection is incorrectly global across concurrent async resolutions.
5. Runtime generated SQL and route behavior are largely absent from tests.
6. Startup and injection failures use panics or HTTP 200/string rejection semantics instead of structured errors.
7. Rust CI, formatting/lint enforcement, and a clear contributor/release workflow are missing.

# Refactoring Roadmap

1. **Add regression harnesses first.** Start with SQLite runtime tests for generated queries, an Axum `oneshot` route test, and concurrent DI tests. Add MySQL/Postgres service-backed tests for dialect-specific paths.
2. **Fix database safety boundaries.** Introduce dialect-specific identifier rendering and remove or correctly key table-dependent `OnceLock` caches. Keep public method signatures stable where possible.
3. **Fix query semantics.** Qualify base columns when joins are present, reject unsupported joined `set_*` filters, define empty `_in` behavior, and correct/test SQLite week boundaries and `_this_week` grouping.
4. **Constrain MySQL select-back behavior.** Either generate key-based select-back from actual values or make unsupported schema shapes compile-time errors.
5. **Fix component/request errors.** Make resolution state task-local and change `Inject<T>` to a controlled 500 rejection. Then design a fallible provider/build API without mixing it into the first correctness patch.
6. **Add CI gates.** Run formatting, Clippy, workspace tests, backend feature checks, and docs build on pull requests. Pin deployment host keys separately.
7. **Improve the public workflow.** Add a root README, MSRV policy, accurate deployment docs, and a configurable router/server builder while retaining the simple `serve(port)` wrapper.
8. **Refactor macro internals last.** Once behavior is protected, split query planning/rendering/token emission along stable boundaries and remove only abstractions that reduce duplication without changing the public DSL.

# Feature Roadmap

## Priority order

1. Correct all backend-generated SQL and add live integration coverage.
2. Make DI and HTTP failures observable and correctly classified.
3. Add configurable bootstrap, global middleware composition, and graceful shutdown hooks where background tasks need them.
4. Add duplicate registration/path diagnostics and, if demand exists, component qualifiers/scopes.
5. Add optional generated-SQL diagnostics and a documented scalable alternative to random full-table sorting.

# Quick Wins

- Add the Rust CI workflow with `cargo fmt`, Clippy, workspace tests, and feature checks.
- Fix the single current Clippy lint and make the formatting policy consistent.
- Change `Inject<T>` to return a generic 500 response instead of a 200 string response.
- Make root `axum`, `tokio`, and `dotenvy` optional behind `router`.
- Make empty `_in` a deterministic no-match condition.
- Correct the SQLite Monday boundary and add a small regression test.
- Replace dynamic `ssh-keyscan` trust with a pinned `known_hosts` entry.
- Add a root README containing the existing quickstart and verification commands.

# Long-Term Improvements

- Introduce a fallible component graph/bootstrap API with structured startup errors.
- Replace the shared DI resolution stack with task-local or explicitly propagated resolution context.
- Generate repository-specific fixed SQL, or otherwise separate compile-time table metadata from reusable values helpers.
- Split the database macro generator into metadata parsing, query planning, dialect rendering, and token emission.
- Add backend service integration tests and migration fixtures for PostgreSQL, MySQL, and SQLite.
- Add lifecycle/shutdown management for background tasks started by `OnCreate`.
- Consider explicit component scopes/qualifiers if one concrete type must be registered more than once.

# Recommended Next Step

**Create a database code-generation regression test suite and fix P0-1/P0-2 together.**

This is the single highest-value next task because it addresses both a documented backend promise and a confirmed wrong-table write path. Start with SQLite tests that can run cheaply in every pull request, add a MySQL service test for identifier quoting, and then change the dialect renderer and SQL-cache design. Until those tests exist, refactoring the large macro file or adding new DSL features risks extending the same unverified failure surface.
