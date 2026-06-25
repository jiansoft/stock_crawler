use rust_decimal::Decimal;

/// 更新股息分配率命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePayoutRatioCommand {
    /// 股利資料序號
    pub serial: i64,
    /// 現金配發率
    pub payout_ratio_cash: Decimal,
    /// 股票配發率
    pub payout_ratio_stock: Decimal,
    /// 合計配發率
    pub payout_ratio: Decimal,
}

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

/// 股息分配率爬蟲資料防腐層轉譯器。
pub struct DividendAclMapper;

impl DividendAclMapper {
    /// 將 Goodinfo 股利 DTO 轉譯為 `UpdatePayoutRatioCommand`。
    pub fn from_dto(
        serial: i64,
        dto: &crate::infra::crawler::goodinfo::dividend::GoodInfoDividend,
    ) -> UpdatePayoutRatioCommand {
        UpdatePayoutRatioCommand {
            serial,
            payout_ratio_cash: dto.payout_ratio_cash,
            payout_ratio_stock: dto.payout_ratio_stock,
            payout_ratio: dto.payout_ratio,
        }
    }

    /// 將 `UpdatePayoutRatioCommand` 套用至 `Dividend`。
    pub fn update_payout_ratio_entity(
        dividend: &crate::domain::dividend::entity::Dividend,
        cmd: &UpdatePayoutRatioCommand,
    ) -> crate::domain::dividend::entity::Dividend {
        let mut d = dividend.clone();
        d.payout_ratio_cash = cmd.payout_ratio_cash;
        d.payout_ratio_stock = cmd.payout_ratio_stock;
        d.payout_ratio = cmd.payout_ratio;
        d
    }
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
    fn test_dividend_acl_mapping() {
        let mut goodinfo =
            crate::infra::crawler::goodinfo::dividend::GoodInfoDividend::new("2330".to_string());
        goodinfo.payout_ratio_cash = dec!(45.5);
        goodinfo.payout_ratio_stock = dec!(0.0);
        goodinfo.payout_ratio = dec!(45.5);

        let cmd = DividendAclMapper::from_dto(123, &goodinfo);
        assert_eq!(cmd.serial, 123);
        assert_eq!(cmd.payout_ratio_cash, dec!(45.5));

        let mut d = crate::domain::dividend::entity::Dividend {
            serial: 123,
            year: 2024,
            year_of_dividend: 2024,
            quarter: "Q4".to_string(),
            security_code: "2330".to_string(),
            earnings_cash_dividend: rust_decimal_macros::dec!(0),
            capital_reserve_cash_dividend: rust_decimal_macros::dec!(0),
            cash_dividend: rust_decimal_macros::dec!(0),
            earnings_stock_dividend: rust_decimal_macros::dec!(0),
            capital_reserve_stock_dividend: rust_decimal_macros::dec!(0),
            stock_dividend: rust_decimal_macros::dec!(0),
            sum: rust_decimal_macros::dec!(0),
            payout_ratio_cash: rust_decimal_macros::dec!(0.0),
            payout_ratio_stock: rust_decimal_macros::dec!(0.0),
            payout_ratio: rust_decimal_macros::dec!(0.0),
            ex_dividend_date_cash: "".to_string(),
            ex_dividend_date_stock: "".to_string(),
            payable_date_cash: "".to_string(),
            payable_date_stock: "".to_string(),
            created_time: chrono::Local::now(),
            updated_time: chrono::Local::now(),
        };
        d = DividendAclMapper::update_payout_ratio_entity(&d, &cmd);
        assert_eq!(d.payout_ratio_cash, dec!(45.5));
    }

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
