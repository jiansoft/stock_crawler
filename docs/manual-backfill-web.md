# Manual Backfill Web / API / gRPC

`manual_backfill.rs` 原本把手動資料回補包在 `#[ignore]` 單元測試裡。現在主程式提供三種入口：

- Web UI：給人用瀏覽器操作。
- HTTP JSON API：給腳本、後台或其他服務用 REST-like 方式串接。
- gRPC API：給已經使用本專案 gRPC port 的服務直接串接。

三種入口共用同一個背景 job 狀態；不管從 Web、HTTP 或 gRPC 觸發，都可以查到同一批 job。

## 啟動

啟動主程式：

```powershell
cargo run
```

Web UI / HTTP API 預設監聽：

```text
http://127.0.0.1:9002/manual-backfill
```

可用環境變數調整 Web / HTTP API 位址：

```powershell
$env:MANUAL_BACKFILL_WEB_ADDR = "0.0.0.0:9002"
cargo run
```

gRPC 使用既有設定 `SYSTEM_GRPC_USE_PORT`，預設依 `app.json` 是 `9001`：

```text
0.0.0.0:9001
```

## Web UI

開啟：

```text
http://127.0.0.1:9002/manual-backfill
```

畫面提供三個操作：

| 功能 | 輸入 | 說明 |
| --- | --- | --- |
| Closing Aggregate | `YYYY-MM-DD` 交易日 | 重跑每日收盤事件匯總 |
| Received Dividends | 股票代號 | 重算指定股票目前持股的已領股利紀錄 |
| Historical Dividends | 股票代號 | 從 Yahoo 回補單檔股票歷年股利，並同步重算已領股利 |

按下 `Start` 後會建立背景 job，畫面每 3 秒輪詢 job 狀態。

## HTTP API

Base URL：

```text
http://127.0.0.1:9002
```

### 建立收盤匯總回補

```http
POST /api/manual-backfill/closing-aggregate
Content-Type: application/json

{
  "date": "2026-04-30"
}
```

### 建立已領股利紀錄回補

```http
POST /api/manual-backfill/received-dividend-records
Content-Type: application/json

{
  "security_code": "0056"
}
```

### 建立歷年股利回補

```http
POST /api/manual-backfill/historical-dividends
Content-Type: application/json

{
  "security_code": "2845"
}
```

### 建立 job 的回應格式

```json
{
  "job": {
    "id": "20260504103000-1",
    "kind": "historical_dividends",
    "input": "2845",
    "status": "running",
    "message": "queued",
    "started_at": "2026-05-04T10:30:00+08:00",
    "finished_at": null
  }
}
```

### 查詢所有 job

```http
GET /api/manual-backfill/jobs
```

回應：

```json
[
  {
    "id": "20260504103000-1",
    "kind": "historical_dividends",
    "input": "2845",
    "status": "succeeded",
    "message": "historical dividends backfill completed: upserted_count=12",
    "started_at": "2026-05-04T10:30:00+08:00",
    "finished_at": "2026-05-04T10:30:18+08:00"
  }
]
```

### 查詢單一 job

```http
GET /api/manual-backfill/jobs/{id}
```

例如：

```http
GET /api/manual-backfill/jobs/20260504103000-1
```

### HTTP curl 範例

```bash
curl -X POST http://127.0.0.1:9002/api/manual-backfill/historical-dividends \
  -H "content-type: application/json" \
  -d '{"security_code":"2845"}'
```

```bash
curl http://127.0.0.1:9002/api/manual-backfill/jobs
```

## gRPC API

Proto 檔案：

```text
etc/proto/manual_backfill.proto
```

Service：

```proto
service ManualBackfill {
  rpc StartClosingAggregate(ClosingAggregateRequest) returns (BackfillJobResponse) {}
  rpc StartReceivedDividendRecords(SecurityCodeRequest) returns (BackfillJobResponse) {}
  rpc StartHistoricalDividends(SecurityCodeRequest) returns (BackfillJobResponse) {}
  rpc ListJobs(ListJobsRequest) returns (ListJobsResponse) {}
  rpc GetJob(GetJobRequest) returns (BackfillJobResponse) {}
}
```

### gRPC method 對照

