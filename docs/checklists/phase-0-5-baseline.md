# Phase 0.5 DDD Refactor Baseline Checklist

Date: 2026-06-05
Branch: `refactor/ddd-phase-0-5`
Worktree: `D:\Projects\Eddie\stock_crawler_ddd_phase_0_5`
Base commit: `ee9ed0c feat(cache): add price validation to filter out abnormal stock prices exceeding 10.5% difference from last close`

## Gate Results

- [x] `cargo --version`: `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`
- [x] `rustc --version`: `rustc 1.95.0 (59807616e 2026-04-14)`
- [x] `cargo install cargo-llvm-cov`: installed `cargo-llvm-cov 0.8.7`
- [x] `cargo check`: passed
- [x] `cargo build`: passed
- [x] `cargo test`: passed, `206 passed; 0 failed; 139 ignored`
- [x] `cargo llvm-cov --all-features --workspace --html`: passed

Coverage report:

- HTML: `target/llvm-cov/html/index.html`
- Total region coverage: `41.19%`
- Total function execution: `40.29%`
- Total line coverage: `36.09%`
- Branch coverage: not reported by current run

## Current Gate Decision

- [x] Phase 0.5 is complete for the agreed pre-refactor baseline scope.
- [x] Phase 1 may start only from a separate worktree after this baseline is reviewed; use `git worktree add ../stock_crawler_ddd_refactor -b refactor/ddd-phase-1`.
- [x] The `139 ignored` tests are categorized by file, exemption reason, risk, and replacement validation in `docs/checklists/phase-0-5-ignored-tests-exemptions.md`.

## Priority 1: Pure Logic Unit Tests

Focus these first because they can be tested without PostgreSQL, Redis, network calls, scheduler jobs, or external services.

- [x] `src/core/declare.rs`
  - Current line coverage: `98.21%`
  - Added iterator completeness, market exchange mapping, industry uniqueness, and invalid serial coverage.
- [x] `src/core/util/text.rs`
  - Current line coverage: `100.00%`
  - Replaced print-only checks with assertions and added parser success/error, truncate, split, and escape cleanup coverage.
- [x] `src/app/calculation/dividend_record.rs`
  - Current line coverage: `31.03%`
  - Added characterization cases for date eligibility, cash-only, stock-only, invalid dates, and zero-share outcomes.
- [x] `src/app/event/trace/stats.rs`
  - Current line coverage: `96.46%`
  - Added reset, accumulation, zero-denominator rate, activity detection, and non-resetting snapshot coverage.
- [x] `src/app/event/trace/stock_price.rs`
  - Current line coverage: `51.78%`
  - Added tests for target cache diagnostics, grouped symbols, boundary keys, rounding behavior, and cache query behavior.
- [x] `src/infra/cache/ttl.rs`
  - Current line coverage: `97.67%`
  - Added daily/trace expiry, previous-value, and invalidation characterization.
- [x] `src/infra/cache/share.rs`
  - Current line coverage: `78.62%`
  - Added pure cache set/replace/lookups, static lookup fallback, current IP, stock snapshot validation, and outlier filtering coverage. `Share::load()` remains an integration path.

## Priority 2: Parser and Mapper Characterization

These protect behavior that Phase 1 and Phase 2 are likely to move behind domain or repository boundaries.

- [x] `src/infra/crawler/yahoo/dividend.rs`
  - Current line coverage: `90.86%`
  - Added malformed row, missing columns, half-year labels, paid-year grouping, malformed numeric, and ordering cases.
- [x] `src/infra/crawler/mops/annual_profit.rs`
  - Current line coverage: `61.65%`
  - Added financial report conversion edge cases, malformed x-axis/year parsing, fallback body values, preferred net-profit source, and formula boundary cases.
- [x] `src/infra/crawler/yahoo/price/class_quote.rs`
  - Current line coverage: `48.96%`
  - Added URL/category key generation, suffix stripping, decimal parsing, missing symbol, malformed decimal, special missing value, and pagination cases.
- [x] `src/infra/crawler/histock/price.rs`
  - Current line coverage: `53.48%`
  - Added row parser, non-data row skip, positive/negative/no-change parsing, malformed numeric errors, diagnostics, and changed-price collection characterization.
- [x] `src/infra/database/table/quote/daily_quote/mod.rs`
  - Current line coverage: `29.60%`
  - Added tests for mapping, CSV output, exchange DTO conversion, defaulting, signed change handling, and key behavior. SQL and copy-in paths remain integration paths.
- [x] `src/infra/database/table/stock/mod.rs`
  - Current line coverage: `40.25%`
  - Added tests for preference share detection, TDR detection, key behavior, clone, stock info request, and crawler DTO conversion.

## I/O Exemption Categories To Finalize

Each ignored or low-coverage external I/O area must keep a concrete exemption note before Phase 0.5 can close.

- [x] PostgreSQL / SQLx table operations under `src/infra/database/table/**`
  - Risk: repository and DB row behavior can regress during Phase 2.
  - Replacement validation: mapper/unit tests plus targeted ignored integration tests.
- [x] Redis wrapper and Redis-backed flows under `src/infra/nosql/redis.rs` and trace/backfill modules.
  - Risk: cache key behavior and idempotency can drift.
  - Replacement validation: fake or in-memory cache contract tests.
- [x] External HTTP crawler modules under `src/infra/crawler/**`
  - Risk: parser behavior and upstream response changes are mixed with network availability.
  - Replacement validation: parser fixtures and ignored live smoke tests.
- [x] Scheduler, background tasks, web, gRPC, Telegram, and manual backfill entrypoints.
  - Risk: orchestration regressions may not appear in pure unit tests.
  - Replacement validation: fake handlers, message formatting tests, and targeted smoke tests.

## Agent Assignments

- Tooling agent: verified Rust toolchain, cargo gates, cargo metadata, and the initial `cargo-llvm-cov` gap.
- Test inventory agent: identified pure logic, parser/mapper, and external I/O boundaries for Phase 0.5 test work.
- Main agent: created the Phase 0.5 worktree, installed `cargo-llvm-cov`, ran the baseline gates, and recorded this checklist.
- Yahoo dividend parser agent: added Yahoo dividend parser characterization tests and verified `cargo test infra::crawler::yahoo::dividend::tests`.
- MOPS annual profit parser agent: added annual profit parser/formula characterization tests and verified `cargo test infra::crawler::mops::annual_profit::tests`.

## Final Phase 0.5 Gate

- [x] `cargo fmt`
- [x] `cargo check`
- [x] `cargo build`
- [x] `cargo test`: `206 passed; 0 failed; 139 ignored`
- [x] `cargo llvm-cov --all-features --workspace --html`: passed
- [x] `cargo llvm-cov report --summary-only`: total line coverage `36.09%`
