# 執行計畫:stock_mcp_go 資料來源由「直連資料庫」改為「stock_rust WebAPI」

- 狀態:進行中（Phase 1–4 實作與驗證中）
- 日期:2026-07-13
- 影響範圍:`stock_rust`(新增唯讀 Data API)、`stock_mcp_go`(替換資料存取層)
- 不影響:MCP tool 的對外行為與輸出格式(四個 tool 的摘要、structuredContent、錯誤訊息維持不變)

---

## 0. 實作執行規範(執行本計畫前必讀;本節以 AI agent 可直接遵循的指令形式撰寫)

### 0.1 分支規範

- **stock_rust:本計畫的所有變更一律在分支 `feature/data-api-v1` 內進行。**
  - 開始實作前:從最新的 `main` 切出 `git checkout -b feature/data-api-v1`;若分支已存在則直接 checkout 續作,不要重建。
  - 禁止直接 commit 到 `main`。Phase 完成後以 PR / merge 回 `main`,合併時機由使用者決定。
- **stock_mcp_go:不開分支,直接在 `main` 上進行。** 該專案尚未 push 到 GitHub、沒有需要保護的遠端主線;開始 Phase 2 前先 `git init` 並建立初始 commit,之後所有變更直接 commit 到 `main` 即可。

### 0.1a Commit 作者規範(嚴格禁止事項)

**所有 commit 訊息一律不得包含任何 Claude / AI 助理的掛名或署名資訊**——不可出現 `Co-Authored-By: Claude`、`Generated with Claude Code`、模型名稱(Claude、Sonnet、Opus 等)或任何等價的 AI 屬名字樣,無論是在 commit message 內文或 trailer。

- 這條規則**覆蓋**一般 Claude Code 預設會在 commit 訊息附加 `Co-Authored-By: Claude ...` 的行為;在 stock_rust 與 stock_mcp_go 這兩個專案內,該預設行為必須關閉。
- commit 作者(`git config user.name` / `user.email`)與訊息內容,一律只呈現實際操作者(使用者)的身分,不呈現任何 AI 協作痕跡。
- 這是硬性規則,不因分支、Phase 或專案而有例外。

### 0.2 註解品質規範(rustdoc / godoc,保母級標準)

**觸發條件:實作本計畫的過程中,凡是「讀到(開啟閱讀)、異動、新增」的每一個 `.rs` 與 `.go` 檔案,都必須執行本節檢查。** 不達標時在同一分支內補寫;「只讀到、未異動」的檔案補註解可獨立成 commit(訊息註明 docs only),避免與功能變更混雜。

達標定義(以下每一條都必須滿足):

1. **模組/套件層級**:檔案開頭有模組說明(Rust 用 `//!`,Go 用 package 註解),回答三個問題:這個檔案是什麼、為什麼存在、與其他模組的關係。
2. **對外項目全覆蓋**:每個 Rust `pub` 項目、Go exported 識別字都有文件註解,說明用途、參數、回傳值與錯誤情況(Go 註解須以識別字開頭,符合 godoc 慣例)。
3. **逐區塊、必要時逐行**:函式內部依邏輯區塊加註解;不直覺的單行(特殊 API 行為、易混淆語法、非顯而易見的邊界處理)要逐行說明。
4. **「為什麼」優先於「做什麼」**:程式碼本身已表達「做什麼」時,註解要補「為什麼這樣做」「不這樣做會發生什麼問題」。
5. **新手視角**:假設讀者是第一天接觸該語言的新手。語言特有概念(Rust 的所有權、`?` 運算子、`Option`/`Result`;Go 的 goroutine、channel、介面隱式實作、指標型別表達 NULL 等)在該檔案首次出現時,用一兩句話解釋。領域術語(參數化查詢、LEFT JOIN 的 NULL 語意、SSE、statement timeout、常數時間比較……)同樣要解釋。
6. **品質標竿**:以 `stock_mcp_rust/src/repository.rs` 的教學式註解為基準——它示範了「概念解說 + 設計理由 + 邊界行為」三個層次都要寫到。
7. **語言**:註解一律繁體中文;程式識別字與技術名詞保留原文。

驗收方式:

- Rust:`cargo doc --no-deps` 零警告;本計畫新增/異動的檔案在啟用 `#![warn(missing_docs)]` 時零警告。
- Go:`gofmt -l .` 無輸出、`go vet ./...` 通過,並逐一確認 exported 識別字的 godoc 覆蓋。
- 第 6 節每個 Phase 的完工檢查都包含本節規範,未達標不得視為完工。

---

## 1. 背景與目標

### 現況

`stock_mcp_go` 直接連 `stock_rust` 的 PostgreSQL,耦合點是資料庫 schema:

```text
stock_mcp_go ──(pgx,唯讀 SQL)──▶ PostgreSQL ◀──(寫入)── stock_rust
```

任何 schema 變動都可能在沒有編譯期警告的情況下弄壞 MCP 端;此外 MCP 端需要持有資料庫帳號,增加一組憑證的管理與外洩面。

### 目標

`stock_rust` 成為唯一的資料提供者,對內提供**版本化、有文件、可線上測試**的唯讀 WebAPI;`stock_mcp_go` 改為 API client,不再持有任何資料庫憑證:

```text
stock_mcp_go ──(HTTP + API key)──▶ stock_rust Data API ──▶ PostgreSQL
```

耦合點從「資料庫 schema」變成「明確的 API 契約(OpenAPI)」。

---

## 2. 技術路線選擇

需求:「走什麼路就要提供相應的文件可供線上測試與查看(像 Swagger 那樣)」。三條路線與各自的線上文件方案:

| 路線 | 線上測試文件 | 評估 |
|---|---|---|
| **REST + OpenAPI(建議)** | Swagger UI(`utoipa` + `utoipa-swagger-ui`) | stock_rust 已用 axum 0.8,utoipa 生態直接支援;Go 端可用 `oapi-codegen` 從 openapi.json 產生 client,契約變動在編譯期就會發現 |
| gRPC | grpcui / Buf Studio(需開 server reflection) | 效能佳但雙端都要引入 protobuf 工具鏈;查詢型內部 API 用不到 streaming,killer feature 不成立 |
| GraphQL | GraphiQL / Apollo Sandbox | 彈性查詢對「固定四種查詢」是過度設計,Rust 端(async-graphql)導入成本最高 |

**決定:REST + OpenAPI 3,Swagger UI 作為線上文件。** 理由:與既有 axum web 層無縫整合、四個查詢語意本來就是資源導向、Go/Rust 兩端工具鏈都成熟。

選用套件(stock_rust 端):

- `utoipa`(OpenAPI derive,以 `#[derive(ToSchema)]`、`#[utoipa::path]` 標注)
- `utoipa-axum`(router 與 OpenAPI 同步註冊,避免路由與文件不一致)
- `utoipa-swagger-ui`(Swagger UI 靜態資源內嵌)

---

## 3. API 契約設計

### 3.1 原則

- 掛在既有 axum web server 上,新增 `interfaces/web/data_api` 模組,路徑前綴 **`/api/v1`**。
- **唯讀**:只有 GET,重用(或平移)現有唯讀查詢,禁止任何寫入。
- 欄位命名、NULL 語意、日期格式**逐欄對齊** `stock_mcp_go/stock/models.go` 現行輸出:snake_case、缺值一律 JSON `null`(絕不以 0 取代)、日期 `YYYY-MM-DD`、timestamp 為 UTC ISO 8601。
- 錯誤回應統一 `{"error": "<訊息>"}`,不洩漏 SQL、主機、堆疊。
- 驗證:`Authorization: Bearer <DATA_API_KEY>`(內部服務間金鑰,與 MCP 對外的 `MCP_API_KEY` 是兩把不同的 key);`/api/v1/healthz` 與 Swagger UI 免驗證(僅綁內網位址)。

### 3.2 Endpoints(對應 `stock.Querier` 的四個方法)

