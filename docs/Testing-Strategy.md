# 測試策略

更新日期：2026-06-13

## Phase 2 Baseline Commands

進入核心重構前必須執行：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

本專案 CI 目前實際使用：

```bash
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test --release -- --nocapture --test-threads=1
```

兩者差異需在 Phase 2 釐清，尤其是 `--all-features` 目前 Cargo.toml 未定義 features，但仍應確認不破壞。

## 測試分類

| 類型 | 範例 | 目標 |
|---|---|---|
| Pure unit | `core::util::datetime`、parser helper、domain entity | 快速、 deterministic、每次必跑 |
| Fixture parser | TWSE/TPEX quote、Yahoo dividend、MOPS annual profit | 不連網鎖定外部格式解析 |
| DB integration | `infra/database/repository/*`、table CRUD | CI Postgres 啟動後必跑 |
| Redis integration | `infra/nosql/redis.rs`、cache invalidation | CI Redis 啟動後必跑或明確 skip |
| Live-network manual | 真實 crawler visit、Telegram send | 不作為一般重構 gate，需手動或 feature gate |

## Characterization Tests 優先區

- `app/scheduler.rs`：cron expression、任務名稱、啟動補償邏輯。
- `app/backfill/acl.rs`：外部 DTO 到 command/domain entity 的欄位映射。
- `infra/crawler/*`：日期推論、缺欄容忍、數字格式、民國年轉換。
- `interfaces/web/backfill_admin.rs`：job 狀態轉換、錯誤 response 格式。
- `interfaces/rpc/server/*_service.rs`：request/response 欄位與錯誤碼。
- `infra/database/repository/*`：domain/table mapping 與 cache invalidation 副作用。

## 測試缺口

- 缺少 top-level `tests/` integration suite。
- 多數 integration tests colocated，DB/Redis/live-network 邊界不明。
- 沒有 benches 目錄；效能改善目前無 benchmark baseline。
- 部分測試含 `println!` 與 `expect("TODO: panic message")`，失敗診斷品質不穩。
- CI test job 有 DB/Redis，但 clippy job 沒有 DB/Redis；compile-time SQLx macro 若擴大使用需注意。

