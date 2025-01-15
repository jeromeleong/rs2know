use clap::{Parser, Subcommand};
use walkdir::WalkDir;
use std::path::Path;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use reqwest::Client;
use tracing::{info, warn, error, debug};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};
use std::time::Duration;
use git2::Repository;
use std::collections::HashSet;

const MAX_RETRIES: u32 = 5;
const RETRY_DELAY_MS: u64 = 1000;

#[derive(Parser, Debug)]
#[command(author, version, about = "一個用於分析 Rust 程式碼並進行 AI 分析的命令列工具")]
struct Args {
    /// Rust 專案的路徑
    #[arg(short, long, default_value = ".")]
    path: String,
    /// OpenAI（或其他 GPT 服務）端點
    #[arg(long, default_value = "https://api.openai.com/v1/chat/completions")]
    api_url: String,
    /// OpenAI API 金鑰或 GPT 令牌
    #[arg(long, default_value = "")]
    api_key: String,
    /// GPT 模型名稱
    #[arg(long, default_value = "gpt-4o-mini")]
    model: String,
    /// 是否跳過 AI 分析
    #[arg(long)]
    skip_ai: bool,
    /// 僅輸出 JSON 格式（無 markdown）
    #[arg(long)]
    json: bool,
    /// 輸出檔案路徑（預設：rust_analysis_report.{json|md}）
    #[arg(short, long)]
    output: Option<String>,
    /// 日誌級別 (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,
    /// 子命令
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// 更新現有的 JSON 報告，只分析修改過的文件
    Update {
        /// 現有的 JSON 報告路徑
        #[arg(short, long)]
        report: String,
    },
    /// 從 JSON 生成 Markdown 報告
    GenerateMd {
        /// JSON 報告路徑
        #[arg(short, long)]
        report: String,
        /// 輸出的 Markdown 檔案路徑
        #[arg(short, long)]
        output: Option<String>,
    },
}

