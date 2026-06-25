use crate::infra::crawler::share::RevenueDto;
use rust_decimal::Decimal;

/// 更新單月營收命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRevenueCommand {
    /// 股票代號
    pub symbol: String,
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
    /// 營收月份 (YYYYMM 格式整數)
    pub date: i64,
}

/// 營收爬蟲資料防腐層轉譯器。
pub struct RevenueAclMapper;

impl RevenueAclMapper {
    /// 將原始爬蟲 DTO 轉譯成 `UpdateRevenueCommand`。
    pub fn from_dto(dto: &RevenueDto) -> UpdateRevenueCommand {
        UpdateRevenueCommand {
            symbol: dto.stock_symbol.clone(),
            monthly: dto.monthly,
            last_month: dto.last_month,
            last_year_this_month: dto.last_year_this_month,
            monthly_accumulated: dto.monthly_accumulated,
            last_year_monthly_accumulated: dto.last_year_monthly_accumulated,
            compared_with_last_month: dto.compared_with_last_month,
            compared_with_last_year_same_month: dto.compared_with_last_year_same_month,
            accumulated_compared_with_last_year: dto.accumulated_compared_with_last_year,
            date: dto.date,
        }
    }

    /// 將 `UpdateRevenueCommand` 轉譯為資料庫 Table 模型 `Revenue`。
    pub fn from_command(
        cmd: &UpdateRevenueCommand,
    ) -> crate::infra::database::table::financial::revenue::Revenue {
        use chrono::Local;
        let mut r = crate::infra::database::table::financial::revenue::Revenue::new();
        r.stock_symbol = cmd.symbol.clone();
        r.monthly = cmd.monthly;
        r.last_month = cmd.last_month;
        r.last_year_this_month = cmd.last_year_this_month;
        r.monthly_accumulated = cmd.monthly_accumulated;
        r.last_year_monthly_accumulated = cmd.last_year_monthly_accumulated;
        r.compared_with_last_month = cmd.compared_with_last_month;
        r.compared_with_last_year_same_month = cmd.compared_with_last_year_same_month;
        r.accumulated_compared_with_last_year = cmd.accumulated_compared_with_last_year;
        r.date = cmd.date;
        r.create_time = Local::now();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_revenue_acl_mapping() {
        let dto = RevenueDto {
            stock_symbol: "2330".to_string(),
            monthly: dec!(1000.0),
            last_month: dec!(900.0),
            last_year_this_month: dec!(950.0),
            monthly_accumulated: dec!(5000.0),
            last_year_monthly_accumulated: dec!(4500.0),
            compared_with_last_month: dec!(11.1),
            compared_with_last_year_same_month: dec!(5.3),
            accumulated_compared_with_last_year: dec!(11.1),
            date: 202605,
        };

        let cmd = RevenueAclMapper::from_dto(&dto);
        assert_eq!(cmd.symbol, "2330");
        assert_eq!(cmd.monthly, dec!(1000.0));
        assert_eq!(cmd.date, 202605);

        let entity = RevenueAclMapper::from_command(&cmd);
        assert_eq!(entity.stock_symbol, "2330");
        assert_eq!(entity.monthly, dec!(1000.0));
        assert_eq!(entity.date, 202605);
    }
}
