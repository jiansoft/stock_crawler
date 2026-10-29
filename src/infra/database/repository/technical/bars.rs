//! 技術指標日線、除權息與股數變動事件，以及期間內的還原日線。

use super::*;

/// 截止日（含）之前最近 `$3` 個有成交的交易日，由新到舊；只讀
/// `(stock_symbol, "Date" DESC)` 覆蓋索引內的欄位（index-only scan）。
const PRICE_BARS_SQL: &str = r#"
SELECT "Date", "ClosingPrice", "HighestPrice", "LowestPrice"
FROM "DailyQuotes"
WHERE stock_symbol = $1
  AND "ClosingPrice" > 0
  AND ($2::date IS NULL OR "Date" <= $2)
ORDER BY "Date" DESC
LIMIT $3
"#;

/// 與 [`PRICE_BARS_SQL`] 同一批交易日的成交量；走 `(stock_symbol, "Date" DESC) INCLUDE ("ClosingPrice",
/// "TradingVolume")` 覆蓋索引（index-only scan）。
const VOLUMES_SQL: &str = r#"
SELECT "Date", "TradingVolume"::float8
FROM "DailyQuotes"
WHERE stock_symbol = $1
  AND "ClosingPrice" > 0
  AND ($2::date IS NULL OR "Date" <= $2)
ORDER BY "Date" DESC
LIMIT $3
"#;

/// 多檔股票在 `$2`～`$3`（含）有成交的日線，依代號、日期由舊到新；走
/// `(stock_symbol, "Date" DESC)` 覆蓋索引。價格在資料庫端轉成 `float8`：七年、數百檔的日線有數十萬列，
/// 在 pi 上逐列解析 `NUMERIC` 比查詢本身慢一個數量級。
const RANGE_BARS_SQL: &str = r#"
SELECT stock_symbol::text, "Date", "ClosingPrice"::float8, "HighestPrice"::float8, "LowestPrice"::float8
FROM "DailyQuotes"
WHERE stock_symbol = ANY($1)
  AND "Date" BETWEEN $2 AND $3
  AND "ClosingPrice" > 0
ORDER BY stock_symbol, "Date"
"#;

/// 多檔股票各自在一段或多段 `[start, end]`（含）期間有成交的日線，依代號、日期由舊到新；`$1`～`$3`
/// 為等長的代號、起日、迄日陣列，同一檔的各段不重疊。價格同樣轉成 `float8`。
const WINDOW_BARS_SQL: &str = r#"
SELECT q.stock_symbol::text, q."Date", q."ClosingPrice"::float8, q."HighestPrice"::float8,
       q."LowestPrice"::float8
FROM UNNEST($1::text[], $2::date[], $3::date[]) AS w(stock_symbol, start_date, end_date)
JOIN "DailyQuotes" q ON q.stock_symbol = w.stock_symbol AND q."Date" BETWEEN w.start_date AND w.end_date
WHERE q."ClosingPrice" > 0
ORDER BY q.stock_symbol, q."Date"
"#;

/// 除息（`ex-dividend_date1` 配現金）、除權（`ex-dividend_date2` 配股）與
/// `corporate_action`（減資、分割、反向分割）事件，依股票與日期合併。
///
/// 除權息日欄位是文字，`-` 等非日期值以 CASE 擋掉再轉型。同一天有多列股利時取最大值：
/// 債券 ETF 有月配與季配兩列記同一筆配息的情形，加總會重複計算。
const PRICE_ADJUSTMENTS_SQL: &str = r#"
WITH events AS (
    SELECT security_code AS stock_symbol,
           CASE WHEN "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' THEN "ex-dividend_date1"::date END AS date,
           cash_dividend AS cash, 0::numeric AS stock, 1::numeric AS share_ratio
    FROM dividend
    WHERE security_code = ANY($1) AND cash_dividend > 0
    UNION ALL
    SELECT security_code,
           CASE WHEN "ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' THEN "ex-dividend_date2"::date END,
           0, stock_dividend, 1
    FROM dividend
    WHERE security_code = ANY($1) AND stock_dividend > 0
    UNION ALL
    SELECT stock_symbol, effective_date, 0, 0, share_ratio
    FROM corporate_action
    WHERE stock_symbol = ANY($1) AND share_ratio > 0
)
SELECT stock_symbol, date, MAX(cash), MAX(stock),
       COALESCE(MAX(share_ratio) FILTER (WHERE share_ratio <> 1), 1)
