# 風險與待辦

更新日期：2026-06-13

## 高風險

- `src/main.rs` 啟動流程過重，但啟動順序與錯誤通知是可觀察行為，不可直接拆改。
- **mimalloc 全域 allocator**（僅 Linux musl）：`init_mimalloc_env()` 在 `.init_array` 設定五個環境參數（PURGE_DELAY、PURGE_DECOMMITS、ALLOW_THP 等），調整時需驗證 RSS 與回收行為，不應在一般重構中改動。詳見 `Performance-Notes.md`。
- **gRPC 自我測試**：`main()` 啟動後會呼叫 `interfaces::rpc::client::test_client::run_test()`；若 gRPC server 未正常啟動，此函式失敗時**會觸發 Telegram 告警**。任何改動 gRPC 啟動時序的重構都必須在有 gRPC 環境的情況下驗證。
- `core/config.rs` 的 env override 與 `expect` fail-fast 行為不可任意改。
- `core/logging/` 自製 logging 承載檔案、輪替與 Seq；轉 `tracing` 需分階段。
- generated gRPC files 在 `src/interfaces/rpc/`，只應由 `build.rs` 產生。
- `infra/database/table/*` SQL 行為與 schema 綁定，不能在一般重構中順手改 SQL。
- 外部 crawler 依賴第三方網站格式，live tests 可能不穩。

## 待確認（To Be Verified）

- 根目錄 `Dockerfile` 是否仍被任何部署流程使用；內容目前疑似 Go 專案。
- `failed_log.txt`、`failed_log2.txt` 是否應保留、忽略或刪除。
- 舊文件 `docs/checklists/phase-*` 是否仍代表已完成狀態。
- README 與舊架構文件中的 DDNS/DNS 清理狀態是否完全同步。
- 是否需要保持 `cargo test --release -- --nocapture --test-threads=1` 作為唯一 CI-like local gate。

## Phase 2 待辦

- 執行 baseline commands 並記錄結果。
- 若 clippy/test 失敗，先分類為既有失敗、環境缺失或本地 dirty worktree 影響。
- 盤點 `#[ignore]`、live-network tests、DB/Redis tests。
- 為第一個重構目標補 characterization tests。

