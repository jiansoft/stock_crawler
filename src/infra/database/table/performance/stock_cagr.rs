use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::performance::entity::{
    CagrPeriod, SimulationOutcome, StockCagr as DomainStockCagr,
};

/// `stock_cagr` 資料表的資料列模型。
///
/// 領域層以 `Option<SimulationOutcome>` 包裝三個報酬口徑，資料庫則是平鋪欄位；
/// 兩者的對應規則：
/// - `Some(outcome)` → 該口徑各欄位填值。
/// - `None` → 該口徑各欄位為 `NULL`。
/// - 讀回時以 `*_end_value` 是否為 `NULL` 判定該口徑是否存在。
///
/// 另注意口徑 A（純價格）在資料表中沒有現金欄位——純價格模擬不會產生現金股利，
/// [`SimulationOutcome::cash_received`] 恆為零，故不浪費一個欄位儲存，
/// 讀回時直接補零。
#[derive(sqlx::FromRow, Debug, Clone, PartialEq, Eq)]
pub struct StockCagr {
    /// 計算基準日（期末交易日）。
    pub date: NaiveDate,
    /// 股票代號。
    pub stock_symbol: String,
    /// 統計期間代碼（見 [`CagrPeriod::code`]）。
    pub period: String,
    /// 實際採用的期初交易日。
    pub base_date: Option<NaiveDate>,
    /// 期初收盤價。
    pub base_price: Option<Decimal>,
    /// 期末收盤價。
    pub end_price: Option<Decimal>,
    /// 實際年數。
    pub years: Option<Decimal>,
    /// 口徑 A：期末持有股數。
    pub price_end_shares: Option<Decimal>,
    /// 口徑 A：期末總價值。
    pub price_end_value: Option<Decimal>,
    /// 口徑 A：區間總報酬率（%）。
    pub price_return_pct: Option<Decimal>,
    /// 口徑 A：年化報酬率（%）。
    pub price_cagr_pct: Option<Decimal>,
    /// 口徑 B：期末持有股數。
    pub total_shares: Option<Decimal>,
    /// 口徑 B：累積現金股利。
    pub total_cash: Option<Decimal>,
    /// 口徑 B：期末總價值。
    pub total_end_value: Option<Decimal>,
    /// 口徑 B：區間總報酬率（%）。
    pub total_return_pct: Option<Decimal>,
    /// 口徑 B：年化報酬率（%）。
    pub total_cagr_pct: Option<Decimal>,
    /// 口徑 C：期末持有股數。
    pub reinv_shares: Option<Decimal>,
    /// 口徑 C：剩餘現金。
    pub reinv_cash: Option<Decimal>,
    /// 口徑 C：期末總價值。
    pub reinv_end_value: Option<Decimal>,
    /// 口徑 C：區間總報酬率（%）。
    pub reinv_return_pct: Option<Decimal>,
    /// 口徑 C：年化報酬率（%）。
    pub reinv_cagr_pct: Option<Decimal>,
    /// 該股最早的報價日。
    pub first_quote_date: Option<NaiveDate>,
    /// 期初日順延天數。
    pub shortfall_days: Option<i32>,
    /// 期初資料是否齊全。
    pub data_complete: bool,
    /// 是否偵測到疑似減資或分割的異常跳動。
    pub has_anomaly: bool,
    /// 期間內採計的除權息次數。
    pub dividend_events: i32,
}

/// 單一口徑攤平後的資料庫欄位：（股數, 現金, 期末價值, 區間總報酬率, 年化報酬率）。
type FlatOutcome = (
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
);

/// 將單一口徑的模擬結果攤平成資料庫欄位。
///
/// `None` 時全部為 `None`，讓資料庫維持 `NULL`——以 0 代替會讓
/// 「資料不足」被誤讀成「零報酬」。
fn flatten_outcome(outcome: Option<SimulationOutcome>) -> FlatOutcome {
    match outcome {
        Some(o) => (
            Some(o.end_shares),
            Some(o.cash_received),
            Some(o.end_value),
            Some(o.total_return_pct),
            Some(o.cagr_pct),
        ),
        None => (None, None, None, None, None),
    }
}

/// 由攤平欄位還原單一口徑的模擬結果。
///
/// 以 `end_value` 是否存在作為該口徑存在與否的判準；其餘欄位若因舊資料
/// 而缺漏則補零，避免整列讀取失敗。
fn restore_outcome(
    end_shares: Option<Decimal>,
    cash_received: Option<Decimal>,
    end_value: Option<Decimal>,
    total_return_pct: Option<Decimal>,
    cagr_pct: Option<Decimal>,
) -> Option<SimulationOutcome> {
    end_value.map(|value| SimulationOutcome {
        end_shares: end_shares.unwrap_or_default(),
        cash_received: cash_received.unwrap_or_default(),
        end_value: value,
        total_return_pct: total_return_pct.unwrap_or_default(),
        cagr_pct: cagr_pct.unwrap_or_default(),
    })
}

