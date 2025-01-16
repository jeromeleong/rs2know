use crate::models::ProjectAnalysis;
use anyhow::{anyhow, Result};
use git2::{BranchType, ObjectType, Repository};
use std::collections::HashSet;
use std::path::Path;
use tracing::info;

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
        (None, _) => true,       // 沒有 Git，需要重新分析
        (Some(_), None) => true, // 有 Git 但沒有分析結果
        (Some(current), Some(analysis)) => {
            // 檢查當前版本是否已經分析過
            if let Some(analyzed) = analysis.analyzed_versions {
                !analyzed.contains(&current.to_string())
            } else {
                // 如果沒有 analyzed_versions，回退到舊的檢查邏輯
                match &analysis.git_version {
                    None => true,
                    Some(stored) => stored != current,
                }
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

/// 切換到指定的 Git commit 進行分析，並返回原始分支名稱
fn switch_to_commit(repo: &Repository, commit_id: &str) -> Result<String> {
    // 獲取當前 HEAD 引用
    let head = repo.head()?;
    let original_branch = head
        .shorthand()
        .ok_or_else(|| anyhow!("無法獲取當前分支名稱"))?
        .to_string();

    // 找到目標 commit
    let obj = repo.revparse_single(commit_id)?;
    let commit = obj.peel_to_commit()?;

    // 創建並切換到臨時分支
    let branch_name = format!("temp-analysis-{}", commit_id);
    repo.branch(&branch_name, &commit, false)?;
    let treeish = repo.revparse_single(&branch_name)?;
    repo.checkout_tree(&treeish, None)?;
    repo.set_head(&format!("refs/heads/{}", branch_name))?;

    Ok(original_branch)
}

/// 清理臨時分析分支，並切換回原始分支
fn cleanup_analysis_branch(
    repo: &Repository,
    commit_id: &str,
    original_branch: &str,
) -> Result<()> {
    let branch_name = format!("temp-analysis-{}", commit_id);

    // 切換回原始分支
    let obj = repo.revparse_single(&format!("refs/heads/{}", original_branch))?;
    repo.checkout_tree(&obj, None)?;
    repo.set_head(&format!("refs/heads/{}", original_branch))?;

    // 刪除臨時分支
    let mut branch = repo.find_branch(&branch_name, BranchType::Local)?;
    branch.delete()?;

    Ok(())
}

/// 更新報告，處理 Git 歷史記錄
pub async fn update_report(project_path: &Path, args: &crate::Args) -> Result<()> {
    let mut config = crate::config::get_effective_config(project_path)?;
    let history = get_git_history(project_path)?;

    // 獲取已分析的版本
    let mut analyzed_versions = Vec::new();
    if let Some(output) = &config.output {
        if let Ok(analysis) = serde_json::from_value::<ProjectAnalysis>(output.clone()) {
            if let Some(versions) = analysis.analyzed_versions {
                analyzed_versions.extend(versions);
            }
            if let Some(version) = analysis.git_version {
                if !analyzed_versions.contains(&version) {
                    analyzed_versions.push(version);
                }
            }
        }
    }

    // 檢查版本連續性
    if !check_version_continuity(&analyzed_versions, &history) {
        info!("檢測到版本不連續，需要重新分析");
        analyzed_versions.clear();
        crate::handle_default_analysis(args, project_path).await?;

        // 更新分析結果，加入當前版本到已分析列表
        if let Some(current_version) = get_git_version(project_path)? {
            if let Some(output) = &config.output {
                if let Ok(mut analysis) = serde_json::from_value::<ProjectAnalysis>(output.clone())
                {
                    analysis.analyzed_versions = Some(vec![current_version.clone()]);
                    config.output = Some(serde_json::json!(analysis));
                    config.save(project_path)?;
                }
            }
        }
        return Ok(());
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
    let repo = Repository::open(project_path)?;
    for version in versions_to_analyze {
        info!("分析版本：{}", version);

        // 切換到目標版本，並獲取原始分支名稱
        let original_branch = switch_to_commit(&repo, &version)?;

        // 分析當前版本
        let result = crate::handle_default_analysis(args, project_path).await;

        // 清理臨時分支，並切換回原始分支
        cleanup_analysis_branch(&repo, &version, &original_branch)?;

        // 檢查分析結果
        result?;

        // 更新分析結果，加入新版本到已分析列表
        if let Some(output) = &config.output {
            if let Ok(mut analysis) = serde_json::from_value::<ProjectAnalysis>(output.clone()) {
                if analysis.analyzed_versions.is_none() {
                    analysis.analyzed_versions = Some(Vec::new());
                }
                analysis
                    .analyzed_versions
                    .as_mut()
                    .unwrap()
                    .push(version.clone());
                config.output = Some(serde_json::json!(analysis));
                config.save(project_path)?;
            }
        }
    }

    Ok(())
}
