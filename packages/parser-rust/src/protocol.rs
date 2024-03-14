use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::collections::HashMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BuildTimeRenderingStream {
    Raw(BuildTimeRenderingStreamRaw),
    Repeat(BuildTimeRenderingStreamRepeat),
    Signal(BuildTimeRenderingStreamSignal),
    When(BuildTimeRenderingStreamWhen),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BuildTimeRenderingStreamRaw {
    pub value: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BuildTimeRenderingStreamRepeat {
    pub value: String,
    pub template: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BuildTimeRenderingStreamSignal {
    pub value: String,
    #[serde(rename = "defaultValue")]
    pub default_value: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BuildTimeRenderingStreamWhen {
    pub value: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BuildTimeRenderingTemplate {
    pub style: Option<String>,
    pub template: String,
}

pub type BuildTimeRenderingStreamTemplateRecords = HashMap<String, BuildTimeRenderingTemplate>;

#[derive(Clone, Serialize, Deserialize)]
pub struct BuildTimeRenderingProtocol {
    pub streams: Vec<BuildTimeRenderingStream>,
    pub templates: BuildTimeRenderingStreamTemplateRecords,
}

pub fn load_protocol_from_file(file_path: &str) -> Result<BuildTimeRenderingProtocol, serde_json::Error> {
    let file = File::open(file_path).unwrap();
    let reader = BufReader::new(file);
    let protocol = serde_json::from_reader(reader)?;
    Ok(protocol)
}