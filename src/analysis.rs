use crate::models::ProjectAnalysis;
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::info;

#[derive(Debug)]
pub struct FileStats {
    pub loc: usize,
    pub blank_lines: usize,
    pub comment_lines: usize,
    pub code_lines: usize,
    pub code_hash: String,
}

/// Analyze code content and generate stats including a hash of the code
pub fn analyze_code(content: &str) -> FileStats {
    let mut stats = FileStats {
        loc: 0,
        blank_lines: 0,
        comment_lines: 0,
        code_lines: 0,
        code_hash: String::new(),
    };
    
    let mut code_content = String::new();
    
    for line in content.lines() {
        let line = line.trim();
        stats.loc += 1;
        
        if line.is_empty() {
            stats.blank_lines += 1;
        } else if line.starts_with("//") || line.starts_with("/*") || line.starts_with("*") {
            stats.comment_lines += 1;
        } else {
            stats.code_lines += 1;
            code_content.push_str(line);
            code_content.push('\n');
        }
    }
    
    // Calculate hash of code content
    let mut hasher = Sha256::new();
    hasher.update(code_content.as_bytes());
    let result = hasher.finalize();
    stats.code_hash = format!("{:x}", result);
    
    stats
}

/// Update the analysis report for the project
pub async fn update_report(
    project_path: &Path,
    args: &crate::Args,
    input: &Option<String>,
    api_url: &Option<String>,
    api_key: &Option<String>,
    model: &Option<String>,
    keep: bool,
) -> Result<()> {
    info!("Starting project analysis update");
    
    // Create a modified Args with overridden parameters
    let mut modified_args = args.clone();
    
    // First try to get credentials from command line arguments
    if let Some(url) = api_url {
        modified_args.api_url = Some(url.clone());
    }
    if let Some(key) = api_key {
        modified_args.api_key = Some(key.clone());
    }
    if let Some(m) = model {
        modified_args.model = Some(m.clone());
    }
    
    // Load previous analysis and config
    let mut previous_analysis = None;
    if let Some(input_path) = input {
        if let Ok(content) = std::fs::read_to_string(input_path) {
            if let Ok(analysis) = serde_json::from_str::<ProjectAnalysis>(&content) {
                previous_analysis = Some(analysis);
                info!("Loaded previous analysis from {}", input_path);
            }
        }
    } else if project_path.join(".pj.yml").exists() {
        let config = crate::config::get_effective_config(project_path)?;
        previous_analysis = config
            .generated
            .as_ref()
            .and_then(|v| serde_json::from_value::<ProjectAnalysis>(v.clone()).ok());
            
        // If API credentials not provided in command line, use from config
        if modified_args.api_url.is_none() {
            modified_args.api_url = Some(config.api_url);
        }
        if modified_args.api_key.is_none() {
            modified_args.api_key = Some(config.api_key);
        }
        if modified_args.model.is_none() {
            modified_args.model = Some(config.model);
        }
        // If output path not provided in command line, use from config
        if modified_args.output.is_none() {
            modified_args.output = config.output;
        }
    }
    
    // Perform analysis
    let (analyses, project_summary) = crate::perform_analysis(&modified_args, project_path).await?;
    
    // If keep flag is set, preserve existing code_hash, ai_analysis, and project summary
    let mut preserved_analyses = analyses.clone();
    let mut preserved_summary = project_summary.clone();
    
    if keep {
        if let Some(prev) = &previous_analysis {
            preserved_summary = Some(prev.summary.clone());
            for analysis in &mut preserved_analyses {
                if let Some(prev_file) = prev.file_analyses.iter().find(|f| f.file_path == analysis.file_path) {
                    analysis.code_hash = prev_file.code_hash.clone();
                    analysis.ai_analysis = prev_file.ai_analysis.clone();
                }
            }
        }
    }
    
    // Create preserved project analysis for config/json
    let preserved_project_analysis = crate::utils::create_project_analysis(preserved_analyses, preserved_summary);
    
    // Save to config if not using --input
    crate::utils::save_to_config(project_path, &preserved_project_analysis, input, &args.output)?;
    
    if args.json {
        // Generate JSON report only
        crate::utils::save_json_report(&preserved_project_analysis, true, &args.output)?;
        info!("Generated JSON report only");
    } else {
        // Generate fresh markdown report with new analysis
        let fresh_project_analysis = crate::utils::create_project_analysis(analyses, project_summary);

        let output_path = args.output
            .clone()
            .unwrap_or_else(|| "analysis_report.md".to_string());

        crate::markdown::generate_markdown_report(
            Some(fresh_project_analysis.file_analyses),
            Some(fresh_project_analysis.summary),
            &output_path,
            &args.output,
        )
        .await?;
        info!("Generated markdown report only: {}", output_path);
    }
    
    info!("Analysis update completed successfully");
    Ok(())
}
