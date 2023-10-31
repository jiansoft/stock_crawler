use anyhow::{Context, Result};
use rust_decimal::Decimal;
use sqlx::{postgres::PgQueryResult, FromRow};

use crate::{
    database,
    util::convert::FromValue
};

///
#[derive(FromRow, Debug)]
pub struct QualifiedForeignInstitutionalInvestor {
    pub stock_symbol: String,
    /// 已發行股數
    pub issued_share: i64,
    /// 全體外資及陸資持有股數
    pub qfii_shares_held: i64,
    /// 全體外資及陸資持股比率
    pub qfii_share_holding_percentage: Decimal,
}

impl QualifiedForeignInstitutionalInvestor {
    pub fn new(
        stock_symbol: String,
        issued_share: i64,
        qfii_shares_held: i64,
        qfii_share_holding_percentage: Decimal,
    ) -> Self {
        QualifiedForeignInstitutionalInvestor {
            stock_symbol,
            issued_share,
            qfii_shares_held,
            qfii_share_holding_percentage,
        }
    }

    /// 更新個股的合格境外機構投資者的數據
    pub async fn update(&self) -> Result<PgQueryResult> {
        let sql = r#"
UPDATE
    stocks
SET
    issued_share = $2,
    qfii_shares_held = $3,
    qfii_share_holding_percentage = $4
WHERE
    stock_symbol = $1;
"#;
        sqlx::query(sql)
            .bind(&self.stock_symbol)
            .bind(self.issued_share)
            .bind(self.qfii_shares_held)
            .bind(self.qfii_share_holding_percentage)
            .execute(database::get_connection())
            .await
            .context(format!("Failed to update({:#?}) from database", self))
    }
}

//上櫃股票
impl From<Vec<String>> for QualifiedForeignInstitutionalInvestor {
    fn from(item: Vec<String>) -> Self {
        let stock_symbol = item[1].get_string(None);
        let issued_share = item[5].get_i64(None);
        let qfii_shares_held = item[9].get_i64(None);
        let qfii_share_holding_percentage = item[13].get_decimal(Some(vec!['\u{a0}']));

        QualifiedForeignInstitutionalInvestor::new(
            stock_symbol,
            issued_share,
            qfii_shares_held,
            qfii_share_holding_percentage,
        )
    }
}

// 上市股票
impl From<Vec<serde_json::Value>> for QualifiedForeignInstitutionalInvestor {
    fn from(item: Vec<serde_json::Value>) -> Self {
        let stock_symbol = item[0].get_string(None);
        let issued_share = item[3].get_i64(None);
        let qfii_shares_held = item[5].get_i64(None);
        let qfii_share_holding_percentage = item[7].get_decimal(None);

        QualifiedForeignInstitutionalInvestor::new(
            stock_symbol,
            issued_share,
            qfii_shares_held,
            qfii_share_holding_percentage,
        )
    }
}
