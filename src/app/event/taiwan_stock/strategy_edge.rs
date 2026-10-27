//! # 策略優勢的滾動檢查
//!
//! 2026-10-25 的決議：策略換股通知所依據的策略「不再勝過 0050」就停用通知。10-27 的 T7 重算結論是仍勝
//! 0050、但優勢偏薄（2019 起、次日開盤＋20 bps，換股日偏移中位數 31.77% vs 0050 30.36%，勝 0050 的
//! 偏移 65%），而通知附的實盤累積從 10-01 才開始，看不出趨勢。這裡把手動重算改成排程：
//!
//! - 每月第一個交易日 07:30，對每個要通知的策略（與 [`super::strategy_alert`] 相同的清單）跑兩段換股日
//!   偏移掃描：近 [`RECENT_MONTHS`] 個月與 [`FULL_FROM`] 起全期，成交假設一律次日開盤＋[`SLIPPAGE_BPS`] bps。
//! - 近 [`RECENT_MONTHS`] 個月的扣成本年化中位數低於 0050，或勝 0050 報酬的偏移比例不到
//!   [`MIN_BEAT_RATE`] 時發 Telegram 告警，附上兩段數字與停用建議。**不會自動停用通知**：停用會改變對外
//!   通知，由使用者確認。
//! - 結果存 Redis（[`REDIS_KEY`]），每週市場摘要附在策略一段。
//!
//! 回測引擎在 `interfaces::web::data_api`，這裡只透過 [`ports::StrategyGateway::offset_edge`] 取得分布。

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use chrono::{Datelike, Local, Months, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

use super::strategy_alert::{AlertStrategy, NAME_ENV, QUERY_ENV, alert_strategies};
use crate::{
    app::ports::{self, StrategyGateway, StrategyOffsetEdge},
    core::alert,
    domain::strategy::StrategyPresetRepository,
    infra::{
        crawler::twse, database::repository::strategy_preset::PgStrategyPresetRepository,
        nosql::redis,
    },
};

/// 近期一段的月數。
pub const RECENT_MONTHS: u32 = 24;
/// 全期一段的起日（相對強度資料從 2019 年起）。
pub const FULL_FROM: &str = "2019-01-02";
/// 單邊滑價（基點），與 T7 的成交假設相同。
pub const SLIPPAGE_BPS: u16 = 20;
/// 勝 0050 報酬的偏移比例低於此值就告警。
pub const MIN_BEAT_RATE: f64 = 0.5;
/// 最近一次檢查結果的 Redis 鍵。
pub const REDIS_KEY: &str = "strategy_edge:latest";
/// Redis 保存天數：超過一個月沒有新的檢查（排程失敗）時，週報不再附舊數字。
const REDIS_TTL_DAYS: usize = 40;
/// 檢查時會被覆寫或移除的回測參數。
const OVERRIDDEN_KEYS: [&str; 5] = [
    "from",
    "to",
    "execution_price",
    "slippage_bps",
    "rebalance_offset",
];

/// 一段期間的偏移分布摘要。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeWindow {
    /// 實際起日。
    pub from: NaiveDate,
    /// 實際迄日。
    pub to: NaiveDate,
    /// 跑了幾個偏移。
    pub offsets: usize,
    /// 扣成本年化報酬中位數（%）。
    pub median_return: Option<f64>,
    /// 0050 年化報酬中位數（%）。
    pub etf_median_return: Option<f64>,
    /// 勝 0050 報酬的偏移比例（0–1）。
    pub beat_rate: Option<f64>,
}

impl From<StrategyOffsetEdge> for EdgeWindow {
    fn from(edge: StrategyOffsetEdge) -> Self {
        Self {
            from: edge.from,
            to: edge.to,
            offsets: edge.offsets,
            median_return: edge.median_return,
            etf_median_return: edge.etf_median_return,
            beat_rate: edge.beat_rate,
        }
    }
}

