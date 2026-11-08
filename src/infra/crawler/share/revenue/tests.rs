//! 月營收共用型別的測試。

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
