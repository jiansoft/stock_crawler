//! Seq 伺服器網址的安全檢查（HTTPS、本機明文、拒絕遠端明文與仿冒主機），與據此決定的 API key 去留。

use super::*;

/// HTTPS 端點可安全攜帶 API key。
#[test]
fn seq_endpoint_accepts_https() {
    assert!(seq_endpoint_protects_api_key("https://seq.example.com"));
    assert!(seq_endpoint_protects_api_key(
        "https://192.168.111.224:5341"
    ));
}

/// loopback 的明文端點可接受：封包不會離開本機。
#[test]
fn seq_endpoint_accepts_plaintext_loopback() {
    for endpoint in [
        "http://localhost:5341",
        "http://LOCALHOST",
        "http://127.0.0.1:5341",
        "http://127.1.2.3",
        "http://[::1]:5341",
    ] {
        assert!(
            seq_endpoint_protects_api_key(endpoint),
            "{endpoint} 應被視為安全"
        );
    }
}

/// 明文的非 loopback 端點必須被判定為不安全。
///
/// `.env` 範例中的 `http://192.168.111.224:5341` 正是這一類 ——
/// 區域網路同樣可能被側錄。
#[test]
fn seq_endpoint_rejects_plaintext_remote() {
    for endpoint in [
        "http://192.168.111.224:5341",
        "http://seq.example.com",
        "http://user:pass@seq.example.com:5341",
        "http://[2001:db8::1]:5341",
        "ftp://seq.example.com",
        "seq.example.com",
        "",
    ] {
        assert!(
            !seq_endpoint_protects_api_key(endpoint),
            "{endpoint} 不應被視為安全"
        );
    }
}

/// 主機名稱以 localhost 開頭但實為他站者，不得誤判為 loopback。
#[test]
fn seq_endpoint_rejects_lookalike_hosts() {
    assert!(!seq_endpoint_protects_api_key("http://localhost.evil.com"));
    assert!(!seq_endpoint_protects_api_key("http://notlocalhost"));
}

/// 金鑰只送往 HTTPS 或本機端點；遠端明文捨棄；沒設定金鑰時不論端點都不送。
#[test]
fn seq_api_key_is_dropped_for_plaintext_remote() {
    assert_eq!(
        seq_api_key("https://seq.example.com", "k1").as_deref(),
        Some("k1")
    );
    assert_eq!(
        seq_api_key("http://127.0.0.1:5341", "k2").as_deref(),
        Some("k2")
    );
    assert_eq!(
        seq_api_key("http://192.168.111.224:5341", "k3"),
        None,
        "區網明文端點會把金鑰攤在網路上"
    );
    assert_eq!(seq_api_key("https://seq.example.com", ""), None);
}
