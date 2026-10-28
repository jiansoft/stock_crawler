use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use sqlx::postgres::PgQueryResult;

use self::sql::ESTIMATE_UPSERT_SQL;
use crate::{core::declare::Industry, infra::database};

mod sql;

/// 個股估值資料。
///
/// 彙整價格法、股利法、PBR 法、PER 法的加權估值，以及僅供參考的預估股利，
/// 供排名與市場統計使用。
#[derive(sqlx::FromRow, Debug, Default)]
pub struct Estimate {
    /// 估值日期。
    pub date: NaiveDate,
    /// 參考的最後一筆日報價日期（字串格式）。
    pub last_daily_quote_date: String,
    /// 股票代號。
    pub security_code: String,
    /// 股票名稱。
    pub name: String,
    /// 當日收盤價。
    pub closing_price: f64,
    /// 估值百分比（收盤價相對便宜價）。
    pub percentage: f64,
    /// 加權便宜價。
    pub cheap: f64,
    /// 加權合理價。
    pub fair: f64,
    /// 加權昂貴價。
    pub expensive: f64,
    /// 價格法便宜價。
    pub price_cheap: f64,
    /// 價格法合理價。
    pub price_fair: f64,
    /// 價格法昂貴價。
    pub price_expensive: f64,
    /// 股利法便宜價。
    pub dividend_cheap: f64,
    /// 股利法合理價。
    pub dividend_fair: f64,
    /// 股利法昂貴價。
    pub dividend_expensive: f64,
    /// 預估股利法便宜價（不計入加權）。
    pub eps_cheap: f64,
    /// 預估股利法合理價（不計入加權）。
    pub eps_fair: f64,
    /// 預估股利法昂貴價（不計入加權）。
    pub eps_expensive: f64,
    /// PBR 法便宜價。
    pub pbr_cheap: f64,
    /// PBR 法合理價。
    pub pbr_fair: f64,
    /// PBR 法昂貴價。
    pub pbr_expensive: f64,
    /// 參與統計的年度數。
    pub year_count: i32,
    /// 內部排序或索引欄位。
    pub index: i32,
}

impl Estimate {
    /// 建立單一股票指定日期的估值模型預設值。
    pub fn new(security_code: String, date: NaiveDate) -> Self {
        Estimate {
            date,
            last_daily_quote_date: "".to_string(),
            security_code,
            name: "".to_string(),
            closing_price: 0.0,
            percentage: 0.0,
            cheap: 0.0,
            fair: 0.0,
            expensive: 0.0,
            price_cheap: 0.0,
            price_fair: 0.0,
            price_expensive: 0.0,
            dividend_cheap: 0.0,
            dividend_fair: 0.0,
            dividend_expensive: 0.0,
            eps_cheap: 0.0,
            eps_fair: 0.0,
            eps_expensive: 0.0,
            pbr_cheap: 0.0,
            pbr_fair: 0.0,
            pbr_expensive: 0.0,
            year_count: 0,
            index: 0,
        }
    }

