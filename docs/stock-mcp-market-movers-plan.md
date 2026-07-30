# 執行計畫：新增 `get_market_movers`（當日漲跌幅／成交量排行）

- 狀態：開發完成（M1／M2 完成並全綠；M0-1／M0-3 需盤中時段實測、M3 部署待執行）
- 日期：2026-07-31
- 影響範圍：`stock_rust`（`infra/cache` 新增列舉方法、`data_api` 新增一個 endpoint）、`stock_mcp_go`（新增 API client 方法與一個 MCP tool）
- 前置條件：`docs/stock-mcp-expanded-tools-plan.md` 的 15 個工具已部署上線（D-1／D-2 於 2026-07-30 驗證通過）
- 契約基礎：本文件**沿用** `stock-mcp-expanded-tools-plan.md` 的 §3 共通契約（JSON 格式、null 語意、錯誤語意、查詢限制、envelope 慣例）與 §5.3 保母級註解規範，只在本文件描述差異處

---

## 1. 背景與目標

現有 15 個 MCP 工具中，沒有任何一個能回答「今天漲最多的是哪幾檔」「今天成交量最大的是哪幾檔」：

| 現有工具 | 缺口 |
|---|---|
| `get_latest_daily_quote`、`get_price_history`、`get_realtime_snapshot` | 有漲跌幅與成交量，但**必須先知道股票代號**，一次一檔，無法跨市場排序 |
| `get_market_breadth` | 只有上漲／下跌／平盤**家數**等聚合計數，沒有個股清單 |
| `get_market_index_history` | 是 TAIEX 大盤整體數字，不是個股 |
| `screen_stocks` | 唯一有排序的工具，但 `sort_by` 白名單清一色是基本面（`revenue_yoy`／`eps`／`roe`／`dividend_yield`／`valuation_percentage`），沒有任何價格或量的欄位 |

目標是新增**一個** MCP 工具 `get_market_movers`，同時滿足兩種時段：

- **盤中**：回傳當下的即時漲跌幅／成交量排行。
- **收盤後**：回傳當日最終的收盤排行。

並且讓呼叫端**永遠知道自己拿到的是哪一種**——資料來源、資料日期與是否即時都必須出現在回應中，不可讓 LLM 把前一交易日的收盤資料誤讀成「今天的即時行情」。

## 2. 可行性結論（已查證程式碼）

**可行，且不需要新增任何爬蟲或資料表。** 兩種時段各自已有現成資料：

### 2.1 盤中：全市場即時快照已存在於記憶體

`src/infra/crawler/price_tasks.rs` 在開盤期間啟動兩條全市場即時來源（HiStock 排行榜頁 ＋ Yahoo 類股輪詢），解析結果寫入 `infra::cache::SHARE` 的 `stock_snapshots`（`RwLock<HashMap<String, RealtimeSnapshot>>`）。

`RealtimeSnapshot`（`src/infra/cache/realtime.rs:16`）欄位已完全涵蓋需求：

```text
symbol、name、source_site、price、change、change_range(漲跌幅 %)、
open、high、low、last_close、volume(單位：張)、updated_at
```

也就是說，盤中排行**完全在記憶體內計算**，不碰資料庫，成本極低。

### 2.2 收盤後：`DailyQuotes` 已有當日最終資料

`"DailyQuotes"` 表存有 `"ChangeRange"`（漲跌幅 %）、`"Change"`、`"TradingVolume"`、`"TradeValue"`、`"Transaction"` 等欄位（`get_price_history` 的 SQL 已在讀這些欄位），由 15:00 的收盤排程寫入。既有索引 `"DailyQuotes_Date_include_symbol_idx" ("Date" desc)` 可支撐「取最新一日全部列後 top-N 排序」的查詢型態。

### 2.3 兩個必須誠實面對的限制

