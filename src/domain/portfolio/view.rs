//! 持股的可見範圍（會員系統計畫 7.6 N2）。
//!
//! 所有持股查詢都改收 [`HoldingView`]，取代「`member_id = 0` 代表全部會員」的寫法：
//! 「全部」只到某一個家庭為止，這同時就是家庭邊界（2.7）。

/// 會員系統上線前既有的預設家庭（遷移時四位會員都屬於它）。
pub const DEFAULT_HOUSEHOLD_ID: i64 = 1;

/// 一個持股查詢的可見範圍：某個家庭的全體成員，或其中一位成員。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HoldingView {
    household_id: i64,
    /// `None` 為該家庭全體成員（含已停用、不含已刪除）。
    member_id: Option<i64>,
}

impl HoldingView {
    /// 某個家庭的全體成員。
    pub const fn household(household_id: i64) -> Self {
        Self {
            household_id,
            member_id: None,
        }
    }

    /// 某個家庭裡的一位成員；`member_id` 不大於 0 時視為全體成員。
    pub const fn member(household_id: i64, member_id: i64) -> Self {
        Self {
            household_id,
            member_id: if member_id > 0 { Some(member_id) } else { None },
        }
    }

    /// 預設家庭的全體成員：搬遷期間（N2～N3）通知與舊呼叫端用它表達原本的 `member_id = 0`。
    pub const fn default_household() -> Self {
        Self::household(DEFAULT_HOUSEHOLD_ID)
    }

    /// 家庭編號。
    pub const fn household_id(&self) -> i64 {
        self.household_id
    }

    /// 指定的成員；全體成員時為 `None`。
    pub const fn member_id(&self) -> Option<i64> {
        self.member_id
    }

    /// 回應裡的 `member_id`：全體成員為 0（家庭合計），與 Data API 原本的語意相同。
    pub const fn member_id_or_total(&self) -> i64 {
        match self.member_id {
            Some(member_id) => member_id,
            None => 0,
        }
    }
}

#[cfg(test)]
mod tests;
