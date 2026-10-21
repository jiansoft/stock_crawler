use chrono::Local;
use concat_string::concat_string;

use super::*;

/// 啟動只會回固定狀態碼與 body 的本機 HTTP 伺服器，回傳網址。
async fn spawn_fixed_server(status_line: &'static str, body: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 {status_line}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    format!("http://{addr}/api")
}

/// 非 2xx 與 JSON 解析失敗時，錯誤訊息裡的 body 是單行片段（多行 HTML 不會把一筆日誌拆成多行）。
#[tokio::test]
async fn json_errors_flatten_body_to_single_line() {
    let html = "<!DOCTYPE html>\n<html lang=\"en-us\">\n  <head>";

    let url = spawn_fixed_server("502 Bad Gateway", html).await;
    let err = get_json::<serde_json::Value>(&url)
        .await
        .expect_err("502 應回錯誤");
    let message = err.to_string();
    assert!(message.contains("status 502"), "{message}");
    assert!(
        message.ends_with("Body: <!DOCTYPE html> <html lang=\"en-us\"> <head>"),
        "{message}"
    );

    let url = spawn_fixed_server("200 OK", html).await;
    for err in [
        get_json::<serde_json::Value>(&url)
            .await
            .expect_err("不是 JSON"),
        post_use_json::<serde_json::Value, serde_json::Value>(&url, None, None)
            .await
            .expect_err("不是 JSON"),
    ] {
        let message = err.to_string();
        assert!(
            message.starts_with("Error parsing response JSON"),
            "{message}"
        );
        assert!(!message.contains('\n'), "{message}");
    }
}

/// 手動驗證外部 HTTP 來源是否可正常請求。
///
/// 這個測試需要實際連線 TWSE 與 httpbin，預設測試集不應依賴外部網路，
/// 因此標記為 ignored，避免 CI 或離線環境因網路限制而失敗。
#[tokio::test]
#[ignore]
async fn test_request() {
    let url = concat_string!(
        "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json&date=",
        Local::now().format("%Y%m%d").to_string(),
        "&_=",
        Local::now().timestamp_millis().to_string()
    );

    tracing::debug!("request_get:{:?}", get(&url, None).await);

    let bytes = reqwest::get("https://httpbin.org/ip")
        .await
        .unwrap()
        .bytes()
        .await;

    println!("bytes: {:#?}", bytes);
}

#[tokio::test]
async fn test_get() {
    match get("https://jiansoft.mooo.com/stock/revenues", None).await {
        Ok(_) => {}
        Err(why) => {
            tracing::error!("Failed to get because {:?}", why);
        }
    }
}