| Method | Request | 說明 |
| --- | --- | --- |
| `manual_backfill.ManualBackfill/StartClosingAggregate` | `{ "date": "2026-04-30" }` | 建立收盤匯總回補 job |
| `manual_backfill.ManualBackfill/StartReceivedDividendRecords` | `{ "security_code": "0056" }` | 建立已領股利紀錄回補 job |
| `manual_backfill.ManualBackfill/StartHistoricalDividends` | `{ "security_code": "2845" }` | 建立歷年股利回補 job |
| `manual_backfill.ManualBackfill/ListJobs` | `{}` | 查詢所有 job |
| `manual_backfill.ManualBackfill/GetJob` | `{ "id": "20260504103000-1" }` | 查詢單一 job |

### grpcurl 範例

如果 gRPC server 沒有啟用 TLS：

```bash
grpcurl -plaintext \
  -import-path etc/proto \
  -proto manual_backfill.proto \
  -d '{"security_code":"2845"}' \
  127.0.0.1:9001 \
  manual_backfill.ManualBackfill/StartHistoricalDividends
```

查詢所有 job：

```bash
grpcurl -plaintext \
  -import-path etc/proto \
  -proto manual_backfill.proto \
  -d '{}' \
  127.0.0.1:9001 \
  manual_backfill.ManualBackfill/ListJobs
```

查詢單一 job：

```bash
grpcurl -plaintext \
  -import-path etc/proto \
  -proto manual_backfill.proto \
  -d '{"id":"20260504103000-1"}' \
  127.0.0.1:9001 \
  manual_backfill.ManualBackfill/GetJob
```

如果 gRPC server 啟用 TLS，請依你的憑證設定改用 `grpcurl` 的 `-cacert`、`-cert`、`-key`、`-authority` 等參數。

### gRPC 回應格式

```json
{
  "job": {
    "id": "20260504103000-1",
    "kind": "historical_dividends",
    "input": "2845",
    "status": "running",
    "message": "queued",
    "startedAt": "2026-05-04T10:30:00+08:00",
    "finishedAt": ""
  }
}
```

gRPC 的 `finished_at` 是 proto3 string，尚未完成時會回空字串；HTTP API 則會回 `null`。

## Job 狀態

| 狀態 | 說明 |
| --- | --- |
| `running` | 背景任務執行中 |
| `succeeded` | 回補完成 |
| `failed` | 回補失敗，`message` 會包含錯誤內容 |

Job 只保存在記憶體，服務重啟後歷史 job 會消失；實際結果請以 log 與資料庫內容為準。

## 串接建議

人員操作：

1. 開啟 `/manual-backfill`。
2. 輸入日期或股票代號。
3. 按 `Start`。
4. 在下方 job table 看 `status` 與 `message`。

HTTP API 串接：

1. 呼叫對應的 `POST /api/manual-backfill/...` 建立 job。
2. 保存回傳的 `job.id`。
3. 每 3 到 10 秒呼叫 `GET /api/manual-backfill/jobs/{id}`。
4. 當 `status` 變成 `succeeded` 或 `failed` 後停止輪詢。

gRPC 串接：

1. 使用 `etc/proto/manual_backfill.proto` 產生 client。
2. 呼叫 `Start...` method 建立 job。
3. 保存回傳的 `job.id`。
4. 用 `GetJob` 輪詢狀態，或用 `ListJobs` 做管理頁。

## 程式檔案

- `src/web/mod.rs`：Axum server 啟動入口，讀取 `MANUAL_BACKFILL_WEB_ADDR`。
- `src/web/manual_backfill.rs`：Web UI、HTTP API routes、背景 job 狀態管理。
- `etc/proto/manual_backfill.proto`：gRPC service 定義。
- `src/rpc/server/manual_backfill_service.rs`：gRPC service 實作。
- `src/rpc/server/mod.rs`：註冊 `ManualBackfillServer`。
- `src/main.rs`：主流程啟動 Web server；gRPC server 仍沿用既有 `rpc::server::start()`。
- `src/backfill/dividend/missing_or_multiple.rs`：將單檔歷年股利回補從 test-only 改為 binary 可呼叫。

## 注意事項

- 目前沒有登入驗證。Web / HTTP API 預設只綁定 `127.0.0.1`；不要直接暴露到公開網路。
- gRPC 是否對外開放取決於既有 `SYSTEM_GRPC_USE_PORT` 與部署網路設定。
- 回補任務會寫資料庫，也會呼叫外部資料來源；正式執行前請確認 `.env` / `app.json` 指向正確環境。
- 歷年股利回補會直接打 Yahoo 並寫入 `dividend` 表，完成後會同步重算目前持股的已領股利紀錄。
