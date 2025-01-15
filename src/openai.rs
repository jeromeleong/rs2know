use anyhow::{anyhow, Result};

use reqwest::Client;

use serde::Deserialize;

use std::time::Duration;

use tracing::{debug, error, info, warn};

use crate::models::{AIAnalysis, FileAnalysis, ProjectSummary};

const MAX_RETRIES: u32 = 5;

const RETRY_DELAY_MS: u64 = 1000;

const DEFAULT_MODELS: &[&str] = &[
    "gpt-4o",
    "gpt-4o-mini",
    "claude-3-5-sonnet",
    "claude-3-5-haiku",
    "gemini-2.0-flash-exp",
];

#[derive(Deserialize)]

struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]

struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]

struct ChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize)]

struct ModelResponse {
    data: Vec<Model>,
}

#[derive(Debug, Deserialize)]

struct Model {
    id: String,
}

/// 從 API 獲取可用的模型列表

pub async fn get_available_models(api_url: &str, api_key: &str) -> Result<Vec<String>> {
    let endpoint = format!("{}/models", api_url.trim_end_matches('/'));

    debug!("獲取可用模型列表：{}", endpoint);

    let client = Client::new();

    let resp = match client
        .get(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => resp,

        Err(e) => {
            warn!("無法獲取模型列表：{}", e);

            return Ok(DEFAULT_MODELS.iter().map(|&s| s.to_string()).collect());
        }
    };

    if !resp.status().is_success() {
        warn!("獲取模型列表失敗：{}", resp.status());

        return Ok(DEFAULT_MODELS.iter().map(|&s| s.to_string()).collect());
    }

    match resp.json::<ModelResponse>().await {
        Ok(model_resp) => {
            let models: Vec<String> = model_resp.data.into_iter().map(|m| m.id).collect();

            if models.is_empty() {
                warn!("未找到可用的 GPT 模型，使用預設列表");

                Ok(DEFAULT_MODELS.iter().map(|&s| s.to_string()).collect())
            } else {
                Ok(models)
            }
        }

        Err(e) => {
            warn!("解析模型列表失敗：{}", e);

            Ok(DEFAULT_MODELS.iter().map(|&s| s.to_string()).collect())
        }
    }
}

pub async fn do_ai_analysis_with_retry(
    api_url: &str,

    api_key: &str,

    model: &str,

    code: &str,

    file_path: &str,
) -> Result<Option<AIAnalysis>> {
    let mut retries = 0;

    while retries < MAX_RETRIES {
        match do_ai_analysis(api_url, api_key, model, code).await {
            Ok(analysis) => return Ok(Some(analysis)),

            Err(e) => {
                if retries < MAX_RETRIES - 1 {
                    let delay = RETRY_DELAY_MS * (retries as u64 + 1);

                    warn!(
                        "AI 分析失敗 (重試 {}/{}): {} - {}",
                        retries + 1,
                        MAX_RETRIES,
                        file_path,
                        e
                    );

                    tokio::time::sleep(Duration::from_millis(delay)).await;

                    retries += 1;
                } else {
                    error!(
                        "AI 分析在重試{}次後仍然失敗：{} - {}",
                        MAX_RETRIES, file_path, e
                    );

                    return Ok(None);
                }
            }
        }
    }

    Ok(None)
}

async fn do_ai_analysis(
    api_url: &str,

    api_key: &str,

    model: &str,

    code: &str,
) -> Result<AIAnalysis> {
    let endpoint = format!("{}/chat/completions", api_url.trim_end_matches('/'));

    info!("發送 API 請求至：{}", endpoint);

    let prompt = format!(

"分析這個 Rust 文件並直接返回 JSON 格式的結構化信息，不要加入任何 markdown 標記。JSON 格式如下：

{{

\"main_functions\": [\"主要函數清單\"],

\"core_structs\": [

{{

\"name\": \"結構體名稱\",

\"description\": \"結構體描述\"

}}

],

\"error_types\": [\"錯誤類型清單\"],

\"functions_details\": [

{{

\"name\": \"函數名稱\",

\"description\": \"函數描述\",

\"parameters\": [\"參數清單\"],

\"return_type\": \"返回類型\"

}}

],

\"code_complexity\": \"程式碼複雜度評估\"

}}

  

以下是需要分析的程式碼：

  

{}",

