mod analysis;
mod openai;
mod models;
mod markdown;
mod config;

use clap::{Parser, Subcommand};
use anyhow::{Result, anyhow};
use tracing::{info, error};
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about = "一個用於分析 Rust 程式碼並進行 AI 分析的命令列工具")]
struct Args {
    /// Rust 專案的路徑
    #[arg(short, long, default_value = ".")]
    path: String,
    /// OpenAI（或其他 GPT 服務）端點
    #[arg(long)]
    api_url: Option<String>,
    /// OpenAI API 金鑰或 GPT 令牌
    #[arg(long)]
    api_key: Option<String>,
    /// GPT 模型名稱
    #[arg(long)]
    model: Option<String>,
    /// 是否跳過 AI 分析
    #[arg(long)]
    skip_ai: bool,
    /// 僅輸出 JSON 格式（無 markdown）
    #[arg(long)]
    json: bool,
    /// 輸出檔案路徑（預設：analysis_report.{json|md}）
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
    /// 初始化專案配置
    Init,
    /// 配置設定
    Config {
        /// 使用全局配置
        #[arg(short, long)]
        global: bool,
    },
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

    let project_path = PathBuf::from(&args.path);
    if !project_path.exists() {
        error!("指定的路徑不存在：{}", project_path.display());
        return Err(anyhow!("指定的路徑不存在：{}", project_path.display()));
    }

    match &args.command {
        Some(Commands::Init) => {
            config::init_project(&project_path)?;
        }
        Some(Commands::Config { global }) => {
            config::configure_interactive(&project_path, *global).await?;
        }
        Some(Commands::Update { report }) => {
            analysis::update_report(report, &args.path, &args).await?;
        }
        Some(Commands::GenerateMd { report, output }) => {
            markdown::generate_md_from_json(report, output.as_deref()).await?;
        }
        None => {
            info!("開始分析路徑：{}", project_path.display());
            
            // 載入配置
            let config = config::get_effective_config(&project_path)?;
            
            // 命令行參數優先於配置文件
            let api_url = args.api_url.unwrap_or(config.api_url);
            let api_key = args.api_key.unwrap_or(config.api_key);
            let model = args.model.unwrap_or(config.model);
            
            let mut analyses = Vec::new();
            let mut total_files = 0_usize;
            let mut total_loc = 0_usize;
            
            // 遞迴掃描目錄
            for entry in walkdir::WalkDir::new(&project_path)
                .into_iter()
                .filter_entry(|e| {
                    let skip = e.path()
                        .components()
                        .any(|c| c.as_os_str() == "target");
                    if skip {
                        tracing::debug!("跳過目錄：{}", e.path().display());
                    }
                    !skip
                })
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() && path.extension().map_or(false, |ext| ext == "rs") {
                    total_files += 1;
                    tracing::debug!("分析檔案：{}", path.display());
                    
                    let code_str = match std::fs::read_to_string(path) {
                        Ok(content) => content,
                        Err(e) => {
                            error!("無法讀取檔案：{} - {}", path.display(), e);
                            continue;
                        }
                    };
                    
                    let stats = analysis::analyze_code(&code_str);
                    total_loc += stats.loc;
                    
                    // 計算相對路徑
                    let relative_path = path.strip_prefix(&project_path)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .to_string();
                    
                    let ai_analysis = if !args.skip_ai && !api_key.is_empty() {
                        info!("開始對檔案進行 AI 分析：{}", relative_path);
                        match openai::do_ai_analysis_with_retry(&api_url, &api_key, &model, &code_str, &relative_path).await {
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
                            tracing::debug!("跳過 AI 分析（已設定 skip_ai）");
                        } else if api_key.is_empty() {
                            tracing::warn!("跳過 AI 分析（API key 為空）");
                        }
                        None
                    };
                    
                    analyses.push(models::FileAnalysis {
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
                "analysis_report.json".to_string()
            } else {
                "analysis_report.md".to_string()
            };
            
            let project_summary = if !args.skip_ai && !api_key.is_empty() {
                match openai::generate_project_summary_with_retry(&analyses, &api_url, &api_key, &model).await {
                    Ok(Some(summary)) => Some(summary),
                    Ok(None) => Some(models::ProjectSummary {
                        total_files,
                        total_loc,
                        main_features: vec![],
                        code_architecture: String::new(),
                        key_components: vec![],
                        tech_stack: vec![],
                        recommendations: vec![],
                    }),
                    Err(e) => {
                        error!("專案總結系統錯誤：{}", e);
                        Some(models::ProjectSummary {
                            total_files,
                            total_loc,
                            main_features: vec![],
                            code_architecture: String::new(),
                            key_components: vec![],
                            tech_stack: vec![],
                            recommendations: vec![],
                        })
                    }
                }
            } else {
                Some(models::ProjectSummary {
                    total_files,
                    total_loc,
                    main_features: vec![],
                    code_architecture: String::new(),
                    key_components: vec![],
                    tech_stack: vec![],
                    recommendations: vec![],
                })
            };

            if args.json || output_path.ends_with(".json") {
                // 生成 JSON 輸出
                let project_analysis = models::ProjectAnalysis {
                    summary: project_summary.unwrap_or(models::ProjectSummary {
                        total_files,
                        total_loc,
                        main_features: vec![],
                        code_architecture: String::new(),
                        key_components: vec![],
                        tech_stack: vec![],
                        recommendations: vec![],
                    }),
                    file_analyses: analyses,
                };
                
                let json_report = serde_json::to_string_pretty(&project_analysis)?;
                std::fs::write(&output_path, &json_report)?;
                info!("分析完成！JSON 報告已寫入 {}", output_path);
            } else {
                // 生成 Markdown 報告
                markdown::generate_markdown_report(Some(analyses), project_summary, &output_path).await?;
                info!("分析完成！Markdown 報告已寫入 {}", output_path);
            }
        }
    }
    
    Ok(())
}