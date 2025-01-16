use anyhow::{anyhow, Result};
use dialoguer::{theme::ColorfulTheme, Input, Select};
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs;
use std::path::Path;
use tracing::{info, warn};
const CONFIG_FILE: &str = ".pj.yml";
const GLOBAL_CONFIG_DIR: &str = ".config/pj";
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub generated: Option<serde_json::Value>,
    #[serde(skip)]
    pub last_analysis: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            api_url: "https://api.openai.com/v1/".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            generated: None,
            last_analysis: None,
            output: None,
        }
    }
}
impl Config {
    pub fn load(project_dir: &Path) -> Result<Self> {
        let config_path = project_dir.join(CONFIG_FILE);
        if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            Ok(serde_yaml::from_str(&content)?)
        } else {
            // Try to load from global config
            if let Some(global_config) = Self::load_global()? {
                Ok(global_config)
            } else {
                Ok(Self::default())
            }
        }
    }
    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let config_path = project_dir.join(CONFIG_FILE);
        let content = serde_yaml::to_string(self)?;
        fs::write(&config_path, content)?;
        Ok(())
    }
    pub fn load_global() -> Result<Option<Self>> {
        if let Some(home) = dirs::home_dir() {
            let global_config_path = home.join(GLOBAL_CONFIG_DIR).join(CONFIG_FILE);
            if global_config_path.exists() {
                let content = fs::read_to_string(&global_config_path)?;
                Ok(Some(serde_yaml::from_str(&content)?))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
    pub fn save_global(&self) -> Result<()> {
        if let Some(home) = dirs::home_dir() {
            let global_config_dir = home.join(GLOBAL_CONFIG_DIR);
            fs::create_dir_all(&global_config_dir)?;
            let global_config_path = global_config_dir.join(CONFIG_FILE);
            let content = serde_yaml::to_string(self)?;
            fs::write(&global_config_path, content)?;
            Ok(())
        } else {
            Err(anyhow!("無法找到使用者主目錄"))
        }
    }
}
pub async fn configure_interactive(project_dir: &Path, global: bool) -> Result<()> {
    let theme = ColorfulTheme::default();
    let current_config = if global {
        Config::load_global()?.unwrap_or_default()
    } else {
        Config::load(project_dir)?
    };
    println!("\n🔧 Project Helper 配置設定");
    println!("==================");
    if global {
        println!("正在設定全局配置\n");
    } else {
        println!("正在設定專案配置\n");
    }
    // API URL
    let api_url: String = Input::with_theme(&theme)
        .with_prompt("API URL")
        .with_initial_text(&current_config.api_url)
        .interact_text()?;
    // API Key
    let api_key: String = Input::with_theme(&theme)
        .with_prompt("API Key")
        .with_initial_text(&current_config.api_key)
        .interact_text()?;
    // Model selection
    let models = match crate::openai::get_available_models(&api_url, &api_key).await {
        Ok(models) => {
            info!("成功獲取可用模型列表");
            models
        }
        Err(e) => {
            warn!("無法獲取模型列表：{}，使用預設列表", e);
            vec![
                "gpt-4".to_string(),
                "gpt-4o-mini".to_string(),
                "gpt-3.5-turbo".to_string(),
            ]
        }
    };
    let default_index = models
        .iter()
        .position(|m| m == &current_config.model)
        .unwrap_or(0);
    let model_index = Select::with_theme(&theme)
        .with_prompt("選擇模型")
        .default(default_index)
        .items(&models)
        .interact()?;
    let new_config = Config {
        api_url,
        api_key,
        model: models[model_index].clone(),
        generated: current_config.generated,
        last_analysis: current_config.last_analysis,
        output: current_config.output,
    };
    if global {
        new_config.save_global()?;
        info!("已更新全局配置");
    } else {
        new_config.save(project_dir)?;
        info!("已更新專案配置");
    }
    Ok(())
}
pub fn init_project(project_dir: &Path) -> Result<()> {
    // 檢查是否已經存在配置文件
    let config_path = project_dir.join(CONFIG_FILE);
    if config_path.exists() {
        return Err(anyhow!("配置文件已存在：{}", config_path.display()));
    }
    // 創建默認配置
    let config = Config::default();
    config.save(project_dir)?;
    info!("已創建配置文件：{}", config_path.display());
    // 檢查並更新 .gitignore
    let gitignore_path = project_dir.join(".gitignore");
    if gitignore_path.exists() {
        let mut content = fs::read_to_string(&gitignore_path)?;
        if !content.contains(CONFIG_FILE) {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(CONFIG_FILE);
            content.push('\n');
            fs::write(&gitignore_path, content)?;
            info!("已將 {} 添加到 .gitignore", CONFIG_FILE);
        }
    } else {
        fs::write(&gitignore_path, format!("{}\n", CONFIG_FILE))?;
        info!("已創建 .gitignore 並添加 {}", CONFIG_FILE);
    }
    Ok(())
}
pub fn update_config(project_dir: &Path, updates: Config, global: bool) -> Result<()> {
    if global {
        updates.save_global()?;
        info!("已更新全局配置");
    } else {
        let config_path = project_dir.join(CONFIG_FILE);
        if !config_path.exists() {
            return Err(anyhow!("配置文件不存在，請先執行 pj init"));
        }
        updates.save(project_dir)?;
        info!("已更新項目配置");
    }
    Ok(())
}
pub fn get_effective_config(project_dir: &Path) -> Result<Config> {
    Config::load(project_dir)
}
