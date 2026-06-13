# 重構計畫

更新日期：2026-06-13

本計畫遵守 Behavior-Preserving Refactoring。任何核心重構前，必須先完成 Phase 2 行為保護。

## 整體重構方向

1. 先建立可重複執行的 baseline：fmt、clippy、test、DB/Redis bootstrap、外部 API 測試分類。
2. 優先補 characterization tests，鎖定 parser、排程、ACL mapper、repository mapping、Web/RPC handler 可觀察行為。
3. 從低風險邊界開始：文件、測試 fixture、adapter 分離、dependency direction 修正。
4. 每次只處理一個模組或一個主題，避免大規模跨層搬移。
5. 最後才考慮 workspace 拆分；目前先維持單 crate 以降低風險。

## 推薦的新架構

短期維持目前五層架構：

```text
core
domain
app
infra
interfaces
```

中期可逐步收斂為：

```text
interfaces/adapters -> app/use_cases -> domain
infra/adapters      -> domain repository ports
bootstrap           -> wiring / startup
```

workspace 拆分只在邊界穩定後評估，避免 public API、build.rs、generated code 與 CI 一次變動過大。

## 分層設計建議

- `domain`：只保留 entity、value object、domain event、repository trait；避免 `anyhow` 長期作為 domain contract。
- `app`：定義 use case input/output 與流程編排；避免直接知道 Telegram、Axum、Tonic DTO。
- `infra`：實作 repository、crawler、cache、Redis、PostgreSQL；不得依賴 `interfaces`。
- `interfaces`：HTTP/gRPC/Bot adapter，只做 transport 轉換與呼叫 use case。
- `main/bootstrap`：集中 wiring、health check、shutdown、服務生命週期。

## 模組拆分建議

優先順序：

1. `interfaces/web/backfill_admin.rs`：拆 handler、state、response DTO、job runner。
2. `app/backfill/acl.rs`：依 quote、stock、dividend、financial、index 拆 mapper。
3. `app/event/handlers.rs`：拆通知格式、資料讀取、domain event handler。
4. `infra/cache/share.rs`：拆 snapshot、loader、query facade、test helper。
5. `infra/database/table/quote/daily_quote/mod.rs`：拆 query、command、row mapping、statistics extension。

## 錯誤處理策略

- 短期：保留 public function 回傳型別，使用 `anyhow::Context` 增補上下文，不改呼叫端語意。
- 中期：為 infra adapter 建立 `thiserror` enum，例如 database/crawler/telegram/web use case error，再於邊界轉成 `anyhow`。
- domain trait 長期可使用 domain-specific error，但這會影響 public API，需另開 ADR 與遷移計畫。
- 移除 `unwrap/expect` 僅限非測試與非不可能失敗常數路徑，且需測試證明錯誤路徑等價或更可控。

## 測試策略

- 先執行 Phase 2 baseline commands。
- 將測試分成 pure unit、fixture parser、DB/Redis integration、live-network manual。
- 對 parser 與 ACL mapper 加 characterization tests。
- 對排程註冊建立非啟動式測試，鎖定 cron expression 與 job 名稱。
- DB repository tests 應可偵測 DB 不可用並清楚 skip，或由 CI service 保證執行。

## 效能優化方向

- 先量測，不先改：記錄 cache load 時間、即時報價任務記憶體、HTTP fallback latency、Redis invalidation 次數。
- clone 優化以 hot path 為主：即時報價、cache snapshot、daily quote bulk insert、ACL mapper。
- 對大 Vec/HashMap 優先使用借用、iterator、`Arc<str>` 或 `Cow<'_, str>`，但需證明不改 ownership 語意。
- RwLock hot path 應檢查鎖粒度與 await 邊界。

## 預計重構順序

1. Phase 2：baseline verification 與 characterization tests。
2. Phase 3：確認此計畫，建立 ADR 與回滾策略。
3. Testing：fixture 化 live parser tests。
4. Dependency direction：移除 infra 到 interfaces 的通知依賴。
5. Web manual backfill：拆 state/handler/job runner。
6. Backfill ACL：依領域拆 mapper。
7. Event handlers：拆通知格式與事件 handler。
8. Database table God Module：逐步拆檔，不改 SQL。
9. Startup bootstrap：從 `main.rs` 拆出 service lifecycle。
10. Error typing 與 tracing：最後進行，因影響面最大。

## 風險與回滾策略

- 每個 Phase 或子主題都建立短分支，例如 `refactor/phase-2-baseline`、`refactor/web-backfill-split`。
- 不直接在 `main` 上進行重構；所有重構變更都透過 PR 合併回 `main`。
- 一個 PR 只涵蓋一個 Phase、模組或主題，避免把測試、架構搬移與行為修正混在一起。
- PR 內容必須包含目的、影響範圍、修改檔案、驗證命令與結果、已知風險。
- PR 通過格式化、Clippy、測試與必要審查後，才合併回 `main`；合併後再開下一個重構分支。
- 每次只改一個主題並建立小 commit。
- 修改前記錄 baseline 測試結果。
- 行為不確定處標示「待確認（To Be Verified）」並停止修改該區域。
- 若 `cargo test --all-features` 因環境失敗，需區分環境問題與程式 regressions。
- SQL/proto/config default 不與一般重構同 PR 修改。
