use std::path::Path;
use anyhow::Result;
use crate::models::{CodeStats, ProjectAnalysis};
use git2::Repository;
use std::collections::HashSet;
use tracing::info;

pub fn analyze_code(content: &str) -> CodeStats {
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

pub async fn update_report(report_path: &str, project_path: &str, args: &crate::Args) -> Result<()> {
    // 讀取現有的報告
    let report_content = std::fs::read_to_string(report_path)?;
    let mut project_analysis: ProjectAnalysis = serde_json::from_str(&report_content)?;
    
    // 取得 Git 倉庫
    let repo = Repository::open(project_path)?;
    let mut options = git2::StatusOptions::new();
    options.include_untracked(true);
    
    // 取得修改過的檔案
    let statuses = repo.statuses(Some(&mut options))?;
    let mut modified_files = HashSet::new();
    for entry in statuses.iter() {
        if let Some(path) = entry.path() {
            modified_files.insert(path.to_string());
        }
    }
    
    // 重新分析修改過的檔案
    let project_path = Path::new(project_path);
    for analysis in &mut project_analysis.file_analyses {
        if modified_files.contains(&analysis.file_path) {
            let file_path = project_path.join(&analysis.file_path);
            info!("重新分析檔案：{}", file_path.display());
            
            let code_str = std::fs::read_to_string(&file_path)?;
            let stats = analyze_code(&code_str);
            
            analysis.loc = stats.loc;
            analysis.blank_lines = stats.blank_lines;
            analysis.comment_lines = stats.comment_lines;
            analysis.code_lines = stats.code_lines;
            
            // 如果有 API key，重新進行 AI 分析
            if !args.skip_ai {
                let config = crate::config::get_effective_config(project_path)?;
                let api_url = args.api_url.as_deref().unwrap_or(&config.api_url);
                let api_key = args.api_key.as_deref().unwrap_or(&config.api_key);
                let model = args.model.as_deref().unwrap_or(&config.model);

                if !api_key.is_empty() {
                    info!("開始對檔案進行 AI 分析：{}", analysis.file_path);
                    analysis.ai_analysis = match crate::openai::do_ai_analysis_with_retry(
                        api_url,
                        api_key,
                        model,
                        &code_str,
                        &analysis.file_path,
                    ).await {
                        Ok(Some(ai_result)) => {
                            info!("AI 分析成功：{}", analysis.file_path);
                            Some(ai_result)
                        }
                        Ok(None) => None,
                        Err(e) => {
                            tracing::error!("AI 分析錯誤：{} - {}", analysis.file_path, e);
                            None
                        }
                    };
                }
            }
        }
    }
    
    // 如果有 API key，重新生成專案總結
    if !args.skip_ai {
        let config = crate::config::get_effective_config(project_path)?;
        let api_url = args.api_url.as_deref().unwrap_or(&config.api_url);
        let api_key = args.api_key.as_deref().unwrap_or(&config.api_key);
        let model = args.model.as_deref().unwrap_or(&config.model);

        if !api_key.is_empty() {
            match crate::openai::generate_project_summary_with_retry(
                &project_analysis.file_analyses,
                api_url,
                api_key,
                model
            ).await {
                Ok(Some(summary)) => {
                    project_analysis.summary = summary;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::error!("專案總結錯誤：{}", e);
                }
            }
        }
    }
    
    // 寫入更新後的報告
    let json_report = serde_json::to_string_pretty(&project_analysis)?;
    std::fs::write(report_path, &json_report)?;
    info!("報告已更新：{}", report_path);
    
    Ok(())
}
