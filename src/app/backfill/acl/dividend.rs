use rust_decimal::Decimal;

/// 儲存/更新股息明細命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDividendCommand {
    /// 股票代碼
    pub security_code: String,
    /// 發放年度
    pub year: i32,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 季度/半年資訊
    pub quarter: String,
    /// 現金股利
    pub cash_dividend: Decimal,
    /// 股票股利
    pub stock_dividend: Decimal,
    /// 股利合計
    pub sum: Decimal,
    /// 除息日
    pub ex_dividend_date1: String,
    /// 除權日
    pub ex_dividend_date2: String,
    /// 現金股利發放日
    pub payable_date1: String,
    /// 股票股利發放日
    pub payable_date2: String,
}

/// Yahoo 股利明細爬蟲資料防腐層轉譯器。
pub struct YahooDividendAclMapper;

impl YahooDividendAclMapper {
    /// 將 Yahoo 股利明細 DTO 轉譯為 `SaveDividendCommand`。
    pub fn from_dto(
        stock_symbol: &str,
        dto: &crate::infra::crawler::yahoo::dividend::YahooDividendDetail,
    ) -> SaveDividendCommand {
        SaveDividendCommand {
            security_code: stock_symbol.to_string(),
            year: dto.year,
            year_of_dividend: dto.year_of_dividend,
            quarter: dto.quarter.clone(),
            cash_dividend: dto.cash_dividend,
            stock_dividend: dto.stock_dividend,
            sum: dto.cash_dividend + dto.stock_dividend,
            ex_dividend_date1: dto.ex_dividend_date1.clone(),
            ex_dividend_date2: dto.ex_dividend_date2.clone(),
            payable_date1: dto.payable_date1.clone(),
            payable_date2: dto.payable_date2.clone(),
        }
    }

    /// 將 `SaveDividendCommand` 轉譯為領域模型 `Dividend`。
    pub fn from_command(cmd: &SaveDividendCommand) -> crate::domain::dividend::entity::Dividend {
        use chrono::Local;
        use rust_decimal::Decimal;
        crate::domain::dividend::entity::Dividend {
            serial: 0,
            security_code: cmd.security_code.clone(),
            year: cmd.year,
            year_of_dividend: cmd.year_of_dividend,
            quarter: cmd.quarter.clone(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: cmd.cash_dividend,
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: cmd.stock_dividend,
            sum: cmd.sum,
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: cmd.ex_dividend_date1.clone(),
            ex_dividend_date_stock: cmd.ex_dividend_date2.clone(),
            payable_date_cash: cmd.payable_date1.clone(),
            payable_date_stock: cmd.payable_date2.clone(),
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_yahoo_dividend_acl_mapping() {
        let detail = crate::infra::crawler::yahoo::dividend::YahooDividendDetail {
            year: 2025,
            year_of_dividend: 2024,
            quarter: "Q4".to_string(),
            cash_dividend: dec!(3.5),
            stock_dividend: dec!(0.2),
            ex_dividend_date1: "2025-07-01".to_string(),
            ex_dividend_date2: "-".to_string(),
            payable_date1: "2025-08-01".to_string(),
            payable_date2: "-".to_string(),
        };

        let cmd = YahooDividendAclMapper::from_dto("2454", &detail);
        assert_eq!(cmd.security_code, "2454");
        assert_eq!(cmd.sum, dec!(3.7));

        let entity = YahooDividendAclMapper::from_command(&cmd);
        assert_eq!(entity.security_code, "2454");
        assert_eq!(entity.sum, dec!(3.7));
        assert_eq!(entity.ex_dividend_date_cash, "2025-07-01");
    }
}