    /// 依指定日期與年份清單，批次重建所有股票估值資料。
    ///
    /// ### 估值計算公式說明：
    /// 以價格法、股利法、PBR 法、PER 法四種模型加權出「便宜價」、「合理價」與「昂貴價」；
    /// 另存一組預估股利（`eps_*` 欄位）供參考，不計入加權。
    ///
    /// **1. 加權比例 (Weights)：**
    /// *   價格法 (20%) + 股利法 (25%) + PBR 法 (20%) + PER 法 (35%)
    /// *   各法便宜/合理/昂貴三價都 > 0 才算有效；**缺項不以 0 計入**，權重在有效方法之間按比例重新分配。
    /// *   PER 法計入加權時，不超過「所有有效方法（含 PER）同一價位中位數」的 3 倍（`per_*` 欄位仍存原值）：
    ///     景氣循環股在獲利高峰時本益比位階 × 高峰 EPS 會嚴重高估（青雲 PER 合理 1,895、股價 290）。
    ///     中位數含 PER 本身：只排除其他三法會被看歷史的價格法、股利法拉低，連台積電都被誤截。
    /// *   有效方法少於 2 個時不給估價（加權三價與百分比都寫 0；percentage 欄位不允許 NULL）。
    ///
    /// **2. 各別估值法細節：**
    /// *   **價格法 (Price-based)：**
    ///     *   便宜價：指定年份內 `LowestPrice` 的 10% 分位數。
    ///     *   合理價：指定年份內 `ClosingPrice` 的 50% 分位數。
    ///     *   昂貴價：指定年份內 `HighestPrice` 的 80% 分位數。
    /// *   **股利法 (Dividend-based)：**
    ///     *   基準：指定年份內「年均股利」。
    ///     *   便宜/合理/昂貴：基準 × 15 / 20 / 25。
    /// *   **預估股利（`eps_*` 欄位，不計入加權）：**
    ///     *   基準：`近四季 EPS` × `各發放年度盈餘分配率的中位數`（個股 → 產業 → 60%）。
    ///     *   盈餘分配率取 `dividend.payout_ratio`（股利 ÷ 涵蓋期間 EPS），不再用同一年的 EPS 相除。
    ///     *   便宜/合理/昂貴：基準 × 15 / 20 / 25。
    ///     *   本質與股利法同為殖利率法；計入加權會讓殖利率重複計權，低估低配發率的成長股。
    /// *   **PBR 法 (Price-to-Book Ratio)：**
    ///     *   基準：最新一季 `每股淨值`。
    ///     *   倍數：指定年份內 `PBR` 的 10% / 50% / 80% 分位數。
    ///     *   便宜/合理/昂貴：基準 × 倍數。
    /// *   **PER 法 (Price-Earning Ratio，本益比河流圖)：**
    ///     *   基準：`近四季 EPS`（與資料庫本益比同一口徑）。
    ///     *   倍數：指定年份內 `PER` 的 10% / 50% / 80% 分位數，只取 0～100 倍；
    ///         有效天數不足 250 個交易日時 PER 法無效。
    ///
    /// **3. 分割／減資還原（`corporate_action`）：**
    /// *   報價、股利、財報都是事件當時的每股數字；事件之後股數改變，舊數字要除以「之後所有事件
    ///     股數比例的乘積」才能和現在的股價比較（5904 寶雅 1:10 分割後收盤 69.7，未還原的合理價 568）。
    /// *   歷史股價依報價日、股利依除權息日、近四季 EPS 與每股淨值依各季季底換算。
    /// *   本益比、PBR 是比率不受影響；配發率是原始股利 ÷ 涵蓋期間原始 EPS，比例同樣不受影響。
    ///
    /// **4. 百分比 (Percentage) 計算：**
    /// *   公式：`(當前收盤價 / 加權便宜價) * 100`。
    /// *   數值越低代表股價相對越便宜。
    ///
    /// `years` 格式為逗號分隔字串，例如 `\"2026,2025,2024\"`。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert_all(date: NaiveDate, years: String) -> Result<PgQueryResult> {
        // 依股票代號雜湊分成 ESTIMATE_BUCKETS 桶同時跑：瓶頸是 297 萬筆日K 的 5 組
        // 百分位數排序，ordered-set aggregate 無法平行化，單一語句只用得到一顆 CPU
        // （2026-10-08 正式庫實測 7.6～8.2 秒 → 4 桶 2.8～3.0 秒，結果逐筆相同）。
        // 各桶寫入的股票互不重疊，不會互相鎖住；任一桶失敗整體回報失敗，
        // 已完成的桶照常保留（與單檔重算相同的 upsert 語意，沒有先刪後寫）。
        let buckets = (0..ESTIMATE_BUCKETS)
            .map(|bucket| run_estimate_upsert(date, &years, None, (ESTIMATE_BUCKETS, bucket)));
        futures::future::try_join_all(buckets)
            .await
            .map(|results| {
                let mut total = PgQueryResult::default();
                total.extend(results);
                total
            })
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert_all() from database for date: {} with years: {}. Error: {:?}",
                    date,
                    years,
                    why,
                )
            })
    }

    /// 只重算單一股票的估值資料。
    ///
    /// 與 [`Self::upsert_all`] 共用同一段 SQL，只多一個代號篩選：產業配發率中位數等
    /// 需要全市場資料的中間結果照常以全市場計算，單檔重算的結果才會和批次重建一致。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert(&self, years: String) -> Result<PgQueryResult> {
        run_estimate_upsert(self.date, &years, Some(&self.security_code), (1, 0))
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert({:#?}) from database for years: {}. Error: {:?}",
                    self,
                    years,
                    why,
                )
            })
    }
}

/// 全市場重建時同時執行的桶數（各佔批次連線池一條連線，批次池上限 8）。
///
/// 2026-10-08 為 4 桶、每桶 2.8～3.0 秒；日K 延伸到 2019 年後，10-20～10-26 每天 04:00 四桶都要
/// 5.1～7.1 秒，固定觸發批次池 5 秒的慢查詢警告。資料庫主機 16 核，改成 8 桶讓每桶的排序量減半，
/// 不必放寬警告門檻。
const ESTIMATE_BUCKETS: i32 = 8;

/// 執行估值重建 SQL；`security_code` 為 `None` 時重建全市場。
///
/// `(bucket_count, bucket)` 只處理代號雜湊落在第 `bucket` 桶的股票；`(1, 0)` 為不分桶。
async fn run_estimate_upsert(
    date: NaiveDate,
    years: &str,
    security_code: Option<&str>,
    (bucket_count, bucket): (i32, i32),
) -> std::result::Result<PgQueryResult, sqlx::Error> {
    // 全市場重建每桶本來就要 2～3 秒，走批次連線池以免天天觸發 1 秒慢查詢警告；
    // 單檔重算維持一般連線池，變慢時照樣會被警告。
    let pool = match security_code {
        None => database::get_batch_connection(),
        Some(_) => database::get_connection(),
    };
    sqlx::query(ESTIMATE_UPSERT_SQL)
        .bind(date)
        .bind(years)
        .bind(Industry::ExchangeTradedFund.serial())
        .bind(security_code)
        .bind(bucket_count)
        .bind(bucket)
        // 不快取成具名 prepared statement：同一連線執行約 5 次後 PostgreSQL 會改用 generic plan，
        // 代號與分桶條件剪不掉（2026-10-28 正式庫實測一桶 custom 1.5 秒、generic 90 秒）。每天兩次
        // 的排程之間連線早已回收，但回補多個日期時同一連線會連續執行。每次重新規劃只多約 10 ms。
        .persistent(false)
        .execute(pool)
        .await
}

#[cfg(test)]
mod tests;