1. **13:30～15:00 的空窗**：13:30 收盤時 `stop_price_tasks()` 會停止採集並 `clear_stock_snapshots()` 清空快取（`app/event/taiwan_stock/closing.rs:34`、`app/event/trace/stock_price.rs:608`），但當日 `DailyQuotes` 要到 15:00 的收盤排程才寫入。**這段時間內查詢，拿到的必然是前一交易日的收盤資料**——不可假裝是當日。此時 `data_as_of` 必須如實回前一交易日日期，MCP 摘要也必須明講。
2. **即時快照不是交易所行情**：來源是第三方網站（HiStock／Yahoo），沿用現有 `realtime-snapshot` endpoint 的既有措辭「第三方採集的近即時報價快照，不宣稱為交易所保證即時行情」。

## 3. 資料來源自動切換規則

這是本工具唯一的核心邏輯，必須寫死在 Rust handler 內，不由呼叫端指定：

```text
SHARE.stock_snapshots 非空  → source = "realtime"（記憶體排行）
SHARE.stock_snapshots 為空  → source = "closing"（DailyQuotes 最新一日排行）
兩者都無資料               → 404 {"error":"查無漲跌幅排行資料"}
```

判定依據沿用既有 `SHARE.stock_snapshots_are_empty()`，與 `get_realtime_snapshot` 的時段判定完全一致，不另外自行判斷時鐘或交易日曆——**採集任務有沒有在跑，才是「現在是不是盤中」的唯一事實來源**（服務重啟、國定假日、颱風停市等狀況下，時鐘判斷會錯，快取狀態不會）。

刻意**不提供**讓呼叫端強制指定來源的參數：LLM 沒有能力判斷現在是不是盤中，把這個選擇權交出去只會製造錯誤答案。

## 4. 工具契約

### 4.1 MCP input

```json
{
  "rank_by": "top_gainers",
  "market": "all",
  "limit": 20
}
```

- `rank_by`：選填 enum，預設 `top_gainers`。
  - `top_gainers`：漲跌幅由高到低。
  - `top_losers`：漲跌幅由低到高。
  - `top_volume`：成交量由大到小。
- `market`：選填，允許 `all`、`twse`、`tpex`，預設 `all`；對映沿用 expanded-tools 計畫 §3.6 的**排行語意**——過濾 `stocks` 表，`all` = 上市（2）＋上櫃（4），**不含**興櫃（5）與公開發行（1）。
- `limit`：選填，預設 20，範圍 1–50。

**不提供** `top_trade_value`（成交金額排行）：即時快照沒有成交金額欄位，若提供這個排序鍵，同一個參數會在盤中與收盤後回傳語意不同的東西，或必須用時段相關的 422 拒絕——兩者都比不提供更糟。收盤來源仍會在每筆結果輸出 `trade_value` 供參考，只是不能拿來排序。

### 4.2 Data API

```text
GET /api/v1/market/movers?rank_by=top_gainers&market=all&limit=20
```

### 4.3 Response envelope

```json
{
  "data_as_of": "2026-07-31",
  "source": "realtime",
  "is_realtime": true,
  "rank_by": "top_gainers",
  "market": "all",
  "snapshot_updated_at": "2026-07-31T05:21:03+00:00",
  "movers": []
}
```

- `source`：`realtime` 或 `closing`，由 §3 規則決定。
- `is_realtime`：`source == "realtime"` 時為 `true`。**這是 15 個既有工具中唯一可能為 `true` 的分析型回應**，因此 §3.1 那句「所有分析型結果包含 `is_realtime: false`」在本工具不適用，其餘免責聲明照舊。
- `data_as_of`：即時來源為當日日期（台北時區）；收盤來源為該筆 `DailyQuotes` 的實際 `"Date"`。
- `snapshot_updated_at`：僅即時來源有值，取該批快照 `updated_at` 的最大值（UTC ISO 8601），讓呼叫端能判斷資料新鮮度；收盤來源為 `null`。

### 4.4 每筆欄位