code

);

    let body = serde_json::json!({

    "model": model,

    "messages": [

    {

    "role": "system",

    "content": "你是一個 Rust 程式碼分析專家。"

    },

    {

    "role": "user",

    "content": prompt

    }

    ],

    "temperature": 0.2

    });

    let client = Client::new();

    let resp = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();

        let text = resp.text().await.unwrap_or_default();

        error!("API 錯誤回應：{}", text);

        return Err(anyhow!("AI 回應錯誤：{} - {}", status, text));
    }

    let response_text = resp.text().await?;

    debug!("API 回應：{}", response_text);

    let chat_resp: ChatResponse = serde_json::from_str(&response_text)
        .map_err(|e| anyhow!("無法解析 AI 回應的 JSON：{} - 回應：{}", e, response_text))?;

    let content = chat_resp
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("AI 未返回任何選項"))?
        .message
        .content;

    let clean_content = content
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let last_brace = clean_content
        .rfind('}')
        .ok_or_else(|| anyhow!("在 AI 回應中找不到結束大括號"))?;

    let clean_json = &clean_content[..=last_brace];

    let ai_analysis: AIAnalysis =
        serde_json::from_str(clean_json).map_err(|e| anyhow!("無法反序列化 AI 分析：{}", e))?;

    Ok(ai_analysis)
}

pub async fn generate_project_summary_with_retry(
    analyses: &[FileAnalysis],

    api_url: &str,

    api_key: &str,

    model: &str,
) -> Result<Option<ProjectSummary>> {
    let mut retries = 0;

    while retries < MAX_RETRIES {
        match generate_project_summary(analyses, api_url, api_key, model).await {
            Ok(summary) => return Ok(Some(summary)),

            Err(e) => {
                if retries < MAX_RETRIES - 1 {
                    let delay = RETRY_DELAY_MS * (retries as u64 + 1);

                    warn!(
                        "專案總結生成失敗 (重試 {}/{}): {}",
                        retries + 1,
                        MAX_RETRIES,
                        e
                    );

                    tokio::time::sleep(Duration::from_millis(delay)).await;

                    retries += 1;
                } else {
                    error!("專案總結在重試{}次後仍然失敗：{}", MAX_RETRIES, e);

                    return Ok(None);
                }
            }
        }
    }

    Ok(None)
}

async fn generate_project_summary(
    analyses: &[FileAnalysis],

    api_url: &str,

    api_key: &str,

    model: &str,
) -> Result<ProjectSummary> {
    info!("開始生成專案總結");

    let endpoint = format!("{}/chat/completions", api_url.trim_end_matches('/'));

    let analyses_json = serde_json::to_string_pretty(analyses)?;

    let prompt = format!(

"分析這個 Rust 專案的所有檔案分析結果，並生成一個總結。請直接返回 JSON 格式，不要加入任何程式碼區塊標記或其他文字。JSON 格式如下：

{{

\"total_files\": 檔案總數,

\"total_loc\": 總程式碼行數,

\"main_features\": [

\"主要功能1\",

\"主要功能2\"

],

\"code_architecture\": \"專案架構的描述\",

\"key_components\": [

\"關鍵元件1\",

\"關鍵元件2\"

],

\"tech_stack\": [

\"使用的技術1\",

\"使用的技術2\"

],

\"recommendations\": [

\"改進建議1\",

\"改進建議2\"

]

}}

  

以下是專案的檔案分析結果：

{}",

analyses_json

);

    let body = serde_json::json!({

    "model": model,

    "messages": [{

    "role": "system",

    "content": "你是一個專業的 Rust 程式碼分析助手。請分析提供的程式碼並生成結構化的專案總結。請直接返回純 JSON 格式，不要包含任何 markdown 程式碼區塊標記。"

    }, {

    "role": "user",

    "content": prompt

    }],

    });

    debug!("發送專案總結 API 請求");

    let client = Client::new();

    let resp = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();

        let text = resp.text().await.unwrap_or_default();

        error!("生成專案總結時發生錯誤：{}", text);

        return Err(anyhow!("API 錯誤：{} - {}", status, text));
    }

    let response_text = resp.text().await?;

    debug!("專案總結 API 回應：{}", response_text);

    let chat_resp: ChatResponse = serde_json::from_str(&response_text)
        .map_err(|e| anyhow!("無法解析 API 回應：{} - 回應：{}", e, response_text))?;

    let content = chat_resp
        .choices
        .get(0)
        .ok_or_else(|| anyhow!("API 回應中沒有內容"))?
        .message
        .content
        .trim();

    let json_str = if content.starts_with("```json") && content.ends_with("```") {
        content[7..content.len() - 3].trim()
    } else {
        content
    };

    debug!("準備解析的 JSON：{}", json_str);

    let summary: ProjectSummary = serde_json::from_str(json_str)
        .map_err(|e| anyhow!("無法解析專案總結 JSON：{} - 回應：{}", e, json_str))?;

    Ok(summary)
}
