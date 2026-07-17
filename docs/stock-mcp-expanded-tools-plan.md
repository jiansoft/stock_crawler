# 執行計畫：擴充 stock_mcp 台股查詢工具

- 狀態：執行中（Phase 0–3 已完成，Phase 4／部署待辦）
- 日期：2026-07-17
- 影響範圍：`stock_rust`（新增唯讀 Data API）、`stock_mcp_go`（新增 API client 方法與 MCP tools）
- 前置條件：現有五個 MCP tools 與 `/api/v1` Data API 維持可用
- 修訂：2026-07-17 依程式碼實況審查修訂——新增期間標記對映（§3.5）、`market` 參數對映（§3.6）、DB 欄位對照表（§4.2、§4.3）、`valuation_band` 分界定義（§4.4）、`screen_stocks` 最新一期語意（§4.7），並將驗證命令改為 repo 標準（§7.1）
- 修訂：2026-07-18 新增三個市場輔助工具（§4.8 指數歷史、§4.9 股利行事曆、§4.10 QFII 排行）與 `get_market_breadth` 的 `days` 序列參數，對應 Phase 4；新增保母級註解規範（§5.3）、執行進度追蹤（§10）、分支與版控策略（§6）

---

## 1. 背景與目標

目前 `stock_mcp_go` 已提供：

1. `search_stock`
2. `get_latest_daily_quote`
3. `get_price_history`
4. `get_stock_profile`
5. `get_realtime_snapshot`（僅 API client 具備快照能力時註冊）

現有工具涵蓋股票搜尋、價格、日線與基本資料，但 `stock_rust` 已儲存月營收、季／年度財報、股利、估值、殖利率排名、市場廣度、大盤指數與 QFII 持股等資料，尚未透過 MCP 提供。此計畫以既有資料為基礎，分期增加下列十個唯讀工具：

1. `get_monthly_revenue_history`
2. `get_financial_statement_history`
3. `get_dividend_history`
4. `get_stock_valuation`
5. `get_market_breadth`（含 `days` 序列參數）
6. `get_dividend_yield_ranking`
7. `screen_stocks`
8. `get_market_index_history`
9. `get_dividend_calendar`
10. `get_qfii_holding_ranking`

目標是讓 AI client 能回答營收趨勢、獲利能力、配息歷史、估值位置、市場整體強弱、大盤走勢、除權息行事曆、外資持股與條件選股等問題，同時維持資料來源可追溯、數值語意明確、API 契約穩定及查詢範圍有界。

## 2. 範圍與非目標

### 2.1 本計畫範圍

- 所有新增能力均先在 `stock_rust` 提供版本化 REST endpoint 與 OpenAPI 文件，再由 `stock_mcp_go` 包裝成 MCP tool。
- 所有 endpoint 只允許 `GET`，沿用 `Authorization: Bearer <DATA_API_KEY>`。
- 所有 MCP tools 均設定 `ReadOnlyHint: true`。
- 所有輸出同時包含繁體中文摘要與 `structuredContent`。
- 所有資料查詢必須接受 `context.Context`，沿用 API client 的明確 timeout。
- PostgreSQL 查詢必須參數化，限制回傳筆數，不接受呼叫端提供排序欄位或 SQL 片段。

### 2.2 非目標

- 不在此階段新增外部新聞、法人買賣、即時逐筆成交或技術指標資料源。
- 不提供買進、賣出、目標價或報酬保證；估值與篩選結果只能描述資料與計算結果。
- 不開放 `stock_ownership_details`、`daily_money_history*` 等會員持倉、成本與損益資料。現有共用 MCP API key 無法識別實際會員，直接開放會造成跨使用者資料外洩。
- 不為十個工具建立十套 service/repository 抽象。Rust 延續既有 `data_api` 模組；Go 在 `stock` package 內使用由 tool 消費端定義的小介面。
- 不在本次導入新的第三方套件或背景 goroutine。

## 3. 共通契約

### 3.1 資料格式

- JSON 欄位使用 `snake_case`。
- 資料庫 `NUMERIC` 對外使用 JSON number；無法安全轉成 `f64` 時回 `null` 並在 server log 記錄欄位與股票代號，不回傳內部錯誤細節。
- 真正缺值回 `null`，不可用 `0` 代替。資料表中本來就以 `0` 儲存的值維持 `0`，不得自行推斷成缺值。
- 日期使用 `YYYY-MM-DD`；月營收月份使用 `YYYY-MM`；timestamp 使用 UTC ISO 8601。
- 清單固定以資料日期由新到舊排序；排名固定以指標由高到低、股票代號由小到大作為同值排序。
- 所有 response 都包含 `data_as_of`。個股歷史資料取實際回傳資料的最新一期；市場或排行資料取實際統計日期；空清單時為 `null`。
- 所有分析型結果包含 `is_realtime: false` 與免責聲明：`本資料來自 stock_rust 已蒐集與計算的歷史資料，可能有延遲，僅供資訊參考，不構成投資建議。`

### 3.2 股票代號與資源不存在語意

- MCP 層統一對 `symbol` 執行 trim、轉大寫與 1–24 字元驗證；Data API 不重複正規化。
- 個股 endpoint 先確認 `stocks.stock_symbol` 存在：
  - 股票不存在：`404 {"error":"找不到股票代號"}`。
  - 股票存在但指定範圍沒有資料：`200`，回傳空陣列。
- Go API client 將個股 endpoint 的 `404` 轉為既有 `ErrStockNotFound`；tool 回傳 `找不到股票代號：{symbol}`。
- `401`、`5xx`、timeout、無法解析回應等錯誤只記入 server log；MCP 回傳既有安全訊息 `伺服器內部發生未預期錯誤，請稍後再試。`

### 3.3 查詢限制

- `limit` 沒提供時使用各工具預設值；超出範圍回 API `422`，MCP 在送出 API request 前先回 tool error。
- `from` 不可晚於 `to`。
- 不做游標分頁；歷史資料與排行已有明確上限，避免 LLM 一次取得過多 token。
- 查詢只使用既有索引能支援的固定排序。若實際執行計畫出現大範圍 sequential scan，必須先補精確索引，再開放 endpoint。

### 3.4 Response envelope

為避免 Rust 與 Go 各自猜測最外層 JSON 形狀，Data API 固定使用下列 envelope：

