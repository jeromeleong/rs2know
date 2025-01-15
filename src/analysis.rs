use std::path::Path;

use anyhow::{anyhow, Result};

use crate::models::ProjectAnalysis;

use tracing::info;

use git2::Repository;

use std::collections::HashSet;

#[derive(Debug)]

pub struct FileStats {
    pub loc: usize,

    pub blank_lines: usize,

    pub comment_lines: usize,

    pub code_lines: usize,
}

/// 獲取 Git 倉庫的當前提交 hash

pub fn get_git_version(project_path: &Path) -> Result<Option<String>> {
    if !project_path.join(".git").exists() {
        return Ok(None);
    }

    let repo = Repository::open(project_path)?;

    let head = repo.head()?;

    let commit = head.peel_to_commit()?;

    Ok(Some(commit.id().to_string()))
}

/// 檢查是否需要重新分析

pub fn needs_reanalysis(
    current_version: Option<&str>,
    stored_analysis: Option<ProjectAnalysis>,
) -> bool {
    match (current_version, stored_analysis) {
        (None, _) => true, // 沒有 Git，需要重新分析

        (Some(_), None) => true, // 有 Git 但沒有分析結果

        (Some(current), Some(analysis)) => {
            // 當版本號為 null、沒有任何 output 內容、只存在當前版本號時，需要重新分析

            match &analysis.git_version {
                None => true,

                Some(stored) => stored != current,
            }
        }
    }
}

/// 獲取 Git 倉庫的所有提交歷史，按時間順序排序

pub fn get_git_history(project_path: &Path) -> Result<Vec<String>> {
    if !project_path.join(".git").exists() {
        return Ok(vec![]);
    }

    let repo = Repository::open(project_path)?;

    let mut revwalk = repo.revwalk()?;

    revwalk.push_head()?;

    revwalk.set_sorting(git2::Sort::TIME)?;

    let commits: Result<Vec<_>, _> = revwalk
        .map(|id| -> Result<String> {
            let id = id?;

            let commit = repo.find_commit(id)?;

            Ok(commit.id().to_string())
        })
        .collect();

    commits
}

/// 檢查版本之間的連續性

pub fn check_version_continuity(versions: &[String], history: &[String]) -> bool {
    if versions.is_empty() {
        return true;
    }

    let version_set: HashSet<_> = versions.iter().collect();

    let mut continuous = true;

    let mut found_first = false;

    for commit in history {
        if version_set.contains(commit) {
            if !found_first {
                found_first = true;
            } else if !continuous {
                return false;
            }
        } else if found_first {
            continuous = false;
        }
    }

    true
}

pub fn analyze_code(content: &str) -> FileStats {
    let mut stats = FileStats {
        loc: 0,

        blank_lines: 0,

        comment_lines: 0,

        code_lines: 0,
    };

    for line in content.lines() {
        let line = line.trim();

        stats.loc += 1;

        if line.is_empty() {
            stats.blank_lines += 1;
        } else if line.starts_with("//") || line.starts_with("/*") || line.starts_with("*") {
            stats.comment_lines += 1;
        } else {
            stats.code_lines += 1;
        }
    }

    stats
}

/// 更新報告，處理 Git 歷史記錄

pub async fn update_report(project_path: &Path, args: &crate::Args) -> Result<()> {
    let config = crate::config::get_effective_config(project_path)?;

    let history = get_git_history(project_path)?;

    // 獲取已分析的版本

    let mut analyzed_versions = Vec::new();

    if let Some(output) = &config.output {
        if let Ok(analysis) = serde_json::from_value::<ProjectAnalysis>(output.clone()) {
            if let Some(version) = analysis.git_version {
                analyzed_versions.push(version);
            }
        }
    }

    // 檢查版本連續性

    if !check_version_continuity(&analyzed_versions, &history) {
        info!("檢測到版本不連續，需要重新分析");

        return crate::handle_default_analysis(args, project_path).await;
    }

    // 獲取需要分析的版本

    let versions_to_analyze: Vec<_> = history
        .into_iter()
        .filter(|v| !analyzed_versions.contains(v))
        .collect();

    if versions_to_analyze.is_empty() {
        info!("所有版本已分析完成");

        return Ok(());
    }

    info!("發現 {} 個新版本需要分析", versions_to_analyze.len());

    // 按時間順序分析每個版本

    for version in versions_to_analyze {
        info!("分析版本：{}", version);

        // TODO: 切換到指定版本並分析

        // 目前先使用當前版本的分析結果

        crate::handle_default_analysis(args, project_path).await?;
    }

    Ok(())
}
