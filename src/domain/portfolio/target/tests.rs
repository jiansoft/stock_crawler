//! 配置目標 domain 的單元測試：寫入來源的解析與驗證。

use super::*;

/// 未帶來源為未標示（舊版呼叫端）；web／mcp 照實解析；不認得的來源拒絕。
#[test]
fn actor_parse_accepts_known_sources_only() {
    assert_eq!(TargetActor::parse(None, None), Ok(TargetActor::default()));
    assert_eq!(
        TargetActor::parse(Some("web"), Some("1")),
        Ok(TargetActor {
            source: ActorSource::Web,
            actor_ref: "1".to_owned(),
        })
    );
    assert_eq!(
        TargetActor::parse(Some("mcp"), None).map(|actor| actor.source),
        Ok(ActorSource::Mcp)
    );
    assert!(TargetActor::parse(Some("cli"), Some("1")).is_err());
    assert!(TargetActor::parse(Some("WEB"), None).is_err());
}

/// `actor_ref` 最多 64 個字元（以字元計，不是位元組），不可含控制字元。
#[test]
fn actor_parse_limits_reference() {
    let max = "測".repeat(MAX_ACTOR_REF_LEN);
    assert!(TargetActor::parse(Some("mcp"), Some(&max)).is_ok());
    assert!(TargetActor::parse(Some("mcp"), Some(&format!("{max}x"))).is_err());
    assert!(TargetActor::parse(Some("web"), Some("1\n2")).is_err());
}

/// 存進資料庫的字串與解析互為反函數；空字串是未標示。
#[test]
fn actor_source_round_trips_through_database_string() {
    for source in [ActorSource::Unspecified, ActorSource::Web, ActorSource::Mcp] {
        assert_eq!(ActorSource::parse(source.as_str()), Some(source));
    }
    assert_eq!(ActorSource::Unspecified.as_str(), "");
}