FROM events
WHERE date IS NOT NULL
GROUP BY stock_symbol, date
ORDER BY stock_symbol, date
"#;

#[async_trait]
impl PriceBarRepository for PgQuoteRepository {
    async fn fetch_price_bars(
        &self,
        stock_symbol: &str,
        until: Option<NaiveDate>,
        limit: i64,
    ) -> Result<Vec<PriceBar>> {
        let rows: Vec<(NaiveDate, Decimal, Decimal, Decimal)> = sqlx::query_as(PRICE_BARS_SQL)
            .bind(stock_symbol)
            .bind(until)
            .bind(limit)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .rev()
            .map(|(date, close, high, low)| PriceBar {
                date,
                close: close.to_f64().unwrap_or_default(),
                high: high.to_f64().unwrap_or_default(),
                low: low.to_f64().unwrap_or_default(),
            })
            .collect())
    }

    async fn fetch_volumes(
        &self,
        stock_symbol: &str,
        until: Option<NaiveDate>,
        limit: i64,
    ) -> Result<Vec<VolumeBar>> {
        let rows: Vec<(NaiveDate, f64)> = sqlx::query_as(VOLUMES_SQL)
            .bind(stock_symbol)
            .bind(until)
            .bind(limit)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .rev()
            .map(|(date, volume)| VolumeBar { date, volume })
            .collect())
    }

    async fn fetch_price_adjustments(
        &self,
        stock_symbols: &[String],
    ) -> Result<HashMap<String, Vec<PriceAdjustment>>> {
        Ok(self.price_adjustments(stock_symbols).await?)
    }
}

impl PgQuoteRepository {
    /// 多檔股票的除權息與股數變動事件，依代號分組（[`PriceBarRepository::fetch_price_adjustments`]
    /// 的實作，保留 [`sqlx::Error`] 給 Data API 直接轉成錯誤回應）。
    pub(crate) async fn price_adjustments(
        &self,
        stock_symbols: &[String],
    ) -> Result<HashMap<String, Vec<PriceAdjustment>>, sqlx::Error> {
        let rows: Vec<(String, NaiveDate, Decimal, Decimal, Decimal)> =
            sqlx::query_as(PRICE_ADJUSTMENTS_SQL)
                .bind(stock_symbols)
                .fetch_all(database::get_connection())
                .await?;
        let mut out: HashMap<String, Vec<PriceAdjustment>> = HashMap::new();
        for (symbol, date, cash, stock, share_ratio) in rows {
            out.entry(symbol).or_default().push(PriceAdjustment {
                date,
                cash_dividend: cash.to_f64().unwrap_or_default(),
                stock_dividend: stock.to_f64().unwrap_or_default(),
                share_ratio: share_ratio.to_f64().unwrap_or(1.0),
            });
        }
        Ok(out)
    }