| Endpoint | 對應 Querier 方法 | 成功回應 | 查無資料 |
|---|---|---|---|
| `GET /api/v1/stocks/search?query=&limit=` | `SearchStock` | `{"stocks":[Stock]}` | `200`,空陣列 |
| `GET /api/v1/stocks/{symbol}/latest-quote` | `LatestDailyQuote` | `{"stock":Stock,"quote":DailyQuote\|null}` | `404`(股票不存在) |
| `GET /api/v1/stocks/{symbol}/price-history?from=&to=&limit=` | `PriceHistory` | `{"quotes":[HistoricalQuote]}` | 股票代號不存在:`404`;股票存在但指定區間查無資料:`200` 空陣列(見下方語意修正說明) |
| `GET /api/v1/stocks/{symbol}/profile` | `StockProfile` | `StockProfile` 物件 | `404`(股票不存在) |
| `GET /api/v1/healthz` | — | `{"status":"ok"}` | — |

語意細節,原則是「HTTP 狀態碼要反映資源是否真實存在,而不是照抄舊 SQL 恰好的行為」——如果為了「MCP 層零改動」而遷就一個非刻意設計、只是舊查詢沒做存在性檢查所產生的副作用,屬於因小失大;下面 `price-history` 這一項刻意修正,且**允許、也要求**同步調整 MCP 層(`stock_mcp_go`)對應的程式碼,不是只能改 stock_rust 這一側:

- `search`:`query` 1–100 字元、`limit` 預設 10(1–50);代號完全符合排最前。這是「集合搜尋」語意,查無符合關鍵字本來就該回 `200` 空陣列,不牽涉「資源是否存在」的判斷,維持不變。
- `latest-quote`:股票代號不存在 → `404`;股票存在但沒有日報價 → `200` 且 `quote: null`。不變。
- `price-history`(**語意修正,不再沿用舊 SQL 行為**):股票代號不存在 → `404`,跟 `latest-quote`/`profile` 一致;股票存在、但指定日期區間內查無歷史資料 → `200` 空陣列。

  舊行為(`stock_mcp_go/stock/repository.go` 目前的 `PriceHistory`)是直接對 `"DailyQuotes"` 下 `WHERE stock_symbol = $1`,從未檢查 `stocks` 表裡這個代號到底存不存在,所以「代號打錯字」跟「代號正確但這段期間沒交易資料」兩種完全不同的情況,回應長得一模一樣(都是空陣列)——這不是刻意設計,只是原始 SQL 圖方便的副作用,規格書當初把它寫成「跟現行行為一致」只是為了遷移時省事,不是這個副作用本身有價值。這次剛好透過遷移的機會一併修正,讓四個工具在「資源不存在」這件事上的語意一致,呼叫端(不論是人或 LLM)看到 404 就知道「代號打錯了」,看到 200 空陣列就知道「代號沒錯,只是這段時間沒資料」,不需要再去猜。
- `profile`:`quote_history_record.security_code = stocks."SecurityCode"` 的關聯假設維持不變。
- symbol 由呼叫端(MCP)先 trim + 轉大寫後再送出,API 端不再二次正規化(單一事實來源)。

**這個修正必須同時套用到兩個地方,不能只改一邊:**

