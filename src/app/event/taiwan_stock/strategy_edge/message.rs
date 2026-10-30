//! 策略優勢檢查的告警訊息、每週摘要文字與日誌。

use chrono::Datelike;

use super::{EdgeRecord, EdgeWindow, RECENT_MONTHS, SLIPPAGE_BPS, Verdict, verdict};

/// 百分比文字（兩位小數）；沒有為「—」。
fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_owned(), |value| format!("{value:.2}%"))
}

/// 一段期間的摘要文字：`近 24 個月（2024-10-28～2026-10-27，20 個偏移）：策略 X%／0050 Y%，勝 0050 偏移 Z%`。
pub fn window_text(label: &str, window: &EdgeWindow) -> String {
    let beat = window
        .beat_rate
        .map_or_else(|| "—".to_owned(), |rate| format!("{:.0}%", rate * 100.0));
    format!(
        "{label}（{}～{}，{} 個偏移）：策略 {}／0050 {}，勝 0050 偏移 {beat}",
        window.from,
        window.to,
        window.offsets,
        percent(window.median_return),
        percent(window.etf_median_return)
    )
}

/// 近期一段的標籤。
fn recent_label() -> String {
    format!("近 {RECENT_MONTHS} 個月")
}

/// 全期一段的標籤。
fn full_label(window: &EdgeWindow) -> String {
    format!("{} 起", window.from.year())
}

/// 優勢轉弱的告警內容。
pub fn alert_message(record: &EdgeRecord, reasons: &[String]) -> String {
    let mut lines = vec![
        format!("{}：{}", record.display_name(), reasons.join("；")),
        window_text(&recent_label(), &record.recent),
    ];
    if let Some(full) = &record.full {
        lines.push(window_text(&full_label(full), full));
    }
    lines.push(format!(
        "成交假設：次日開盤＋{SLIPPAGE_BPS} bps，換股日偏移的中位數"
    ));
    lines.push(
        "依 10-25 決議建議停用此策略的換股通知（strategy_preset.alert）；不會自動停用。".to_owned(),
    );
    lines.push("僅供參考，不構成投資建議，亦不會自動下單。".to_owned());
    lines.join("\n")
}

/// 每週市場摘要的策略優勢各行（開頭為空行）；沒有紀錄回空。
pub fn weekly_lines(records: &[EdgeRecord]) -> Vec<String> {
    let Some(first) = records.first() else {
        return Vec::new();
    };
    let mut lines = vec![
        String::new(),
        format!(
            "策略優勢（{} 檢查，次日開盤＋{SLIPPAGE_BPS} bps，換股日偏移中位數）",
            first.checked
        ),
    ];
    for record in records {
        let mark = match verdict(&record.recent) {
            Verdict::Weakened(_) => "⚠️ ",
            Verdict::Holds | Verdict::Unknown => "",
        };
        lines.push(format!(
            "• {mark}{} {}",
            record.display_name(),
            window_text(&recent_label(), &record.recent)
        ));
        if let Some(full) = &record.full {
            lines.push(format!(
                "• {} {}",
                record.display_name(),
                window_text(&full_label(full), full)
            ));
        }
    }
    lines
}
/// 優勢轉弱的策略各一則告警（標題、內容）。
pub fn alerts(records: &[EdgeRecord]) -> Vec<(String, String)> {
    records
        .iter()
        .filter_map(|record| match verdict(&record.recent) {
            Verdict::Weakened(reasons) => Some((
                format!("策略優勢轉弱：{}", record.display_name()),
                alert_message(record, &reasons),
            )),
            Verdict::Holds | Verdict::Unknown => None,
        })
        .collect()
}

/// 每筆紀錄一行 info（判斷不了的另記 warn）。
pub(super) fn log_records(records: &[EdgeRecord]) {
    for record in records {
        let mut text = window_text(&recent_label(), &record.recent);
        if let Some(full) = &record.full {
            text.push('；');
            text.push_str(&window_text(&full_label(full), full));
        }
        match verdict(&record.recent) {
            Verdict::Holds => tracing::info!("策略優勢檢查 {}：{text}", record.display_name()),
            Verdict::Weakened(_) => {
                tracing::warn!("策略優勢檢查 {}（轉弱）：{text}", record.display_name())
            }
            Verdict::Unknown => {
                tracing::warn!(
                    "策略優勢檢查 {}（缺數字無法判斷）：{text}",
                    record.display_name()
                )
            }
        }
    }
}
