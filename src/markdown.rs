use crate::models::{FileAnalysis, ProjectAnalysis, ProjectSummary};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use tracing::info;
pub async fn generate_markdown_report(
    analyses: Option<Vec<FileAnalysis>>,
    project_summary: Option<ProjectSummary>,
    output_path: &str,
) -> Result<()> {
    let mut md_content = String::new();
    md_content.push_str("# Rust 程式碼分析報告\n\n");
    // Add project summary if available
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
    // Add file analyses if available
    if let Some(analyses) = analyses {
        // 按目錄組織文件
        let mut dir_files: HashMap<String, Vec<&FileAnalysis>> = HashMap::new();
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
                    md_content.push_str(&format!(" - [{}](#{})\n", file_name, safe_file));
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
                                md_content.push_str(&format!(
                                    "- **{}**：{}\n",
                                    struct_info.name, struct_info.description
                                ));
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
                                        md_content.push_str(&format!(" - {}\n", param));
                                    }
                                }
                                md_content.push_str(&format!("- 返回類型：{}\n", func.return_type));
                                md_content
                                    .push_str(&format!("- 複雜度：{}\n\n", ai.code_complexity));
                            }
                        }
                        md_content.push_str("#### 程式碼複雜度\n\n");
                        md_content.push_str(&format!("{}\n\n", ai.code_complexity));
                    }
                    md_content.push_str("---\n\n");
                }
            }
        }
    }
    // Write to file
    std::fs::write(output_path, md_content)?;
    info!("Markdown 報告已生成並寫入 {}", output_path);
    Ok(())
}
pub async fn generate_md_from_json(report_path: &str, output_path: Option<&str>) -> Result<()> {
    // 讀取 JSON 報告
    let report_content = std::fs::read_to_string(report_path)?;
    let project_analysis: ProjectAnalysis = serde_json::from_str(&report_content)?;
    // 決定輸出路徑
    let output = match output_path {
        Some(path) => path.to_string(),
        None => "analysis_report.md".to_string(),
    };
    generate_markdown_report(
        Some(project_analysis.file_analyses),
        Some(project_analysis.summary),
        &output,
    )
    .await
}