| 類型 | JSON 形狀 |
|---|---|
| 月營收 | `{"stock_symbol":"2330","data_as_of":"2026-06","revenues":[]}` |
| 財報 | `{"stock_symbol":"2330","data_as_of":"2026-Q1","statements":[]}` |
| 股利 | `{"stock_symbol":"2330","data_as_of":"2025-Q4","dividends":[]}` |
| 個股估值 | `{"stock_symbol":"2330","data_as_of":"2026-07-16","valuation":{...}}`；無資料時 `valuation:null`、`data_as_of:null` |
| 市場廣度 | `{"data_as_of":"2026-07-16","breadth":{...},"history":[]}`；`history` 恆為陣列（新到舊、共 `days` 筆），`breadth` 恆等於 `history[0]`，避免回應形狀隨參數改變 |
| 殖利率排行 | `{"data_as_of":"2026-07-16","stocks":[]}` |
| 條件選股 | `{"data_as_of":null,"stocks":[]}`；各指標日期放在每筆股票內，因混合資料沒有單一正確日期 |
| 指數歷史 | `{"data_as_of":"2026-07-17","points":[]}` |
| 股利行事曆 | `{"data_as_of":null,"events":[]}`；各事件日期放在每筆事件內 |
| QFII 排行 | `{"data_as_of":null,"stocks":[]}`；`stocks` 表快照沒有列級日期，見 §4.10 |

MCP `structuredContent` 在對應 envelope 外再加入 `data_kind`、`is_realtime` 與 `disclaimer`，不重新命名或重排內層資料欄位。空陣列必須序列化成 `[]`，不可變成 `null`。

期間排序固定如下：

- 月營收：`month DESC`。
- 財報：`year DESC`；同年度依 `A`、`Q4`、`Q3`、`Q2`、`Q1` 排序。使用 `period_type=quarterly` 時不會出現 `A`。
- 股利：`dividend_year DESC`；同年度依 `A`、`H2`、`H1`、`Q4`、`Q3`、`Q2`、`Q1` 排序，最後以 `paid_year DESC` 穩定排序。
- 個股估值與市場廣度：只回一筆最近有效資料。
- 殖利率排行與條件選股：依各工具指定的排序規則，最後一律以 `stock_symbol ASC` 打破同值排序。

### 3.5 期間標記與資料庫值對映

資料庫 `financial_statement.quarter` 與 `dividend.quarter` 以**空字串 `''` 代表年度資料**，實際值域為 `''`、`Q1`–`Q4`、`H1`、`H2`，**不存在 `A`**（證據：`financial_statement.rs` 年度匯總 EPS 寫入 `quarter = ''`；`yield_rank.rs` 以 `quarter = ''` 取年度股利；goodinfo 股利解析器全年度列 `quarter` 為空字串）。API 契約統一以 `A` 表示年度，實作必須做雙向對映：

- 輸出：DB `''` → API `A`；`Q1`–`Q4`、`H1`、`H2` 原樣輸出。
- 篩選：`period_type=annual` → SQL `quarter = ''`；`period_type=quarterly` → SQL `quarter IN ('Q1','Q2','Q3','Q4')`。
- 排序：期間順序必須以 `CASE quarter WHEN '' THEN ... END` 明確表達，不可假設字典序。
- `data_as_of` 期間格式值域：月營收 `YYYY-MM`；財報與股利取實際最新一期的期間標記，可能為 `YYYY-A`、`YYYY-H1`、`YYYY-H2`、`YYYY-Q1`–`YYYY-Q4`；日期型資料 `YYYY-MM-DD`。§3.4 表中的範例值僅為示意，完整值域以本節為準。

### 3.6 `market` 參數對映

`stocks.stock_exchange_market_id` 的實際值域（`core::declare::StockExchangeMarket`）：`1` 公開發行、`2` 上市（TWSE）、`4` 上櫃（TPEx）、`5` 興櫃；`daily_stock_price_stats` 另以 `0` 代表全市場合併統計列（由 `GROUPING SETS` 產生，實際存在）。三個使用 `market` 參數的工具底層語意不同，必須分別實作：

- `get_market_breadth`：查統計表既有統計列。`all` → `stock_exchange_market_id = 0`；`twse` → `= 2`；`tpex` → `= 4`。
- `get_dividend_yield_ranking`、`screen_stocks`：直接過濾 `stocks` 表，該表沒有 id `0` 列。`all` → `stock_exchange_market_id IN (2, 4)`；`twse` → `= 2`；`tpex` → `= 4`。**不含**公開發行（`1`）與興櫃（`5`）：興櫃缺乏可靠收盤價與估值資料，混入排行與選股會產生誤導性結果。此處刻意不沿用 `StockExchange` 把興櫃併入 TPEx 的內部慣例。

## 4. 新增工具契約

### 4.1 `get_monthly_revenue_history`

用途：查詢單一股票的月營收趨勢。

MCP input：

```json
{
  "symbol": "2330",
  "from": "2024-01",
  "to": "2026-06",
  "limit": 24
}
```

- `symbol`：必填。
- `from`、`to`：選填，格式 `YYYY-MM`。
- `limit`：選填，預設 24，範圍 1–120。

Data API：

```text
GET /api/v1/stocks/{symbol}/monthly-revenues?from=YYYY-MM&to=YYYY-MM&limit=24
```

資料來源：`"Revenue"`；資料庫 `Date` 為 `YYYYMM` 整數，API 必須轉成 `YYYY-MM`，不可直接暴露內部編碼。

每筆欄位：

- `month`
- `monthly_revenue`
- `last_month_revenue`
- `last_year_same_month_revenue`
- `monthly_accumulated_revenue`
- `last_year_monthly_accumulated_revenue`
- `month_over_month_percent`
- `year_over_year_percent`
- `accumulated_year_over_year_percent`
- `average_price`
- `lowest_price`
- `highest_price`

`structuredContent.data_kind` 使用 `monthly_revenue_history`，資料陣列欄位為 `revenues`。

### 4.2 `get_financial_statement_history`

用途：查詢單一股票的季／年度獲利能力與每股數據。

MCP input：

```json
{
  "symbol": "2330",
  "period_type": "quarterly",
  "limit": 12
}
```

- `period_type`：選填，允許 `quarterly`、`annual`、`all`，預設 `quarterly`。
- `limit`：選填，預設 12，範圍 1–40。
- `quarterly` 僅回 `Q1`–`Q4`；`annual` 僅回 `A`；`all` 全部回傳。期間標記的 DB 對映（年度在 DB 為空字串）見 §3.5。

Data API：

```text
GET /api/v1/stocks/{symbol}/financial-statements?period_type=quarterly&limit=12
```

資料來源：`financial_statement`。

每筆欄位：`year`、`quarter`、`gross_profit_margin`、`operating_profit_margin`、`pre_tax_income_margin`、`net_income_margin`、`net_asset_value_per_share`、`sales_per_share`、`earnings_per_share`、`profit_before_tax_per_share`、`return_on_equity`、`return_on_assets`、`updated_at`。

