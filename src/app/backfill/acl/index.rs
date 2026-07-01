use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 儲存大盤加權指數命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveIndexCommand {
    /// 日期
    pub date: NaiveDate,
    /// 收盤指數
    pub index: Decimal,
    /// 漲跌點數
    pub change: Decimal,
    /// 成交股數
    pub trading_volume: Decimal,
    /// 成交金額
    pub trade_value: Decimal,
    /// 成交筆數
    pub transaction: Decimal,
}

/// 大盤加權指數爬蟲資料防腐層轉譯器。
pub struct IndexAclMapper;

impl IndexAclMapper {
    /// 將爬蟲取得的大盤原始字串資料列轉譯為 `SaveIndexCommand`。
    pub fn from_strings(item: &[String]) -> Option<SaveIndexCommand> {
        if item.len() != 6 {
            return None;
        }

        let split_date: Vec<&str> = item[0].split('/').collect();
        if split_date.len() != 3 {
            return None;
        }

        let year = split_date[0].parse::<i32>().ok()?;
        let gregorian_year = crate::core::util::datetime::roc_year_to_gregorian_year(year);
        let date_str = format!("{}-{}-{}", gregorian_year, split_date[1], split_date[2]);
        let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").ok()?;

        let trading_volume = crate::core::util::text::parse_decimal(&item[1], None).ok()?;
        let trade_value = crate::core::util::text::parse_decimal(&item[2], None).ok()?;
        let transaction = crate::core::util::text::parse_decimal(&item[3], None).ok()?;
        let index = crate::core::util::text::parse_decimal(&item[4], None).ok()?;
        let change = crate::core::util::text::parse_decimal(&item[5], None).ok()?;

        Some(SaveIndexCommand {
            date,
            index,
            change,
            trading_volume,
            trade_value,
            transaction,
        })
    }

    /// 將 `SaveIndexCommand` 轉譯為市場指數領域實體 `MarketIndex`。
    pub fn from_command(cmd: &SaveIndexCommand) -> crate::domain::market_index::MarketIndex {
        crate::domain::market_index::MarketIndex::new(
            "TAIEX".to_string(),
            cmd.date,
            cmd.index,
            cmd.change,
            cmd.trade_value,
            cmd.transaction,
            cmd.trading_volume,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_index_acl_mapping() {
        let item = vec![
            "112/05/20".to_string(), // date
            "1,234,567".to_string(), // trading_volume
            "4,567,890".to_string(), // trade_value
            "100,000".to_string(),   // transaction
            "16,000.50".to_string(), // index
            "150.25".to_string(),    // change
        ];

        let cmd = IndexAclMapper::from_strings(&item);
        assert!(cmd.is_some());
        let cmd = cmd.unwrap();
        assert_eq!(cmd.date, NaiveDate::from_ymd_opt(2023, 5, 20).unwrap());
        assert_eq!(cmd.index, dec!(16000.50));
        assert_eq!(cmd.change, dec!(150.25));

        let entity = IndexAclMapper::from_command(&cmd);
        assert_eq!(entity.category, "TAIEX");
        assert_eq!(entity.index, dec!(16000.50));
        assert_eq!(entity.change, dec!(150.25));
    }
}