/// 用於最終輸出報告
#[derive(Debug, Serialize, Deserialize)]
struct FileAnalysis {
    file_path: String,
    loc: usize,
    blank_lines: usize,
    comment_lines: usize,
    code_lines: usize,
    ai_analysis: Option<AIAnalysis>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AIAnalysis {
    main_functions: Vec<String>,
    core_structs: Vec<CoreStruct>,
    error_types: Vec<String>,
    functions_details: Vec<FunctionDetail>,
    code_complexity: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CoreStruct {
    name: String,
    description: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FunctionDetail {
    name: String,
    description: String,
    parameters: Vec<String>,
    return_type: String,
    complexity: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectSummary {
    total_files: usize,
    total_loc: usize,
    main_features: Vec<String>,
    code_architecture: String,
    key_components: Vec<String>,
    tech_stack: Vec<String>,
    recommendations: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProjectAnalysis {
    summary: ProjectSummary,
    file_analyses: Vec<FileAnalysis>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // 設置日誌級別
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&args.log_level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    match &args.command {
        Some(Commands::Update { report }) => {
            update_report(report, &args.path, &args).await?;
        }
        Some(Commands::GenerateMd { report, output }) => {
            generate_md_from_json(report, output.as_deref()).await?;
        }
        None => {
            // 檢查路徑是否存在
            let project_path = Path::new(&args.path);
            if !project_path.exists() {
                error!("指定的路徑不存在：{}", project_path.display());
                return Err(anyhow!("指定的路徑不存在：{}", project_path.display()));
            }
            
            info!("開始分析路徑：{}", project_path.display());
            debug!("使用的設定：{:?}", args);
            
            let mut analyses: Vec<FileAnalysis> = Vec::new();
            let mut total_files = 0_usize;
            let mut total_loc = 0_usize;
            
            // 遞迴掃描目錄
            for entry in WalkDir::new(project_path)
                .into_iter()
                .filter_entry(|e| {
                    let skip = e.path()
                        .components()
                        .any(|c| c.as_os_str() == "target");
                    if skip {
                        debug!("跳過目錄：{}", e.path().display());
                    }
                    !skip
                })
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() && path.extension().map_or(false, |ext| ext == "rs") {
                    total_files += 1;
                    debug!("分析檔案：{}", path.display());
                    
                    let code_str = match std::fs::read_to_string(path) {
                        Ok(content) => content,
                        Err(e) => {
                            error!("無法讀取檔案：{} - {}", path.display(), e);
                            continue;
                        }
                    };
                    
                    let stats = analyze_code(&code_str);
                    total_loc += stats.loc;
                    
                    // 計算相對路徑
                    let relative_path = path.strip_prefix(project_path)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .to_string();
                    
                    let ai_analysis = if !args.skip_ai && !args.api_key.is_empty() {
                        info!("開始對檔案進行 AI 分析：{}", relative_path);
                        match do_ai_analysis_with_retry(&args.api_url, &args.api_key, &args.model, &code_str, &relative_path).await {
                            Ok(Some(ai_result)) => {
                                info!("AI 分析成功：{}", relative_path);
                                Some(ai_result)
                            }
                            Ok(None) => None,
                            Err(e) => {
                                error!("AI 分析系統錯誤：{} - {}", relative_path, e);
                                None
                            }
                        }
                    } else {
                        if args.skip_ai {
                            debug!("跳過 AI 分析（已設定 skip_ai）");
                        } else if args.api_key.is_empty() {
                            warn!("跳過 AI 分析（API key 為空）");
                        }
                        None
                    };
                    
                    analyses.push(FileAnalysis {
                        file_path: relative_path,
                        loc: stats.loc,
                        blank_lines: stats.blank_lines,
                        comment_lines: stats.comment_lines,
                        code_lines: stats.code_lines,
                        ai_analysis,
                    });
                }
            }
            
            info!("偵測到的 Rust 檔案數：{}", total_files);
            info!("程式碼總行數：{}", total_loc);
            
            // 決定輸出路徑和格式
            let output_path = if let Some(path) = args.output {
                path
            } else if args.json {
                "rust_analysis_report.json".to_string()
            } else {
                "rust_analysis_report.md".to_string()
            };
            
            if args.json || output_path.ends_with(".json") {
                // 生成專案總結（如果啟用了 AI 分析）
                let project_summary = if !args.skip_ai && !args.api_key.is_empty() {
                    match generate_project_summary_with_retry(&analyses, &args.api_url, &args.api_key, &args.model).await {
                        Ok(Some(summary)) => ProjectAnalysis {
                            summary,
                            file_analyses: analyses,
                        },
                        Ok(None) => ProjectAnalysis {
                            summary: ProjectSummary {
                                total_files,
                                total_loc,
                                main_features: vec![],
                                code_architecture: String::new(),
                                key_components: vec![],
                                tech_stack: vec![],
                                recommendations: vec![],
                            },
                            file_analyses: analyses,
                        },
                        Err(e) => {
                            error!("專案總結系統錯誤：{}", e);
                            ProjectAnalysis {
                                summary: ProjectSummary {
                                    total_files,
                                    total_loc,
                                    main_features: vec![],
                                    code_architecture: String::new(),
                                    key_components: vec![],
                                    tech_stack: vec![],
                                    recommendations: vec![],
                                },
                                file_analyses: analyses,
                            }
                        }
                    }
                } else {
                    ProjectAnalysis {
                        summary: ProjectSummary {
                            total_files,
                            total_loc,
                            main_features: vec![],
                            code_architecture: String::new(),
                            key_components: vec![],
                            tech_stack: vec![],
                            recommendations: vec![],
                        },
                        file_analyses: analyses,
                    }
                };

                // 生成 JSON 輸出
                let json_report = serde_json::to_string_pretty(&project_summary)?;
                std::fs::write(&output_path, &json_report)?;
                info!("分析完成！JSON 報告已寫入 {}", output_path);
            } else {
                // 生成 Markdown 報告
                let mut md_content = String::new();
                md_content.push_str("# Rust 程式碼分析報告\n\n");

                // 生成專案總結（如果啟用了 AI 分析）
                let project_summary = if !args.skip_ai && !args.api_key.is_empty() {
                    match generate_project_summary_with_retry(&analyses, &args.api_url, &args.api_key, &args.model).await {
                        Ok(Some(summary)) => Some(summary),
                        Ok(None) => None,
                        Err(e) => {
                            error!("專案總結系統錯誤：{}", e);
                            None
                        }
                    }
                } else {
                    None
                };

                // 如果有專案總結，加入總結部分
                if let Some(summary) = project_summary {
                    md_content.push_str("## 專案總結\n\n");
                    
                    md_content.push_str("### 基本資訊\n\n");
                    md_content.push_str(&format!("- 總檔案數：{}\n", summary.total_files));
                    md_content.push_str(&format!("- 總程式碼行數：{}\n\n", summary.total_loc));
                    
                    if !summary.main_features.is_empty() {
                        md_content.push_str("### 主要功能\n\n");
                        for feature in &summary.main_features {
                            md_content.push_str(&format!("- {}\n", feature));
                        }
                        md_content.push_str("\n");
                    }
                    
                    if !summary.code_architecture.is_empty() {
                        md_content.push_str("### 程式架構\n\n");
                        md_content.push_str(&format!("{}\n\n", summary.code_architecture));
                    }
                    
                    if !summary.key_components.is_empty() {
                        md_content.push_str("### 關鍵元件\n\n");
                        for component in &summary.key_components {
                            md_content.push_str(&format!("- {}\n", component));
                        }
                        md_content.push_str("\n");
                    }
                    
                    if !summary.tech_stack.is_empty() {
                        md_content.push_str("### 技術堆疊\n\n");
                        for tech in &summary.tech_stack {
                            md_content.push_str(&format!("- {}\n", tech));
                        }
                        md_content.push_str("\n");
                    }
                    
                    if !summary.recommendations.is_empty() {
                        md_content.push_str("### 改進建議\n\n");
                        for rec in &summary.recommendations {
                            md_content.push_str(&format!("- {}\n", rec));
                        }
                        md_content.push_str("\n");
                    }
                    
                    md_content.push_str("---\n\n");
                }
                
                // 按目錄組織文件
                let mut dir_files: std::collections::HashMap<String, Vec<&FileAnalysis>> = std::collections::HashMap::new();
                
                for analysis in &analyses {
                    let parent = Path::new(&analysis.file_path)
                        .parent()
                        .and_then(|p| p.to_str())
                        .unwrap_or("root")
                        .to_string();
                    dir_files.entry(parent.clone()).or_default().push(analysis);
                }
                
                // 生成目錄
                md_content.push_str("## 目錄\n\n");
                
                // 為每個目錄生成目錄項
                let mut sorted_dirs: Vec<String> = dir_files.keys().cloned().collect();
                sorted_dirs.sort();
                
                for dir in &sorted_dirs {
                    let safe_dir = if dir.as_str() == "root" {
                        "根目錄".to_string()
                    } else {
                        dir.replace('/', "-")
                    };
                    md_content.push_str(&format!("- [{}](#{})\n", dir, safe_dir));
                    
                    if let Some(files) = dir_files.get(dir) {
                        for analysis in files {
                            let file_name = Path::new(&analysis.file_path)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .unwrap_or(&analysis.file_path);
                            let safe_file = file_name.replace('.', "-");
                            md_content.push_str(&format!("  - [{}](#{})\n", file_name, safe_file));
                        }
                    }
                }
                md_content.push_str("\n---\n\n");
                
                // 按目錄生成內容
                for dir in &sorted_dirs {
                    if let Some(files) = dir_files.get(dir) {
                        let display_dir = if dir.as_str() == "root" {
                            "根目錄".to_string()
                        } else {
                            dir.clone()
                        };
                        md_content.push_str(&format!("## {}\n\n", display_dir));
                        
                        for analysis in files {
                            let file_name = Path::new(&analysis.file_path)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .unwrap_or(&analysis.file_path);
                            
                            md_content.push_str(&format!("### {}\n\n", file_name));
                            if let Some(ai) = &analysis.ai_analysis {
                                if !ai.main_functions.is_empty() {
                                    md_content.push_str("#### 主要函數\n\n");
                                    for func in &ai.main_functions {
                                        md_content.push_str(&format!("- {}\n", func));
                                    }
                                    md_content.push_str("\n");
                                }
                                
                                if !ai.core_structs.is_empty() {
                                    md_content.push_str("#### 核心結構體\n\n");
                                    for struct_info in &ai.core_structs {
                                        md_content.push_str(&format!("- **{}**：{}\n", struct_info.name, struct_info.description));
                                    }
                                    md_content.push_str("\n");
                                }
                                
                                if !ai.error_types.is_empty() {
                                    md_content.push_str("#### 錯誤類型\n\n");
                                    for error in &ai.error_types {
                                        md_content.push_str(&format!("- {}\n", error));
                                    }
                                    md_content.push_str("\n");
                                }
                                
                                if !ai.functions_details.is_empty() {
                                    md_content.push_str("#### 函數詳情\n\n");
                                    for func in &ai.functions_details {
                                        md_content.push_str(&format!("##### {}\n\n", func.name));
                                        md_content.push_str(&format!("- 說明：{}\n", func.description));
                                        if !func.parameters.is_empty() {
                                            md_content.push_str("- 參數：\n");
                                            for param in &func.parameters {
                                                md_content.push_str(&format!("  - {}\n", param));
                                            }
                                        }
                                        md_content.push_str(&format!("- 返回類型：{}\n", func.return_type));
                                        md_content.push_str(&format!("- 複雜度：{}\n\n", func.complexity));
                                    }
                                }
                                
                                md_content.push_str("#### 程式碼複雜度\n\n");
                                md_content.push_str(&format!("{}\n\n", ai.code_complexity));
                            }
                            
                            md_content.push_str("---\n\n");
                        }
                    }
                }
                
                std::fs::write(&output_path, md_content)?;
                info!("分析完成！Markdown 報告已寫入 {}", output_path);
            }
        }
    }
    
    Ok(())
}

struct CodeStats {
    loc: usize,
    blank_lines: usize,
    comment_lines: usize,
    code_lines: usize,
}

fn analyze_code(content: &str) -> CodeStats {
    let mut stats = CodeStats {
        loc: 0,
        blank_lines: 0,
        comment_lines: 0,
        code_lines: 0,
    };
    
    let mut in_block_comment = false;
    
    for line in content.lines() {
        let trimmed = line.trim();
        stats.loc += 1;
        
        if trimmed.is_empty() {
            stats.blank_lines += 1;
            continue;
        }
        
        if in_block_comment {
            stats.comment_lines += 1;
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        
        if trimmed.starts_with("/*") {
            stats.comment_lines += 1;
            in_block_comment = true;
            if !trimmed.contains("*/") {
                continue;
            }
            in_block_comment = false;
        } else if trimmed.starts_with("//") {
            stats.comment_lines += 1;
        } else {
            stats.code_lines += 1;
        }
    }
    
    stats
}

/// 呼叫 GPT/OpenAI API 進行 AI 分析
async fn do_ai_analysis_with_retry(
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
                    warn!("AI 分析失敗 (重試 {}/{}): {} - {}", retries + 1, MAX_RETRIES, file_path, e);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    retries += 1;
                } else {
                    error!("AI 分析在重試{}次後仍然失敗：{} - {}", MAX_RETRIES, file_path, e);
                    return Ok(None);
                }
            }
        }
    }
    Ok(None)
}

/// 呼叫 GPT/OpenAI API 進行 AI 分析
async fn do_ai_analysis(
    api_url: &str,
    api_key: &str,
    model: &str,
    code: &str
) -> Result<AIAnalysis> {
    info!("發送 API 請求至：{}", api_url);
    let prompt = format!(
        "分析這個 Rust 文件並直接返回 JSON 格式的結構化信息，不要加入任何 markdown 標記。JSON 格式如下：
{{
  \"main_functions\": [\"函數名稱及簡要說明\"],
  \"core_structs\": [
    {{
      \"name\": \"結構體名稱\",
      \"description\": \"結構體說明\"
    }}
  ],
  \"error_types\": [\"錯誤類型描述\"],
  \"functions_details\": [
    {{
      \"name\": \"函數名稱\",
      \"description\": \"函數說明\",
      \"parameters\": [\"參數列表\"],
      \"return_type\": \"返回類型\",
      \"complexity\": \"代碼複雜度評估\"
    }}
  ],
  \"code_complexity\": \"整體代碼複雜度評估\"
}}
分析內容需要包括：
1. 主要功能列表
2. 核心類型/結構體及其說明
3. 錯誤類型分類（如果有）
4. 最小功能單元（函數）的詳細信息
5. 代碼複雜度
代碼內容：
{}", 
        code
    );
    
    #[derive(Serialize)]
    struct ChatRequest<'a> {
        model: &'a str,
        messages: Vec<ChatMessage<'a>>,
    }
    
    #[derive(Serialize)]
    struct ChatMessage<'a> {
        role: &'a str,
        content: &'a str,
    }
    
    let body = ChatRequest {
        model,
        messages: vec![ChatMessage {
            role: "user",
            content: &prompt,
        }],
    };
    
    debug!("請求內容：{}", serde_json::to_string_pretty(&body)?);
    
    let client = Client::new();
    let resp = client
        .post(api_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("AI 請求失敗：{}", e))?;
    
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
    
    // 獲取內容並移除任何 markdown 格式
    let content = chat_resp.choices.into_iter().next()
        .ok_or_else(|| anyhow!("AI 未返回任何選項"))?
        .message
        .content;
    
    let clean_content = content
        .trim_start_matches("```json")
        .trim_start_matches("```")  // 以防沒有 "json"
        .trim_end_matches("```")
        .trim();

    // 找到 JSON 物件的最後一個大括號
    let last_brace = clean_content.rfind('}')
        .ok_or_else(|| anyhow!("在 AI 回應中找不到結束大括號"))?;
    let clean_json = &clean_content[..=last_brace];

    // 將清理後的內容反序列化為 AIAnalysis
    let ai_analysis: AIAnalysis = serde_json::from_str(clean_json)
        .map_err(|e| anyhow!("無法反序列化 AI 分析：{}", e))?;
    
    Ok(ai_analysis)
}

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

