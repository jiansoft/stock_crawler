use chrono::NaiveDate;

use crate::domain::performance::entity::{CagrCoverage, CagrMetric, CagrPeriod, StockCagr};

/// 排行榜排序鍵。
///
/// 短期間（3M／6M）年化後會被放大約 4 倍，單季 +20% 會變成 +107%，
/// 統計意義薄弱，因此展示層在短期間應改用 [`CagrSortKey::TotalReturn`]。
/// 這裡以列舉而非字串表達，讓「排序鍵」在型別層面就是封閉集合，
/// SQL 端只能對應到白名單內的欄位字面值，不存在拼接注入的空間。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CagrSortKey {
    /// 依年化報酬率排序。
    #[default]
    Cagr,
    /// 依區間總報酬率排序。
    TotalReturn,
}

impl CagrSortKey {
    /// 全部排序鍵。
    pub const ALL: [CagrSortKey; 2] = [CagrSortKey::Cagr, CagrSortKey::TotalReturn];

    /// API 查詢參數使用的代碼。
    pub fn code(&self) -> &'static str {
        match self {
            CagrSortKey::Cagr => "cagr",
            CagrSortKey::TotalReturn => "total_return",
        }
    }

    /// 自代碼還原，無法辨識時回傳 `None`。
    pub fn from_code(code: &str) -> Option<CagrSortKey> {
        CagrSortKey::ALL.into_iter().find(|k| k.code() == code)
    }

    /// 依期間長度決定合適的預設排序鍵。
    pub fn default_for(period: CagrPeriod) -> CagrSortKey {
        if period.annualization_is_meaningful() {
            CagrSortKey::Cagr
        } else {
            CagrSortKey::TotalReturn
        }
    }
}

/// 排行榜查詢條件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CagrRankingQuery {
    /// 計算基準日。
    pub date: NaiveDate,
    /// 統計期間。
    pub period: CagrPeriod,
    /// 報酬口徑。
    pub metric: CagrMetric,
    /// 排序鍵。
    pub sort: CagrSortKey,
    /// 交易所市場編號篩選（`None` 表全部）。
    pub market_id: Option<i32>,
    /// 產業分類編號篩選（`None` 表全部）。
    pub industry_id: Option<i32>,
    /// 代號或名稱關鍵字篩選。
    pub keyword: Option<String>,
    /// 是否一併回傳資料不足的項目。
    ///
    /// 預設為 `true`：「查得到但算不出來」與「查不到」是兩件不同的事，
    /// 直接濾掉會讓使用者困惑於自己持有的股票為何消失。
    pub include_incomplete: bool,
    /// 單頁筆數。
    pub limit: i64,
    /// 起始位移。
    pub offset: i64,
}

impl CagrRankingQuery {
    /// 以基準日與期間建立預設查詢：主指標口徑、依期間長度決定排序鍵。
    pub fn new(date: NaiveDate, period: CagrPeriod) -> Self {
        Self {
            date,
            period,
            metric: CagrMetric::Total,
            sort: CagrSortKey::default_for(period),
            market_id: None,
            industry_id: None,
            keyword: None,
            include_incomplete: true,
            limit: 50,
            offset: 0,
        }
    }
}

/// 排行榜中的單一項目。
///
/// 在 [`StockCagr`] 之外附上展示所需的股票名稱與名次 —— 這兩者不屬於
/// 計算結果本身，但少了它們前端得再查一次股票母檔。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CagrRankingItem {
    /// 計算結果。
    pub cagr: StockCagr,
    /// 股票名稱。
    pub name: String,
    /// 產業分類編號。
    pub industry_id: i32,
    /// 名次。
    ///
    /// 資料不足者為 `None` —— 這類項目不佔名次且排在所有可算項目之後，
    /// 否則排行榜會出現「第 1204 名，報酬率 —」這種無意義的列。
    pub rank: Option<i64>,
}

/// 排行榜查詢結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CagrRankingPage {
    /// 本頁項目。
    pub items: Vec<CagrRankingItem>,
    /// 符合篩選條件的總筆數（供分頁計算）。
    pub total: i64,
    /// 樣本涵蓋統計。
    pub coverage: CagrCoverage,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
    }

    #[test]
    fn sort_key_code_round_trips_and_rejects_unknown() {
        for key in CagrSortKey::ALL {
            assert_eq!(CagrSortKey::from_code(key.code()), Some(key));
        }
        assert_eq!(CagrSortKey::from_code("total_cagr_pct"), None);
        assert_eq!(CagrSortKey::default(), CagrSortKey::Cagr);
    }

    #[test]
    fn short_periods_default_to_total_return() {
        // 3M/6M 年化後會被放大約四倍，預設排序必須退回區間總報酬。
        assert_eq!(
            CagrSortKey::default_for(CagrPeriod::M3),
            CagrSortKey::TotalReturn
        );
        assert_eq!(
            CagrSortKey::default_for(CagrPeriod::M6),
            CagrSortKey::TotalReturn
        );
        for period in [CagrPeriod::Y1, CagrPeriod::Y3, CagrPeriod::Y10] {
            assert_eq!(CagrSortKey::default_for(period), CagrSortKey::Cagr);
        }
    }

    #[test]
    fn new_query_uses_total_metric_and_includes_incomplete() {
        let query = CagrRankingQuery::new(date(2026, 8, 7), CagrPeriod::Y1);

        assert_eq!(query.metric, CagrMetric::Total);
        assert_eq!(query.sort, CagrSortKey::Cagr);
        // 「查得到但算不出來」與「查不到」是兩件事，預設不可濾掉前者。
        assert!(query.include_incomplete);
        assert_eq!(query.limit, 50);
        assert_eq!(query.offset, 0);
        assert!(query.market_id.is_none());
        assert!(query.industry_id.is_none());
        assert!(query.keyword.is_none());
    }

    #[test]
    fn new_query_picks_sort_key_from_period_length() {
        let short = CagrRankingQuery::new(date(2026, 8, 7), CagrPeriod::M3);
        assert_eq!(short.sort, CagrSortKey::TotalReturn);
    }
}