API 欄位與 DB 欄位名稱不同者對照如下（其餘同名）：

| API 欄位 | DB 欄位（`financial_statement`） |
|---|---|
| `gross_profit_margin` | `gross_profit`（值已是百分比） |
| `pre_tax_income_margin` | `"pre-tax_income"`（含連字號，SQL 需加引號） |
| `net_income_margin` | `net_income` |
| `profit_before_tax_per_share` | `profit_before_tax` |
| `updated_at` | `updated_time`（轉 UTC ISO 8601） |

`structuredContent.data_kind` 使用 `financial_statement_history`，資料陣列欄位為 `statements`。

### 4.3 `get_dividend_history`

用途：查詢單一股票的現金／股票股利、除權息日與發放日。

MCP input：

```json
{
  "symbol": "2330",
  "from_year": 2020,
  "to_year": 2026,
  "limit": 30
}
```

- `from_year`、`to_year`：選填，西元年 1990–目前年度加 1。
- `limit`：選填，預設 20，範圍 1–80。
- 年份篩選依 `year_of_dividend`，避免「股利所屬年度」與「實際發放年度」混淆。

Data API：

```text
GET /api/v1/stocks/{symbol}/dividends?from_year=2020&to_year=2026&limit=30
```

資料來源：`dividend`。

每筆欄位：`paid_year`、`dividend_year`、`quarter`、`cash_dividend`、`stock_dividend`、`total_dividend`、`earnings_cash_dividend`、`capital_reserve_cash_dividend`、`earnings_stock_dividend`、`capital_reserve_stock_dividend`、`cash_payout_ratio`、`stock_payout_ratio`、`total_payout_ratio`、`ex_dividend_date`、`ex_rights_date`、`cash_payable_date`、`stock_payable_date`、`updated_at`。

API 欄位與 DB 欄位名稱不同者對照如下（其餘同名；`quarter` 對映見 §3.5）：

| API 欄位 | DB 欄位（`dividend`） |
|---|---|
| `paid_year` | `year`（發放年度） |
| `dividend_year` | `year_of_dividend`（股利所屬年度） |
| `total_dividend` | `sum` |
| `cash_payout_ratio` | `payout_ratio_cash` |
| `stock_payout_ratio` | `payout_ratio_stock` |
| `total_payout_ratio` | `payout_ratio` |
| `ex_dividend_date` | `ex_dividend_date1` |
| `ex_rights_date` | `ex_dividend_date2` |
| `cash_payable_date` | `payable_date1` |
| `stock_payable_date` | `payable_date2` |
| `updated_at` | `updated_time`（轉 UTC ISO 8601） |

資料庫中的日期欄位型別為字串，空字串、`-`、`尚未公布` 等標記對外統一為 `null`；合法日期才輸出 `YYYY-MM-DD`。

`structuredContent.data_kind` 使用 `dividend_history`，資料陣列欄位為 `dividends`。

### 4.4 `get_stock_valuation`

用途：查詢指定股票最新或指定日期的估值區間。這是模型計算結果，不是目標價或投資建議。

MCP input：

```json
{
  "symbol": "2330",
  "date": "2026-07-16"
}
```

- `date`：選填，格式 `YYYY-MM-DD`。未提供時取該股票最新一筆估值；提供時取 `date <= 指定日期` 的最近一筆，避免非交易日固定回空。回溯上限 31 天：窗口內沒有資料視為無資料，不無限往回掃描（春節連假最長約 10 個交易日，31 天足以涵蓋）。

Data API：

```text
GET /api/v1/stocks/{symbol}/valuation?date=2026-07-16
```

資料來源：`estimate`。

回傳欄位：

- 基本欄位：`stock_symbol`、`date`、`closing_price`、`percentage`、`year_count`。
- 加權估值：`cheap`、`fair`、`expensive`。
- 價格法：`price_cheap`、`price_fair`、`price_expensive`。
- 股利法：`dividend_cheap`、`dividend_fair`、`dividend_expensive`。
- EPS 法：`eps_cheap`、`eps_fair`、`eps_expensive`。
- PBR 法：`pbr_cheap`、`pbr_fair`、`pbr_expensive`。
- PER 法：`per_cheap`、`per_fair`、`per_expensive`。
- `valuation_band`：由 `closing_price` 相對加權 `cheap/fair/expensive` 計算，值域為 `undervalued`、`fair_valued`、`overvalued`、`highly_overvalued`。分界必須與 `daily_stock_price_stats::upsert` 的統計 SQL 完全一致：`closing_price <= cheap` → `undervalued`；`cheap < closing_price <= fair` → `fair_valued`；`fair < closing_price <= expensive` → `overvalued`；`closing_price > expensive` → `highly_overvalued`。同一天同一股票的分類必須能與 `get_market_breadth` 的分布數字互相對帳。

股票存在但所選日期以前沒有估值資料時回 `200` 且 `valuation: null`。`structuredContent.data_kind` 使用 `stock_valuation`。

### 4.5 `get_market_breadth`

用途：查詢市場整體漲跌、均線位置及估值分布；`days > 1` 時回傳最近 N 個交易日的序列，供趨勢判讀（例如「近一個月上漲家數變化」）。

MCP input：

```json
{
  "market": "all",
  "date": "2026-07-16",
  "days": 1
}
```

- `market`：選填，允許 `all`、`twse`、`tpex`，預設 `all`；對映見 §3.6（統計表以 `0` 代表全市場合併列）。
- `date`：選填；未提供時取該市場最新資料，提供時取 `date <= 指定日期` 的最近一筆，回溯上限 31 天。
- `days`：選填，預設 1，範圍 1–60；以 `date`（或最新資料日）為終點，往前取最多 `days` 個**有統計資料的交易日**。實際存在的資料日不足 `days` 時回傳實際筆數，不補洞、不視為錯誤。

Data API：

```text
GET /api/v1/market/breadth?market=all&date=2026-07-16&days=1
```

資料來源：`daily_stock_price_stats`。

回傳欄位：`date`、`market`、`undervalued`、`fair_valued`、`overvalued`、`highly_overvalued`、`below_5_day_moving_average`、`above_5_day_moving_average`、`below_20_day_moving_average`、`above_20_day_moving_average`、`below_60_day_moving_average`、`above_60_day_moving_average`、`below_120_day_moving_average`、`above_120_day_moving_average`、`below_240_day_moving_average`、`above_240_day_moving_average`、`stocks_up`、`stocks_down`、`stocks_unchanged`、`updated_at`。

