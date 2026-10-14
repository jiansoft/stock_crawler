//! # 除權息日漲跌修正
//!
//! 證交所與櫃買中心的每日收盤行情在除權息當天，漲跌方向欄是「X」等標記、漲跌價差為 0.00，
//! 日線因此記成漲跌 0、漲跌幅 0（2024 年以來約 98% 的除權息日如此）：市場廣度把這些股票
//! 算成平盤，漲跌排行與產業表現也看不到它們當天的實際漲跌。
//!
//! 官方「除權除息計算結果表」有當天的開盤競價基準（依升降單位進位後的參考價），以它重算
//! 漲跌＝收盤 − 基準、漲跌幅＝漲跌 ÷ 基準 × 100，與交易所網站一致。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 除權息當天的開盤競價基準。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExRightReference {
    /// 股票代號。
    pub stock_symbol: String,
    /// 除權息交易日。
    pub date: NaiveDate,
    /// 開盤競價基準（上市「開盤競價基準」、上櫃「開始交易基準價」）。
    pub reference_price: Decimal,
}

/// 除權息日漲跌修正需要的倉儲操作。
#[async_trait]
pub trait ExRightChangeRepository: Send + Sync {
    /// 以基準價重算日線中「漲跌為 0、有成交」的除權息日，回傳更新的日線列數；
    /// 已有非 0 漲跌的列不動，重複執行不會改變結果。
    ///
    /// `with_stats` 為 `true` 時，同一交易內把市場廣度統計表當天的上漲、下跌、平盤家數
    /// 依修正結果調整（只計入統計表取樣範圍內的股票）。歷史回補要開；收盤流程在統計表
    /// 寫入之前修正，不需要。
    async fn apply_ex_right_references(
        &self,
        references: &[ExRightReference],
        with_stats: bool,
    ) -> Result<u64>;
}
