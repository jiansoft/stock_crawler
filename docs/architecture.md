# stock_crawler 架構指南

更新日期：2026-05-08

## 目錄結構

```text
src/
├─ main.rs           # 程式入口：組裝各層、啟動排程與服務
├─ core/             # 共用基礎設施（無業務邏輯）
│  ├─ config.rs      # 應用程式設定（app.json + 環境變數）
│  ├─ declare.rs     # 共用型別與常數定義
│  ├─ logging/       # 非同步檔案日誌
│  └─ util/          # 通用工具（HTTP client、日期、文字、原子操作、診斷）
├─ app/              # 應用層：業務流程編排與用例
│  ├─ scheduler.rs   # cron 排程器（所有定時任務的註冊點）
│  ├─ backfill/      # 歷史資料回補流程
│  ├─ event/         # 盤後事件處理（收盤計算、追蹤觸發）
│  ├─ calculation/   # 衍生計算（資金歷史等）
│  └─ manual_backfill.rs  # 手動回補（#[cfg(test)] 測試入口）
├─ infra/            # 基礎設施層：外部系統存取
│  ├─ cache/         # 共享記憶體快取（Redis 背景同步）
│  ├─ nosql/         # Redis 客戶端
│  ├─ database/      # PostgreSQL 資料存取
│  │  ├─ table/      # 各資料表 CRUD（依領域分組）
│  │  │  ├─ stock/       # 股票主檔、交易所、市場、產業
│  │  │  ├─ quote/       # 日報價、最後報價、歷史報價、統計
│  │  │  ├─ dividend/    # 股利主檔、配發日、明細
│  │  │  ├─ financial/   # 財報、估值、營收
│  │  │  ├─ money_flow/  # 資金流向與法人
│  │  │  └─ (根層)       # config、trace、yield_rank 等橫切輔助表
│  │  └─ ...
│  └─ crawler/       # 外部資料來源爬蟲
│     ├─ twse/       # 台灣證券交易所
│     ├─ tpex/       # 櫃買中心
│     ├─ yahoo/      # Yahoo 股市
│     ├─ goodinfo/   # 台灣股市資訊網
│     └─ ...         # 其他資料來源
└─ interfaces/       # 對外介面層
   ├─ rpc/           # gRPC 服務（含 proto 產碼）
   ├─ web/           # HTTP/Axum Web 服務
   └─ bot/           # Telegram Bot 通知
```

## 分層規則

### 依賴方向（嚴格單向）

```text
main.rs → app / interfaces
app     → core / infra
infra   → core
interfaces → core / app / infra
core    → （不依賴其他層）
```

### 禁止的依賴

| 來源層 | 禁止依賴 | 原因 |
|--------|----------|------|
| `core` | `app`, `infra`, `interfaces` | core 是最底層，不可反向依賴 |
| `infra` | `app`, `interfaces` | 基礎設施不應知道業務流程 |
| `app` | `interfaces` | 應用層不應知道對外介面細節 |

## 新模組放置規範

### 放入 `core/` 的條件

- 不含任何業務邏輯
- 被 2 個以上的其他層使用
- 例：新的通用工具函式、共用型別定義

### 放入 `app/` 的條件

- 編排多個 `infra` 模組完成一個業務用例
- 例：新的排程任務、新的回補流程、新的盤後事件處理

### 放入 `infra/` 的條件

- 封裝對外部系統的存取（資料庫、API、快取）
- 例：新的爬蟲來源、新的資料表 CRUD

### 放入 `interfaces/` 的條件

- 提供對外服務入口（gRPC、HTTP、Bot）
- 例：新的 API endpoint、新的 Bot 指令

### `infra/database/table/` 領域分組

新增資料表時，依業務領域放入對應子目錄：

| 領域 | 子目錄 | 包含內容 |
|------|--------|----------|
| 股票主檔 | `stock/` | 股票、交易所、市場、產業、持股明細 |
| 報價 | `quote/` | 日報價、最後報價、歷史報價、統計 |
| 股利 | `dividend/` | 股利主檔、配發日、明細延伸 |
| 財務 | `financial/` | 財報、估值、營收 |
| 資金流向 | `money_flow/` | 法人買賣超、資金歷史 |
| 橫切/輔助 | `table/` 根層 | config、trace、yield_rank |

## 建置相關

### Proto 產碼

- `.proto` 檔案位於 `etc/proto/`
- `build.rs` 會將產碼輸出至 `src/interfaces/rpc/`
- 修改 `.proto` 後需執行 `cargo build` 重新產生

### 跨平台建置

- **Windows 開發**：`cargo build` 或 `cargo check`
- **Linux musl 交叉編譯**：使用 `build.bat` / `build.ps1`（需 Zig + CMake）
- **容器部署**：使用 `Dockerfile_live`（distroless 映像）
