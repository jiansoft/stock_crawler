use serde::{Deserialize, Serialize};

use super::state::BackfillJob;

/// 收盤彙總手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct ClosingAggregateRequest {
    /// 交易日期，格式必須為 `YYYY-MM-DD`。
    pub(super) date: String,
}

/// 台股加權指數手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct TaiwanStockIndexRequest {
    /// 回補目標日期，格式必須為 `YYYY-MM-DD`。
    ///
    /// TWSE API 會依此日期回傳該月份所有交易日的指數資料。
    pub(super) date: String,
}

/// 各股每日收盤報價手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct DailyQuotesRequest {
    /// 交易日期，格式必須為 `YYYY-MM-DD`。
    pub(super) date: String,
}

/// 年度類手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct YearRequest {
    /// 回補目標年度，例如 `2026`。
    pub(super) year: i32,
}

/// 證券代號類手動回補的 HTTP request body。
#[derive(Debug, Deserialize)]
pub(super) struct SecurityCodeRequest {
    /// 股票、ETF 或其他證券代號。
    pub(super) security_code: String,
}

/// 建立 job 成功時的 HTTP response body。
#[derive(Debug, Serialize)]
pub(super) struct StartJobResponse {
    /// 已建立的 job。
    pub(super) job: BackfillJob,
}

/// API 錯誤回應。
#[derive(Debug, Serialize)]
pub(super) struct ErrorResponse {
    /// 可讀的錯誤原因。
    pub(super) error: String,
}

/// Manual backfill 單頁操作介面。
pub(super) const INDEX_HTML: &str = r##"<!doctype html>
<html lang="zh-Hant">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Manual Backfill</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f6f7f9;
      --panel: #ffffff;
      --text: #1d252d;
      --muted: #65717d;
      --line: #dce2e8;
      --accent: #146c63;
      --accent-dark: #0f504a;
      --danger: #b42318;
      --ok: #157347;
      --running: #8a5a00;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      font-size: 15px;
      line-height: 1.5;
    }
    header {
      border-bottom: 1px solid var(--line);
      background: var(--panel);
    }
    .wrap {
      width: min(1120px, calc(100% - 32px));
      margin: 0 auto;
    }
    header .wrap {
      min-height: 72px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 16px;
    }
    h1 {
      margin: 0;
      font-size: 24px;
      font-weight: 700;
    }
    main {
      padding: 24px 0 40px;
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 16px;
      align-items: start;
    }
    .panel {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      padding: 16px;
    }
    h2 {
      margin: 0 0 12px;
      font-size: 18px;
    }
    label {
      display: block;
      color: var(--muted);
      font-size: 13px;
      margin-bottom: 6px;
    }
    input {
      width: 100%;
      height: 40px;
      border: 1px solid var(--line);
      border-radius: 6px;
      padding: 0 10px;
      font: inherit;
      background: #fff;
      color: var(--text);
    }
    button {
      width: 100%;
      height: 40px;
      border: 0;
      border-radius: 6px;
      margin-top: 12px;
      background: var(--accent);
      color: #fff;
      font: inherit;
      font-weight: 650;
      cursor: pointer;
    }
    button:hover { background: var(--accent-dark); }
    button:disabled { opacity: .65; cursor: progress; }
    .jobs {
      margin-top: 18px;
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      overflow: hidden;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      table-layout: fixed;
    }
    th, td {
      padding: 10px 12px;
      border-bottom: 1px solid var(--line);
      text-align: left;
      vertical-align: top;
      overflow-wrap: anywhere;
    }
    th {
      color: var(--muted);
      font-size: 13px;
      font-weight: 650;
      background: #fbfcfd;
    }
    tr:last-child td { border-bottom: 0; }
    .status {
      display: inline-flex;
      align-items: center;
      min-height: 24px;
      padding: 2px 8px;
      border-radius: 999px;
      font-size: 13px;
      font-weight: 650;
      background: #eef1f4;
    }
    .running { color: var(--running); }
    .succeeded { color: var(--ok); }
    .failed { color: var(--danger); }
    .toast {
      color: var(--muted);
      min-height: 22px;
      margin-top: 10px;
      overflow-wrap: anywhere;
    }
    @media (max-width: 840px) {
      .grid { grid-template-columns: 1fr; }
      header .wrap { align-items: flex-start; flex-direction: column; padding: 16px 0; }
      th:nth-child(1), td:nth-child(1) { display: none; }
    }
  </style>