| 欄位 | 即時來源 | 收盤來源（DB 欄位） |
|---|---|---|
| `rank` | 1 起算 | 1 起算 |
| `stock_symbol`、`name` | 快照 ／ `stocks` 表 | `stocks` 表 |
| `market_id`、`industry_id` | `stocks` 表 | `stocks` 表 |
| `price` | `price` | `"ClosingPrice"` |
| `change` | `change` | `"Change"` |
| `change_percent` | `change_range` | `"ChangeRange"` |
| `open`／`high`／`low` | 同名欄位 | `"OpeningPrice"`／`"HighestPrice"`／`"LowestPrice"` |
| `last_close` | `last_close` | **`null`**（`DailyQuotes` 沒有昨收欄位） |
| `volume_lots`（張） | `volume` | **`null`** |
| `volume_shares`（股） | **`null`** | `"TradingVolume"` |
| `trade_value`（元） | **`null`** | `"TradeValue"` |
| `transaction`（筆） | **`null`** | `"Transaction"` |
| `source_site` | 快照 `source_site` | **`null`** |

**成交量刻意不做單位換算**：即時快照的單位是「張」、`DailyQuotes` 是「股」，一律除以 1000 互轉看似方便，但零股交易會讓換算結果失真，而「用 `null` 誠實表示這個來源沒有這個單位的數字」符合 §3.1 的 null 語意。`top_volume` 排序時各來源用自己那個欄位排，語意不混。

### 4.5 過濾規則（兩來源一致）

- 排除 `stocks."SuspendListing" = true`。
- 排除成交量為 0 者：沒有成交的股票，漲跌幅與量排行都沒有意義。
- ~~排除 `change_percent` 為 `null` 者~~（2026-07-31 實作時查證修正）：`"DailyQuotes"` 的價量欄位皆為 `numeric(18,4) NOT NULL`，即時快照的對應欄位是 `Decimal` 而非 `Option`，兩個來源都不可能出現 `null`，因此不需要這條過濾。DTO 仍用 `Option<f64>`，只為了在 `NUMERIC` 無法安全轉成 `f64` 時能依 §3.1 輸出 `null`。
- 即時來源另外排除「快照裡有、但 `stocks` 表查不到」的代號（例如剛上市尚未同步）：無法判斷市場別與產業，直接排除並在 server log 記錄代號，不猜測。

### 4.6 排序

- `top_gainers`：`change_percent DESC`，同值 `stock_symbol ASC`。
- `top_losers`：`change_percent ASC`，同值 `stock_symbol ASC`。
- `top_volume`：對應的量欄位 `DESC`，同值 `stock_symbol ASC`。

### 4.7 MCP 輸出

`structuredContent.data_kind` 使用 `market_movers`，資料陣列欄位為 `movers`。

摘要（`content[0].text`）必須包含下列三項，缺一不可：

1. 資料來源與時段：即時來源寫「盤中即時（快照時間 HH:mm）」，收盤來源寫「YYYY-MM-DD 收盤」。
2. 收盤來源且 `data_as_of` 不等於今天時，明確加註「今日收盤資料尚未產生，以下為前一交易日 YYYY-MM-DD 的收盤排行」——這正是 13:30～15:00 空窗的情況。
3. 即時來源時加註「資料來自第三方網站採集，非交易所即時行情，可能有延遲」。

免責聲明沿用既有 `AnalysisDisclaimer`。

## 5. 實作配置

### 5.1 stock_rust

- `src/infra/cache/snapshot.rs`：新增一個列舉方法（例如 `all_stock_snapshots() -> Vec<RealtimeSnapshot>`），在 `RwLock` 讀鎖內一次複製後即釋放鎖。**不可**把排序與過濾寫在持鎖區間內，避免拉長對即時採集任務的鎖競爭；全市場約 2,000 筆小型 struct 的複製成本遠低於持鎖排序的代價。
- `src/interfaces/web/data_api/dto.rs`：新增 `MoversParams`、`MoverItem`、`MoversResponse` 與 `ToSchema`。
- `src/interfaces/web/data_api/handlers.rs`：新增 `market_movers` handler，內含 §3 的來源切換、兩條各自的取數路徑與共用的排序／截斷邏輯。
- `src/interfaces/web/data_api/mod.rs`：註冊路由並把 path 與 schema 加入 `ApiDoc`（既有 OpenAPI 測試會精確比對 path 數量，需同步更新斷言）。