1. **stock_rust 的新 API endpoint**(Phase 1):`price-history` handler 在查歷史資料前,先以 `stock_symbol` 對 `stocks` 表做一次存在性檢查(主鍵查詢,成本很低),不存在就回 `404`;存在才繼續查歷史區間,查無資料回 `200` 空陣列。
2. **`stock_mcp_go` 的 DB 直連模式**(`stock/repository.go` 的 `PriceHistory` 方法,現在就可以先改,不用等到 Phase 2):比照 `LatestDailyQuote`/`StockProfile` 已經在用的手法,先確認股票代號存在,不存在時回傳一個能被上層識別成「找不到股票」的訊號,而不是直接回傳空 slice。建議做法:比照 `errors.Is(err, pgx.ErrNoRows)` 這個既有慣例,定義一個套件層級的 sentinel error(例如 `stock.ErrStockNotFound`),`PriceHistory` 在代號不存在時回傳 `(nil, ErrStockNotFound)`,代號存在但查無資料時維持回傳 `([]HistoricalQuote{}, nil)`。`tools.go` 的 `priceHistory` handler 對應改成:偵测到 `ErrStockNotFound` 時回傳跟 `latestDailyQuote`/`stockProfile` 同樣格式的 tool error「找不到股票代號:%s」,而不是現在這種「不管代號存不存在,查無資料就統一顯示一段空陣列摘要」的寫法。
3. 兩邊都改的原因:Phase 3 要求「同一批查詢在 db/api 兩模式輸出 JSON 必須逐 byte 等價」——如果只修 API 這一側、DB 直連模式維持舊行為,「查一個不存在的代號」這個案例在兩個模式下會給出不同的 HTTP 語意(一個 404、一個空陣列),等價驗證永遠對不上,也會讓「哪個才是正確行為」變得曖昧。同步修正後,Phase 3 驗證的是「兩邊都實作了同一套正確語意」,而不是「兩邊都複製了同一個舊副作用」。
4. 受影響的檔案(供之後執行時對照):`stock_mcp_go/stock/repository.go`(新增存在性檢查與 sentinel error)、`stock_mcp_go/stock/tools.go`(`priceHistory` handler 判斷邏輯、可能需要調整 `Querier` 介面裡 `PriceHistory` 的說明註解)、對應的 `repository_test.go`/`tools_test.go`(補上「未知代號」與「代號存在但查無資料」兩種情境分開驗證)、`stock_rust` 新 endpoint 的 handler 與測試、以及兩份 README 裡 `get_price_history`/`price-history` 的行為說明。

### 3.3 文件與線上測試入口(stock_rust)

| 路徑 | 內容 |
|---|---|
| `/swagger-ui` | Swagger UI,可直接在瀏覽器對每個 endpoint 試打(含 Authorize 按鈕輸入 Bearer key) |
| `/api-docs/openapi.json` | OpenAPI 3 規格檔,**同時是 Go client 的 codegen 來源** |

OpenAPI 中以 `components.securitySchemes` 宣告 bearer auth,讓 Swagger UI 的「Authorize」可以直接測授權路徑。

### 3.4 即時報價快照 endpoint(Phase 4 選配,契約先定義於此)

stock_rust 盤中在記憶體(`infra/cache/share.rs` 的 `SHARE.stock_snapshots`)持有由 HiStock/Yahoo/cmoney 等站點採集的近即時報價;直連 DB 看不到這份資料,改走 API 後才有辦法對外提供。**定位是「新增能力」,不是修正——既有四個 tool 的日報價契約與 `is_realtime: false` 標示是正確的,維持不變。**

```text
GET /api/v1/stocks/{symbol}/realtime-snapshot
```

- `200`:

  ```json
  {
    "stock_symbol": "2330",
    "name": "台積電",
    "price": 2440.0,
    "change": 25.0,
    "change_range": 1.0352,
    "open": 2460.0,
    "high": 2480.0,
    "low": 2440.0,
    "last_close": 2415.0,
    "volume_lots": 34665.0,
    "source_site": "HiStock",
    "updated_at": "2026-07-13T05:30:12Z"
  }
  ```

- `404`,且錯誤訊息**必須區分兩種情況**:
  - 非交易時段(快取整批為空):`{"error":"目前非交易時段,無即時報價快照"}`
  - 盤中但該代號無快照:`{"error":"查無此股票的即時報價快照"}`

契約注意事項:

- **`volume_lots` 單位是「張」**,與日報價 `trading_volume`(股)不同——欄位名刻意帶單位,避免 LLM 與使用者混用。
- `updated_at` 為快照寫入快取的 UTC 時間,是 MCP 端 `data_as_of` 的來源(前置修正見 Phase 4)。
- 資料來源為第三方站點爬蟲,可能延遲數秒至數分鐘,**不得宣稱為交易所保證即時行情**。

---

## 4. 工作分解

### Phase 1:stock_rust 新增 Data API(不動 stock_mcp_go)

1. `Cargo.toml` 加入 `utoipa`、`utoipa-axum`、`utoipa-swagger-ui`。
2. 新增 `src/interfaces/web/data_api/`:
   - `dto.rs`:`Stock`、`DailyQuote`、`HistoricalQuote`、`StockProfile`、`ErrorBody` 等 `ToSchema` 型別(欄位對齊 3.1 原則)。
   - `handlers.rs`:四個 handler + healthz,`#[utoipa::path]` 標注參數、回應與範例。
   - `auth.rs`:Bearer key 驗證 middleware(常數時間比較;`DATA_API_KEY` 環境變數)。
   - `mod.rs`:`router()` 組裝,掛 Swagger UI 與 openapi.json。
