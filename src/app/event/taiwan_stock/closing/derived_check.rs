//! 收盤衍生資料檢查：技術訊號、未來報酬與相對強度缺了就只重算缺的那一步並告警。

use super::*;

/// 收盤匯總最後幾步產生的衍生資料（依計算順序：相對強度要用到未來報酬）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Derived {
    /// 當天的技術訊號（`daily_technical_signal`）。
    Signals,
    /// 5 個交易日前基準日的 5 日報酬（`daily_forward_return`）。
    ForwardReturns,
    /// 當天的相對強度（`daily_relative_strength`）。
    RelativeStrength,
}

impl Derived {
    fn label(self) -> &'static str {
        match self {
            Self::Signals => "技術訊號",
            Self::ForwardReturns => "未來報酬",
            Self::RelativeStrength => "相對強度",
        }
    }
}

/// 行情完整後檢查衍生資料：當天沒有技術訊號，或 5 個交易日前的基準日沒有 5 日報酬
/// （收盤匯總在這兩步之前中斷、或這兩步失敗），就只重算缺的那一步並告警。
///
/// 當天不是交易日（沒有日線）或交易日不足 5 天時不檢查。
pub(super) async fn ensure_derived(date: NaiveDate) -> Result<()> {
    let report = check_derived(&PgQuoteRepository::new(), date, |derived| async move {
        match derived {
            Derived::Signals => calculation::technical_signal::calculate_technical_signals(date)
                .await
                .map(|_| ()),
            Derived::ForwardReturns => calculation::forward_return::calculate_forward_returns(date)
                .await
                .map(|_| ()),
            Derived::RelativeStrength => {
                calculation::relative_strength::calculate_relative_strength(date)
                    .await
                    .map(|_| ())
            }
        }
    })
    .await?;
    if let Some((title, body)) = report {
        alert::send_alert(title, &body).await;
    }
    Ok(())
}

/// [`ensure_derived`] 的本體：檢查、以 `recompute` 重算缺的項目，回傳要發的告警（標題、內容）；
/// 不需要告警時回 `None`。倉儲與重算可替換以便測試。
pub(crate) async fn check_derived<R, F, Fut>(
    repo: &R,
    date: NaiveDate,
    recompute: F,
) -> Result<Option<(&'static str, String)>>
where
    R: ForwardReturnRepository + DerivedDataQuery,
    F: Fn(Derived) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let dates = repo.fetch_trading_dates(date, 6).await?;
    let Some(base) = forward_return::base_date(date, 5, &dates) else {
        return Ok(None);
    };
    let before = missing_derived(repo.count_derived(date, base).await?);
    if before.is_empty() {
        tracing::info!("{date} 技術訊號與未來報酬完整");
        return Ok(None);
    }

    tracing::warn!("{date} 衍生資料缺漏，重算: {before:?}");
    let mut errors = Vec::new();
    for derived in &before {
        if let Err(why) = recompute(*derived).await {
            tracing::error!("{date} 重算{}失敗: {why:#}", derived.label());
            errors.push(format!("{}：{why:#}", derived.label()));
        }
    }
    let after = missing_derived(repo.count_derived(date, base).await?);
    let title = if after.is_empty() {
        "收盤衍生資料已補算"
    } else {
        "收盤衍生資料仍缺漏"
    };
    Ok(Some((
        title,
        derived_report(date, &before, &errors, &after),
    )))
}

/// 筆數為 0 的衍生資料。
pub(crate) fn missing_derived(counts: DerivedCounts) -> Vec<Derived> {
    let mut missing = Vec::new();
    if counts.signals == 0 {
        missing.push(Derived::Signals);
    }
    if counts.forward_returns == 0 {
        missing.push(Derived::ForwardReturns);
    }
    if counts.relative_strength == 0 {
        missing.push(Derived::RelativeStrength);
    }
    missing
}

/// 衍生資料告警內容：缺哪些、重算失敗的原因、重算後的狀況。
pub(super) fn derived_report(
    date: NaiveDate,
    before: &[Derived],
    errors: &[String],
    after: &[Derived],
) -> String {
    let names = |items: &[Derived]| {
        items
            .iter()
            .map(|item| item.label())
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = vec![format!("{date} 檢查時缺：{}", names(before))];
    if errors.is_empty() {
        lines.push("已重算".to_owned());
    } else {
        lines.push(format!("重算失敗：{}", errors.join("；")));
    }
    lines.push(if after.is_empty() {
        "重算後都已寫入".to_owned()
    } else {
        format!(
            "重算後仍缺：{}；請查看收盤匯總日誌，必要時手動回補",
            names(after)
        )
    });
    lines.join("\n")
}