沒有任何可用統計資料時回 `404 {"error":"查無市場廣度資料"}`。回應形狀不隨 `days` 改變：`history` 恆為陣列（新到舊）、`breadth` 恆等於 `history[0]`（見 §3.4），`data_as_of` 取 `history[0].date`。`structuredContent.data_kind` 使用 `market_breadth`。

### 4.6 `get_dividend_yield_ranking`

用途：查詢指定日期、上市櫃市場或產業的殖利率排名。

MCP input：

```json
{
  "date": "2026-07-16",
  "market": "twse",
  "industry_id": 24,
  "limit": 20
}
```

- `date`：選填；未提供時取全表最新日期，提供時取 `date <= 指定日期` 的最近一個有資料交易日，回溯上限 31 天。
- `market`：選填，允許 `all`、`twse`、`tpex`，預設 `all`；對映見 §3.6（`all` 為上市＋上櫃，不含興櫃與公開發行）。
- `industry_id`：選填，正整數；以 `stocks.stock_industry_id` 過濾。未知或查無資料的 `industry_id` 回 `200` 空陣列，不做白名單驗證——與「條件合法但無資料」語意一致，且產業代碼未來可能增減。
- `limit`：選填，預設 20，範圍 1–50。

Data API：

```text
GET /api/v1/market/dividend-yield-ranking?date=2026-07-16&market=twse&industry_id=24&limit=20
```

資料來源：`yield_rank` JOIN `stocks` JOIN 對應的 `DailyQuotes` 與 `dividend`。

每筆欄位：`rank`、`stock_symbol`、`name`、`market_id`、`industry_id`、`date`、`closing_price`、`dividend`、`dividend_yield_percent`。

同一查詢條件沒有結果時回 `200` 空陣列；整張表沒有任何可用日期時才回 404。`structuredContent.data_kind` 使用 `dividend_yield_ranking`，資料陣列欄位為 `stocks`。

### 4.7 `screen_stocks`

用途：用有限且明確的條件篩選股票。這個工具只回傳資料符合項目，不替使用者做投資決策。

MCP input：

```json
{
  "market": "twse",
  "industry_id": 24,
  "valuation_band": "undervalued",
  "min_revenue_yoy_percent": 10,
  "min_eps": 5,
  "min_roe_percent": 10,
  "min_dividend_yield_percent": 3,
  "sort_by": "dividend_yield",
  "sort_order": "desc",
  "limit": 20
}
```

允許條件：

- `market`：`all`、`twse`、`tpex`，預設 `all`。
- `industry_id`：選填。
- `valuation_band`：選填，值域同估值工具。
- `min_revenue_yoy_percent`：選填，範圍 -100–10000。
- `min_eps`：選填，範圍 -10000–10000。
- `min_roe_percent`：選填，範圍 -10000–10000。
- `min_dividend_yield_percent`：選填，範圍 0–1000。
- `sort_by`：固定 enum `stock_symbol`、`revenue_yoy`、`eps`、`roe`、`dividend_yield`、`valuation_percentage`，預設 `stock_symbol`。
- `sort_order`：`asc` 或 `desc`，預設 `asc`。
- `limit`：預設 20，範圍 1–50。

Data API：

```text
GET /api/v1/stocks/screen?...固定白名單 query parameters...
```

查詢以**各股票自己**在每張資料表的最新一期為準（而非全表最新期）：`Revenue` 取該股最新月份、`financial_statement` 取該股最新 `Q1`–`Q4` 記錄、`estimate` 取該股最新日期、`yield_rank` 取該股最新日期。理由：月營收在每月 10 日前分批公布，若以全表最新月份為準，尚未公布的股票會整批消失；以各股最新一期為準的代價是不同股票的指標期間可能不同，因此每一個回傳項目必須附帶 `revenue_month`、`financial_period`、`valuation_date`、`yield_date`，使呼叫者知道各指標不是同一天資料。

為避免久未更新的資料被誤當現況參與篩選，各指標設新鮮度上限：`revenue_month` 距查詢日超過 3 個月、`financial_period` 超過 2 個季度、`valuation_date` 與 `yield_date` 超過 31 天者，該指標輸出 `null`，且以該指標為條件時該股票不符合篩選。

回傳欄位：`stock_symbol`、`name`、`market_id`、`industry_id`、`revenue_yoy_percent`、`earnings_per_share`、`return_on_equity`、`dividend_yield_percent`、`valuation_band`、`valuation_percentage`，以及上述各資料日期。

SQL 排序不可直接插入呼叫端字串。Rust handler 必須將 `sort_by + sort_order` 對應到程式內固定的 SQL 分支。至少提供一個篩選條件；完全沒有篩選條件時回 `422`，避免把工具當成無限制全市場資料匯出。`structuredContent.data_kind` 使用 `stock_screening_result`。

### 4.8 `get_market_index_history`

用途：查詢台股大盤指數（TAIEX）歷史走勢，回答「大盤最近表現如何」這類問題，與 `get_market_breadth` 互補（前者看指數點位，後者看內部強弱）。

MCP input：

```json
{
  "from": "2026-06-01",
  "to": "2026-07-17",
  "limit": 30
}
```

- `from`、`to`：選填，格式 `YYYY-MM-DD`；`from` 不可晚於 `to`。
- `limit`：選填，預設 30，範圍 1–365（與 `get_price_history` 慣例一致）。
- 不提供指數類別參數：endpoint 固定查 `category = 'TAIEX'`（`index` 表目前以 TAIEX 為主）。未來若要開放其他指數，以新增選填 query 參數的向後相容方式擴充，不預先設計。

Data API：

```text
GET /api/v1/market/index-history?from=2026-06-01&to=2026-07-17&limit=30
```

資料來源：`index`（`category = 'TAIEX'`）。

每筆欄位：

- `date`
- `index`（收盤指數）
- `change`（漲跌點數）
- `trade_value`（成交金額，元）
- `transaction`（成交筆數）
- `trading_volume`（成交股數）

排序 `date DESC`；`data_as_of` 取最新一筆 `date`。查無資料時回 `200` 空陣列（大盤指數沒有「代號不存在」的 404 語意）。`structuredContent.data_kind` 使用 `market_index_history`，資料陣列欄位為 `points`。

### 4.9 `get_dividend_calendar`

用途：查詢日期區間內除權息與股利發放的行事曆，回答「這個月有哪些股票要除息」、「2330 的股利何時發放」這類問題。

MCP input：

```json
{
  "from": "2026-07-01",
  "to": "2026-07-31",
  "event_type": "all",
  "limit": 50
}
```

