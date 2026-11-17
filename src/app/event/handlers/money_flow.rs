//! `MoneyFlowRecalculated` 事件處理：重新計算並發送 Telegram 市值變化通知。

use std::fmt::Write;

use anyhow::Result;
use rust_decimal::Decimal;

use crate::app::event::taiwan_stock::{
    format_decimal_with_fixed_two_commas as format_decimal_with_commas, member_label,
};
// 通知走 core::alert（port），跳脫工具走 core::util::text——
// app 層不 import interfaces::bot，維持「外層依賴內層」的合法方向。
use crate::app::notify;
use crate::core::{alert, util::text};
use crate::domain::household::DEFAULT_HOUSEHOLD_ID;
use crate::domain::money_flow::{
    entity::MoneyFlowMemberWithPreviousDay, repository::MoneyFlowRepository,
};
use crate::domain::portfolio::view::HoldingView;
use crate::infra::database::repository::{holding_view, money_flow::PgMoneyFlowRepository};

use super::EventDispatcher;

impl EventDispatcher {
    /// 處理 `MoneyFlowRecalculated` 事件：重新計算並發送 Telegram 市值變化通知。
    pub(super) async fn handle_money_flow_recalculated(date: chrono::NaiveDate) -> Result<()> {
        let money_flow_repo = PgMoneyFlowRepository::new();
        // 透過倉儲獲取會員收盤與前日市值之對照資料
        let rows = money_flow_repo
            .fetch_member_money_history_with_previous_day(date)
            .await?;
        // 依收件者範圍只列範圍內成員的市值（會員系統計畫 7.6）
        let dedup_key = format!("{}:{date}", alert::Topic::HoldingsMarketValue.code());
        notify::notify_holdings(alert::Topic::HoldingsMarketValue, &dedup_key, |view| {
            let rows = &rows;
            async move {
                let members = holding_view::member_ids(&view).await?;
                Ok(Self::build_money_change_message(&rows_in_view(
                    rows, &view, &members,
                )))
            }
        })
        .await;

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

/// 範圍內的市值列：只含本人時只有本人那一列；含家庭成員時為家庭成員與家庭合計（`member_id = 0`）。
///
/// 合計列目前只有預設家庭（結算依家庭分組在 Phase 7，2.7），其他家庭的範圍不列合計。
fn rows_in_view(
    rows: &[MoneyFlowMemberWithPreviousDay],
    view: &HoldingView,
    members: &[i64],
) -> Vec<MoneyFlowMemberWithPreviousDay> {
    rows.iter()
        .filter(|row| match view.member_id() {
            Some(member_id) => row.member_id == member_id,
            None => {
                members.contains(&row.member_id)
                    || (row.member_id == 0 && view.household_id() == DEFAULT_HOUSEHOLD_ID)
            }
        })
        .cloned()
        .collect()
}