3. 在 `interfaces/web/mod.rs` 的 `start` 將 data_api router merge 進既有 app(沿用現有 graceful shutdown 機制);監聽位址沿用/擴充現有環境變數,預設仍綁 `127.0.0.1`。
4. 查詢實作:重用既有唯讀查詢路徑;若現有 domain 層沒有對應查詢,以 `stock_mcp_rust/src/repository.rs` 的四條 SQL 為準平移(參數化、statement timeout)。`price-history` handler 依 3.2 節的語意修正,先做一次 `stocks` 存在性檢查再查歷史區間(不存在回 `404`,存在但查無資料回 `200` 空陣列)。
5. 測試:
   - handler 單元測試(axum `oneshot`):401、422 參數驗證、404(含 `price-history` 對「代號不存在」的 404,與「代號存在但區間無資料」的 200 空陣列要分開兩條測試,不能只測其中一種)、`quote: null`、空陣列語意。
   - OpenAPI snapshot 測試:openapi.json 能生成且包含四個 path。
6. 驗證:`cargo build`、`cargo test`、瀏覽器開 `/swagger-ui` 實測四個 endpoint。

### Phase 2:stock_mcp_go 新增 API client(與 DB 模式並存)

1. 從 `/api-docs/openapi.json` 以 `oapi-codegen` 產生型別與 client(`internal codegen` 檔案進 repo,`go generate` 可重生);若 spec 品質不足以 codegen,退而手寫薄 client。
2. 新增 `stock/apiclient.go`:實作既有 `stock.Querier` 介面。除了 3.2 節說明的 `price-history` 語意修正之外,`tools.go` 其餘三個工具的 handler 邏輯不需要改動——「零改動」只適用於 `search`/`latest-quote`/`profile` 這三個,`price-history` 因為前面已決定修正語意,是刻意的例外。錯誤對應:

   | API 回應 | Querier 行為 |
   |---|---|
   | `200` | 正常轉換(型別已是 JSON number/null,不再需要 pgtype 轉換) |
   | `404`(latest-quote / profile) | 回傳 `nil, nil`(既有「股票不存在」語意) |
   | `404`(price-history) | 回傳 `nil, stock.ErrStockNotFound`,對應 `tools.go` 的 `priceHistory` 判斷邏輯(見 3.2 節) |
   | `401` / `5xx` / 逾時 / 連線失敗 | 回傳 error → tools 層回「伺服器內部發生未預期錯誤」,細節只進 log |

3. `config`:新增 `DATA_SOURCE=db|api`(過渡期開關,預設 `db`)、`STOCK_RUST_API_BASE_URL`、`STOCK_RUST_API_KEY`、`API_TIMEOUT_MS`(預設 5000,對齊原 statement timeout)。`DATA_SOURCE=api` 時 `DATABASE_URL` 改為非必填。
4. `main.go`:依 `DATA_SOURCE` 選擇注入 `*stock.Repository` 或 `*stock.APIClient`。
5. http client:自建 `http.Client`(明確 timeout),連線內網 stock_rust;不記錄 API key。
6. 測試:
   - `httptest.Server` 假 stock_rust:四個 endpoint 的成功、404、5xx、逾時、`quote:null`、空陣列;`price-history` 額外測「代號不存在回 404 → tool error 找不到股票代號」與「代號存在但區間無資料回 200 空陣列 → 摘要文字」兩條分開的案例。
   - 既有 tools 測試(fakeQuerier):`search`/`latest-quote`/`profile` 三個維持原樣不變;`price-history` 的測試需要新增/調整,涵蓋 `ErrStockNotFound` 這個新路徑。
   - 整合測試:`TEST_STOCK_RUST_API_URL` 啟用、未設定跳過(慣例同 `TEST_DATABASE_URL`)。

### Phase 3:切換與收斂

