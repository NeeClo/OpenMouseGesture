// 概要: gestures.jsonとconfig.jsonの読み書きを管理し、設定の永続化を担当
// 入出力:
//   - 入力: ファイルパス（デバッグ時はプロジェクトルート/config/、リリース時はexeと同ディレクトリ）
//   - 出力: Result<T, String>形式の設定データ（JSONデシリアライズ結果）

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ValidationError {
    InvalidFormat(String),
    MissingRequiredField(String),
    InvalidValue(String),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ValidationError::InvalidFormat(msg) => write!(f, "無効な形式: {}", msg),
            ValidationError::MissingRequiredField(field) => write!(f, "必須フィールドが不足: {}", field),
            ValidationError::InvalidValue(msg) => write!(f, "無効な値: {}", msg),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GestureTemplate {
    pub name: String,
    pub points: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub trigger_type: String,
    pub gesture: String,
    pub wheel_trigger: Option<String>,
    pub action_type: String,
    pub keystroke: Option<String>,
    pub modifiers: Option<Vec<String>>,
    pub command: Option<String>,
    pub url: Option<String>,
    pub operation: Option<String>,
    pub ignore_exe: Option<Vec<String>>,
}

impl Default for Action {
    fn default() -> Self {
        Self {
            name: String::new(),
            trigger_type: "gesture".to_string(),
            gesture: String::new(),
            wheel_trigger: None,
            action_type: String::new(),
            keystroke: None,
            modifiers: None,
            command: None,
            url: None,
            operation: None,
            ignore_exe: None,
        }
    }
}

impl Action {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_empty() {
            return Err(ValidationError::MissingRequiredField("name".to_string()));
        }
        
        if self.trigger_type != "gesture" && self.trigger_type != "wheel" {
            return Err(ValidationError::InvalidValue(
                format!("trigger_type は 'gesture' または 'wheel' である必要があります: {}", self.trigger_type)
            ));
        }
        
        if self.action_type != "keystroke" && self.action_type != "command" 
            && self.action_type != "url" && self.action_type != "window_operation" {
            return Err(ValidationError::InvalidValue(
                format!("action_type は 'keystroke', 'command', 'url', 'window_operation' のいずれかである必要があります: {}", self.action_type)
            ));
        }
        
        if self.trigger_type == "gesture" && self.gesture.is_empty() {
            return Err(ValidationError::MissingRequiredField(
                "trigger_type='gesture'の場合はgestureフィールドが必須です".to_string()
            ));
        }
        
        if self.trigger_type == "wheel" && self.wheel_trigger.as_ref().map_or(true, |s| s.is_empty()) {
            return Err(ValidationError::MissingRequiredField(
                "trigger_type='wheel'の場合はwheel_triggerフィールドが必須です".to_string()
            ));
        }
        
        Ok(())
    }
}

fn default_trajectory_color() -> String {
    "#228B22".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub trajectory: bool,
    #[serde(default = "default_trajectory_color")]
    pub trajectory_color: String,
    pub ignore_exe: Vec<String>,
    pub actions: Vec<Action>,
}

impl Config {
    fn validate(&self) -> Result<(), ValidationError> {
        if !is_valid_hex_color(&self.trajectory_color) {
            return Err(ValidationError::InvalidValue(format!(
                "trajectory_color は '#RRGGBB' 形式である必要があります: {}",
                self.trajectory_color
            )));
        }

        for (idx, action) in self.actions.iter().enumerate() {
            action.validate().map_err(|e| {
                ValidationError::InvalidValue(format!("actions[{}]: {}", idx, e))
            })?;
        }
        Ok(())
    }
}

fn is_valid_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(|b| b.is_ascii_hexdigit())
}

impl GestureTemplate {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_empty() {
            return Err(ValidationError::MissingRequiredField("name".to_string()));
        }
        
        if self.points.is_empty() {
            return Err(ValidationError::MissingRequiredField("points".to_string()));
        }
        
        Ok(())
    }
}

pub struct ConfigManager {
    config_dir: PathBuf,
}

impl ConfigManager {
    pub fn new() -> Result<Self, String> {
        let config_dir = if cfg!(debug_assertions) {
            let manifest_dir = env!("CARGO_MANIFEST_DIR");
            PathBuf::from(manifest_dir)
                .parent()
                .ok_or("Failed to get project root directory")?
                .join("config")
        } else {
            std::env::current_exe()
                .map_err(|e| format!("Failed to get executable path: {}", e))?
                .parent()
                .ok_or("Failed to get executable directory")?
                .to_path_buf()
        };

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        Ok(ConfigManager { config_dir })
    }

    pub fn load_gestures(&self) -> Result<Vec<GestureTemplate>, String> {
        let path = self.config_dir.join("gestures.json");

        if !path.exists() {
            let default_gestures = include_str!("../../config/default-gestures.json");
            let gestures: Vec<GestureTemplate> = serde_json::from_str(default_gestures)
                .map_err(|e| format!("Failed to parse default gestures: {}", e))?;
            self.save_gestures(&gestures)?;
            return Ok(gestures);
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read gestures.json: {}", e))?;

        let gestures: Vec<GestureTemplate> = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse gestures.json: {}", e))?;

        for (idx, gesture) in gestures.iter().enumerate() {
            if let Err(e) = gesture.validate() {
                eprintln!("[WARNING] gestures.json validation error at index {}: {}", idx, e);
                return Err(format!("gestures.jsonの検証エラー: インデックス{}: {}", idx, e));
            }
        }

        Ok(gestures)
    }

    pub fn save_gestures(&self, gestures: &[GestureTemplate]) -> Result<(), String> {
        // 保存前に検証
        for (idx, gesture) in gestures.iter().enumerate() {
            if let Err(e) = gesture.validate() {
                eprintln!("[ERROR] Cannot save gestures: validation error at index {}: {}", idx, e);
                return Err(format!("gestures.jsonの検証エラー: インデックス{}: {}", idx, e));
            }
        }
        
        let path = self.config_dir.join("gestures.json");
        let content = serde_json::to_string_pretty(gestures)
            .map_err(|e| format!("Failed to serialize gestures: {}", e))?;
        
        fs::write(&path, content)
            .map_err(|e| format!("Failed to write gestures.json: {}", e))?;

        Ok(())
    }

    pub fn load_config(&self) -> Result<Config, String> {
        let path = self.config_dir.join("config.json");

        if !path.exists() {
            let default_config = include_str!("../../config/default-config.json");
            let config: Config = serde_json::from_str(default_config)
                .map_err(|e| format!("Failed to parse default config: {}", e))?;
            self.save_config(&config)?;
            return Ok(config);
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read config.json: {}", e))?;

        let config: Config = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config.json: {}", e))?;

        if let Err(e) = config.validate() {
            eprintln!("[WARNING] config.json validation error: {}", e);
            return Err(format!("config.jsonの検証エラー: {}", e));
        }

        Ok(config)
    }

    pub fn save_config(&self, config: &Config) -> Result<(), String> {
        // 保存前に検証
        if let Err(e) = config.validate() {
            eprintln!("[ERROR] Cannot save config: validation error: {}", e);
            return Err(format!("config.jsonの検証エラー: {}", e));
        }
        
        let path = self.config_dir.join("config.json");
        let content = serde_json::to_string_pretty(config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        
        fs::write(&path, content)
            .map_err(|e| format!("Failed to write config.json: {}", e))?;

        Ok(())
    }

    pub fn config_dir(&self) -> &PathBuf {
        &self.config_dir
    }
}
