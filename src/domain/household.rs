//! # 家庭（Household）
//!
//! 會員系統中資料可見範圍的邊界：合計與跨成員檢視只涵蓋同一個家庭，每位會員恰屬一個家庭
//! （`member_household` 表）。會員系統 Phase 7 之前只有家庭 #1，每日結算、配置目標與價格追蹤
//! 都歸這個家庭；Phase 7 才依家庭分組。

/// 預設家庭編號：Phase 7 依家庭分組之前，所有資料都屬於這個家庭。
pub const DEFAULT_HOUSEHOLD_ID: i64 = 1;
