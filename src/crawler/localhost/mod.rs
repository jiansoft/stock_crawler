use std::sync::OnceLock;

use anyhow::Result;
use futures::future::join_all;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

use crate::util;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "localhost:44328";

use serde::{Deserialize, Serialize};
use tokio::task;

#[derive(Serialize, Deserialize, Debug)]
struct TransferRequest {
    #[serde(rename = "playerId")]
    pub player_id: String,
    #[serde(rename = "transferFrom")]
    pub transfer_from: String,
    #[serde(rename = "transferType")]
    pub transfer_type: String,
    #[serde(rename = "transferTo")]
    pub transfer_to: String,
    #[serde(rename = "amount")]
    pub amount: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct TransferResponse {
    #[serde(rename = "success")]
    pub success: bool,
    #[serde(rename = "status")]
    pub status: String,
    #[serde(rename = "message")]
    pub message: String,
}

const TOKEN:  &str = "Bearer eyJhbGciOiJSUzI1NiIsImtpZCI6IjY2RTJBODRFNzYwMDMyQUY2MjI4RUQ5MDVEODQ0RTM4MzhGMjlDMTUiLCJ4NXQiOiJadUtvVG5ZQU1xOWlLTzJRWFlST09EanluQlUiLCJ0eXAiOiJhdCtqd3QifQ.eyJzdWIiOiI3YzU3ZGRhNS03MDZlLWU5NjAtZjQ3MC0zYTEzMjI4ZDNhMDEiLCJwcmVmZXJyZWRfdXNlcm5hbWUiOiJzd2VldDAwMSIsImVtYWlsIjoiZWRkaWVAa29vY28uY29tLnR3Iiwicm9sZSI6IlBsYXllciIsInRlbmFudGlkIjoiNzJlZWE1MDYtMWU5ZS04Y2JiLWE2NjUtM2EwZmI2ZjM3MzVlIiwicGhvbmVfbnVtYmVyIjoiODg2OTE5MTE4NDU2IiwicGhvbmVfbnVtYmVyX3ZlcmlmaWVkIjoiRmFsc2UiLCJlbWFpbF92ZXJpZmllZCI6IkZhbHNlIiwidW5pcXVlX25hbWUiOiJzd2VldDAwMSIsIm9pX3Byc3QiOiJTcGFya19SZWFjdCIsImNsaWVudF9pZCI6IlNwYXJrX1JlYWN0Iiwib2lfdGtuX2lkIjoiODg1M2ViNDEtY2EzNi00MDAyLWFlMDEtM2ExNDE0NjJhOWFkIiwiYXVkIjoiU3BhcmsiLCJzY29wZSI6IlNwYXJrIiwianRpIjoiMjUzNWYzNDUtNTE5Mi00MmQ3LWIyOGEtNWYzMTA0NDM4YjFhIiwiaXNzIjoiaHR0cHM6Ly9sb2NhbGhvc3Q6NDQzMjgvIiwiZXhwIjoxNzIyMzYyMTY3LCJpYXQiOjE3MjIzMTg5Njd9.C2FZhvILmz40-TnxZR70VrXQQp619-p9JzJwp-GUtBXhKkc6ylFRyOnkl_NB0PP6wMalAbJRFhZqwsBgDGkfJPlOyA-d8AqtX5mF9zEVdsYJQqyad2ltAIdhy2gMhM7dB74GbIJdILU1P3J2zHPubZDuG7pNeKsUTxbCd-_Pa-0RbcMt0G1L7rnU93UksCo6xYypASlhD9OhSLgAJ_F-zLgnQT1gtRJyKYwC0gbSHEbrzVkjB7izbxHfP_8Tu19-fdvTt-zT-3GkzLWbKpys4oksu7QJ8NaIxsPxlTRX3UeCxz9k5tyQzVRYdXZvgKH_neSIHvSyDFK8qLrSs1RRMg";

async fn transfer_fund() -> Result<Vec<TransferResponse>> {
    let url = DDNS_URL.get_or_init(|| {
        format!(
            "https://{host}/api/app/transfer-balance/transfer-fund",
            host = HOST,
        )
    });

    //let TOKEN = "Bearer eyJhbGciOiJSUzI1NiIsImtpZCI6IjY2RTJBODRFNzYwMDMyQUY2MjI4RUQ5MDVEODQ0RTM4MzhGMjlDMTUiLCJ4NXQiOiJadUtvVG5ZQU1xOWlLTzJRWFlST09EanluQlUiLCJ0eXAiOiJhdCtqd3QifQ.eyJzdWIiOiIxNTU5NzA5OC04ZjI1LWRhODAtZTAwMS0zYTEzNzE2NTMxYzkiLCJwcmVmZXJyZWRfdXNlcm5hbWUiOiJvYWswMTIwMTIiLCJlbWFpbCI6Im9vZWlAZ21haWwuY29tIiwidGVuYW50aWQiOiJiN2M4NjVmYi1hNzQxLWQyMTgtNDY5Yi0zYTEyNzI2YWE1ZGQiLCJmYW1pbHlfbmFtZSI6IuatkOWNgeS6jCIsInBob25lX251bWJlciI6IjkzODQxIiwicGhvbmVfbnVtYmVyX3ZlcmlmaWVkIjoiRmFsc2UiLCJlbWFpbF92ZXJpZmllZCI6IkZhbHNlIiwidW5pcXVlX25hbWUiOiJvYWswMTIwMTIiLCJvaV9wcnN0IjoiU3BhcmtfUmVhY3QiLCJjbGllbnRfaWQiOiJTcGFya19SZWFjdCIsIm9pX3Rrbl9pZCI6IjNiMTA3N2M1LTBmMjMtM2M4My02NzJiLTNhMTNmYTdjMmNiZiIsImF1ZCI6IlNwYXJrIiwic2NvcGUiOiJTcGFyayIsImp0aSI6IjFkNzEwZmM4LTI4MTktNDVhZC1hYWY4LWE1MDM4M2JhYTdhOSIsImlzcyI6Imh0dHBzOi8vbG9jYWxob3N0OjQ0MzI4LyIsImV4cCI6MTcyMTkyNzYzMSwiaWF0IjoxNzIxODg0NDMxfQ.KVbov9oqh7X1zVv8HAJk6xJPFQCJuwU-SO_Z6DU0y-CqOvwxjnAIYqoove6WRA50ZVHL6lW2OvkYWsCJ68Bxbu9bg_-EPzLY3X1MwSlRWemf7Z8C3NtI8TyvCleYjD3KQM7aIaWwwQVP0pUIjHHdqBf0_EBZoowA1ds6sFsnHf5KI88L1gjm_F5UMBBZXd6hoi_bJTguZ-zdvu5TW6eeCDkRh-9lDiPMRZ95pHAOeNuJ2AGrGvzswq8AZjeMpyYtcaZ_bv4gHsZ3auOD01m4cO38wu2wtOw-bKFO2pIuHhnmu033pPm0G48dR8vi9dYROcYDBXLBF6jI-xpbOgvVWg";
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(TOKEN)?);
    headers.insert("content-type", "application/json".parse()?);

