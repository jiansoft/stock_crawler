# 效能筆記

更新日期：2026-06-13

## Allocator 設定（Linux musl 環境）

`main.rs` 在 `target_os = "linux"` 且 `target_env = "musl"` 的條件下，
使用 **mimalloc** 作為全域 allocator（`#[global_allocator] static GLOBAL_ALLOCATOR: MiMalloc`）。

啟動時透過 `.init_array` 在 `main()` 前呼叫 `init_mimalloc_env()`，
設定以下環境參數以最佳化長期執行的記憶體行為：

| 環境變數 | 值 | 效果 |
|---|---|---|
| `MIMALLOC_PURGE_DELAY` | `0` | 頁面一旦空出就立即 purge |
| `MIMALLOC_PURGE_DECOMMITS` | `1` | purge 時使用 decommit / `MADV_DONTNEED`，讓 RSS 較快下降 |
| `MIMALLOC_ALLOW_THP` | `0` | 避免 THP 讓 purge 顆粒變粗，影響回收速度 |
| `MIMALLOC_ALLOW_LARGE_OS_PAGES` | `0` | 避免不可 purge 的 large OS pages |
| `MIMALLOC_ARENA_EAGER_COMMIT` | `0` | 避免過早 commit 大 arena |

> **⚠️ 注意**：mimalloc 與這些環境參數屬於高風險設定，調整時需驗證記憶體行為。
> **Windows 開發環境**下這些段落不會編譯（`cfg` 條件限定 Linux musl）。

## 目前效能敏感路徑

- 啟動：PostgreSQL ping、`infra::cache::SHARE.load()`、scheduler register、gRPC/Web start。
- 盤中：HiStock/Yahoo 即時報價背景快取、price trace 判斷、Redis 去重。
- 收盤：每日報價抓取、bulk save、估價與資金流計算。
- 回補：外部 crawler + database upsert + cache update。
- 日誌：檔案日誌與 Seq forwarding。

## Clone / Allocation 觀察

- ACL mapper、crawler DTO、domain entity conversion 中有大量 `String::clone()`。
- async task spawn 與 `Arc::clone` 多數可能是必要生命週期成本。
- `SHARE` 以 HashMap/RwLock 保存大量股票、報價、指數與快照，clone snapshot 需小心。
- 重構前應以 targeted benchmark 或 tracing span 統計 hot path，再決定是否使用借用、`Cow`、`Arc<str>` 或批次 API。

## Benchmark 建議

目前未發現 `benches/`。建議 Phase 2/3 後補最小基準：

- parser benchmark：TWSE/TPEX daily quote fixture。
- mapper benchmark：daily quote DTO -> domain -> table row。
- cache benchmark：`SHARE` snapshot lookup/update。
- DB benchmark：僅在 local opt-in 環境執行，不作為一般 CI 必跑。

## 不建議立即優化

- 不應先全域替換 `String` 為 `Arc<str>` 或 `Cow`。
- 不應在缺少測量時改變 cache 結構。
- 不應調整排程並行度、HTTP retry、rate limit，這些都可能改變外部行為。