/// 一個策略一次檢查的結果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeRecord {
    /// 策略名稱（`.env` 那一組沒有名稱時為空）。
    pub name: String,
    /// 檢查日。
    pub checked: NaiveDate,
    /// 近 [`RECENT_MONTHS`] 個月。
    pub recent: EdgeWindow,
    /// [`FULL_FROM`] 起全期；失敗時為 `None`（只看近期一段）。
    pub full: Option<EdgeWindow>,
}

impl EdgeRecord {
    /// 通知與日誌用的策略名稱。
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "策略"
        } else {
            &self.name
        }
    }
}

/// 近期一段的判斷。
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// 仍勝過 0050。
    Holds,
    /// 輸 0050 或勝率不到一半，附原因。
    Weakened(Vec<String>),
    /// 缺數字（期間內沒有任何一期），無法判斷。
    Unknown,
}

/// 依近期一段判斷策略是否仍勝過 0050：中位數低於 0050，或勝 0050 的偏移比例低於 [`MIN_BEAT_RATE`]
/// （剛好一半不算）。
pub fn verdict(window: &EdgeWindow) -> Verdict {
    let (Some(median), Some(etf), Some(beat)) = (
        window.median_return,
        window.etf_median_return,
        window.beat_rate,
    ) else {
        return Verdict::Unknown;
    };
    let mut reasons = Vec::new();
    if median < etf {
        reasons.push(format!(
            "扣成本年化中位數 {median:.2}% 低於 0050 的 {etf:.2}%"
        ));
    }
    if beat < MIN_BEAT_RATE {
        reasons.push(format!(
            "勝 0050 的換股日偏移只有 {:.0}%（門檻 {:.0}%）",
            beat * 100.0,
            MIN_BEAT_RATE * 100.0
        ));
    }
    if reasons.is_empty() {
        Verdict::Holds
    } else {
        Verdict::Weakened(reasons)
    }
}

/// 把策略條件改成檢查用的條件：起日換成 `from`、不設迄日（算到最新）、成交價固定為次日開盤加
/// [`SLIPPAGE_BPS`] bps；其餘條件不變。
pub fn edge_query(query: &str, from: NaiveDate) -> String {
    let mut pairs: Vec<String> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or_default();
            !OVERRIDDEN_KEYS.contains(&key)
        })
        .map(str::to_owned)
        .collect();
    pairs.push(format!("from={from}"));
    pairs.push("execution_price=next_open".to_owned());
    pairs.push(format!("slippage_bps={SLIPPAGE_BPS}"));
    pairs.join("&")
}

/// `today` 是否為當月第一個交易日（今天是交易日，且當月較早的平日都是休市日）。
pub fn is_first_trading_day(today: NaiveDate, holidays: &HashSet<NaiveDate>) -> bool {
    let trading = |date: NaiveDate| {
        !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) && !holidays.contains(&date)
    };
    trading(today)
        && (1..today.day())
            .filter_map(|day| today.with_day(day))
            .all(|date| !trading(date))
}

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