</head>
<body>
  <header>
    <div class="wrap">
      <h1>Manual Backfill</h1>
      <div id="summary" class="toast">Loading jobs...</div>
    </div>
  </header>
  <main class="wrap">
    <section class="grid" aria-label="Backfill forms">
      <form class="panel" data-endpoint="/api/manual-backfill/daily-quotes">
        <h2>Daily Quotes</h2>
        <label for="daily-quotes-date">Trading date</label>
        <input id="daily-quotes-date" name="date" type="date" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
      <form class="panel" data-endpoint="/api/manual-backfill/closing-aggregate">
        <h2>Closing Aggregate</h2>
        <label for="closing-date">Trading date</label>
        <input id="closing-date" name="date" type="date" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
      <form class="panel" data-endpoint="/api/manual-backfill/taiwan-stock-index">
        <h2>Taiwan Stock Index</h2>
        <label for="index-date">Trading date</label>
        <input id="index-date" name="date" type="date" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
      <form class="panel" data-endpoint="/api/manual-backfill/received-dividend-records">
        <h2>Received Dividends</h2>
        <label for="received-code">Security code</label>
        <input id="received-code" name="security_code" inputmode="latin" placeholder="0056" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
      <form class="panel" data-endpoint="/api/manual-backfill/historical-dividends">
        <h2>Historical Dividends</h2>
        <label for="historical-code">Security code</label>
        <input id="historical-code" name="security_code" inputmode="latin" placeholder="2845" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
      <form class="panel" data-endpoint="/api/manual-backfill/multiple-dividend-historical-dividends">
        <h2>Multi Dividend History</h2>
        <label for="multiple-dividend-year">Dividend year</label>
        <input id="multiple-dividend-year" name="year" type="number" min="1900" max="3000" step="1" placeholder="2026" required>
        <button type="submit">Start</button>
        <div class="toast"></div>
      </form>
    </section>
    <section class="jobs" aria-label="Backfill jobs">
      <table>
        <thead>
          <tr>
            <th style="width: 18%">Job</th>
            <th style="width: 18%">Type</th>
            <th style="width: 14%">Input</th>
            <th style="width: 14%">Status</th>
            <th>Message</th>
          </tr>
        </thead>
        <tbody id="jobs-body">
          <tr><td colspan="5">No jobs yet.</td></tr>
        </tbody>
      </table>
    </section>
  </main>
  <script>
    const jobsBody = document.querySelector("#jobs-body");
    const summary = document.querySelector("#summary");

    document.querySelectorAll("form[data-endpoint]").forEach((form) => {
      form.addEventListener("submit", async (event) => {
        event.preventDefault();
        const button = form.querySelector("button");
        const toast = form.querySelector(".toast");
        const data = Object.fromEntries(new FormData(form).entries());
        button.disabled = true;
        toast.textContent = "Starting...";

        try {
          const response = await fetch(form.dataset.endpoint, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify(data)
          });
          const body = await response.json();
          if (!response.ok) throw new Error(body.error || "request failed");
          toast.textContent = `Started job ${body.job.id}`;
          await refreshJobs();
        } catch (error) {
          toast.textContent = error.message;
        } finally {
          button.disabled = false;
        }
      });
    });

    async function refreshJobs() {
      try {
        const response = await fetch("/api/manual-backfill/jobs");
        const jobs = await response.json();
        renderJobs(jobs);
        const running = jobs.filter((job) => job.status === "running").length;
        summary.textContent = `${jobs.length} jobs, ${running} running`;
      } catch (error) {
        summary.textContent = error.message;
      }
    }

    function renderJobs(jobs) {
      if (!jobs.length) {
        jobsBody.innerHTML = '<tr><td colspan="5">No jobs yet.</td></tr>';
        return;
      }
      jobsBody.replaceChildren(...jobs.map((job) => {
        const row = document.createElement("tr");
        row.innerHTML = `
          <td>${escapeHtml(job.id)}</td>
          <td>${escapeHtml(job.kind)}</td>
          <td>${escapeHtml(job.input)}</td>
          <td><span class="status ${job.status}">${escapeHtml(job.status)}</span></td>
          <td>${escapeHtml(job.message)}</td>
        `;
        return row;
      }));
    }

    function escapeHtml(value) {
      return String(value).replace(/[&<>"']/g, (char) => ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#039;"
      }[char]));
    }

    refreshJobs();
    setInterval(refreshJobs, 3000);
  </script>
</body>
</html>"##;