### 5.2 stock_mcp_go

- `stock/models.go`：新增 `MarketMovers`、`Mover` 與 `MoversOptions`。
- `stock/apiclient.go`：新增 `MarketMovers(ctx, MoversOptions) (*MarketMovers, error)`，回傳完整 envelope（沿用 expanded-tools 計畫 §5.2 的實作決策）。
- `stock/tools.go`：新增 `get_market_movers`，`ReadOnlyHint: true`。
- 介面歸屬：加進既有的 `MarketDataQuerier`（同屬市場層級查詢），沿用型別斷言註冊，**不新增第五個介面**。
- `README.md`：工具數由 15 → 16，補上本工具的參數、回傳示例與時段語意說明。

## 6. 分期執行

分支策略：`stock_rust` 從最新 `main` 切 `feature/market-movers`，禁止直接 commit 到 `main`，PR 合併時機由使用者決定；`stock_mcp_go` 直接 commit 到 `main`，訊息標明 Phase 編號。

### Phase M0：資料檢核

1. 盤中實測 `SHARE.stock_snapshots` 的實際筆數與涵蓋範圍，確認「全市場」名副其實（HiStock 來源是排行榜頁，須驗證是否涵蓋全部上市櫃，而非僅前 N 名）。**這是本計畫唯一的真正風險點，必須在動工前於盤中時段實測。**
2. 對收盤來源查詢執行 `EXPLAIN (ANALYZE, BUFFERS)`，確認取最新一日全部列 ＋ top-N 排序的實際成本；只在證明需要時補索引。
3. 確認 `stocks` 表對快照內所有代號的覆蓋率，量化 §4.5 最後一條會排除多少檔。

### Phase M1：Rust endpoint

實作 §5.1，含單元測試與 `movers_endpoint_db_semantics` 整合測試。

### Phase M2：Go client ＋ MCP tool

實作 §5.2，含 `httptest.Server` 測試與 table-driven tool 測試。

### Phase M3：部署驗證

盤中與收盤後**各驗證一次**，確認同一個工具在兩個時段回傳正確的 `source`、`is_realtime` 與 `data_as_of`。

## 7. 測試重點（僅列與既有工具不同者）

- **來源切換**：快照非空 → `realtime`；快照為空 → `closing`；兩者皆空 → 404。以可注入的快取狀態測試，不依賴真實時鐘。
- **13:30～15:00 空窗**：`data_as_of` 為前一交易日時，摘要必須出現「今日收盤資料尚未產生」字樣。
- **null 語意**：即時來源的 `volume_shares`／`trade_value`／`transaction`／`last_close` 必須是 `null` 而非 `0`；收盤來源的 `volume_lots`／`source_site` 同理。
- **排序**：三種 `rank_by` 的方向正確，同值以 `stock_symbol ASC` 穩定排序。
- **過濾**：暫停上市、零成交量、快照有但 `stocks` 表無的代號都被排除。
- **`is_realtime` 例外**：本工具是唯一可能回 `true` 的分析型工具，需有測試固定此行為，避免日後有人「統一」成 `false`。

驗證命令沿用 expanded-tools 計畫 §7.1（Rust）與 §7.2（Go）。

## 8. 執行進度追蹤

> 更新規則同 expanded-tools 計畫 §10：完成標 `[x]` ＋ `yyyy-MM-ddTHH:mm:ss`，中斷標 `[~]` 並註明中斷點。

### Phase M0：資料檢核

