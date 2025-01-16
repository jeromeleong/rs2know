use anyhow::{anyhow, Result};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{warn, error, info};
use crate::models::{ProjectAnalysis, ProjectSummary, FileAnalysis};
use serde_json;

pub const MAX_RETRIES: u32 = 5;
pub const RETRY_DELAY_MS: u64 = 1000;

/// Generic retry mechanism for async operations
pub async fn retry<F, T, E>(mut f: F) -> Result<T>
where
    F: FnMut() -> tokio::task::JoinHandle<Result<T, E>>,
    E: std::fmt::Display,
{
    for attempt in 1..=MAX_RETRIES {
        match f().await {
            Ok(result) => match result {
                Ok(value) => return Ok(value),
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        warn!("Attempt {} failed: {}. Retrying...", attempt, e);
                        sleep(Duration::from_millis(RETRY_DELAY_MS * attempt as u64)).await;
                        continue;
                    } else {
                        error!("All {} attempts failed: {}", MAX_RETRIES, e);
                        return Err(anyhow!("Operation failed after {} attempts: {}", MAX_RETRIES, e));
                    }
                }
            },
            Err(e) => {
                if attempt < MAX_RETRIES {
                    warn!("Join error on attempt {}: {}. Retrying...", attempt, e);
                    sleep(Duration::from_millis(RETRY_DELAY_MS * attempt as u64)).await;
                    continue;
                } else {
                    error!("Join error after {} attempts: {}", MAX_RETRIES, e);
                    return Err(anyhow!("Join error after {} attempts: {}", MAX_RETRIES, e));
                }
            }
        }
    }
    Err(anyhow!("Retry mechanism failed"))
}

/// Generate a default project summary
pub fn create_default_summary(analyses: &[FileAnalysis]) -> ProjectSummary {
    ProjectSummary {
        total_files: analyses.len(),
        total_loc: analyses.iter().map(|a| a.loc).sum(),
        main_features: vec![],
        code_architecture: String::new(),
        key_components: vec![],
        tech_stack: vec![],
        recommendations: vec![],
    }
}

/// Create a project analysis with the given summary and analyses
pub fn create_project_analysis(
    analyses: Vec<FileAnalysis>,
    summary: Option<ProjectSummary>,
) -> ProjectAnalysis {
    ProjectAnalysis {
        summary: summary.unwrap_or_else(|| create_default_summary(&analyses)),
        file_analyses: analyses,
        git_version: None,
        analyzed_versions: None,
    }
}

/// Save analysis results to JSON file if json flag is set
pub fn save_json_report(analysis: &ProjectAnalysis, json_flag: bool, output: &Option<String>) -> Result<()> {
    if json_flag {
        let json_report = serde_json::to_string_pretty(&analysis)?;
        let output_path = output
            .as_deref()
            .unwrap_or("analysis_report.json");
        std::fs::write(output_path, &json_report)?;
        info!("JSON report generated: {}", output_path);
    }
    Ok(())
}

/// Save analysis results to config file
pub fn save_to_config(
    project_path: &std::path::Path,
    analysis: &ProjectAnalysis,
    input: &Option<String>,
    output: &Option<String>,
) -> Result<()> {
    if input.is_none() {
        let mut config = crate::config::get_effective_config(project_path)?;
        config.generated = Some(serde_json::to_value(analysis)?);
        // Update output path in config
        if let Some(out) = output {
            config.output = Some(out.clone());
        }
        config.save(project_path)?;
        info!("Analysis results saved to config file");
    }
    Ok(())
}
