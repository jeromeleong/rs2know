use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]

pub struct ProjectAnalysis {
    pub summary: ProjectSummary,

    pub file_analyses: Vec<FileAnalysis>,

    #[serde(default)]
    pub git_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]

pub struct ProjectSummary {
    pub total_files: usize,

    pub total_loc: usize,

    pub main_features: Vec<String>,

    pub code_architecture: String,

    pub key_components: Vec<String>,

    pub tech_stack: Vec<String>,

    pub recommendations: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]

pub struct FileAnalysis {
    pub file_path: String,

    pub loc: usize,

    pub blank_lines: usize,

    pub comment_lines: usize,

    pub code_lines: usize,

    pub ai_analysis: Option<AIAnalysis>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]

pub struct AIAnalysis {
    pub main_functions: Vec<String>,

    pub core_structs: Vec<StructInfo>,

    pub error_types: Vec<String>,

    pub functions_details: Vec<FunctionDetail>,

    pub code_complexity: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]

pub struct StructInfo {
    pub name: String,

    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]

pub struct FunctionDetail {
    pub name: String,

    pub description: String,

    pub parameters: Vec<String>,

    pub return_type: String,
}
