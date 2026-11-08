//! `MoneyFlowRecalculated` 事件處理：重新計算並發送 Telegram 市值變化通知。

use std::fmt::Write;

use anyhow::Result;
use rust_decimal::Decimal;

use crate::app::event::taiwan_stock::{
    format_decimal_with_fixed_two_commas as format_decimal_with_commas, member_label,
};
// 通知走 core::alert（port），跳脫工具走 core::util::text——
// app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向。
use crate::core::{alert, util::text};
use crate::domain::money_flow::repository::MoneyFlowRepository;
use crate::infra::database::repository::money_flow::PgMoneyFlowRepository;

use super::EventDispatcher;

impl EventDispatcher {
    /// 處理 `MoneyFlowRecalculated` 事件：重新計算並發送 Telegram 市值變化通知。
    pub(super) async fn handle_money_flow_recalculated(date: chrono::NaiveDate) -> Result<()> {
        let money_flow_repo = PgMoneyFlowRepository::new();
        // 透過倉儲獲取會員收盤與前日市值之對照資料
        let rows = money_flow_repo
            .fetch_member_money_history_with_previous_day(date)
            .await?;
        // 建立通知內容並透過 AlertSink port 發送（生產環境為 Telegram adapter）
        if let Some(msg) = Self::build_money_change_message(&rows) {
            alert::send_message(&msg).await;
        }

        Ok(())
    }

    /// 格式化市值變化行文字。
    fn format_money_change_line(
        label: &str,
        market_value: Decimal,
        previous_market_value: Decimal,
    ) -> String {
        let diff = market_value - previous_market_value;
        let percentage = if previous_market_value.is_zero() {
            "N/A".to_string()
        } else {
            format_decimal_with_commas(
                (diff / previous_market_value) * rust_decimal_macros::dec!(100),
            )
        };

        format!(
            "{}:{} {} \\({}%\\)",
            text::escape_markdown_v2(label),
            text::escape_markdown_v2(format_decimal_with_commas(market_value)),
            text::escape_markdown_v2(format_decimal_with_commas(diff)),
            text::escape_markdown_v2(percentage),
        )
    }

    /// 組裝市值變化 Telegram 訊息。
    fn build_money_change_message(
        rows: &[crate::domain::money_flow::entity::MoneyFlowMemberWithPreviousDay],
    ) -> Option<String> {
        let date = rows.first()?.date;
        let mut msg = String::with_capacity(256);
        let _ = writeln!(
            &mut msg,
            "{} 市值變化",
            text::escape_markdown_v2(date.to_string())
        );

        // 合計列
        if let Some(total_row) = rows.iter().find(|row| row.member_id == 0) {
            let _ = writeln!(
                &mut msg,
                "{}",
                Self::format_money_change_line(
                    "合計",
                    total_row.market_value,
                    total_row.previous_market_value
                )
            );
        }

        // 個別會員列
        for row in rows.iter().filter(|row| row.member_id > 0) {
            let _ = writeln!(
                &mut msg,
                "{}",
                Self::format_money_change_line(
                    &member_label(row.member_id),
                    row.market_value,
                    row.previous_market_value,
                )
            );
        }

        Some(msg.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests;
