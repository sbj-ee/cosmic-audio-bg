use cosmic_audio_bg_config::{Config, VisualizationMode};

#[test]
fn config_roundtrips_through_ron_pretty() {
    let mut cfg = Config::default();
    cfg.visualization = VisualizationMode::Composite;
    let text = ron::ser::to_string_pretty(&cfg, ron::ser::PrettyConfig::new()).unwrap();
    let parsed: Config = ron::from_str(&text).unwrap();
    assert_eq!(parsed.visualization, VisualizationMode::Composite);

    cfg.visualization = VisualizationMode::Stripes;
    let text = ron::ser::to_string_pretty(&cfg, ron::ser::PrettyConfig::new()).unwrap();
    let parsed: Config = ron::from_str(&text).unwrap();
    assert_eq!(parsed.visualization, VisualizationMode::Stripes);
}
