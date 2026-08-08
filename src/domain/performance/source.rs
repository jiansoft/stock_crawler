use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::performance::entity::DividendEvent;

/// CAGR 計算所需的原始資料來源介面。
///
/// 與 [`crate::domain::performance::repository::CagrRepository`]（負責寫入與
/// 查詢計算結果）分開，是因為兩者的職責完全不同：此介面只負責「把算之前需要
/// 的原始資料一次撈齊」，全部是批次讀取，且刻意避免逐檔查詢 ——
/// 全市場約 2,600 檔 × 8 個期間，任何 N+1 查詢都會讓排程無法收斂。
#[async_trait]
pub trait CagrSourceRepository: Send + Sync {
    /// 取得不晚於指定日期的最近一個交易日。
    ///
    /// 用於把「執行日 − N 個月」的名目目標日對齊到實際有報價的交易日。
    /// 資料庫中無任何早於該日的報價時回傳 `None`，代表該期間無法計算。
    async fn fetch_trading_day_on_or_before(&self, date: NaiveDate) -> Result<Option<NaiveDate>>;

    /// 取得最新一個交易日。
    async fn fetch_latest_trading_day(&self) -> Result<Option<NaiveDate>>;

    /// 取得計算母體：未下市的股票代號。
    async fn fetch_active_symbols(&self) -> Result<Vec<String>>;

    /// 取得每檔股票在報價資料中的最早日期。
    ///
    /// 這是判定「新上市／資料未涵蓋」的依據。專案的 `stocks` 表沒有上市日期
    /// 欄位（ISIN 爬蟲雖有抓 `listing_date`，但在防腐層即被丟棄），因此改由
    /// 報價資料推斷。注意 `DailyQuotes` 存在 `1970-01-01` 這類預設哨兵值，
    /// 實作必須排除。
    async fn fetch_first_quote_dates(&self) -> Result<Vec<(String, NaiveDate)>>;

    /// 取得指定交易日全市場的收盤價（僅回傳大於零者）。
    async fn fetch_closing_prices_on(&self, date: NaiveDate) -> Result<Vec<(String, Decimal)>>;

    /// 取得每檔股票在指定日期區間內的第一筆報價（日期與收盤價）。
    ///
    /// 用於 2.4 的寬限規則：名目期初日當天無報價時，往後找寬限期內的第一筆。
    async fn fetch_first_quote_within(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<(String, NaiveDate, Decimal)>>;

    /// 取得指定日期之後所有股票的除權息事件。
    ///
    /// 實作必須處理兩件事，否則結果會系統性錯誤：
    ///
    /// 1. **年度彙總列與季度明細列去重**：`dividend` 表中同一
    ///    `(security_code, year)` 可能同時存在 `quarter = ''` 的年度彙總列
    ///    與 `Q1`–`Q4`／`H1`–`H2` 明細列（2026-08-07 實測有 1,619 組），
    ///    無條件加總會讓季配息股的股利被計算兩次。規則是「有明細就只用明細，
    ///    沒明細才用年度彙總列」。
    /// 2. **除權息日的髒值**：欄位型別是 `varchar(10)`，實測含 `'-'`
    ///    （17,553 筆）、`'尚未公布'`、空字串，甚至有 `'1.39%'` 這類百分比
    ///    字串。**絕不可在 SQL 中做 `::date` 轉型**，必須取出原始字串後在
    ///    Rust 端解析，解析失敗者視為該日期不存在（`None`）。
    async fn fetch_dividend_events_since(&self, since: NaiveDate) -> Result<Vec<DividendEvent>>;

    /// 批次取得指定 (股票代號, 日期) 組合的收盤價。
    ///
    /// 供「含息再投入」口徑在除息日買回股數之用。刻意設計成批次介面：
    /// 十年份的除權息事件約有數萬筆，逐筆查詢會產生數萬次往返。
    async fn fetch_closing_prices_at(
        &self,
        pairs: &[(String, NaiveDate)],
    ) -> Result<Vec<(String, NaiveDate, Decimal)>>;

    /// 找出指定期間內疑似減資或分割的「事件」，回傳 (股票代號, 發生日)。
    ///
    /// 本專案沒有記錄減資與股票分割，只能從價格序列反推：單日收盤價出現無法用
    /// 除權息解釋的大幅跳動。期間拉長到五年、十年時，遇上這類未建模事件幾乎是
    /// 必然，沒有這道防護，長期間排行榜的頭尾兩端都不可信。
    ///
    /// **刻意回傳帶日期的事件而非股票清單**：呼叫端要為八個期間各自判定，
    /// 若只回傳代號，就只能用最長期間算一次再套用到所有期間 —— 那會讓 2017 年
    /// 的減資把 3 個月期間的結果也標記為異常，語義完全錯誤。回傳事件讓呼叫端
    /// 用單次查詢的結果依日期切分到各期間。
    async fn fetch_anomaly_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<(String, NaiveDate)>>;
}