async fn generate_project_summary_with_retry(
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
                    warn!("專案總結生成失敗 (重試 {}/{}): {}", retries + 1, MAX_RETRIES, e);
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
        .post(api_url)
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

    // 解析 ChatGPT 回應
    let chat_resp: ChatResponse = serde_json::from_str(&response_text)
        .map_err(|e| anyhow!("無法解析 API 回應：{} - 回應：{}", e, response_text))?;

    // 取得實際的 JSON 內容
    let content = chat_resp.choices.get(0)
        .ok_or_else(|| anyhow!("API 回應中沒有內容"))?
        .message.content.trim();

    // 如果內容被包裹在程式碼區塊中，移除它
    let json_str = if content.starts_with("```json") && content.ends_with("```") {
        content[7..content.len()-3].trim()
    } else {
        content
    };

    debug!("準備解析的 JSON：{}", json_str);

    let summary: ProjectSummary = serde_json::from_str(json_str)
        .map_err(|e| anyhow!("無法解析專案總結 JSON：{} - 回應：{}", e, json_str))?;

    Ok(summary)
}

async fn update_report(report_path: &str, project_path: &str, args: &Args) -> Result<()> {
    // 讀取現有的 JSON 報告
    let report_content = std::fs::read_to_string(report_path)?;
    let mut project_analysis: ProjectAnalysis = serde_json::from_str(&report_content)?;

    // 打開 Git 倉庫
    let repo = Repository::discover(project_path)?;
    let head = repo.head()?.peel_to_commit()?;
    let tree = head.tree()?;
    let diff = repo.diff_tree_to_workdir(Some(&tree), None)?;

    // 收集已修改的文件
    let mut modified_files = HashSet::new();
    diff.foreach(&mut |delta, _| {
        if let Some(path) = delta.new_file().path() {
            if path.extension().map_or(false, |ext| ext == "rs") {
                modified_files.insert(path.to_string_lossy().to_string());
            }
        }
        true
    }, None, None, None)?;

    if modified_files.is_empty() {
        info!("沒有檔案被修改，無需更新報告。");
        return Ok(());
    }

    info!("檢測到修改的檔案數量：{}", modified_files.len());

    // 重新分析修改過的文件
    for analysis in &mut project_analysis.file_analyses {
        if modified_files.contains(&analysis.file_path) {
            let full_path = Path::new(project_path).join(&analysis.file_path);
            if full_path.exists() {
                let code_str = std::fs::read_to_string(&full_path)?;
                let stats = analyze_code(&code_str);
                analysis.loc = stats.loc;
                analysis.blank_lines = stats.blank_lines;
                analysis.comment_lines = stats.comment_lines;
                analysis.code_lines = stats.code_lines;

                if !args.skip_ai && !args.api_key.is_empty() {
                    info!("開始對檔案進行 AI 分析：{}", analysis.file_path);
                    let ai_analysis = do_ai_analysis_with_retry(
                        &args.api_url,
                        &args.api_key,
                        &args.model,
                        &code_str,
                        &analysis.file_path,
                    ).await?;
                    analysis.ai_analysis = ai_analysis;
                }
            }
        }
    }

    // 更新專案總結
    if !args.skip_ai && !args.api_key.is_empty() {
        let summary = generate_project_summary_with_retry(&project_analysis.file_analyses, &args.api_url, &args.api_key, &args.model).await?;
        if let Some(s) = summary {
            project_analysis.summary = s;
        }
    }

    // 寫回更新後的 JSON 報告
    let updated_report = serde_json::to_string_pretty(&project_analysis)?;
    std::fs::write(report_path, updated_report)?;
    info!("報告已更新並寫入 {}", report_path);

    Ok(())
}

