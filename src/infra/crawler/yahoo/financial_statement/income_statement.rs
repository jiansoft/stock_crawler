//! # Yahoo 損益表
//!
//! 對應頁面 `https://tw.stock.yahoo.com/quote/{代號}/income-statement`，
//! 資料來源為 `StockServices.incomeStatements-growthAnalyses`（注意不是 `incomeStatements`，
//! 後者會回 400；名稱取自前端 bundle），回應外層鍵為 `incomeStatementsList`。
//! 支援單季、累計與年度。共同規則（單位、日期、404）見 [`super`]。

use anyhow::Result;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{FinancialStatement, RawStatement, ReportPeriod, deserialize_decimal, fetch};

/// Yahoo 服務名稱。
const SERVICE: &str = "incomeStatements-growthAnalyses";

/// 損益表項目，金額單位為元，每股數值單位為元。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomeStatementItems {
    /// 營業收入。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub revenue: Option<Decimal>,
    /// 營業毛利。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub gross_profit: Option<Decimal>,
    /// 推銷費用。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub selling_expenses: Option<Decimal>,
    /// 管理費用。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub admin_expenses: Option<Decimal>,
    /// 研究發展費用。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub rd_expenses: Option<Decimal>,
    /// 營業費用。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub operating_expenses: Option<Decimal>,
    /// 營業利益。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub operating_profit: Option<Decimal>,
    /// 營業外收入及支出。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub non_operating_income: Option<Decimal>,
    /// 稅前淨利。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub profit_before_tax: Option<Decimal>,
    /// 本期淨利（稅後）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub net_income: Option<Decimal>,
    /// 歸屬母公司業主淨利。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub owner_parent_profit: Option<Decimal>,
    /// 每股營收（元）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub revenue_per_share: Option<Decimal>,
    /// 每股營業利益（元）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub operating_profit_per_share: Option<Decimal>,
    /// 每股稅前淨利（元）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub profit_before_tax_per_share: Option<Decimal>,
    /// 每股盈餘（元）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub eps: Option<Decimal>,
    /// 每股淨值（元）。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub bps: Option<Decimal>,
    /// 負債資金成本率（%），Yahoo 原始欄位 `debtCost`。
    ///
    /// Yahoo 頁面沒有顯示這個欄位（前端 bundle 完全沒有引用），語意由數值推斷：
    /// 期間值會隨期間長度累加（8042 單季約 0.5、2025 全年 2.11），各公司全年值
    /// 落在一般借款利率區間（2330 為 1.21、1101 為 2.73），金融業為 0，
    /// 研判為「利息費用 ÷ 有息負債」。尚未以財報的利息費用逐筆驗證。
    #[serde(default, deserialize_with = "deserialize_decimal")]
    pub debt_cost: Option<Decimal>,
    /// 近四季稅後淨利，單位為**百萬元**（與其他金額欄位的「元」不同），
    /// Yahoo 原始欄位 `netIncomeAcc4Q`。
    ///
    /// 只有累計期別有值，單季與年度為 `null`。Yahoo 頁面沒有顯示這個欄位，
    /// 語意由數值驗證：8042 2024Q1～2026Q2 共 10 期都等於近四季單季淨利加總
    /// （例如 2026Q2 為 `502.45`，四季加總為 502,447 千元）。
    #[serde(
        default,
        rename = "netIncomeAcc4Q",
        deserialize_with = "deserialize_decimal"
    )]
    pub net_income_acc_4q: Option<Decimal>,
}

/// 單一期別的損益表。
pub type IncomeStatement = FinancialStatement<IncomeStatementItems>;

/// `incomeStatements-growthAnalyses` 的回應外層。
#[derive(Debug, Deserialize)]
struct Response {
    #[serde(default, rename = "incomeStatementsList")]
    list: Vec<RawStatement<IncomeStatementItems>>,
}

/// 取得指定股票在該期別的全部損益表，依期間由新到舊排列。
///
/// 沒有財報的標的（如 ETF）回傳空陣列。
///
/// # Errors
///
/// HTTP 失敗、代號不存在（404，[`super::YahooPageNotFoundError`]）、
/// JSON 格式不符或日期無法對應期別時回傳錯誤。
pub async fn visit(stock_symbol: &str, period: ReportPeriod) -> Result<Vec<IncomeStatement>> {
    let response: Response = fetch(SERVICE, stock_symbol, period).await?;
    super::parse_statements(response.list, period)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::super::FiscalPeriod;
    use super::*;

    fn parse_fixture(json: &str, period: ReportPeriod) -> Vec<IncomeStatement> {
        let response: Response = serde_json::from_str(json).expect("fixture should parse");
        super::super::parse_statements(response.list, period).expect("statements")
    }

    #[test]
    fn test_parse_cumulative_quarter_fixture() {
        let result = parse_fixture(
            include_str!("../testdata/financial_income_statement_quarter_sum_8042.json"),
            ReportPeriod::CumulativeQuarter,
        );

        assert_eq!(result.len(), 2);
        let latest = &result[0];
        assert_eq!(latest.stock_symbol, "8042");
        assert_eq!(latest.report_period, ReportPeriod::CumulativeQuarter);
        assert_eq!(
            latest.fiscal_period,
            FiscalPeriod {
                year: 2026,
                quarter: Some(2)
            }
        );
        let items = &latest.items;
        assert_eq!(items.revenue, Some(dec!(2380422000.00)));
        assert_eq!(items.gross_profit, Some(dec!(505926000.00)));
        assert_eq!(items.operating_profit, Some(dec!(184249000.00)));
        assert_eq!(items.non_operating_income, Some(dec!(315963000.00)));
        assert_eq!(items.net_income, Some(dec!(381320000.00)));
        assert_eq!(items.owner_parent_profit, Some(dec!(372410000.00)));
        assert_eq!(items.eps, Some(dec!(2.84)));
        assert_eq!(items.bps, Some(dec!(34.64)));
        assert_eq!(items.net_income_acc_4q, Some(dec!(502.45)));
    }

    #[test]
    fn test_parse_year_fixture() {
        let result = parse_fixture(
            include_str!("../testdata/financial_income_statement_year_8042.json"),
            ReportPeriod::Year,
        );

        let latest = &result[0];
        assert_eq!(
            latest.fiscal_period,
            FiscalPeriod {
                year: 2025,
                quarter: None
            }
        );
        assert_eq!(latest.items.revenue, Some(dec!(3765883000.00)));
        assert_eq!(latest.items.eps, Some(dec!(0.89)));
        // 年度與單季的 netIncomeAcc4Q 是 null，必須是 None 而不是 0。
        assert_eq!(latest.items.net_income_acc_4q, None);
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();

        for period in [
            ReportPeriod::Quarter,
            ReportPeriod::CumulativeQuarter,
            ReportPeriod::Year,
        ] {
            match visit("8042", period).await {
                Ok(result) => {
                    println!("8042 損益表 {:?} {} 筆", period, result.len());
                    if let Some(first) = result.first() {
                        println!("{:?}", first);
                    }
                }
                Err(why) => println!("取得 8042 損益表失敗: {:?}", why),
            }
        }
    }
}