- `from`：選填，預設查詢當日（台北時區）。
- `to`：選填，預設 `from + 30` 天。`to - from` 不可超過 92 天（一季），避免全表匯出。
- `event_type`：選填 enum：`ex_dividend`（除息）、`ex_rights`（除權）、`cash_payable`（現金股利發放）、`stock_payable`（股票股利發放）、`all`，預設 `all`。
- `limit`：選填，預設 50，範圍 1–200。

Data API：

```text
GET /api/v1/market/dividend-calendar?from=2026-07-01&to=2026-07-31&event_type=all&limit=50
```

資料來源：`dividend` 的四個日期欄位（DB 欄位對映同 §4.3：`ex_dividend_date1`、`ex_dividend_date2`、`payable_date1`、`payable_date2`）。

實作注意：這四個欄位在資料庫是**字串型別**，含空字串、`-`、`尚未公布` 等標記——只有可解析為合法日期且落在查詢區間內的列才輸出為事件，其餘一律排除。字串日期無法利用索引做範圍查詢；`dividend` 表每年僅數千筆，全表掃描成本可接受，但 Phase 0 仍須以 `EXPLAIN` 確認實際成本，必要時評估產生持久化的日期欄位（generated column）再開放。

每筆欄位：`stock_symbol`、`name`、`event_type`、`event_date`、`dividend_year`、`quarter`、`cash_dividend`、`stock_dividend`、`total_dividend`。同一列股利若同時有多個日期落在區間內，輸出多筆事件（每筆一個 `event_type`）。

排序：`event_date ASC`（行事曆語意，由近到遠）、同日 `stock_symbol ASC`。`data_as_of` 為 `null`（混合事件沒有單一統計日期，同 §3.4 條件選股的處理）。查無事件回 `200` 空陣列。`structuredContent.data_kind` 使用 `dividend_calendar`，資料陣列欄位為 `events`。

### 4.10 `get_qfii_holding_ranking`

用途：查詢外資（QFII）持股比例或持股數排行。這是**當前快照**：`stocks` 表只保存最近一次排程更新（每日 22:00 UTC）的數字，沒有歷史序列，無法回答趨勢問題——此限制必須寫入 tool 描述與回應摘要，避免 LLM 誤答「外資最近增減持」。

MCP input：

```json
{
  "market": "twse",
  "industry_id": 24,
  "sort_by": "percentage",
  "limit": 20
}
```

- `market`：選填，允許 `all`、`twse`、`tpex`，預設 `all`；對映見 §3.6（過濾 `stocks` 表，`all` 為上市＋上櫃）。
- `industry_id`：選填，語意同 §4.6。
- `sort_by`：固定 enum：`percentage`（`qfii_share_holding_percentage`）、`shares`（`qfii_shares_held`），預設 `percentage`；一律由高到低，同值以 `stock_symbol ASC` 穩定排序。
- `limit`：選填，預設 20，範圍 1–50。

Data API：

```text
GET /api/v1/market/qfii-holding-ranking?market=twse&industry_id=24&sort_by=percentage&limit=20
```

資料來源：`stocks`（`qfii_shares_held`、`qfii_share_holding_percentage`）。排除 `suspend_listing = true` 的股票與持股數為 0 的股票。

每筆欄位：`rank`、`stock_symbol`、`name`、`market_id`、`industry_id`、`qfii_shares_held`、`qfii_share_holding_percentage`、`issued_share`。

`data_as_of` 為 `null`：`stocks` 表沒有列級的 QFII 更新日期欄位，不可偽造一個。MCP 摘要必須改以文字說明「為最近一次每日更新的快照」。`structuredContent.data_kind` 使用 `qfii_holding_ranking`，資料陣列欄位為 `stocks`。

## 5. 架構與程式碼配置

### 5.1 stock_rust

延續現有 `src/interfaces/web/data_api`，不新增 layer package：

- `dto.rs`：新增 request params、response DTO 與 `ToSchema`。
- `handlers.rs`：新增七組 handler、參數驗證、參數化 SQL 與 row-to-DTO 轉換。
- `mod.rs`：註冊路由，將所有新 path 與 schema 加入 `ApiDoc`。

若 `handlers.rs` 因新增內容變得難以導覽，可依資料域拆成 `handlers/financial.rs`、`handlers/market.rs`、`handlers/screener.rs`；拆分只為可讀性，不建立額外 service/repository 層。

所有新增 Rust module、型別、函式與重要邏輯的註解要求見 §5.3。

### 5.2 stock_mcp_go

延續現有 `stock` package：

- `models.go`：新增 API/MCP 共用資料模型。
- `apiclient.go`：新增七個 HTTP 查詢方法，沿用既有 `get`／錯誤對應流程。
- `tools.go`：新增 tool input、JSON Schema、handler 與摘要組裝。
- `apiclient_test.go`、`tools_test.go`：新增對應測試。
- `README.md`：更新 tools 清單、參數、回傳示例與免責聲明。

為避免持續擴大現有四方法 `Querier`，新增能力依消費端分成小介面：

```go
type FinancialQuerier interface {
	MonthlyRevenueHistory(context.Context, string, RevenueHistoryOptions) (*MonthlyRevenueHistory, error)
	FinancialStatementHistory(context.Context, string, StatementHistoryOptions) (*FinancialStatementHistory, error)
	DividendHistory(context.Context, string, DividendHistoryOptions) (*DividendHistory, error)
}
```

實作決策（2026-07-18，Phase 1 落地時定案）：client 方法回傳**完整
envelope**（含 `stock_symbol`、`data_as_of` 與資料清單），而不是只回傳
內層清單。理由：`data_as_of` 必須由 stock_rust 伺服器端單一來源決定，
若 client 只拿清單、MCP 端自行重算最新一期，兩端邏輯遲早不一致。後續
Phase 的新介面也沿用此原則。

```go

type AnalyticsQuerier interface {
	StockValuation(context.Context, string, ValuationOptions) (*StockValuation, error)
	MarketBreadth(context.Context, MarketBreadthOptions) (*MarketBreadth, error)
	DividendYieldRanking(context.Context, YieldRankingOptions) ([]YieldRank, error)
}

type StockScreener interface {
	ScreenStocks(context.Context, ScreenOptions) ([]ScreenedStock, error)
}

type MarketDataQuerier interface {
	MarketIndexHistory(context.Context, IndexHistoryOptions) ([]IndexPoint, error)
	DividendCalendar(context.Context, CalendarOptions) ([]DividendEvent, error)
	QfiiHoldingRanking(context.Context, QfiiRankingOptions) ([]QfiiHolding, error)
}
```

