//! 外資及陸資持股領域實體。

use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 持股通知的門檻：近 20 個交易日外資持股比率變化達 ±3 個百分點。
///
/// 外資持股比率單日變動通常只有零點幾個百分點，20 日累積到 3 個百分點代表
/// 外資明顯在增持或撤出，值得提醒；門檻再低，大型權值股天天都會觸發。
pub const SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS: Decimal =
    Decimal::from_parts(3, 0, 0, false, 0);

/// 單一股票在某個交易日的外資持股快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignHolding {
    /// 股票代號
    pub stock_symbol: String,
    /// 資料日期（交易日）
    pub date: NaiveDate,
    /// 發行股數
    pub issued_share: i64,
    /// 外資及陸資持有股數
    pub shares_held: i64,
    /// 外資及陸資持股比率（%）
    pub share_holding_percentage: Decimal,
}

/// 外資持股變化方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForeignHoldingDirection {
    /// 增持
    Increase,
    /// 減持
    Decrease,
}

impl ForeignHoldingDirection {
    /// Redis 去重鍵與訊息使用的代碼。
    pub fn code(&self) -> &'static str {
        match self {
            ForeignHoldingDirection::Increase => "up",
            ForeignHoldingDirection::Decrease => "down",
        }
    }
}

/// 目前持股的外資持股趨勢，供持股通知使用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldingForeignHoldingAlert {
    /// 股票代號
    pub stock_symbol: String,
    /// 股票名稱
    pub stock_name: String,
    /// 趨勢基準日（最新交易日）
    pub date: NaiveDate,
    /// 基準日的外資持股比率（%）
    pub share_holding_percentage: Decimal,
    /// 近 5 個交易日持股比率變化（百分點）；歷史不足時為 None
    pub change_5d: Option<Decimal>,
    /// 近 20 個交易日持股比率變化（百分點）；歷史不足時為 None
    pub change_20d: Option<Decimal>,
    /// 以持有股數判斷的連續天數：正數連續增持、負數連續減持、0 持平
    pub streak: i32,
}

impl HoldingForeignHoldingAlert {
    /// 近 20 日變化達門檻時回傳方向；未達門檻或歷史不足 20 個交易日時回傳 `None`。
    pub fn significant_direction(&self, threshold: Decimal) -> Option<ForeignHoldingDirection> {
        let change = self.change_20d?;
        if change >= threshold {
            Some(ForeignHoldingDirection::Increase)
        } else if change <= -threshold {
            Some(ForeignHoldingDirection::Decrease)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn alert(change_20d: Option<Decimal>) -> HoldingForeignHoldingAlert {
        HoldingForeignHoldingAlert {
            stock_symbol: "2330".to_string(),
            stock_name: "台積電".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            share_holding_percentage: dec!(72.5),
            change_5d: None,
            change_20d,
            streak: 0,
        }
    }

    #[test]
    fn significant_direction_uses_inclusive_threshold_both_ways() {
        let threshold = SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS;
        assert_eq!(
            alert(Some(dec!(3))).significant_direction(threshold),
            Some(ForeignHoldingDirection::Increase)
        );
        assert_eq!(
            alert(Some(dec!(-3.2))).significant_direction(threshold),
            Some(ForeignHoldingDirection::Decrease)
        );
        assert_eq!(
            alert(Some(dec!(2.99))).significant_direction(threshold),
            None
        );
        assert_eq!(
            alert(Some(dec!(-2.99))).significant_direction(threshold),
            None
        );
    }

    /// 歷史不足 20 個交易日時沒有 20 日變化，不能當成 0 而是完全不通知。
    #[test]
    fn significant_direction_is_none_without_enough_history() {
        assert_eq!(
            alert(None).significant_direction(SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS),
            None
        );
    }

    #[test]
    fn threshold_constant_is_three_percentage_points() {
        assert_eq!(SIGNIFICANT_CHANGE_20D_PERCENTAGE_POINTS, dec!(3));
    }
}
