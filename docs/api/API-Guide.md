# API Guide

更新日期：2026-06-13

> **快速導覽**：本文件為三個介面的概要。
> 完整操作範例（curl / grpcurl）請見 [`manual-backfill-web.md`](../manual-backfill-web.md)。

---

## gRPC

Proto 定義位於 `etc/proto/`，`build.rs` 會產生 Rust stub 到 `src/interfaces/rpc/`。

### 服務與方法

| 服務 | Proto 檔案 | 方法 | 說明 |
|---|---|---|---|
| `ControlService` | `etc/proto/basic.proto` | `SayHello` | 健康檢查 / 連線驗證 |
| `ManualBackfillService` | `etc/proto/manual_backfill.proto` | `StartDailyQuotes` | 手動觸發每日收盤報價回補 |
| `ManualBackfillService` | `etc/proto/manual_backfill.proto` | `StartTaiwanStockIndex` | 手動觸發台股加權指數回補 |
| `ManualBackfillService` | `etc/proto/manual_backfill.proto` | `StartDividend` | 手動觸發單檔歷年股利回補 |
| `StockService` | `etc/proto/stock.proto` | （見 proto 定義） | 股票資訊查詢與推播 |

### 連線方式

```bash
# 健康檢查
grpcurl -plaintext localhost:9000 basic.ControlService/SayHello

# 手動觸發每日報價回補（範例日期）
grpcurl -plaintext -d '{"date": "2024-01-15"}' \
  localhost:9000 manual_backfill.ManualBackfillService/StartDailyQuotes
```

### 重構限制

- 不得手動修改 generated files（`src/interfaces/rpc/*.rs`）。
- 不得改變 proto 欄位、tag、service、method 或 serialization 格式，除非另有明確需求與相容性計畫。
- Service handler 可內部拆分，但 request/response 行為需 characterization tests 保護。

---

## HTTP / Web

HTTP 管理介面位於 `src/interfaces/web/`，主要功能是手動回補管理。
預設綁定 `127.0.0.1`，**不得直接暴露到公開網路**。

### 端點清單

| Method | Path | 說明 |
|---|---|---|
| `GET` | `/` | Web UI 首頁 |
| `POST` | `/api/daily-quotes` | 觸發每日收盤報價回補（body: `{"date": "YYYY-MM-DD"}`）|
| `POST` | `/api/taiwan-stock-index` | 觸發台股加權指數回補（body: `{"date": "YYYY-MM-DD"}`）|
| `POST` | `/api/dividend` | 觸發單檔歷年股利回補（body: `{"stock_symbol": "2330"}`）|
| `GET` | `/api/job-status` | 查詢目前背景 job 狀態 |

> 實際端點以 `src/interfaces/web/backfill_admin.rs` 的路由定義為準，以上為概覽。

### 重構限制

- 不得改變 URL path、HTTP method、response JSON 格式與狀態碼，除非另有明確需求。
- `interfaces/web/backfill_admin.rs` 可拆分 handler/state/job runner，但需先鎖定 job 狀態轉換。
- 目前沒有登入驗證，未來若加入 auth，需另立 ADR 並評估回溯相容性。

---

## Telegram

Telegram adapter 位於 `src/interfaces/bot/telegram.rs`，用於排程提醒、價格追蹤與啟動/錯誤告警。

### 觸發時機

| 觸發事件 | 說明 |
|---|---|
| 啟動自我測試失敗 | `main()` 呼叫 `test_client::run_test()` 失敗時發送告警 |
| 每日收盤排程完成 | 盤後報價抓取完成後發送摘要 |
| 股利除息日提醒 | 依持倉發送當日除息/除權股票通知 |
| 價格追蹤觸發 | 追蹤標的達到目標價或漲跌幅條件時通知 |
| 錯誤告警 | 重要背景任務失敗時發送錯誤訊息 |

### 重構限制

- 不得改變通知觸發時機與訊息格式，除非先補 snapshot/format tests。
- Infra 層不應直接依賴 Telegram adapter；後續應以 app event 或 notification port 解耦。


