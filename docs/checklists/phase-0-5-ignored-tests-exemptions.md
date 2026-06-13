# Phase 0.5 Ignored Tests Exemption Inventory

Date: 2026-06-05
Branch: `refactor/ddd-phase-0-5`

This inventory records the ignored tests that are excluded from the Phase 0.5 hard unit-test gate. The current baseline has `139 ignored` tests. Phase 1 must not start until each category below either has focused unit coverage around its pure logic or remains explicitly exempted with replacement validation.

## Exemption Categories

- `DB`: requires PostgreSQL, SQLx transactions, persisted table state, or generated search index side effects.
- `Redis`: requires Redis or cache keys with external state.
- `HTTP`: calls external websites or public APIs.
- `Scheduler`: requires time, market-session loops, background tasks, or long-running runtime state.
- `Messaging`: sends Telegram, gRPC, or web/service calls.
- `Filesystem`: depends on log rotation files or filesystem timing.
- `Manual`: manual backfill smoke test; intended for operator-triggered verification.

## Replacement Validation Policy

- Pure transformations inside an exempted module still need unit tests or characterization tests.
- Live tests remain `#[ignore]` and should be run only as targeted smoke or integration checks with required services available.
- DB and Redis behavior should be protected by mapper tests, fake/in-memory contract tests, or narrow integration tests.
- HTTP crawler behavior should be protected by parser fixtures and separate ignored live smoke tests.

## Inventory By File