`get_market_breadth` 的 `days` 參數放進 `MarketBreadthOptions`，不改 `AnalyticsQuerier` 方法簽名。

`AddTools` 依型別斷言註冊對應能力，方式與 `SnapshotQuerier` 一致。這可讓過渡期 DB repository 尚未實作新 API 時不暴露無法使用的工具，也避免要求單一大型介面由所有資料來源一次實作。

### 5.3 程式註解與文件規範（保母級標準）

本計畫所有新增或修改的程式碼，一律以「保母級」標準撰寫繁體中文註解——驗收基準是**第一次接觸本專案（甚至第一次接觸 Rust／Go／MCP）的新手，不需要追問任何人就能獨立看懂**：

- **Rust**：每個新增的 module、struct、enum、trait、函式都必須有 Rustdoc（module 用 `//!`，項目用 `///`），完整說明用途、參數、回傳值與錯誤語意（`# Errors` 段落）。handler、SQL 組裝、期間對映（§3.5）、日期回溯、`valuation_band` 分界計算等複雜邏輯，必須逐區塊加上行內註解。
- **Go**：每個 exported 型別、函式、介面都必須有 godoc 註解。tool 定義、輸入驗證、錯誤分層處理必須比照現有 `tools.go` 的風格——在註解中交代背景知識（例如 MCP 協定兩種錯誤層次的差異、介面為何定義在消費端而非實作端），讓新手能從註解本身學到「為什麼這樣設計」。
- **講「為什麼」，不只講「做了什麼」**：涉及台股領域知識之處（ROC 年份、除權息與發放日、`quarter` 空字串代表年度、市場 id 的興櫃排除），必須把領域背景寫進註解，不可假設讀者具備台股常識。
- **逐行／逐區塊註解的重點放在資料流與邊界條件**：每段 SQL 說明各綁定參數的意義與依賴的索引；每個轉換函式說明輸入輸出格式與 null 語意；每個魔術數字（回溯 31 天、新鮮度上限）說明數字的由來。
- **驗收約束**：code review 時，缺少上述註解視同功能未完成，與測試不通過同級，不得以「之後再補」通過審查。

## 6. 分期執行

### 分支與版控策略

兩個專案的版控方式不同，所有 Phase 一體適用：

- **`stock_rust`**：本計畫的所有變更一律在 `feature/data-api-v1` 分支內進行。
  - 開始實作前：從最新的 `main` 切出（`git checkout -b feature/data-api-v1`）；**分支已存在時直接 checkout 續作，不要刪除重建**（目前分支已存在且帶有 Data API 既有實作）。
  - **禁止直接 commit 到 `main`**。每個 Phase 完成後以 PR／merge 方式回 `main`；合併時機由使用者決定，LLM 不得自行合併。
- **`stock_mcp_go`**：不開分支，直接在 `main` 上 commit。
  - 該專案尚未 push 到 GitHub、沒有需要保護的遠端主線；repo 已完成初始化（`main` 上已有 commit），無需再 `git init`。
  - 之後所有變更直接 commit 到 `main` 即可；每個 Phase 完成時的 commit 訊息須標明對應的 Phase 編號，方便與 §10 進度清單互相對照。

1. 以實際資料確認 `Revenue.Date` 全部符合六位 `YYYYMM`。
2. 以 `SELECT DISTINCT quarter` 確認 `financial_statement.quarter`、`dividend.quarter` 的實際值域僅含 `''`、`Q1`–`Q4`、`H1`、`H2`（§3.5 的對映依據）；若出現其他值，先擴充 §3.5 對映再實作。
3. 統計股利日期欄位的空字串、`-`、`尚未公布` 與異常格式。
4. 對七類查詢執行 `EXPLAIN (ANALYZE, BUFFERS)`；只在證明需要時新增索引。
5. 將本文件的 response schema 落入 OpenAPI 測試，固定欄位名稱與 null 語意。

### Phase 1：個股歷史財務工具

實作：

1. `get_monthly_revenue_history`
2. `get_financial_statement_history`
3. `get_dividend_history`

順序：先完成三個 Rust endpoints 與 OpenAPI，再完成 Go API client、MCP tools 與 README。此階段不實作 screen，讓最直接且低耦合的歷史查詢先上線。

### Phase 2：估值與市場分析工具

實作：

1. `get_stock_valuation`
2. `get_market_breadth`（`days` 序列參數在此階段一次做完，避免之後改動已上線契約）
3. `get_dividend_yield_ranking`

上線前確認估值公式說明與資料日期均出現在 response／tool 摘要，且不得使用「建議買進」、「目標價」等措辭。

### Phase 3：條件選股

實作 `screen_stocks`。先以固定白名單條件與最大 50 筆上線，不提供任意運算式、任意欄位選擇或游標分頁。若查詢效能不符合既有 API timeout，縮減可組合條件或新增針對性索引，不提高 timeout 掩蓋問題。

### Phase 4：市場輔助工具

實作：

1. `get_market_index_history`
2. `get_dividend_calendar`
3. `get_qfii_holding_ranking`

三者彼此獨立、與前三個 Phase 也無依賴，可視額度提前或並行實作；排在最後只因價值密度較低。上線前確認 QFII 排行的「快照、無歷史」限制清楚出現在 tool 描述與摘要中。

## 7. 測試與驗收

### 7.1 stock_rust

- 參數測試：合法值、缺省值、格式錯誤、上下界、`from > to`、未知 enum、screen 無篩選條件。
- 資源語意：未知股票 404；已知股票但無資料 200 空陣列／`null`。
- 轉換測試：`YYYYMM → YYYY-MM`、股利無效日期 → `null`、Decimal 轉換失敗 → `null`。
- 排序測試：歷史資料新到舊；排行同值時股票代號穩定排序；行事曆 `event_date ASC`。
- 行事曆測試：字串日期解析（空字串、`-`、`尚未公布` 排除）、同列多日期產生多事件、區間上限 92 天。
- 廣度序列測試：`days` 上下界、資料日不足 `days`、`breadth` 恆等於 `history[0]`。
- QFII 排行測試：`suspend_listing` 與零持股排除、兩種 `sort_by`。
- 安全測試：401 不查資料庫；錯誤 response 不含 SQL、主機或堆疊。
- OpenAPI 測試：十個 path、所有 query params、security scheme 與 response schema 均存在。
- DB 整合測試：每類至少覆蓋正常資料、空結果與指定日期落在非交易日／無資料日。

驗證命令：