1. 部署 stock_rust(含 Data API)→ `stock_mcp_go` 以 `DATA_SOURCE=api` 起一份,跑第 6 節驗證清單。
2. 平行期比對:同一批查詢在 db/api 兩模式輸出 JSON 必須逐 byte 等價(可寫一次性 diff 腳本)。
3. 預設值改為 `api`,觀察一段時間(建議 ≥ 一週的排程寫入週期)。
4. 收斂:移除 pgx 相依、`stock/repository.go`、`DATABASE_URL` 相關設定與文件;README 更新架構圖與環境變數表;`.env.example` 同步。
5. 資料庫端:撤銷(或不再發放)MCP 專用唯讀帳號。

### Phase 4(選配):即時報價快照 tool——必須在 Phase 3 等價驗證完成後才開始

先完成四個既有 tool 的逐 byte 等價遷移(那是驗證 API 管道正確性的基準),再加新功能,避免「遷移回歸」與「新功能 bug」混在同一批變更裡無法歸因。

**前置修正(stock_rust,擋路項)**:

1. `RealtimeSnapshot`(`infra/cache/realtime.rs`)新增 `updated_at: DateTime<Utc>` 欄位,在快照寫入/更新快取的統一入口(`snapshot.rs` 的 setter)蓋章;沒有時間戳就無法提供 `data_as_of`,此項未完成前不得開發 endpoint。
2. 確認收盤 `clear_stock_snapshots()` 的清空時機,使「非交易時段」的 404 判斷有明確依據(快取為空即視為非交易時段)。

**stock_rust**:

3. 實作 3.4 的 endpoint(utoipa 標注、掛進 Swagger UI 與 openapi.json),讀取 `SHARE.stock_snapshots`,Decimal→f64 轉換失敗視為該欄位缺值處理並記 log。

**stock_mcp_go**:

4. 定義獨立的小介面(consumer 端定義,勿擴充既有 `Querier`——DB 模式的 `Repository` 沒有即時資料,無法實作該方法):

   ```go
   type SnapshotQuerier interface {
       RealtimeSnapshot(ctx context.Context, symbol string) (*RealtimeSnapshot, error)
   }
   ```

   僅由 API client 實作;`DATA_SOURCE=api` 時才註冊本 tool,`db` 模式下 tools/list 不出現此工具。

5. 新 MCP tool `get_realtime_snapshot`:
   - 輸入:`symbol`(規則同其他 tool:trim、轉大寫、1–24 字元)。
   - 輸出 `structuredContent`:`data_kind: "realtime_snapshot"`、`data_as_of`(取 API 的 `updated_at`)、**`is_realtime: false`**(來源是第三方爬蟲、非交易所保證,依規格紅線「不可宣稱為交易所逐筆或保證即時行情」誠實標示)、專屬 disclaimer:

     > 本資料為盤中由第三方站點採集的近即時報價快照,可能有數秒至數分鐘延遲,非交易所保證即時行情,僅供資訊參考。

   - 404 時回 tool error,訊息透傳 API 的兩種區分,並引導改用 `get_latest_daily_quote` 查最近收盤資料。
6. 測試:httptest 假 API 覆蓋 200/兩種 404/5xx;tools 測試覆蓋 `data_as_of` 取值與錯誤訊息;`db` 模式下確認工具未註冊。

---

## 5. 兩份文件的分工(避免文件漂移)

| 文件 | 位置 | 角色 |
|---|---|---|
| `openapi.json`(由 utoipa 從程式碼生成) | stock_rust,`/api-docs/openapi.json` | **唯一契約事實來源**;Swagger UI 與 Go codegen 都吃它 |
| 本計畫文件 | `stock_rust/docs/` | 執行計畫與決策紀錄,完工後改標「已完成」並保留決策脈絡 |

規則:任何 API 形狀的變動只改 stock_rust 程式碼(utoipa 標注)→ 重新生成 spec → Go 端 `go generate` 重生 client → 編譯錯誤即為影響面清單。不手寫、不另存第二份 API 文件。

---

## 6. 驗證清單(完工定義)

stock_rust:

- [ ] `cargo build`、`cargo test` 通過
- [ ] `/swagger-ui` 可開啟,四個 endpoint 可在 UI 內帶 Bearer key 實測
- [ ] `/api-docs/openapi.json` 可下載且能被 `oapi-codegen` 消化
- [ ] 無 key / 錯誤 key → 401;不存在代號 → search 回 200 空陣列、latest-quote/profile/**price-history** 皆回 404(三者語意一致,price-history 不再是例外)
- [ ] 股票存在但無日報價 → `200` + `quote: null`;股票存在但指定區間無歷史資料 → `200` 空陣列(跟「代號不存在」的 404 明確區分開)
- [ ] 所有變更都在 `feature/data-api-v1` 分支上,未直接 commit `main`
- [ ] 本次讀到/異動/新增的 `.rs` 檔案全部通過第 0.2 節註解品質規範(`cargo doc --no-deps` 零警告)

stock_mcp_go:

- [ ] `go build ./...`、`go vet ./...`、`go test ./...` 通過
- [ ] `stock/repository.go` 的 `PriceHistory` 已補上存在性檢查(`ErrStockNotFound` sentinel),`tools.go` 的 `priceHistory` 對「代號不存在」回傳跟 `latestDailyQuote`/`stockProfile` 一致的 tool error,DB 直連模式與 API 模式此案例行為相同
- [ ] `DATA_SOURCE=api` 端對端:MCP `initialize` → 四個 tool 全部正確,輸出與 db 模式逐 byte 等價(含「未知代號查歷史」這個修正後的案例)
- [ ] stock_rust 停機時:tool 回「伺服器內部發生未預期錯誤」,不洩漏內部位址;MCP server 本身不 crash
- [ ] 401/429、healthz 行為不變(既有 web 測試全綠)
- [ ] 已 `git init` 並建立初始 commit;變更直接在 `main` 上進行(不開分支)
- [ ] 本次讀到/異動/新增的 `.go` 檔案全部通過第 0.2 節註解品質規範(exported 識別字 godoc 全覆蓋)

Phase 4(若執行):

- [ ] `RealtimeSnapshot.updated_at` 已加入且由 setter 統一蓋章
- [ ] `/api/v1/stocks/{symbol}/realtime-snapshot` 在 Swagger UI 可實測,兩種 404 訊息正確
- [ ] `get_realtime_snapshot` 僅在 `DATA_SOURCE=api` 時出現於 tools/list;`data_as_of` 來自 `updated_at`;`is_realtime: false` 與專屬 disclaimer 正確

---

## 7. 安全與部署注意事項

- Data API 只綁內網/迴環位址,**不對網際網路開放**;對外的仍然只有 MCP endpoint(HTTPS 反向代理後方)。
- `DATA_API_KEY` 與 `MCP_API_KEY` 分開發放與輪替;兩把 key 都不落 log、不進 git。
- Swagger UI 隨 Data API 只在內網可及;若未來需要對外,必須加上與 API 相同的驗證或關閉 UI(只留 spec 檔)。
- API 模式下 MCP 端不再需要資料庫憑證——這是本次遷移的主要安全收益,收斂階段務必真的把帳號權限收回。

## 8. 風險與開放問題

| 項目 | 說明 | 需要的決定 |
|---|---|---|
| 延遲增加 | 多一跳 HTTP(內網,預估 +1–5ms),對日報價查詢可忽略 | 無;若未來有高頻需求再議快取 |
| stock_rust 成為單點 | 原本 DB 掛掉一樣全掛,單點沒有變多;但 stock_rust 重啟頻率高於 DB | 確認 stock_rust 的部署/重啟策略,必要時 MCP 端加重試(冪等 GET,可安全 retry 1 次) |
| utoipa 與 axum 0.8 版本對齊 | 需選用支援 axum 0.8 的 utoipa-axum/utoipa-swagger-ui 版本 | Phase 1 第一步先確認版本組合可編譯 |
| 分頁 | 現有 limit 上限(50/365)夠用,不做游標分頁 | 無 |
| 即時快照品質 | 第三方站點採集,個股覆蓋率與延遲無 SLA;`source_site` 欄位保留來源可追溯 | 無(已以 disclaimer 與 `is_realtime: false` 誠實標示) |