| Count | File | Category | Risk | Replacement validation |
| ---: | --- | --- | --- | --- |
| 1 | `src/app/backfill/delisted_company.rs` | DB/HTTP | Backfill orchestration can drift after repository extraction. | Add parser/use-case unit tests; keep live backfill smoke ignored. |
| 3 | `src/app/backfill/dividend/missing_or_multiple.rs` | DB/HTTP/Redis | Dividend backfill can duplicate or miss historical records. | Keep existing key/mapper unit tests; add fake repository cases before Phase 1 closes. |
| 1 | `src/app/backfill/dividend/payout_ratio.rs` | DB/Redis/HTTP | Payout ratio updates can regress silently. | Add pure calculation/mapper tests; run ignored smoke with services. |
| 1 | `src/app/backfill/dividend/unannounced_ex_dividend_date.rs` | DB/Redis/HTTP | Date-change detection can miss announcement updates. | Existing comparison tests cover core; keep live smoke ignored. |
| 1 | `src/app/backfill/etf.rs` | DB/HTTP | ETF import can write wrong market metadata. | Add DTO-to-row mapper tests before Phase 1 closes. |
| 1 | `src/app/backfill/financial_statement/annual.rs` | DB/HTTP | Annual financial backfill can miswrite EPS/ROE/ROA. | Add parser/mapper tests around crawler outputs. |
| 1 | `src/app/backfill/financial_statement/mod.rs` | DB/HTTP | Shared financial orchestration can miss zero-value updates. | Keep ignored smoke; add use-case tests with fake repositories. |
| 1 | `src/app/backfill/financial_statement/quarter.rs` | DB/HTTP | Quarterly statement import can drift. | Add parser/mapper characterization. |
| 1 | `src/app/backfill/isin.rs` | DB/HTTP | ISIN stock registry updates are Phase 1/2 critical. | Add ACL/mapper tests before domain extraction. |
| 1 | `src/app/backfill/net_asset_value_per_share/emerging.rs` | DB/HTTP | Emerging NAV backfill can map values incorrectly. | Add mapper tests; keep live smoke ignored. |
| 1 | `src/app/backfill/net_asset_value_per_share/zero_value.rs` | DB/HTTP | Zero-value repair can update wrong stocks. | Add filter/selection unit tests with fake data. |
| 1 | `src/app/backfill/qualified_foreign_institutional_investor.rs` | DB/HTTP | QFII import can misclassify ownership. | Add parser/mapper tests. |
| 2 | `src/app/backfill/quote.rs` | DB/HTTP/Scheduler | Quote backfill can miss dates or write duplicate quote rows. | Add date-selection and mapper tests; keep live backfill ignored. |
| 2 | `src/app/backfill/revenue.rs` | DB/HTTP | Revenue backfill can rebuild wrong monthly state. | Add date/key mapper tests; keep DB smoke ignored. |
| 1 | `src/app/backfill/stock_weight.rs` | DB/HTTP | Stock weight import can overwrite weights incorrectly. | Add CSV/parser mapper tests. |
| 1 | `src/app/backfill/taiwan_stock_index.rs` | DB/HTTP | Index backfill can miswrite market index rows. | Add mapper tests; keep live smoke ignored. |
| 1 | `src/app/calculation/daily_quotes.rs` | DB | Moving-average calculation relies on persisted quote rows. | Add fake quote repository tests when repository boundary exists. |
| 3 | `src/app/calculation/dividend_record.rs` | DB | Dividend record writes require transaction and cumulative updates. | Pure eligibility tests added; keep transaction smoke ignored. |
| 1 | `src/app/calculation/estimated_price.rs` | DB | Estimated price calculation can drift with persisted data. | Add pure formula tests before touching calculation service. |
| 1 | `src/app/calculation/money_history.rs` | DB | Money history aggregation can mis-sum daily flows. | Add aggregation tests with in-memory rows. |
| 1 | `src/app/event/taiwan_stock/annual_eps.rs` | DB/HTTP/Messaging | Event can send stale annual EPS notifications. | Add message builder and selection tests. |
| 3 | `src/app/event/taiwan_stock/closing.rs` | DB/HTTP/Messaging | Closing aggregation and notification can drift. | Existing formatting tests cover some output; add fake event tests. |
| 1 | `src/app/event/taiwan_stock/ex_dividend.rs` | DB/HTTP/Messaging | Ex-dividend notifications can miss holdings. | Existing grouping/date tests cover core; keep live smoke ignored. |
| 1 | `src/app/event/taiwan_stock/payable_date.rs` | DB/HTTP/Messaging | Payable-date notifications can group incorrectly. | Existing message grouping tests cover core; keep live smoke ignored. |
| 4 | `src/app/event/trace/stock_price.rs` | Redis/DB/HTTP/Messaging/Scheduler | Price trace can duplicate or miss alerts. | Cache/key/boundary tests added; keep Redis/Telegram smoke ignored. |
| 6 | `src/app/manual_backfill.rs` | Manual/DB/HTTP | Operator backfill commands can affect production-like data. | Keep ignored; run only explicit manual smoke. |
| 1 | `src/app/scheduler.rs` | Scheduler | Scheduler test depends on runtime timing. | Add pure schedule split tests where possible. |
| 5 | `src/core/logging/rotate.rs` | Filesystem | File rotation can be flaky under parallel test runs. | Consider tempdir-based deterministic tests later. |
| 1 | `src/core/util/http/mod.rs` | HTTP | HTTP helper depends on external network. | Add request builder tests; keep live GET ignored. |
| 2 | `src/infra/crawler/fugle/price.rs` | HTTP | Live quote API availability can vary. | Add response fixture parser tests. |
| 1 | `src/infra/crawler/histock/annual_profit.rs` | HTTP | Live page can change. | Existing text parser test covers core; keep live smoke ignored. |
| 5 | `src/infra/crawler/histock/price.rs` | HTTP/Scheduler | Live price and cache tasks depend on network/timing. | Existing row parser tests cover core; add more fixture cases. |
| 2 | `src/infra/crawler/mod.rs` | HTTP | Site selection and live quote fetch depend on external providers. | Existing latency/stat tests cover pure logic; keep live smoke ignored. |
| 1 | `src/infra/crawler/mops/annual_profit.rs` | HTTP | Live MOPS response can vary. | Existing parser/formula tests cover core; add malformed fixture tests. |
| 1 | `src/infra/crawler/share.rs` | HTTP | Public IP lookup depends on network. | Existing normalization/error tests cover core. |
| 1 | `src/infra/crawler/taifex/stock_weight.rs` | HTTP | TAIFEX response can change. | Add parser fixture tests. |
| 1 | `src/infra/crawler/tpex/etf.rs` | HTTP | TPEX ETF response can change. | Add fixture parser tests. |
| 1 | `src/infra/crawler/tpex/net_asset_value_per_share.rs` | HTTP | TPEX NAV response can change. | Add fixture parser tests. |
| 1 | `src/infra/crawler/tpex/quote.rs` | HTTP | TPEX quote response can change. | Add fixture parser tests. |
| 1 | `src/infra/crawler/twse/etf.rs` | HTTP | TWSE ETF response can change. | Add fixture parser tests. |
| 1 | `src/infra/crawler/twse/holiday_schedule.rs` | HTTP | Holiday source can change. | Add calendar parser fixture tests. |
| 1 | `src/infra/crawler/twse/international_securities_identification_number.rs` | HTTP | ISIN crawler is Phase 1/2 critical. | Add ISIN ACL/mapper fixture tests. |
| 1 | `src/infra/crawler/twse/public.rs` | HTTP | Public-company source can change. | Add fixture parser tests. |
| 1 | `src/infra/crawler/twse/qualified_foreign_institutional_investor/listed.rs` | HTTP | Listed QFII source can change. | Add parser fixture tests. |
| 1 | `src/infra/crawler/twse/qualified_foreign_institutional_investor/over_the_counter.rs` | HTTP | OTC QFII source can change. | Add parser fixture tests. |
| 1 | `src/infra/crawler/twse/quote.rs` | HTTP | TWSE quote table can change. | Existing table-detection test covers part; add fixture rows. |
| 1 | `src/infra/crawler/twse/revenue.rs` | HTTP | Revenue source can change. | Add parser fixture tests. |
| 1 | `src/infra/crawler/twse/suspend_listing.rs` | HTTP | Suspend-listing source can change. | Add fixture parser tests. |
| 1 | `src/infra/crawler/twse/taiwan_capitalization_weighted_stock_index.rs` | HTTP | Market index source can change. | Add fixture parser tests. |
| 1 | `src/infra/crawler/wespai/profit.rs` | HTTP | Live profit source can change. | Add parser fixture tests. |
| 1 | `src/infra/crawler/yahoo/dividend.rs` | HTTP | Yahoo dividend page can change. | Existing parser tests cover core; add malformed row cases. |
| 1 | `src/infra/crawler/yahoo/price/cache.rs` | HTTP/Scheduler | Background cache task depends on timing and live providers. | Existing apply-snapshot tests cover core. |
| 3 | `src/infra/crawler/yahoo/price/class_quote.rs` | HTTP | Yahoo class quote paging depends on live API. | Existing parser/pagination tests cover core; add malformed fixtures. |
| 2 | `src/infra/crawler/yahoo/price/quote_page.rs` | HTTP | Yahoo quote page can change. | Add quote page fixture parser tests. |
| 1 | `src/infra/crawler/yahoo/profile.rs` | HTTP | Yahoo profile can change. | Existing no-data cache tests cover core. |
| 2 | `src/infra/crawler/yuanta/price.rs` | HTTP | Yuanta quote source can change. | Add fixture parser tests. |
| 1 | `src/infra/database/table/config.rs` | DB | Config fetch depends on DB state. | Keep DB integration ignored. |
| 7 | `src/infra/database/table/dividend/mod.rs` | DB | Dividend upsert/fetch logic is high-risk for Phase 2. | Add mapper tests; keep DB integration ignored. |
| 5 | `src/infra/database/table/dividend/**` | DB | Dividend extension/detail tables require DB. | Add model/unit tests for pure construction if present. |
| 2 | `src/infra/database/table/financial/estimate.rs` | DB | Estimate writes require DB. | Add pure row builder tests if available. |
| 4 | `src/infra/database/table/financial/financial_statement.rs` | DB | Financial statement fetches require DB. | Add mapper tests before repository extraction. |
| 2 | `src/infra/database/table/financial/revenue.rs` | DB | Revenue rebuild/fetch requires DB. | Existing date test covers part; add pure date/key tests. |
| 1 | `src/infra/database/table/index.rs` | DB | Index fetch requires DB. | Keep DB integration ignored. |
| 4 | `src/infra/database/table/money_flow/**` | DB | Money-flow writes require DB. | Add aggregation/mapper tests with in-memory rows. |
| 7 | `src/infra/database/table/quote/daily_quote/mod.rs` | DB | Quote table operations are Phase 2 critical. | Add mapping/CSV/key tests; keep DB integration ignored. |
| 5 | `src/infra/database/table/quote/**` | DB | Quote stats/history require DB. | Add pure aggregation tests where possible. |
| 2 | `src/infra/database/table/stock/extension/weight.rs` | DB | Weight update requires DB. | Add parser/mapper tests in crawler/backfill layer. |
| 6 | `src/infra/database/table/stock/mod.rs` | DB | Stock row has mixed DB, mapper, and business logic. | Add Stock conversion/key/preference/TDR tests before Phase 2. |
| 3 | `src/infra/database/table/stock/**` | DB | Stock ownership/word tables require DB. | Add pure model/key tests where possible. |
| 1 | `src/infra/database/table/trace.rs` | DB | Trace fetch requires DB. | Add Trace key/constructor tests if not covered. |
| 1 | `src/infra/database/table/yield_rank.rs` | DB | Yield-rank upsert requires DB. | Keep DB integration ignored. |
| 1 | `src/infra/nosql/redis.rs` | Redis | Redis connectivity and key behavior depend on service. | Existing non-ignored Redis value tests cover some behavior; add fake contract tests if possible. |
| 1 | `src/interfaces/bot/telegram.rs` | Messaging | Sends Telegram message. | Existing escaping test covers pure logic; keep send smoke ignored. |
| 1 | `src/interfaces/rpc/client/stock_service/mod.rs` | Messaging | Calls stock service gRPC endpoint. | Keep ignored; add request mapping tests. |
| 1 | `src/interfaces/rpc/server/control_service.rs` | Messaging | Starts or calls gRPC server path. | Existing request conversion tests cover part; keep live server ignored. |

## Phase 0.5 Closure Criteria

> **Phase 0.5 正式關閉**（Closed: 2026-06-05）。所有 Priority 1/2 測試已完成，詳細結果見 `phase-0-5-baseline.md`。

- [x] Priority 1 pure logic tests are complete.
  （詳見 `phase-0-5-baseline.md` Priority 1 各項，全數 `[x]`）
- [x] Priority 2 parser/mapper characterization tests are complete.
  （詳見 `phase-0-5-baseline.md` Priority 2 各項，全數 `[x]`）
- [x] Ignored tests are categorized by file and risk.
- [x] For all Phase 1/2 critical files, replacement validation is present or explicitly deferred with reason.
  （DB/Redis/HTTP/Scheduler/Messaging 均已在 I/O Exemption Categories 中列明替代驗證方式）
