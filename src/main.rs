mod analysis;
mod config;
mod markdown;
mod models;
mod openai;
mod utils;
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{error, info};
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};
use crate::models::{FileAnalysis, ProjectAnalysis, ProjectSummary};
#[derive(Parser, Debug, Clone)]
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
    /// 輸出路徑（僅適用於生成文件的命令）
    #[arg(short, long, global = true)]
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
    /// 輸入報告檔案路徑（舊的分析報告 JSON）
    #[arg(long)]
    input: Option<String>,
    /// 保留現有的分析結果，只更新 Markdown 報告
    #[arg(long)]
    keep: bool,
    /// 子命令
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand, Debug, Clone)]
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
    Update {
        /// 輸入報告檔案路徑
        #[arg(long)]
        input: Option<String>,
        /// API URL
        #[arg(long)]
        api_url: Option<String>,
        /// API Key
        #[arg(long)]
        api_key: Option<String>,
        /// 模型名稱
        #[arg(long)]
        model: Option<String>,
        /// 保留現有的分析結果，只更新 Markdown 報告
        #[arg(long)]
        keep: bool,
    },
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
    if args.input.is_some() || has_config {
        info!("Detected --input or existing .pj.yml, updating analysis report...");
        analysis::update_report(
            project_path,
            args,
            &args.input,
            &args.api_url,
            &args.api_key,
            &args.model,
            args.keep,
        )
        .await?;
    } else {
        info!("No .pj.yml or --input detected, performing fresh analysis...");
        let (analyses, project_summary) = perform_analysis(args, project_path).await?;
        
        // Create project analysis
        let project_analysis = utils::create_project_analysis(analyses.clone(), project_summary.clone());
        
        if args.json {
            // Generate JSON report only
            utils::save_json_report(&project_analysis, true, &args.output)?;
            info!("Generated JSON report only");
        } else {
            // Generate markdown report only
            let output_path = args.output.clone().unwrap_or_else(|| "analysis_report.md".to_string());
            markdown::generate_markdown_report(
                Some(analyses),
                project_summary,
                &output_path,
                &args.output,
            )
            .await?;
            info!("Generated markdown report only: {}", output_path);
        }
    }
    Ok(())
}
pub async fn perform_analysis(
    args: &Args,
    project_path: &Path,
) -> Result<(Vec<FileAnalysis>, Option<ProjectSummary>)> {
    let mut analyses = Vec::new();
    let mut previous_analyses = HashMap::new();
    // Load previous analyses from input file or config
    if let Some(input_path) = &args.input {
        info!("Loading previous analysis from input file: {}", input_path);
        if let Ok(content) = std::fs::read_to_string(input_path) {
            if let Ok(project_analysis) = serde_json::from_str::<ProjectAnalysis>(&content) {
                info!("Successfully loaded {} previous analyses", project_analysis.file_analyses.len());
                for analysis in project_analysis.file_analyses {
                    previous_analyses.insert(analysis.file_path.clone(), analysis);
                }
            }
        }
    } else if let Some(output) = &config::get_effective_config(project_path)?.generated {
        info!("Loading previous analysis from config");
        if let Ok(project_analysis) = serde_json::from_value::<ProjectAnalysis>(output.clone()) {
            info!("Successfully loaded {} previous analyses from config", project_analysis.file_analyses.len());
            for analysis in project_analysis.file_analyses {
                previous_analyses.insert(analysis.file_path.clone(), analysis);
            }
        }
    }

    // Analyze Rust files
    info!("Starting file analysis in: {}", project_path.display());
    for entry in walkdir::WalkDir::new(project_path)
        .into_iter()
        .filter_entry(|e| {
            let path = e.path();
            let should_include = !path.components().any(|c| {
                let name = c.as_os_str().to_string_lossy();
                name == ".git" || name == ".pj.yml" || name == "target"
            });
            info!("Checking path: {} -> {}", path.display(), should_include);
            should_include
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() || !entry.path().to_string_lossy().ends_with(".rs") {
            continue;
        }

        let relative_path = entry
            .path()
            .strip_prefix(project_path)?
            .to_string_lossy()
            .into_owned();

        info!("Analyzing file: {}", relative_path);

        // Skip files that haven't changed
        let content = std::fs::read_to_string(entry.path())?;
        let file_stats = analysis::analyze_code(&content);
        // Skip files that haven't changed
        if let Some(prev_analysis) = previous_analyses.get(&relative_path) {
            if prev_analysis.code_hash == file_stats.code_hash {
                info!("Skipping AI analysis for unchanged file: {}", relative_path);
                analyses.push(prev_analysis.clone());
                continue;
            }
            info!("File changed, will reanalyze: {} (old hash: {}, new hash: {})", 
                relative_path, prev_analysis.code_hash, file_stats.code_hash);
        } else {
            info!("New file found: {}", relative_path);
        }

        if !args.skip_ai {
            info!("Starting AI analysis for file: {}", relative_path);
            let api_url = args.api_url.as_deref().unwrap_or("https://api.openai.com/v1");
            let api_key = args
                .api_key
                .as_ref()
                .ok_or_else(|| anyhow!("API key is required for AI analysis"))?;
            let model = args.model.as_deref().unwrap_or("gpt-3.5-turbo");

            let ai_analysis = match openai::do_ai_analysis_with_retry(
                api_url,
                api_key,
                model,
                &content
            )
            .await
            {
                Ok(analysis) => {
                    info!("AI analysis successful: {}", relative_path);
                    analysis
                }
                Err(e) => {
                    error!("AI analysis failed for {}: {}", relative_path, e);
                    None
                }
            };

            analyses.push(FileAnalysis {
                file_path: relative_path,
                loc: file_stats.loc,
                blank_lines: file_stats.blank_lines,
                comment_lines: file_stats.comment_lines,
                code_lines: file_stats.code_lines,
                code_hash: file_stats.code_hash,
                ai_analysis,
            });
        }
    }

    info!("File analysis completed, found {} files", analyses.len());

    // Generate project summary
    let project_summary = if !args.skip_ai && !analyses.is_empty() {
        info!("開始生成專案總結");
        let api_url = args.api_url.as_deref().unwrap_or("https://api.openai.com/v1");
        let api_key = args
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow!("API key is required for AI analysis"))?;
        let model = args.model.as_deref().unwrap_or("gpt-3.5-turbo");

        match openai::generate_project_summary_with_retry(&analyses, api_url, api_key, model).await {
            Ok(summary) => Some(summary),
            Err(e) => {
                error!("Failed to generate project summary: {}", e);
                None
            }
        }
    } else {
        None
    }.flatten();

    Ok((analyses, project_summary))
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
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
            if args.output.is_some() {
                return Err(anyhow!("`--output` 選項不適用於 `init` 子命令"));
            }
            config::init_project(&project_path)?;
            info!("專案初始化完成");
        }
        Some(Commands::Config { global }) => {
            if args.output.is_some() {
                return Err(anyhow!("`--output` 選項不適用於 `config` 子命令"));
            }
            config::configure_interactive(&project_path, *global).await?;
            info!("配置設定完成");
        }
        Some(Commands::Update { input, api_url, api_key, model, keep }) => {
            let update_args = Args {
                path: args.path.clone(),
                api_url: api_url.clone().or(args.api_url.clone()),
                api_key: api_key.clone().or(args.api_key.clone()),
                model: model.clone().or(args.model.clone()),
                output: args.output.clone(),
                json: args.json,
                skip_ai: args.skip_ai,
                log_level: args.log_level.clone(),
                input: input.clone().or(args.input.clone()),
                keep: *keep,
                command: None,
            };
            analysis::update_report(
                &project_path,
                &update_args,
                input,
                api_url,
                api_key,
                model,
                *keep,
            )
            .await?;
            info!("分析報告已更新");
        }
        Some(Commands::GenerateMd { report, output }) => {
            markdown::generate_md_from_json(report, output.as_deref(), &args.output).await?;
        }
        None => {
            handle_default_analysis(&args, &project_path).await?;
        }
    }
    Ok(())
}