/// 對每個策略跑兩段偏移掃描；近期一段失敗的策略記入錯誤清單，全期一段失敗只記警告。
pub(super) async fn check(
    gateway: &dyn StrategyGateway,
    strategies: &[AlertStrategy],
    today: NaiveDate,
) -> (Vec<EdgeRecord>, Vec<String>) {
    let recent_from = today
        .checked_sub_months(Months::new(RECENT_MONTHS))
        .unwrap_or(today);
    let full_from = NaiveDate::parse_from_str(FULL_FROM, "%Y-%m-%d").unwrap_or(recent_from);
    let mut records = Vec::new();
    let mut failures = Vec::new();
    for strategy in strategies {
        let recent = match gateway
            .offset_edge(&edge_query(&strategy.query, recent_from))
            .await
        {
            Ok(edge) => EdgeWindow::from(edge),
            Err(error) => {
                tracing::error!("策略優勢檢查：{} 近期回測失敗：{error:?}", strategy.name);
                failures.push(format!("{}：{error:#}", strategy.name));
                continue;
            }
        };
        let full = match gateway
            .offset_edge(&edge_query(&strategy.query, full_from))
            .await
        {
            Ok(edge) => Some(EdgeWindow::from(edge)),
            Err(error) => {
                tracing::warn!(
                    "策略優勢檢查：{} 全期回測失敗，只看近期：{error:?}",
                    strategy.name
                );
                None
            }
        };
        records.push(EdgeRecord {
            name: strategy.name.clone(),
            checked: today,
            recent,
            full,
        });
    }
    (records, failures)
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
fn log_records(records: &[EdgeRecord]) {
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

/// 最近一次存入 Redis 的檢查結果；沒有、過期或讀取失敗回空（失敗記警告）。
pub async fn load_latest() -> Vec<EdgeRecord> {
    match redis::CLIENT.get_string(REDIS_KEY).await {
        Ok(json) => serde_json::from_str(&json).unwrap_or_else(|error| {
            tracing::warn!("策略優勢檢查：Redis 內容無法解析：{error}");
            Vec::new()
        }),
        Err(error) => {
            tracing::debug!("策略優勢檢查：讀取 Redis 失敗或沒有紀錄：{error}");
            Vec::new()
        }
    }
}

/// 存入 Redis 供每週市場摘要讀取；失敗只記警告。
async fn store(records: &[EdgeRecord]) {
    let json = match serde_json::to_string(records) {
        Ok(json) => json,
        Err(error) => {
            tracing::warn!("策略優勢檢查：序列化失敗：{error}");
            return;
        }
    };
    if let Err(error) = redis::CLIENT
        .set(REDIS_KEY, json, REDIS_TTL_DAYS * 24 * 60 * 60)
        .await
    {
        tracing::warn!("策略優勢檢查：寫入 Redis 失敗：{error}");
    }
}

/// 排程入口（每月 1–10 日 07:30）：今天是當月第一個交易日才檢查。
///
/// # Errors
///
/// 見 [`run`]。
pub async fn execute() -> Result<()> {
    let today = Local::now().date_naive();
    let holidays: HashSet<NaiveDate> = match twse::holiday_schedule::visit(today.year()).await {
        Ok(holidays) => holidays.into_iter().map(|holiday| holiday.date).collect(),
        Err(error) => {
            tracing::warn!("策略優勢檢查：讀取休市日失敗，只以週末判斷：{error:?}");
            HashSet::new()
        }
    };
    if !is_first_trading_day(today, &holidays) {
        tracing::debug!("策略優勢檢查：{today} 不是當月第一個交易日，略過");
        return Ok(());
    }
    run(today).await
}

/// 立即檢查一次（排程與手動回補入口共用）：記日誌、轉弱時告警、結果存 Redis。
///
/// # Errors
///
/// 讀不到具名策略、策略 gateway 未註冊，或有策略的近期回測失敗時回傳錯誤（其餘策略照常檢查）。
pub async fn run(today: NaiveDate) -> Result<()> {
    let presets = PgStrategyPresetRepository::new()
        .list()
        .await
        .context("讀取具名策略失敗")?;
    let strategies = alert_strategies(
        presets,
        std::env::var(QUERY_ENV).ok(),
        std::env::var(NAME_ENV).ok(),
    );
    if strategies.is_empty() {
        tracing::info!("策略優勢檢查：沒有要通知的具名策略，也未設定 {QUERY_ENV}，略過");
        return Ok(());
    }
    let gateway = ports::strategy_gateway()?;
    let (records, failures) = check(gateway.as_ref(), &strategies, today).await;
    log_records(&records);
    for (title, message) in alerts(&records) {
        alert::send_alert(&title, &message).await;
    }
    if !records.is_empty() {
        store(&records).await;
    }
    if !failures.is_empty() {
        bail!("策略優勢檢查失敗：{}", failures.join("；"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