    let transfer: TransferRequest = TransferRequest {
        player_id: "15597098-8f25-da80-e001-3a13716531c9".to_string(),
        transfer_from: "86406c2a-3736-4fb6-9a81-b672255f32b1".to_string(),
        transfer_type: "MainToSupplier".to_string(),
        transfer_to: "ca8da9a2-777c-7cf1-c1cf-3a0fe02845f9".to_string(),
        amount: "1".to_string(),
    };

    // Create a vector to hold the tasks
    let mut tasks = Vec::new();

    for _ in 0..20 {
        let headers = {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, HeaderValue::from_str(TOKEN)?);
            headers.insert("content-type", "application/json".parse()?);
            headers
        };

        let transfer = TransferRequest {
            player_id: "15597098-8f25-da80-e001-3a13716531c9".to_string(),
            transfer_from: "86406c2a-3736-4fb6-9a81-b672255f32b1".to_string(),
            transfer_type: "MainToSupplier".to_string(),
            transfer_to: "ca8da9a2-777c-7cf1-c1cf-3a0fe02845f9".to_string(),
            amount: "8589".to_string(),
        };

        // Clone url to move into async block
        let url = url.clone();

        // Create an async task and push it into the tasks vector
        tasks.push(task::spawn(async move {
            util::http::post_use_json::<TransferRequest,TransferResponse>(&url, Some(headers), Some(&transfer))
                .await
                .unwrap()
        }));
    }

    // Use join_all to await all tasks concurrently
    let results = join_all(tasks).await;

    // Collect results into a vector
    let output: Vec<TransferResponse> = results.into_iter().map(|res| res.unwrap()).collect();

     Ok(output)
}

 async fn on_site_message() -> Result<Vec<String>> {
    // Create a vector to hold the tasks
    let mut tasks = Vec::new();

    for _ in 0..1 {
        let headers = {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, HeaderValue::from_str(TOKEN)?);
            headers.insert("content-type", "application/json".parse()?);
            headers.insert("__tenant", "spark99".parse()?);
            headers
        };

       /* let url = DDNS_URL.get_or_init(|| {
            format!(
                "https://{host}/api/app/personal-on-site-message/update-onsite-isclaim-by-Id?Id=799",
                host = HOST,
            )
        });*/
        let url = DDNS_URL.get_or_init(|| {
            "https://localhost:44328/api/WelfareCenter/Receive?Id=078a281b-80c5-4205-bf4b-589112f2b9ab".to_string()
        });

        // Create an async task and push it into the tasks vector
        tasks.push(task::spawn(async move {
            util::http::get(url, Some(headers)).await.unwrap()
        }));
    }

    // Use join_all to await all tasks concurrently
    let results = join_all(tasks).await;

    // Collect results into a vector
    let output: Vec<String> = results.into_iter().map(|res| res.unwrap()).collect();

    Ok(output)
}


#[cfg(test)]
mod tests {
    use crate::crawler::localhost::{on_site_message, transfer_fund};
    use crate::logging;

    #[tokio::test]
    async fn test_transfer_fund() {
        match transfer_fund().await {
            Ok(ip) => {
                print!("{:?}", ip)
            }
            Err(why) => {
                print!("{}", format!("Failed to get because {:?}", why));
                logging::error_file_async(format!("Failed to get because {:?}", why));
            }
        }
    }

    #[tokio::test]
    async fn test_on_site_message() {
        match on_site_message().await {
            Ok(ip) => {
                print!("{:?}", ip)
            }
            Err(why) => {
                print!("{}", format!("Failed to get because {:?}", why));
                logging::error_file_async(format!("Failed to get because {:?}", why));
            }
        }
    }
}