    /// 多檔股票在 `from`～`to`（含）有成交的日線，依代號分組、由舊到新，收盤、最高、最低已依期間內的
    /// 除權息與股數變動還原（見 [`adjust_bars`]）；期間內沒有成交的股票不在結果中。回測的期間停損用。
    pub(crate) async fn fetch_adjusted_bars(
        &self,
        stock_symbols: &[String],
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<HashMap<String, Vec<PriceBar>>, sqlx::Error> {
        let rows: Vec<(String, NaiveDate, f64, f64, f64)> = sqlx::query_as(RANGE_BARS_SQL)
            .bind(stock_symbols)
            .bind(from)
            .bind(to)
            .fetch_all(database::get_batch_connection())
            .await?;
        let bars = group_bars(rows);
        let adjustments = self.price_adjustments(stock_symbols).await?;
        Ok(bars
            .into_iter()
            .map(|(symbol, series)| {
                let events = adjustments.get(&symbol).map_or(&[][..], Vec::as_slice);
                let adjusted = adjust_bars(&series, events);
                (symbol, adjusted)
            })
            .collect())
    }

    /// 各檔在指定期間（`(代號, 起日, 迄日)`，同一檔的各段不重疊）有成交的**未還原**日線，依代號分組、
    /// 由舊到新。呼叫端依各段分別還原（見 `screen_backtest::stops::load_holding_windows`）。
    pub(crate) async fn fetch_bar_windows(
        &self,
        windows: &[(String, NaiveDate, NaiveDate)],
    ) -> Result<HashMap<String, Vec<PriceBar>>, sqlx::Error> {
        let symbols: Vec<&str> = windows
            .iter()
            .map(|(symbol, _, _)| symbol.as_str())
            .collect();
        let starts: Vec<NaiveDate> = windows.iter().map(|(_, start, _)| *start).collect();
        let ends: Vec<NaiveDate> = windows.iter().map(|(_, _, end)| *end).collect();
        let rows: Vec<(String, NaiveDate, f64, f64, f64)> = sqlx::query_as(WINDOW_BARS_SQL)
            .bind(&symbols)
            .bind(&starts)
            .bind(&ends)
            .fetch_all(database::get_batch_connection())
            .await?;
        Ok(group_bars(rows))
    }
}

/// 各 `(代號, 日期)` 當天的開盤價／收盤價比；開盤或收盤不為正的不回傳（呼叫端視為 1）。
const OPEN_RATIOS_SQL: &str = r#"
SELECT q.stock_symbol::text, q."Date", q."OpeningPrice"::float8 / q."ClosingPrice"::float8
FROM UNNEST($1::text[], $2::date[]) AS w(stock_symbol, date)
JOIN "DailyQuotes" q ON q.stock_symbol = w.stock_symbol AND q."Date" = w.date
WHERE q."OpeningPrice" > 0 AND q."ClosingPrice" > 0
"#;

impl PgQuoteRepository {
    /// 各 `(代號, 日期)` 當天的開盤價／收盤價比（同一天的還原因子相消，可直接乘在還原收盤上得到還原開盤）。
    pub(crate) async fn fetch_open_ratios(
        &self,
        points: &[(String, NaiveDate)],
    ) -> Result<HashMap<(String, NaiveDate), f64>, sqlx::Error> {
        if points.is_empty() {
            return Ok(HashMap::new());
        }
        let symbols: Vec<&str> = points.iter().map(|(symbol, _)| symbol.as_str()).collect();
        let dates: Vec<NaiveDate> = points.iter().map(|(_, date)| *date).collect();
        let rows: Vec<(String, NaiveDate, f64)> = sqlx::query_as(OPEN_RATIOS_SQL)
            .bind(&symbols)
            .bind(&dates)
            .fetch_all(database::get_batch_connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|(symbol, date, ratio)| ((symbol, date), ratio))
            .collect())
    }
}

/// 依代號分組日線列（SQL 已依代號、日期排序）。
fn group_bars(rows: Vec<(String, NaiveDate, f64, f64, f64)>) -> HashMap<String, Vec<PriceBar>> {
    let mut bars: HashMap<String, Vec<PriceBar>> = HashMap::new();
    for (symbol, date, close, high, low) in rows {
        bars.entry(symbol).or_default().push(PriceBar {
            date,
            close,
            high,
            low,
        });
    }
    bars
}
