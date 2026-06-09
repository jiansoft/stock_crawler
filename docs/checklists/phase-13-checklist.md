# Phase 13 Checklist - 引入領域事件解耦 Telegram 副作用 (Domain Event Pub/Sub)

## A. 基本資訊

- **Phase**: Phase 13 - 引入領域事件解耦 Telegram 副作用
- **Owner**: Antigravity
- **Branch**: `refactor/ddd-phase-13`
- **Start SHA**: `e05fd6d`
- **Rollback Tag**: `pre-phase-13`
- **預估完成日**: 2026-06-09

- [x] 分支已建立並切換至 `refactor/ddd-phase-13`
- [x] 歷史修改已確認無衝突

---

## B. 變更與重構清單

### 1. 定義 Domain 層新事件
- [x] 於 [`src/domain/events.rs`](file:///D:/Project/Eddie/stock_rust/src/domain/events.rs) 擴充 `DomainEvent` 列舉：
  - 新增 `MoneyFlowRecalculated` 事件 (載有 `date: NaiveDate`, `occurred_at: DateTime<Local>`)。
  - 新增 `ExDividendReminderTriggered` 事件 (載有 `date: NaiveDate`, `next_trading_date: NaiveDate`, `occurred_at: DateTime<Local>`)。

### 2. 實作 Application 層 Event Handler 副作用
- [x] 於 [`src/app/event/handlers.rs`](file:///D:/Project/Eddie/stock_rust/src/app/event/handlers.rs) 重構並註冊新事件之處理：
  - `default_handle_event` 內匹配 `DomainEvent::MoneyFlowRecalculated`，異步加載市值對照資料，組裝訊息並發送 Telegram。
  - `default_handle_event` 內匹配 `DomainEvent::ExDividendReminderTriggered`，異步加載當日及下一交易日除權息名單、會員持股，並透過 Telegram 發送。
  - 將原本位於 `closing.rs` 與 `ex_dividend.rs` 的 Telegram 訊息格式化與發送邏輯搬移至 `handlers.rs` 的輔助方法中。

### 3. 重構 Application 層 Use Cases
- [x] 重構 [`src/app/event/taiwan_stock/closing.rs`](file:///D:/Project/Eddie/stock_rust/src/app/event/taiwan_stock/closing.rs)：
  - 移除同步呼叫 `notify_money_change` 及相關格式化函數與 Table 直接依賴。
  - 於 `aggregate` 尾部派發 `MoneyFlowRecalculated` 領域事件。
- [x] 重構 [`src/app/event/taiwan_stock/ex_dividend.rs`](file:///D:/Project/Eddie/stock_rust/src/app/event/taiwan_stock/ex_dividend.rs)：
  - 移除同步調用 `send_market_dividend_message` 與直接向 `bot::telegram` 發送訊息。
  - 於 `execute` 的各個分支與結尾派發 `ExDividendReminderTriggered` 領域事件。
  - 清理未使用的模組引用。

---

## C. Gate 執行與驗證

- [x] `cargo fmt --all -- --check` 格式檢查通過
- [x] `cargo check --tests` 編譯通過
- [x] `cargo clippy --all-targets -- -D warnings` 零警告通過
- [x] `cargo test` 測試套件執行無誤
- [x] 單元與整合測試新增：
  - [x] 於 `handlers.rs` 的單元測試中驗證新增的領域事件分派。
  - [x] 驗證重構後的 `closing.rs` 與 `ex_dividend.rs` 測試行為是否正常。

---

## D. 合併前確認

- [x] 與此重構無關的異動已排除
- [x] 重構後的繁體中文註解與 Rustdoc 已完整撰寫
- [x] PR 說明已附 Checklist 摘要與 Gate 結果
