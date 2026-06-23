use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_shader")]
    pub shader_path: PathBuf,
    #[serde(default = "default_fps_ac")]
    pub fps_ac: u32,
    #[serde(default = "default_fps_battery")]
    pub fps_battery: u32,
    #[serde(default = "default_audio_sensitivity")]
    pub audio_sensitivity: f32,
    #[serde(default = "default_idle_threshold")]
    pub idle_energy_threshold: f32,
    #[serde(default = "default_idle_seconds")]
    pub idle_seconds: f32,
    #[serde(default)]
    pub output: OutputMode,
    #[serde(default)]
    pub visualization: VisualizationMode,
    #[serde(default)]
    pub power: PowerConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    All,
    Named(String),
}

impl Default for OutputMode {
    fn default() -> Self {
        Self::All
    }
}

/// How the audio spectrum is drawn on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualizationMode {
    /// 16 horizontal stripes, one continuous sinusoid per FFT band (default).
    Stripes,
    /// A single composite wave: the superposition (sum) of all 16 band
    /// sinusoids rendered as one glowing green line across the screen.
    Composite,
}

impl Default for VisualizationMode {
    fn default() -> Self {
        Self::Stripes
    }
}

impl VisualizationMode {
    /// Flag passed to the shader via the Uniforms `mode` field.
    pub fn shader_flag(self) -> u32 {
        match self {
            VisualizationMode::Stripes => 0,
            VisualizationMode::Composite => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerConfig {
    #[serde(default = "default_true")]
    pub reduce_fps_on_battery: bool,
    #[serde(default = "default_true")]
    pub pause_on_lid_closed: bool,
}

impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            reduce_fps_on_battery: true,
            pause_on_lid_closed: true,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shader_path: default_shader(),
            fps_ac: default_fps_ac(),
            fps_battery: default_fps_battery(),
            audio_sensitivity: default_audio_sensitivity(),
            idle_energy_threshold: default_idle_threshold(),
            idle_seconds: default_idle_seconds(),
            output: OutputMode::default(),
            visualization: VisualizationMode::default(),
            power: PowerConfig::default(),
        }
    }
}

fn default_shader() -> PathBuf {
    PathBuf::from("/usr/share/cosmic-audio-bg/shaders/sinusoids.wgsl")
}

fn default_fps_ac() -> u32 {
    60
}

fn default_fps_battery() -> u32 {
    30
}

fn default_audio_sensitivity() -> f32 {
    1.0
}

fn default_idle_threshold() -> f32 {
    0.02
}

fn default_idle_seconds() -> f32 {
    3.0
}

fn default_true() -> bool {
    true
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut config: Config = ron::from_str(&text)?;
        config.apply_env_overrides();
        Ok(config)
    }

    pub fn load_with_machine_override(base: &Path, hostname: &str) -> anyhow::Result<Self> {
        let mut config = if base.exists() {
            Self::load(base)?
        } else {
            Config::default()
        };

        let machine_path = base
            .parent()
            .map(|p| p.join("machines").join(format!("{hostname}.ron")))
            .unwrap_or_else(|| PathBuf::from(format!("config/machines/{hostname}.ron")));

        if machine_path.exists() {
            let machine: MachineOverride = ron::from_str(&fs::read_to_string(&machine_path)?)?;
            config.merge(machine);
        }

        config.apply_env_overrides();
        Ok(config)
    }

    fn merge(&mut self, other: MachineOverride) {
        if let Some(path) = other.shader_path {
            self.shader_path = path;
        }
        if let Some(fps) = other.fps_ac {
            self.fps_ac = fps;
        }
        if let Some(fps) = other.fps_battery {
            self.fps_battery = fps;
        }
        if let Some(s) = other.audio_sensitivity {
            self.audio_sensitivity = s;
        }
        if let Some(output) = other.output {
            self.output = output;
        }
        if let Some(visualization) = other.visualization {
            self.visualization = visualization;
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(path) = std::env::var("COSMIC_AUDIO_BG_SHADER") {
            self.shader_path = PathBuf::from(path);
        }
        if let Ok(fps) = std::env::var("COSMIC_AUDIO_BG_FPS") {
            if let Ok(fps) = fps.parse() {
                self.fps_ac = fps;
            }
        }
    }

    pub fn effective_fps(&self, on_battery: bool) -> u32 {
        if on_battery && self.power.reduce_fps_on_battery {
            self.fps_battery.clamp(1, 240)
        } else {
            self.fps_ac.clamp(1, 240)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineOverride {
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub shader_path: Option<PathBuf>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub fps_ac: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub fps_battery: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub audio_sensitivity: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub output: Option<OutputMode>,
    // Enums don't round-trip through the untagged `deserialize_optional_field`
    // helper (a bare RON variant silently parses as `None`), so parse the bare
    // variant directly and wrap it in `Some`.
    #[serde(default, deserialize_with = "deserialize_optional_value")]
    pub visualization: Option<VisualizationMode>,
}

fn deserialize_optional_value<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn deserialize_optional_field<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Help<T> {
        Value(T),
        Option(Option<T>),
    }

    match Help::<T>::deserialize(deserializer)? {
        Help::Value(value) => Ok(Some(value)),
        Help::Option(option) => Ok(option),
    }
}

pub fn default_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("COSMIC_AUDIO_BG_CONFIG") {
        return PathBuf::from(path);
    }

    if let Some(config_dir) = dirs::config_dir() {
        return config_dir.join("cosmic-audio-bg/config.ron");
    }

    PathBuf::from("config/default.ron")
}

pub fn project_root() -> PathBuf {
    std::env::var("COSMIC_AUDIO_BG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn machine_override_accepts_bare_values() {
        let machine: MachineOverride = ron::from_str(
            r#"(
                shader_path: "/tmp/shader.wgsl",
                fps_ac: 45,
            )"#,
        )
        .expect("machine override should parse");

        assert_eq!(
            machine.shader_path,
            Some(PathBuf::from("/tmp/shader.wgsl"))
        );
        assert_eq!(machine.fps_ac, Some(45));
        assert_eq!(machine.output, None);
        assert_eq!(machine.visualization, None);
    }

    #[test]
    fn visualization_defaults_to_stripes() {
        let config: Config = ron::from_str("()").expect("empty config should parse");
        assert_eq!(config.visualization, VisualizationMode::Stripes);
        assert_eq!(config.visualization.shader_flag(), 0);
    }

    #[test]
    fn visualization_parses_composite() {
        let config: Config = ron::from_str(
            r#"(
                visualization: composite,
            )"#,
        )
        .expect("config with visualization should parse");
        assert_eq!(config.visualization, VisualizationMode::Composite);
        assert_eq!(config.visualization.shader_flag(), 1);
    }

    #[test]
    fn machine_override_accepts_visualization() {
        let machine: MachineOverride = ron::from_str(
            r#"(
                visualization: composite,
            )"#,
        )
        .expect("machine override should parse");
        assert_eq!(machine.visualization, Some(VisualizationMode::Composite));
    }
}