async fn generate_md_from_json(report_path: &str, output_path: Option<&str>) -> Result<()> {
    // 讀取 JSON 報告
    let report_content = std::fs::read_to_string(report_path)?;
    let project_analysis: ProjectAnalysis = serde_json::from_str(&report_content)?;

    // 決定輸出路徑
    let output = match output_path {
        Some(path) => path.to_string(),
        None => "rust_analysis_report_generated.md".to_string(),
    };

    // 生成 Markdown 內容
    let mut md_content = String::new();
    md_content.push_str("# Rust 程式碼分析報告\n\n");
    
    // 專案總結
    let summary = &project_analysis.summary;
    md_content.push_str("## 專案總結\n\n");
    md_content.push_str("### 基本資訊\n\n");
    md_content.push_str(&format!("- 總檔案數：{}\n", summary.total_files));
    md_content.push_str(&format!("- 總程式碼行數：{}\n\n", summary.total_loc));
    
    if !summary.main_features.is_empty() {
        md_content.push_str("### 主要功能\n\n");
        for feature in &summary.main_features {
            md_content.push_str(&format!("- {}\n", feature));
        }
        md_content.push_str("\n");
    }
    
    if !summary.code_architecture.is_empty() {
        md_content.push_str("### 程式架構\n\n");
        md_content.push_str(&format!("{}\n\n", summary.code_architecture));
    }
    
    if !summary.key_components.is_empty() {
        md_content.push_str("### 關鍵元件\n\n");
        for component in &summary.key_components {
            md_content.push_str(&format!("- {}\n", component));
        }
        md_content.push_str("\n");
    }
    
    if !summary.tech_stack.is_empty() {
        md_content.push_str("### 技術堆疊\n\n");
        for tech in &summary.tech_stack {
            md_content.push_str(&format!("- {}\n", tech));
        }
        md_content.push_str("\n");
    }
    
    if !summary.recommendations.is_empty() {
        md_content.push_str("### 改進建議\n\n");
        for rec in &summary.recommendations {
            md_content.push_str(&format!("- {}\n", rec));
        }
        md_content.push_str("\n");
    }
    
    md_content.push_str("---\n\n");

    // 添加文件分析
    for analysis in &project_analysis.file_analyses {
        md_content.push_str(&format!("## {}\n\n", analysis.file_path));
        md_content.push_str("### 程式碼統計\n\n");
        md_content.push_str(&format!("- 總行數：{}\n", analysis.loc));
        md_content.push_str(&format!("- 程式碼行數：{}\n", analysis.code_lines));
        md_content.push_str(&format!("- 註解行數：{}\n", analysis.comment_lines));
        md_content.push_str(&format!("- 空白行數：{}\n\n", analysis.blank_lines));
        
        if let Some(ai) = &analysis.ai_analysis {
            md_content.push_str("### AI 分析\n\n");
            
            if !ai.main_functions.is_empty() {
                md_content.push_str("#### 主要函數\n\n");
                for func in &ai.main_functions {
                    md_content.push_str(&format!("- {}\n", func));
                }
                md_content.push_str("\n");
            }
            
            if !ai.core_structs.is_empty() {
                md_content.push_str("#### 核心結構體\n\n");
                for struct_info in &ai.core_structs {
                    md_content.push_str(&format!("- **{}**：{}\n", struct_info.name, struct_info.description));
                }
                md_content.push_str("\n");
            }
            
            if !ai.error_types.is_empty() {
                md_content.push_str("#### 錯誤類型\n\n");
                for error in &ai.error_types {
                    md_content.push_str(&format!("- {}\n", error));
                }
                md_content.push_str("\n");
            }
            
            if !ai.functions_details.is_empty() {
                md_content.push_str("#### 函數詳情\n\n");
                for func in &ai.functions_details {
                    md_content.push_str(&format!("##### {}\n\n", func.name));
                    md_content.push_str(&format!("- 說明：{}\n", func.description));
                    if !func.parameters.is_empty() {
                        md_content.push_str("- 參數：\n");
                        for param in &func.parameters {
                            md_content.push_str(&format!("  - {}\n", param));
                        }
                    }
                    md_content.push_str(&format!("- 返回類型：{}\n", func.return_type));
                    md_content.push_str(&format!("- 複雜度：{}\n\n", func.complexity));
                }
            }
            
            md_content.push_str("#### 程式碼複雜度\n\n");
            md_content.push_str(&format!("{}\n\n", ai.code_complexity));
        }
        
        md_content.push_str("---\n\n");
    }

    // 寫入 Markdown 檔案
    std::fs::write(&output, md_content)?;
    info!("Markdown 報告已生成並寫入 {}", output);

    Ok(())
}