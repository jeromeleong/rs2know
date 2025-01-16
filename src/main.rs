mod analysis;
mod config;
mod markdown;
mod models;
mod openai;
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "一個用於分析 Rust 程式碼並進行 AI 分析的命令列工具"
)]
pub struct Args {
    /// 專案路徑
    #[arg(default_value = ".")]
    path: String,
    /// API URL
    #[arg(long)]
    api_url: Option<String>,
    /// API Key
    #[arg(long)]
    api_key: Option<String>,
    /// 模型名稱
    #[arg(long)]
    model: Option<String>,
    /// 輸出路徑
    #[arg(short, long)]
    output: Option<String>,
    /// 輸出 JSON 格式
    #[arg(long)]
    json: bool,
    /// 跳過 AI 分析
    #[arg(long)]
    skip_ai: bool,
    /// 日誌級別 (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,
    /// 子命令
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 初始化專案
    Init,
    /// 配置設定
    Config {
        /// 是否為全局配置
        #[arg(short, long)]
        global: bool,
    },
    /// 更新分析報告
    Update,
    /// 從 JSON 生成 Markdown 報告
    GenerateMd {
        /// JSON 報告路徑
        report: String,
        /// 輸出 Markdown 路徑
        #[arg(short, long)]
        output: Option<String>,
    },
}
async fn handle_default_analysis(args: &Args, project_path: &Path) -> Result<()> {
    let has_config = project_path.join(".pj.yml").exists();
    let current_version = analysis::get_git_version(project_path)?;
    // 檢查是否需要重新分析
    let needs_analysis = if has_config {
        let config = config::get_effective_config(project_path)?;
        analysis::needs_reanalysis(
            current_version.as_deref(),
            config
                .output
                .as_ref()
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
        )
    } else {
        true
    };
    if !needs_analysis {
        info!("當前版本已分析，無需重新分析");
        return Ok(());
    }
    // 執行分析並獲取結果
    info!("開始分析路徑：{}", project_path.display());
    let (analyses, project_summary) = perform_analysis(args, project_path).await?;
    // 準備分析結果
    let default_summary = models::ProjectSummary {
        total_files: analyses.len(),
        total_loc: analyses.iter().map(|a| a.loc).sum(),
        main_features: vec![],
        code_architecture: String::new(),
        key_components: vec![],
        tech_stack: vec![],
        recommendations: vec![],
    };
    let project_analysis = models::ProjectAnalysis {
        summary: project_summary.clone().unwrap_or(default_summary.clone()),
        file_analyses: analyses.clone(),
        git_version: current_version.clone(),
        analyzed_versions: current_version.map(|v| vec![v]),
    };
    // 如果有配置文件，保存到配置中
    if has_config {
        let mut config = config::get_effective_config(project_path)?;
        config.output = Some(serde_json::to_value(&project_analysis)?);
        config.save(project_path)?;
        info!("分析結果已保存到配置文件");
    }
    // 根據參數決定輸出
    if let Some(output_path) = &args.output {
        if output_path.ends_with(".json") {
            let json_report = serde_json::to_string_pretty(&project_analysis)?;
            std::fs::write(output_path, &json_report)?;
            info!("JSON 報告已生成：{}", output_path);
        } else {
            markdown::generate_markdown_report(Some(analyses), project_summary, output_path)
                .await?;
            info!("Markdown 報告已生成：{}", output_path);
        }
    } else if !has_config || args.json {
        // 只有在沒有配置文件或明確要求 JSON 時才生成 JSON 文件
        let json_report = serde_json::to_string_pretty(&project_analysis)?;
        std::fs::write("analysis_report.json", &json_report)?;
        info!("JSON 報告已生成：analysis_report.json");
    } else {
        // 默認生成 Markdown 報告
        markdown::generate_markdown_report(Some(analyses), project_summary, "analysis_report.md")
            .await?;
        info!("Markdown 報告已生成：analysis_report.md");
    }
    Ok(())
}
async fn perform_analysis(
    args: &Args,
    project_path: &Path,
) -> Result<(Vec<models::FileAnalysis>, Option<models::ProjectSummary>)> {
    let mut analyses = Vec::new();
    let mut total_files = 0_usize;
    let mut total_loc = 0_usize;
    // 載入配置
    let config = config::get_effective_config(project_path)?;
    // 命令行參數優先於配置文件
    let api_url = args.api_url.as_deref().unwrap_or(&config.api_url);
    let api_key = args.api_key.as_deref().unwrap_or(&config.api_key);
    let model = args.model.as_deref().unwrap_or(&config.model);
    // 遞迴掃描目錄
    for entry in walkdir::WalkDir::new(project_path)
        .into_iter()
        .filter_entry(|e| {
            let skip = e.path().components().any(|c| c.as_os_str() == "target");
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
            let code_str = std::fs::read_to_string(path)?;
            let stats = analysis::analyze_code(&code_str);
            total_loc += stats.loc;
            // 計算相對路徑
            let relative_path = path
                .strip_prefix(project_path)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            let ai_analysis = if !args.skip_ai && !api_key.is_empty() {
                info!("開始對檔案進行 AI 分析：{}", relative_path);
                match openai::do_ai_analysis_with_retry(
                    api_url,
                    api_key,
                    model,
                    &code_str,
                    &relative_path,
                )
                .await
                {
                    Ok(Some(ai_result)) => {
                        info!("AI 分析成功：{}", relative_path);
                        Some(ai_result)
                    }
                    Ok(None) => None,
                    Err(e) => {
                        error!("AI 分析錯誤：{} - {}", relative_path, e);
                        None
                    }
                }
            } else {
                if args.skip_ai {
                    debug!("跳過 AI 分析（已設定 skip_ai）");
                } else if api_key.is_empty() {
                    warn!("跳過 AI 分析（API key 為空）");
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
    let project_summary = if !args.skip_ai && !api_key.is_empty() {
        match openai::generate_project_summary_with_retry(&analyses, api_url, api_key, model).await
        {
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
                error!("專案總結錯誤：{}", e);
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
    Ok((analyses, project_summary))
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
            info!("專案初始化完成");
        }
        Some(Commands::Config { global }) => {
            config::configure_interactive(&project_path, *global).await?;
            info!("配置設定完成");
        }
        Some(Commands::Update) => {
            analysis::update_report(&project_path, &args).await?;
            info!("分析報告已更新");
        }
        Some(Commands::GenerateMd { report, output }) => {
            markdown::generate_md_from_json(report, output.as_deref()).await?;
        }
        None => {
            handle_default_analysis(&args, &project_path).await?;
        }
    }
    Ok(())
}
