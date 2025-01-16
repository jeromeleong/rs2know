# pj

## 簡介

**pj** 是一款用於分析 Rust 程式碼並進行 AI 分析的命令列工具。它能夠掃描指定的 Rust 專案，統計程式碼行數、註解行數、空白行數等，並利用 OpenAI 或其他 GPT 服務對程式碼進行深入分析，生成詳細的分析報告。

## 特色功能

- **程式碼分析**：遞迴掃描專案目錄，統計程式碼行數、註解行數及空白行數。
- **AI 分析**：整合 OpenAI GPT 模型，提供函數、結構體、錯誤類型等詳細分析。
- **報告生成**：支持生成 JSON 或 Markdown 格式的分析報告。
- **增量更新**：支援只分析修改過的檔案，提高效率。
- **自訂配置**：靈活的命令列選項，滿足不同需求。
- **日誌管理**：多級別日誌輸出，方便調試與追蹤。

## 安裝指南

### 前置需求

- **Rust**：請確保已安裝 Rust 工具鏈。可透過 [Rust 官方網站](https://www.rust-lang.org/) 安裝。

### 使用 Cargo 安裝

```bash
cargo install --git https://github.com/jeromeleong/pj
```

### 從源碼編譯

1. 克隆倉庫：

    ```bash
    git clone https://github.com/yourusername/pj.git
    cd pj
    ```

2. 編譯專案：

    ```bash
    cargo build --release
    ```

3. 可執行檔位於 `target/release/pj`。

## 快速開始

安裝完成後，即可使用 `pj` 來分析您的 Rust 專案。

### 基本用法

分析一個 Rust 專案：
```bash
pj --path /path/to/your/rust/project
```

更新現有的 JSON 報告（只分析修改過的檔案）：
```bash
pj update --report report.json --path /path/to/project
```

從 JSON 生成 Markdown 報告：
```bash
pj generate-md --report report.json --output report.md
```

### 常用選項

- `-p, --path`：指定 Rust 專案的路徑（預設為當前目錄）。
- `--api-url`：設定 OpenAI 或其他 GPT 服務的 API 端點。
- `--api-key`：提供 OpenAI API 金鑰或 GPT 令牌。
- `--model`：選擇 GPT 模型名稱（預設為 `gpt-4o-mini`）。
- `--skip-ai`：跳過 AI 分析。
- `--json`：僅輸出 JSON 格式報告。
- `-o, --output`：指定輸出檔案路徑。
- `--log-level`：設定日誌級別（`trace`, `debug`, `info`, `warn`, `error`，預設為 `info`）。

### 子命令

- **update**：更新現有的 JSON 報告
  ```bash
  pj update --report report.json --path /path/to/project
  ```

- **generate-md**：從 JSON 生成 Markdown 報告
  ```bash
  pj generate-md --report report.json --output report.md
  ```

## 配置詳情

`pj` 支援多種配置選項，以滿足不同的使用需求。以下是主要配置選項的說明：

| 選項         | 簡介                                               | 預設值                          |
| ------------ | -------------------------------------------------- | ------------------------------- |
| `-p, --path`  | 指定要分析的 Rust 專案路徑                       | `.`（當前目錄）                  |
| `--api-url`   | 設定 GPT 服務的 API 端點                         | `https://api.openai.com/v1/chat/completions` |
| `--api-key`   | 提供 OpenAI API 金鑰或其他 GPT 服務的令牌           | 空字串                           |
| `--model`     | 選擇 GPT 模型名稱                                   | `gpt-4o-mini`                   |
| `--skip-ai`   | 是否跳過 AI 分析                                   | `false`                         |
| `--json`      | 僅輸出 JSON 格式報告，不包含 Markdown             | `false`                         |
| `-o, --output`| 指定報告的輸出檔案路徑（如未指定，依格式自動命名） | `rust_analysis_report.{json|md}` |
| `--log-level` | 設定日誌輸出級別（`trace`, `debug`, `info`, `warn`, `error`） | `info`                           |

## API 文件

**pj** 主要通過 OpenAI 的 GPT API 進行程式碼分析。以下是相關的 API 配置說明：

### 配置 API

- **API URL**：使用 `--api-url` 選項指定 GPT 服務的端點。預設為 OpenAI 的端點。
- **API 金鑰**：使用 `--api-key` 選項提供有效的 API 金鑰或令牌。

### 請求格式

`pj` 會構建一個 JSON 請求，包含要分析的程式碼內容及指令。以下是請求的基本結構：

```json
{
  "model": "gpt-4o-mini",
  "messages": [
    {
      "role": "user",
      "content": "分析這個 Rust 文件並直接返回 JSON 格式的結構化信息..."
    }
  ]
}
```

### 回應處理

AI 分析的回應將被解析為結構化的 `AIAnalysis` 資料結構，包含主要函數、核心結構體、錯誤類型、函數詳情及程式碼複雜度等資訊。

## 報告格式

### JSON 報告

JSON 報告包含以下主要部分：

- 專案總結
  - 總檔案數和程式碼行數
  - 主要功能列表
  - 程式架構描述
  - 關鍵元件
  - 技術堆疊
  - 改進建議

- 檔案分析
  - 每個檔案的 AI 分析結果
    - 主要函數
    - 核心結構體
    - 錯誤類型
    - 函數詳情
    - 程式碼複雜度評估

### Markdown 報告

Markdown 報告以易讀的格式呈現 JSON 報告的內容，包含：

- 專案總結
- 檔案目錄
- 詳細的檔案分析
  - 主要函數說明
  - 核心結構體描述
  - 錯誤類型分類
  - 函數詳情
  - 程式碼複雜度評估

## 常見問題

### 如何跳過 AI 分析？

使用 `--skip-ai` 選項即可跳過 AI 分析：

```bash
pj --path ./my_rust_project --skip-ai
```

### 報告生成失敗，如何處理？

請檢查以下幾點：

1. 確認指定的專案路徑正確且存在。
2. 檢查 API 金鑰是否有效。
3. 確認網路連接正常。
4. 查看日誌輸出，尋找具體錯誤資訊。

### 如何更新現有的報告？

使用 `update` 子命令，只分析修改過的檔案：

```bash
pj update --report report.json --path ./my_rust_project
```

## 環境變數

- `RUST_LOG`：控制日誌輸出級別（可選）

## 注意事項

- 需要有效的 OpenAI API 金鑰才能使用 AI 分析功能
- 分析大型專案時，建議使用 `update` 命令進行增量更新
- 生成的報告預設會保存在當前目錄下

## 授權條款

此專案採用 [MIT 授權](LICENSE)。