- [~] M0-1 盤中實測快照筆數與全市場涵蓋率。
  - 中斷點：**必須在交易時段（週一至週五 09:00–13:30 台北時間）才能執行**，2026-07-31 開發時為深夜，即時快取為空（線上 `/api/v1/stocks/2330/realtime-snapshot` 回「目前非交易時段」）。實作本身不依賴此結果——涵蓋率不足只影響排行的完整度，不影響正確性。開盤後以 `get_market_movers`（`limit=50`）與 HiStock 排行頁比對即可完成。
- [x] M0-2 收盤來源查詢 `EXPLAIN (ANALYZE, BUFFERS)`。 2026-07-31T00:50:15
  - 以 `movers_closing_query_plan_uses_date_index` 整合測試執行。取最新交易日走 `"DailyQuotes_Date_include_symbol_idx"`（0.15ms）；三種排序鍵的排行查詢皆為該索引掃描 2,344 列 ＋ `stocks` Seq Scan（2,365 列）Hash Join ＋ top-N，實測 2.5–3.6ms。**不需補索引。**
- [~] M0-3 快照代號在 `stocks` 表的覆蓋率統計。
  - 中斷點：同 M0-1，需盤中資料。程式端已處理此情況：主檔查不到的代號一律排除並以 `tracing::warn!` 記錄前 10 個代號與總數，開盤後看 log 即可得到實際數字。

### Phase M1：Rust endpoint

- [x] M1-1 `infra/cache/snapshot.rs` 新增列舉方法 ＋ 測試。 2026-07-31T00:50:15
  - `Share::all_stock_snapshots()`：讀鎖內只做複製、鎖外才排序，避免拖慢盤中高頻寫入的採集任務。測試涵蓋空快取（非交易時段）與完整複製。
- [x] M1-2 `data_api` dto ＋ handler ＋ 路由 ＋ OpenAPI ＋ 測試。 2026-07-31T00:50:15
  - `GET /api/v1/market/movers`；OpenAPI path 數 16 → 17。OpenAPI 測試額外固定兩件事：`rank_by` 只有三值（**沒有** `top_trade_value`）、`is_realtime` 為不可 null 的布林。
- [x] M1-3 整合測試涵蓋兩種來源與 404。 2026-07-31T00:50:15
  - `movers_endpoint_db_semantics`（收盤來源、422、三種排序、市場過濾、單位與缺值語意）＋ `realtime_movers_filters_sorts_and_keeps_unit_semantics`（以假代號 `7997x` 操作記憶體快取，驗證過濾、同值排序與即時來源的 null 語意）。
  - commit：stock_rust `feature/market-movers` `6bcf5d7`。

### Phase M2：Go client ＋ MCP tool

- [x] M2-1 `apiclient.go` 方法 ＋ 測試。 2026-07-31T00:50:15
  - `MarketMovers` 回傳完整 envelope；測試涵蓋兩種來源的欄位與單位語意、query string 組裝、`movers: null` → `[]`，以及 404／401／422／500／無效 JSON／timeout。
- [x] M2-2 `tools.go` 工具 ＋ 測試 ＋ README 更新（15 → 16 個工具）。 2026-07-31T00:50:15
  - 掛進既有 `MarketDataQuerier`（不另開第五個介面）。測試涵蓋輸入驗證、大小寫正規化、預設值、三種來源摘要（含 `moversSourceSummary` 以固定時間點驗證空窗措辭）、`is_realtime: true` 的結構化輸出、空 envelope 不 panic，以及 `tools/list` 的 `ReadOnlyHint` 與描述內容。
  - commit：stock_mcp_go `main` `ade3649`。

### Phase M3：部署

- [ ] M3-1 盤中時段實測（`source: realtime`）。
- [ ] M3-2 收盤後實測（`source: closing`，`data_as_of` 為當日）。
- [ ] M3-3 13:30～15:00 空窗時段實測（`source: closing`，`data_as_of` 為前一交易日，摘要有加註）。
