//! `MoneyFlowRecalculated` 事件處理：重新計算並發送 Telegram 市值變化通知。

use std::fmt::Write;

use anyhow::Result;
use rust_decimal::Decimal;

use crate::app::event::taiwan_stock::{
    format_decimal_with_fixed_two_commas as format_decimal_with_commas, member_label,
};
use crate::domain::money_flow::repository::MoneyFlowRepository;
use crate::infra::database::repository::money_flow::PgMoneyFlowRepository;
use crate::interfaces::bot::telegram::Telegram;

use super::EventDispatcher;

impl EventDispatcher {
    /// 處理 `MoneyFlowRecalculated` 事件：重新計算並發送 Telegram 市值變化通知。
    pub(super) async fn handle_money_flow_recalculated(date: chrono::NaiveDate) -> Result<()> {
        let money_flow_repo = PgMoneyFlowRepository::new();
        // 透過倉儲獲取會員收盤與前日市值之對照資料
        let rows = money_flow_repo
            .fetch_member_money_history_with_previous_day(date)
            .await?;
        // 建立通知內容並發送 Telegram 訊息
        if let Some(msg) = Self::build_money_change_message(&rows) {
            crate::interfaces::bot::telegram::send(&msg).await;
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
            Telegram::escape_markdown_v2(label),
            Telegram::escape_markdown_v2(format_decimal_with_commas(market_value)),
            Telegram::escape_markdown_v2(format_decimal_with_commas(diff)),
            Telegram::escape_markdown_v2(percentage),
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
            Telegram::escape_markdown_v2(date.to_string())
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
mod tests {
    use super::*;

    #[test]
    fn test_build_money_change_message_includes_hugo() {
        use crate::domain::money_flow::entity::MoneyFlowMemberWithPreviousDay;
        use chrono::NaiveDate;
        use rust_decimal_macros::dec;

        let date = NaiveDate::parse_from_str("2026-04-02", "%Y-%m-%d").unwrap();
        let previous_date = NaiveDate::parse_from_str("2026-04-01", "%Y-%m-%d").unwrap();
        let rows = vec![
            MoneyFlowMemberWithPreviousDay {
                date,
                previous_date: Some(previous_date),
                member_id: 0,
                market_value: dec!(4273187.20),
                previous_market_value: dec!(4053774.55),
            },
            MoneyFlowMemberWithPreviousDay {
                date,
                previous_date: Some(previous_date),
                member_id: 1,
                market_value: dec!(2195395.10),
                previous_market_value: dec!(2207807.70),
            },
            MoneyFlowMemberWithPreviousDay {
                date,
                previous_date: Some(previous_date),
                member_id: 2,
                market_value: dec!(1500000.00),
                previous_market_value: dec!(1400000.00),
            },
            MoneyFlowMemberWithPreviousDay {
                date,
                previous_date: Some(previous_date),
                member_id: 3,
                market_value: dec!(577792.10),
                previous_market_value: dec!(445966.85),
            },
        ];

        let msg =
            EventDispatcher::build_money_change_message(&rows).expect("message should be built");

        assert!(msg.contains("合計"));
        assert!(msg.contains("Eddie"));
        assert!(msg.contains("Unice"));
        assert!(msg.contains("Hugo"));
        assert!(msg.contains("4,273,187\\.20"));
        assert!(msg.contains("577,792\\.10"));
        assert!(msg.contains("\\-12,412\\.60"));
    }
}
