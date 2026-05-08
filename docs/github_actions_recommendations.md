# GitHub Actions 與外掛建議清單

針對 `stock_crawler` 專案目前的架構（Rust、Protobuf、SQL、Docker），以下是建議加入的 GitHub Actions 與外掛，旨在強化開發流程、提升效能並確保安全性。

## 1. 效能優化 (Performance Optimization)
*   **[Swatinem/rust-cache](https://github.com/Swatinem/rust-cache)**
    *   **用途**：自動緩存 Rust 的 `target` 目錄與 Cargo 目錄。
    *   **效益**：避免每次 CI 都重新編譯未變更的依賴套件，能顯著縮短編譯時間。強烈建議加入現有的 `rust.yml`。

## 2. 依賴與安全性管理 (Dependency & Security)
*   **[Dependabot](https://docs.github.com/en/code-security/dependabot/dependabot-version-updates/configuring-dependabot-version-updates)**
    *   **用途**：自動檢查 `Cargo.toml` 與 GitHub Actions 的版本更新並發送 PR。
    *   **效益**：確保專案使用的是最新且安全的套件版本，無需手動監控。
*   **[EmbarkStudios/cargo-deny-action](https://github.com/EmbarkStudios/cargo-deny-action)**
    *   **用途**：檢查依賴套件是否存在安全性漏洞 (Advisories)、授權合規性 (Licenses) 以及重複的套件版本。
    *   **效益**：對於金融爬蟲類專案，確保供應鏈安全至關重要。

## 3. Docker 自動化 (Docker Automation)
*   **[docker/build-push-action](https://github.com/docker/build-push-action)**
    *   **用途**：當推送到 `main` 分支時，自動建置 Docker Image 並推送到 GHCR (GitHub Container Registry) 或 Docker Hub。
    *   **效益**：簡化部署流程，可搭配現有的 `Dockerfile_live` 進行多平台編譯 (amd64/arm64)。

## 4. 程式碼品質與介面一致性 (Code Quality & Interface)
*   **[taiki-e/install-action](https://github.com/taiki-e/install-action) 搭配 `cargo-llvm-cov`**
    *   **用途**：產生測試覆蓋率報告。
    *   **效益**：視覺化呈現 `src/app/calculation` 等核心邏輯的測試涵蓋狀況，確保計算正確性。
*   **[bufbuild/buf-action](https://github.com/bufbuild/buf-action)**
    *   **用途**：針對 `etc/proto` 下的 Protobuf 檔案進行 Lint 檢查與破壞性變更 (Breaking Change) 檢測。
    *   **效益**：確保 gRPC/Protobuf 介面的穩定性與規範。

## 5. 自動化發佈 (Automated Release)
*   **[release-plz-action](https://github.com/MarcoIeni/release-plz)**
    *   **用途**：根據 Conventional Commits 自動更新版本號、產生 Changelog 並發佈 Release。
    *   **效益**：讓發佈流程專業化且自動化。

---

## 實作計劃 (建議步驟)
1.  **第一階段**：將 `rust-cache` 整合至 `rust.yml`，加速現有 CI。
2.  **第二階段**：建立 `dependabot.yml` 進行自動化依賴管理。
3.  **第三階段**：設定 `cargo-deny` 進行安全性審核。
4.  **第四階段**：針對 `Dockerfile_live` 設定 Docker 自動建置流程。
