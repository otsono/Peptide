/// Generate the .fraytools project file matching the official template format
pub fn generate_fraytools_project(char_name: &str) -> String {
    format!(r##"{{
  "autoHurtboxPrefix": null,
  "autoKeyframeOffsetX": 0,
  "autoKeyframeOffsetY": 0,
  "defaultCollisionBodyLayerAlpha": 0.5,
  "defaultCollisionBodyLayerColor": "0xffa500",
  "defaultCollisionBodyLayerFoot": 0,
  "defaultCollisionBodyLayerHead": 86,
  "defaultCollisionBodyLayerHipWidth": 29,
  "defaultCollisionBodyLayerHipXOffset": 0,
  "defaultCollisionBodyLayerHipYOffset": 0,
  "frame_rate": 60,
  "paletteShaderMode": "RG_MAP",
  "pluginMetadata": {{
    "com.fraymakers.FraymakersMetadata": {{
      "activeCollisionBoxLayerPreset": null,
      "collisionBodyLayerPresets": [],
      "collisionBoxLayerPresets": [{{
        "hitboxAlpha": 0.5,
        "hitboxColor": "#ff0000",
        "hurtboxAlpha": 0.5,
        "hurtboxColor": "#f5e042",
        "grabboxAlpha": 0.5,
        "grabboxColor": "#ff00ff",
        "counterboxAlpha": 0.5,
        "counterboxColor": "#42ecff",
        "reflectboxAlpha": 0.5,
        "reflectboxColor": "#48f748",
        "ledgegrabboxAlpha": 0.5,
        "ledgegrabboxColor": "#bababa",
        "holdboxAlpha": 0.5,
        "holdboxColor": "#8c00ff",
        "absorbboxAlpha": 0.5,
        "absorbboxColor": "#d1d1d1",
        "customboxaAlpha": 0.5,
        "customboxaColor": "#d1d1d1",
        "customboxbAlpha": 0.5,
        "customboxbColor": "#d1d1d1",
        "customboxcAlpha": 0.5,
        "customboxcColor": "#d1d1d1",
        "id": "default-preset",
        "name": "default"
      }}],
      "version": "0.1.0"
    }}
  }},
  "plugins": [
    "com.fraymakers.ContentExporter",
    "com.fraymakers.FraymakersMetadata",
    "com.fraymakers.FraymakersTypes"
  ],
  "publishFolders": [{{
    "id": "build0",
    "path": "./build"
  }}],
  "snapToPixel": true,
  "templateDescription": "{name} — converted from Super Smash Flash 2",
  "templateName": "SSF2 {name} Character",
  "templateVersion": "0.3.0",
  "version": 12
}}"##, name = char_name)
}