與 repo CI gate 一致（`--test-threads=1` 是必要的：多數整合測試共用資料庫狀態）：

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked --features integration-tests -- --test-threads=1
cargo doc --no-deps
```

### 7.2 stock_mcp_go

- `httptest.Server` 覆蓋每個 endpoint 的 200、404、401、422、500、timeout、無效 JSON。
- table-driven tool tests 覆蓋輸入正規化、預設值、上下界、摘要、`structuredContent` 與安全錯誤訊息。
- `tools/list` 驗證只有 injected client 實作對應能力時才註冊該組 tools。
- 驗證所有 tool 都有 `ReadOnlyHint: true`。
- 驗證所有分析型 response 都有 `data_kind`、`data_as_of`、`is_realtime: false`、`disclaimer`。
- 對同一 fixture 比對 Data API JSON 與 MCP `structuredContent`，數值及 null 語意一致。

驗證命令：

```text
gofmt -l .
go vet ./...
go test ./...
```

### 7.3 端對端驗收情境

1. 查 2330 最近 24 個月營收，月份與年增率順序正確。
2. 分別查 2330 季報與年報，`period_type` 不混入其他類型。
3. 查 2330 歷年股利，未公布日期為 `null`。
4. 查週末日期的個股估值與市場廣度，回傳該日期以前最近一個有資料日。
5. 查上市半導體殖利率前 20 名，結果符合日期、市場與產業條件。
6. 以營收年增、EPS、ROE、殖利率及估值組合篩選，結果每筆都附各資料來源日期。
7. 查不存在股票，三個個股歷史工具與估值工具都回一致的 tool error。
8. 關閉 `stock_rust`，每個新 tool 都回安全通用錯誤且 MCP server 不 crash。
9. 查最近 30 個交易日大盤指數走勢，點位與漲跌順序正確。
10. 查本月除權息行事曆，事件依日期由近到遠且不含無效日期標記。
11. 查上市外資持股比例前 20 名，摘要明確標示為快照、非歷史趨勢。
12. 以 `days=20` 查市場廣度序列，`breadth` 與 `history[0]` 一致。

## 8. 文件、相容性與部署

- 新增 endpoint 屬 `/api/v1` 向後相容擴充，不改動既有 endpoint 與五個 MCP tool 的輸出。
- `stock_rust` 必須先部署新 endpoints；確認 Swagger UI 可測後，才部署註冊新 tools 的 `stock_mcp_go`。
- OpenAPI JSON 是 Data API 契約事實來源；MCP README 只描述 tool 對外契約，不複製 SQL 細節。
- 部署後觀察各 endpoint latency、5xx 與 response 大小；screen 與 ranking 額外觀察 PostgreSQL query duration。
- 若需 rollback，只回退 `stock_mcp_go` 的新 tool 註冊即可；保留已上線的唯讀 Data API 不影響舊 client。

## 9. 完工檢查清單

### Phase 1

- [x] 三個個股歷史 endpoints 可在 Swagger UI 測試。 2026-07-18T00:36:33（OpenAPI paths/schemas 有測試固定；實際開 Swagger UI 抽查列入 D-1 部署驗證）
- [x] 三個 MCP tools 出現在 `tools/list` 並回正確 `structuredContent`。 2026-07-18T00:36:33（in-memory MCP client 往返測試）
- [x] 未知股票與已知股票空資料語意一致且有測試。 2026-07-18T00:36:33（Rust `phase1_endpoints_db_semantics` ＋ Go client/tool 測試兩側皆覆蓋）
- [x] 月份、財報期間、股利日期轉換有 deterministic tests。 2026-07-18T00:36:33
- [x] §3.5 期間標記對映（DB `''` ↔ API `A`）與 `data_as_of` 格式值域有 deterministic tests。 2026-07-18T00:36:33（`quarter_mapping_follows_section_3_5`、`month_conversion_roundtrip_and_validation`）

### Phase 2

- [x] 估值 response 清楚標示計算結果與免責聲明。 2026-07-18T01:44:11
- [x] 市場廣度支援 all／twse／tpex 與最近有效日期（含 31 天回溯上限）。 2026-07-18T01:44:11
- [x] `market` 對映依 §3.6：廣度查統計列（含 `0`），排行／選股過濾 `stocks`（`all` = 上市＋上櫃）。 2026-07-18T01:44:11
- [x] 殖利率排行支援日期、市場、產業、limit，排序穩定。 2026-07-18T01:44:11
- [x] 三個工具不出現買賣建議或保證性描述。 2026-07-18T01:44:11

### Phase 3

- [x] screen 至少要求一個篩選條件，最多回 50 筆。 2026-07-18T01:44:11
- [x] sort 欄位與方向使用白名單，沒有動態 SQL 注入風險。 2026-07-18T01:44:11
- [x] 每筆結果帶有各指標資料日期。 2026-07-18T01:44:11
- [x] 查詢在既有 API timeout 內完成，執行計畫沒有不可接受的全表掃描。 2026-07-18T01:44:11（殖利率複合索引後 metric-sort screen 約 134ms）

### Phase 4

- [ ] 指數歷史 endpoint 固定 TAIEX、排序新到舊、`from > to` 回 422。
- [ ] 股利行事曆四種 `event_type`、區間上限 92 天、無效日期標記不產生事件。
- [ ] QFII 排行排除暫停上市與零持股，tool 描述與摘要標明快照語意。
- [ ] 市場廣度 `days` 序列與單日查詢語意一致（`breadth` = `history[0]`）。

### 全部完成

- [ ] `stock_rust` 的 fmt、clippy、test、doc 全部通過。
- [ ] `stock_mcp_go` 的 gofmt、vet、test 全部通過。
- [ ] 所有新增程式碼符合 §5.3 保母級註解規範（Rustdoc／godoc ＋ 逐區塊繁體中文說明）。
- [ ] Swagger UI、OpenAPI、README 與實際 tools/list 一致。
- [ ] 新功能沒有暴露會員持倉、成本、損益或任何憑證。

## 10. 執行進度追蹤（接手指南）

> **用途**：LLM 額度中斷或換人接手時，先讀本節即可掌握目前做到哪裡，不需重新審視整個專案。
>
> **更新規則**（每完成一項立即更新，不可事後補記）：
> 1. 完成的項目把 `[ ]` 改為 `[x]`，並在項目尾端附完成時間，格式 `yyyy-MM-ddTHH:mm:ss`（依 repo 的 code-review 文件慣例）。
> 2. 做到一半被中斷的項目改標 `[~]`，並在項目下一行縮排註明中斷點：改到哪個檔案、哪個函式、哪個測試還沒過。
> 3. 刻意跳過或改變順序的項目，註明原因與日期。
> 4. 接手者開工前先跑 §7.1 驗證命令確認基線是綠的，再從第一個未完成項目繼續。

### Phase 0：契約與資料品質

- [x] P0-1 驗證 `Revenue.Date` 全為六位 `YYYYMM`。 2026-07-18T00:15:05
  - 結果：218,519 筆全數合法，範圍 `201201`–`202606`，異常 0 筆。
- [x] P0-2 `SELECT DISTINCT quarter` 驗證 `financial_statement`／`dividend` 值域符合 §3.5。 2026-07-18T00:15:05
  - 結果：`financial_statement` = `''`(20869)、`Q1`–`Q4`（無 `H1/H2`）；`dividend` = `''`(45946)、`H1`(368)、`H2`(620)、`Q1`–`Q4`。與 §3.5 完全一致，年度＝空字串已證實。
- [x] P0-3 統計股利日期欄位的無效標記種類與數量。 2026-07-18T00:15:05
  - 結果：主要為 `-`（1.7 萬～4 萬筆／欄）與 `尚未公布`（數百筆／欄）。**另發現 `ex-dividend_date1` 有 10 筆殖利率字串（如 `1.39%`）的髒資料**——「僅合法 `YYYY-MM-DD` 才輸出、其餘一律 `null`」的規則可正確處理，無需清資料。
  - 注意：實際 DB 欄名為 `"ex-dividend_date1"`、`"ex-dividend_date2"`（含連字號，SQL 需加引號），§4.3 對照表的 DB 欄名以此為準。
- [~] P0-4 對十類查詢執行 `EXPLAIN (ANALYZE, BUFFERS)`，記錄是否需補索引。 2026-07-18T01:44:11（Phase 1–3 共七類已完成；Phase 4 三類待辦）
  - Phase 1 三類結果：`Revenue` 走 `Revenue_SecurityCode_Date-uidx` 反向掃描（4.5ms）；`financial_statement` 走 `(security_code, year, quarter)` 唯一索引（0.14ms）；`dividend` 走 pkey bitmap ＋ top-N 排序（66 列、0.11ms）。**三者皆不需補索引**。
  - Phase 2/3 結果：估值約 0.03ms、廣度約 0.19ms、殖利率排行約 28ms；metric-sort screen 原約 1,891ms，新增 `yield_rank (security_code, date DESC) INCLUDE (yield)` 後約 134ms（約 14.1 倍），planner 已使用新索引。migration：`migration_20260718_yield_rank_latest_by_stock_index.sql`。
  - Phase 4 尚待指數／行事曆／QFII 三類查詢。
- [x] P0-5 response schema 落入 OpenAPI 測試，固定欄位名稱與 null 語意。 2026-07-18T00:27:24
  - Phase 1–3 已改為依 path 精確驗證 response schema、query 限制、nullable／array item、security 與 500；Phase 4 依相同模式續增。

### Phase 1：個股歷史財務工具

- [x] P1-1 Rust `monthly-revenues` endpoint（dto ＋ handler ＋ 路由 ＋ OpenAPI ＋ 測試）。 2026-07-18T00:27:24
- [x] P1-2 Rust `financial-statements` endpoint（同上，含 §3.5 期間對映）。 2026-07-18T00:27:24
  - 實作注意：DB 欄名為 `"pre-tax_income"`（含連字號），SQL 已用 alias 對映。
- [x] P1-3 Rust `dividends` endpoint（同上，含日期標記轉 null）。 2026-07-18T00:27:24
  - 三個 endpoint 的驗證：fmt／clippy `-D warnings`／單元測試 316 passed 全綠；`phase1_endpoints_db_semantics` 整合測試對實際資料庫驗證 404／200 空陣列／422 語意通過。
- [x] P1-4 Go API client：三個查詢方法 ＋ `apiclient_test.go`。 2026-07-18T00:36:33
  - 方法回傳完整 envelope（見 §5.2 實作決策）；404 → `ErrStockNotFound`；測試覆蓋 200／404／5xx／無效 JSON／query string 組裝／空陣列語意。
- [x] P1-5 Go MCP tools：三個工具 ＋ `tools_test.go` ＋ README 更新。 2026-07-18T00:36:33
  - `FinancialQuerier` 型別斷言註冊（db 模式 4 工具、api 模式 8 工具，`tools/list` 測試驗證）；table-driven 驗證測試；`AnalysisDisclaimer` 出現在所有摘要與 structuredContent。gofmt／vet／test 全綠。
  - commit：stock_mcp_go `main` `4f39d82`。
- [x] P1-6 §9 Phase 1 完工清單全數勾選。 2026-07-18T00:36:33

### Phase 2：估值與市場分析工具

- [x] P2-1 Rust `valuation` endpoint（含 §4.4 分界與 31 天回溯）。 2026-07-18T01:44:11
- [x] P2-2 Rust `market/breadth` endpoint（含 `days` 序列）。 2026-07-18T01:44:11
- [x] P2-3 Rust `dividend-yield-ranking` endpoint。 2026-07-18T01:44:11
- [x] P2-4 Go API client：`AnalyticsQuerier` 三方法 ＋ 測試。 2026-07-18T01:44:11
- [x] P2-5 Go MCP tools：三個工具 ＋ 測試 ＋ README 更新。 2026-07-18T01:44:11
- [x] P2-6 §9 Phase 2 完工清單全數勾選。 2026-07-18T01:44:11

### Phase 3：條件選股

- [x] P3-1 Rust `stocks/screen` endpoint（白名單排序、至少一個條件、各指標日期與新鮮度上限）。 2026-07-18T01:44:11
- [x] P3-2 Go `StockScreener` ＋ tool ＋ 測試 ＋ README 更新。 2026-07-18T01:44:11
- [x] P3-3 §9 Phase 3 完工清單全數勾選。 2026-07-18T01:44:11

### Phase 4：市場輔助工具

- [ ] P4-1 Rust `market/index-history` endpoint。
- [ ] P4-2 Rust `market/dividend-calendar` endpoint。
- [ ] P4-3 Rust `market/qfii-holding-ranking` endpoint。
- [ ] P4-4 Go `MarketDataQuerier` 三方法 ＋ 三個 tools ＋ 測試 ＋ README 更新。
- [ ] P4-5 §9 Phase 4 完工清單全數勾選。

### 部署

- [ ] D-1 `stock_rust` 部署，Swagger UI 驗證所有新 endpoint。
- [ ] D-2 `stock_mcp_go` 部署，`tools/list` 驗證新工具全數註冊。
- [ ] D-3 部署後觀察 latency、5xx、response 大小與 PostgreSQL query duration（§8）。
