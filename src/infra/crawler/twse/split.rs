//! # 上市股票變更面額與 ETF 分割（反分割）恢復買賣參考價格
//!
//! | 函式 | 來源 |
//! |------|------|
//! | [`visit_par_value_changes`] | TWSE `rwd/zh/change/TWTB8U`（變更股票面額恢復買賣參考價格） |
//! | [`visit_etf_splits`] | TWSE `rwd/zh/split/TWTCAU`（ETF 分割（反分割）恢復買賣參考價格） |
//!
//! 兩者與減資的 `TWTAUU` 一樣支援 `startDate`／`endDate` 區間查詢，單一請求即可取回十餘年。
//!
//! ## 比例怎麼算
//!
//! 與減資相同取 `停止買賣前收盤價 ÷ 恢復買賣參考價`。參考價會依升降單位捨入，算出來的比例
//! 帶有微小誤差（0052 2025-11-26 為 245.30 ÷ 35.04 = 7.0006）；分割與反分割一定是整數倍，
//! 因此在 1% 以內時對齊到整數 `N`（分割）或 `1/N`（反分割）。
//!
//! 舊版這些事件是從漲跌停區間反推的，有兩筆推錯：00663L 2025-06-11 記成 6（實為 7）、
//! 00685L 2026-07-07 記成 25（實為 24）；面額變更 8070、6531、8476、6949 與 ETF 的
//! 00632R、0052、00631L 則完全沒登錄。

use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    core::util,
    domain::performance::{CorporateAction, CorporateActionType},
    infra::crawler::twse,
};

/// 端點回應主體；查無資料時沒有 `data`。
#[derive(Debug, Clone, Default, Deserialize)]
struct SplitResponse {
    #[serde(default)]
    stat: String,
    #[serde(default)]
    data: Option<Vec<Vec<String>>>,
}

/// 一張表的欄位位置。
struct Layout {
    /// 恢復買賣日期（民國 `YYY/MM/DD`）。
    date: usize,
    /// 代號。
    symbol: usize,
    /// 停止買賣前收盤價格。
    previous_close: usize,
    /// 恢復買賣參考價。
    reference_price: usize,
    /// 備註前綴。
    note: &'static str,
}

/// TWTB8U：`恢復買賣日期, 股票代號, 名稱, 停止買賣前收盤價格, 恢復買賣參考價, …`
const PAR_VALUE_CHANGE: Layout = Layout {
    date: 0,
    symbol: 1,
    previous_close: 3,
    reference_price: 4,
    note: "變更面額",
};

/// TWTCAU：`恢復買賣日期, ETF代號, 名稱, 分割(反分割), 停止買賣前收盤價格, 恢復買賣參考價, …`
const ETF_SPLIT: Layout = Layout {
    date: 0,
    symbol: 1,
    previous_close: 4,
    reference_price: 5,
    note: "ETF",
};

/// 比例對齊整數倍的容許誤差（1%）。
const SNAP_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 2);

