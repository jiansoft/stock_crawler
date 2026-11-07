//! # 月營收共用型別
//!
//! 定義月營收採集的資料載體 (DTO)，以及由來源原始字串欄位轉換成
//! 該載體的邏輯（欄位為空或無法解析時退回預設值）。

use rust_decimal::Decimal;

/// 營收資訊爬蟲載體 (DTO)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevenueDto {
    /// 股票代號
    pub stock_symbol: String,
    /// 當月營收
    pub monthly: Decimal,
    /// 上月營收
    pub last_month: Decimal,
    /// 去年當月營收
    pub last_year_this_month: Decimal,
    /// 當月累計營收
    pub monthly_accumulated: Decimal,
    /// 去年累計營收
    pub last_year_monthly_accumulated: Decimal,
    /// 上月比較增減(%)
    pub compared_with_last_month: Decimal,
    /// 去年同月增減(%)
    pub compared_with_last_year_same_month: Decimal,
    /// 前期比較增減(%)
    pub accumulated_compared_with_last_year: Decimal,
    /// 營收月份 (YYYYMM 格式整數，如 202605)
    pub date: i64,
}

impl From<Vec<String>> for RevenueDto {
    fn from(item: Vec<String>) -> Self {
        Self {
            stock_symbol: item[0].to_string(),
            monthly: parse_amount(&item[2], "monthly"),
            last_month: parse_amount(&item[3], "last_month"),
            last_year_this_month: parse_amount(&item[4], "last_year_this_month"),
            monthly_accumulated: parse_amount(&item[7], "monthly_accumulated"),
            last_year_monthly_accumulated: parse_amount(&item[8], "last_year_monthly_accumulated"),
            compared_with_last_month: parse_amount(&item[5], "compared_with_last_month"),
            compared_with_last_year_same_month: parse_amount(
                &item[6],
                "compared_with_last_year_same_month",
            ),
            accumulated_compared_with_last_year: parse_amount(
                &item[9],
                "accumulated_compared_with_last_year",
            ),
            date: 0,
        }
    }
}

/// 去掉千分位逗號與空白後解析數字；空字串視為 0，無法解析時記 warn 並視為 0。
fn parse_amount(raw: &str, field: &str) -> Decimal {
    use std::str::FromStr;

    let cleaned = raw.replace([',', ' '], "");
    if cleaned.is_empty() {
        return Decimal::ZERO;
    }
    Decimal::from_str(&cleaned).unwrap_or_else(|why| {
        tracing::warn!(field, raw, "月營收欄位無法解析，改用 0: {why}");
        Decimal::ZERO
    })
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn row(values: [&str; 10]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// 欄位依來源順序對應；千分位與空白會去掉。
    #[test]
    fn maps_source_columns_and_strips_separators() {
        let dto = RevenueDto::from(row([
            "2330",
            "台積電",
            "1,000",
            "900",
            "800",
            "11.11",
            "25",
            "5,000",
            "4 000",
            "25.5",
        ]));

        assert_eq!(dto.stock_symbol, "2330");
        assert_eq!(
            (dto.monthly, dto.last_month, dto.last_year_this_month),
            (dec!(1000), dec!(900), dec!(800))
        );
        assert_eq!(
            (dto.monthly_accumulated, dto.last_year_monthly_accumulated),
            (dec!(5000), dec!(4000))
        );
        assert_eq!(
            (
                dto.compared_with_last_month,
                dto.compared_with_last_year_same_month,
                dto.accumulated_compared_with_last_year
            ),
            (dec!(11.11), dec!(25), dec!(25.5))
        );
        assert_eq!(dto.date, 0, "月份由呼叫端填入");
    }

    /// 空欄位與無法解析的值（例如來源的「-」、「N/A」）都改成 0，不讓整列失敗。
    #[test]
    fn empty_or_malformed_values_fall_back_to_zero() {
        let dto = RevenueDto::from(row([
            "1101", "台泥", "", "-", "N/A", " ", "1.2.3", "", "abc", "--",
        ]));

        assert_eq!(
            (dto.monthly, dto.last_month, dto.last_year_this_month),
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO)
        );
        assert_eq!(
            (dto.monthly_accumulated, dto.last_year_monthly_accumulated),
            (Decimal::ZERO, Decimal::ZERO)
        );
        assert_eq!(
            (
                dto.compared_with_last_month,
                dto.compared_with_last_year_same_month,
                dto.accumulated_compared_with_last_year
            ),
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO)
        );
    }
}