impl From<&DomainStockCagr> for StockCagr {
    fn from(domain: &DomainStockCagr) -> Self {
        let (price_end_shares, _price_cash, price_end_value, price_return_pct, price_cagr_pct) =
            flatten_outcome(domain.price);
        let (total_shares, total_cash, total_end_value, total_return_pct, total_cagr_pct) =
            flatten_outcome(domain.total);
        let (reinv_shares, reinv_cash, reinv_end_value, reinv_return_pct, reinv_cagr_pct) =
            flatten_outcome(domain.reinvested);

        StockCagr {
            date: domain.date,
            stock_symbol: domain.stock_symbol.clone(),
            period: domain.period.code().to_string(),
            base_date: domain.base_date,
            base_price: domain.base_price,
            end_price: domain.end_price,
            years: domain.years,
            price_end_shares,
            price_end_value,
            price_return_pct,
            price_cagr_pct,
            total_shares,
            total_cash,
            total_end_value,
            total_return_pct,
            total_cagr_pct,
            reinv_shares,
            reinv_cash,
            reinv_end_value,
            reinv_return_pct,
            reinv_cagr_pct,
            first_quote_date: domain.first_quote_date,
            shortfall_days: domain.shortfall_days,
            data_complete: domain.data_complete,
            has_anomaly: domain.has_anomaly,
            dividend_events: domain.dividend_events,
        }
    }
}

impl StockCagr {
    /// 還原為領域實體。
    ///
    /// # Errors
    /// 當 `period` 欄位不是已知的期間代碼時回傳錯誤（代表資料表被寫入了
    /// 程式無法解釋的值，靜默忽略會讓排行榜少掉整個期間而難以察覺）。
    pub fn to_domain(&self) -> Result<DomainStockCagr> {
        let period = CagrPeriod::from_code(&self.period)
            .ok_or_else(|| anyhow!("無法辨識的 stock_cagr.period 代碼：{}", self.period))?;

        Ok(DomainStockCagr {
            date: self.date,
            stock_symbol: self.stock_symbol.clone(),
            period,
            base_date: self.base_date,
            base_price: self.base_price,
            end_price: self.end_price,
            years: self.years,
            // 口徑 A 無現金欄位，現金固定補零。
            price: restore_outcome(
                self.price_end_shares,
                Some(Decimal::ZERO),
                self.price_end_value,
                self.price_return_pct,
                self.price_cagr_pct,
            ),
            total: restore_outcome(
                self.total_shares,
                self.total_cash,
                self.total_end_value,
                self.total_return_pct,
                self.total_cagr_pct,
            ),
            reinvested: restore_outcome(
                self.reinv_shares,
                self.reinv_cash,
                self.reinv_end_value,
                self.reinv_return_pct,
                self.reinv_cagr_pct,
            ),
            first_quote_date: self.first_quote_date,
            shortfall_days: self.shortfall_days,
            data_complete: self.data_complete,
            has_anomaly: self.has_anomaly,
            dividend_events: self.dividend_events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn sample_outcome() -> SimulationOutcome {
        SimulationOutcome {
            end_shares: dec!(123.45678),
            cash_received: dec!(321.5),
            end_value: dec!(12345.6789),
            total_return_pct: dec!(23.4567),
            cagr_pct: dec!(7.8901),
        }
    }

    fn sample_domain() -> DomainStockCagr {
        DomainStockCagr {
            date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
            stock_symbol: "2330".to_string(),
            period: CagrPeriod::Y3,
            base_date: NaiveDate::from_ymd_opt(2023, 6, 30),
            base_price: Some(dec!(560.0)),
            end_price: Some(dec!(1000.0)),
            years: Some(dec!(3.001)),
            price: Some(sample_outcome()),
            total: Some(sample_outcome()),
            reinvested: None,
            first_quote_date: NaiveDate::from_ymd_opt(2000, 1, 4),
            shortfall_days: Some(0),
            data_complete: true,
            has_anomaly: false,
            dividend_events: 12,
        }
    }

    #[test]
    fn test_round_trip_keeps_values() {
        let domain = sample_domain();
        let row = StockCagr::from(&domain);
        let restored = row.to_domain().expect("期間代碼應可還原");

        // 口徑 A 的現金不落庫，還原時補零，故先個別比對再比其餘欄位。
        assert_eq!(restored.price.map(|o| o.cash_received), Some(Decimal::ZERO));
        assert_eq!(restored.total, domain.total);
        assert_eq!(restored.reinvested, None);
        assert_eq!(restored.date, domain.date);
        assert_eq!(restored.period, domain.period);
        assert_eq!(restored.years, domain.years);
        assert_eq!(restored.dividend_events, domain.dividend_events);
    }

    #[test]
    fn test_incomplete_row_maps_to_none() {
        let mut domain = sample_domain();
        domain.data_complete = false;
        domain.price = None;
        domain.total = None;
        domain.reinvested = None;
        domain.base_price = None;
        domain.years = None;

        let row = StockCagr::from(&domain);
        assert!(row.total_end_value.is_none());
        assert!(row.price_cagr_pct.is_none());

        let restored = row.to_domain().expect("期間代碼應可還原");
        assert!(restored.price.is_none());
        assert!(restored.total.is_none());
        assert!(restored.reinvested.is_none());
        assert!(!restored.data_complete);
    }

    #[test]
    fn test_unknown_period_code_is_error() {
        let mut row = StockCagr::from(&sample_domain());
        row.period = "Z9".to_string();
        assert!(row.to_domain().is_err());
    }
}