/// 取得指定期間內上市股票的變更面額事件（分割或反分割）。
///
/// # Errors
///
/// HTTP 請求、JSON 反序列化失敗或端點拒絕查詢時回傳錯誤；區間內沒有事件回傳空陣列。
pub async fn visit_par_value_changes(
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<CorporateAction>> {
    visit("change/TWTB8U", start, end, &PAR_VALUE_CHANGE).await
}

/// 取得指定期間內 ETF 的分割與反分割事件。
///
/// # Errors
///
/// HTTP 請求、JSON 反序列化失敗或端點拒絕查詢時回傳錯誤；區間內沒有事件回傳空陣列。
pub async fn visit_etf_splits(start: NaiveDate, end: NaiveDate) -> Result<Vec<CorporateAction>> {
    visit("split/TWTCAU", start, end, &ETF_SPLIT).await
}

async fn visit(
    path: &str,
    start: NaiveDate,
    end: NaiveDate,
    layout: &Layout,
) -> Result<Vec<CorporateAction>> {
    let url = format!(
        "https://www.{host}/rwd/zh/{path}?response=json&startDate={start}&endDate={end}",
        host = twse::HOST,
        start = start.format("%Y%m%d"),
        end = end.format("%Y%m%d"),
    );
    let response = util::http::get_json::<SplitResponse>(&url).await?;
    if response.stat != "OK" && !response.stat.contains("沒有符合條件的資料") {
        anyhow::bail!(
            "TWSE 拒絕 {path} 查詢（{start} ~ {end}）：{}",
            response.stat
        );
    }
    Ok(parse_rows(response.data.unwrap_or_default(), layout))
}

/// 把資料列整理成 [`CorporateAction`]，依 `(代號, 生效日)` 排序；無法解析的列略過。
fn parse_rows(rows: Vec<Vec<String>>, layout: &Layout) -> Vec<CorporateAction> {
    let mut actions: Vec<CorporateAction> = rows
        .iter()
        .filter_map(|row| parse_row(row, layout))
        .collect();
    actions.sort_by(|a, b| {
        a.stock_symbol
            .cmp(&b.stock_symbol)
            .then(a.effective_date.cmp(&b.effective_date))
    });
    actions
        .dedup_by(|a, b| a.stock_symbol == b.stock_symbol && a.effective_date == b.effective_date);
    actions
}

fn parse_row(row: &[String], layout: &Layout) -> Option<CorporateAction> {
    let cell = |index: usize| row.get(index).map(|value| value.trim());
    let effective_date = util::datetime::parse_taiwan_date(cell(layout.date)?)?;
    let stock_symbol = cell(layout.symbol)?;
    if stock_symbol.is_empty() {
        return None;
    }
    // 尚未恢復交易的列價格是 `-`，解析失敗就略過。
    let previous_close = util::text::parse_decimal(cell(layout.previous_close)?, None).ok()?;
    let reference_price = util::text::parse_decimal(cell(layout.reference_price)?, None).ok()?;
    if previous_close <= Decimal::ZERO || reference_price <= Decimal::ZERO {
        return None;
    }

    let share_ratio = snap_ratio(previous_close.checked_div(reference_price)?);
    let (action_type, label) = if share_ratio >= Decimal::ONE {
        (CorporateActionType::Split, "分割")
    } else {
        (CorporateActionType::ReverseSplit, "反分割")
    };
    Some(CorporateAction {
        stock_symbol: stock_symbol.to_string(),
        effective_date,
        action_type,
        share_ratio,
        note: format!(
            "{}（{label}）：{} ÷ {}",
            layout.note,
            previous_close.normalize(),
            reference_price.normalize()
        ),
    })
}

/// 比例在 1% 以內時對齊到整數 `N` 或 `1/N`（四捨五入到小數 8 位，與資料表精度一致）。
fn snap_ratio(ratio: Decimal) -> Decimal {
    let near = |value: Decimal| {
        let whole = value.round();
        (whole >= Decimal::ONE && ((value - whole).abs() / whole) <= SNAP_TOLERANCE)
            .then_some(whole)
    };
    if ratio >= Decimal::ONE {
        near(ratio).unwrap_or(ratio)
    } else {
        near(Decimal::ONE / ratio)
            .map(|whole| (Decimal::ONE / whole).round_dp(8))
            .unwrap_or(ratio)
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    const PAR_VALUE_FIXTURE: &str = include_str!("testdata/par_value_change_twtb8u.json");
    const ETF_SPLIT_FIXTURE: &str = include_str!("testdata/etf_split_twtcau.json");

    fn parse_fixture(fixture: &str, layout: &Layout) -> Vec<CorporateAction> {
        let response: SplitResponse = serde_json::from_str(fixture).expect("fixture 是合法 JSON");
        parse_rows(response.data.unwrap_or_default(), layout)
    }

    fn find<'a>(actions: &'a [CorporateAction], symbol: &str) -> &'a CorporateAction {
        actions
            .iter()
            .find(|action| action.stock_symbol == symbol)
            .unwrap_or_else(|| panic!("找不到 {symbol}"))
    }

    /// 面額變更 10 筆全部是分割，比例為整數倍。
    #[test]
    fn parse_par_value_changes_fixture() {
        let actions = parse_fixture(PAR_VALUE_FIXTURE, &PAR_VALUE_CHANGE);
        assert_eq!(actions.len(), 10);
        assert!(
            actions
                .iter()
                .all(|action| action.action_type == CorporateActionType::Split)
        );

        let chang_wah = find(&actions, "8070");
        assert_eq!(
            chang_wah.effective_date,
            NaiveDate::from_ymd_opt(2020, 8, 17).unwrap()
        );
        assert_eq!(chang_wah.share_ratio, dec!(10));
        assert_eq!(chang_wah.note, "變更面額（分割）：190 ÷ 19");
        assert_eq!(find(&actions, "6949").share_ratio, dec!(20));
        assert_eq!(find(&actions, "6415").share_ratio, dec!(4));
    }

    /// ETF 分割與反分割：比例對齊到整數倍，修正舊版推錯的 00663L（7）與 00685L（24）。
    #[test]
    fn parse_etf_splits_fixture() {
        let actions = parse_fixture(ETF_SPLIT_FIXTURE, &ETF_SPLIT);
        assert_eq!(actions.len(), 11);

        assert_eq!(find(&actions, "0052").share_ratio, dec!(7));
        assert_eq!(find(&actions, "00631L").share_ratio, dec!(22));
        assert_eq!(find(&actions, "00663L").share_ratio, dec!(7));
        assert_eq!(find(&actions, "00685L").share_ratio, dec!(24));

        let reverse = find(&actions, "00632R");
        assert_eq!(reverse.action_type, CorporateActionType::ReverseSplit);
        assert_eq!(reverse.share_ratio, dec!(0.14285714));
        assert_eq!(reverse.note, "ETF（反分割）：3.28 ÷ 22.96");
        assert_eq!(find(&actions, "00676R").share_ratio, dec!(0.16666667));
    }

    /// 1% 以內對齊整數倍；超出容許誤差（不是整數倍的事件）保留原比例。
    #[test]
    fn snap_ratio_aligns_only_near_whole_multiples() {
        assert_eq!(snap_ratio(dec!(7.0006)), dec!(7));
        assert_eq!(snap_ratio(dec!(0.25)), dec!(0.25));
        assert_eq!(snap_ratio(dec!(0.1668)), dec!(0.16666667));
        assert_eq!(snap_ratio(dec!(2.5)), dec!(2.5));
        assert_eq!(snap_ratio(dec!(0.4)), dec!(0.4));
    }

    /// 價格是 `-`（尚未恢復交易）或欄位不足的列略過，不當成 0。
    #[test]
    fn parse_rows_skips_unpriced_and_short_rows() {
        let row = |close: &str, reference: &str| -> Vec<String> {
            ["115/10/20", "1234", "測試", close, reference]
                .into_iter()
                .map(str::to_string)
                .collect()
        };
        let actions = parse_rows(
            vec![
                row("-", "-"),
                row("100", "0"),
                vec!["115/10/20".to_string()],
                row("100", "10"),
            ],
            &PAR_VALUE_CHANGE,
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].share_ratio, dec!(10));
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        let start = NaiveDate::from_ymd_opt(2011, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        let par = visit_par_value_changes(start, end).await.expect("面額變更");
        let etf = visit_etf_splits(start, end).await.expect("ETF 分割");
        dbg!(par.len(), etf.len());
        assert!(!par.is_empty() && !etf.is_empty());
    }
}
