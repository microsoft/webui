use std::path::{Path, PathBuf};

/// Resolved paths for a WebUI app directory.
///
/// An app directory is expected to contain:
///   - `templates/index.html`
///   - `data/state.json`
///   - `assets/` (static files served under `/assets/*`)
#[derive(Clone)]
pub struct AppPaths {
    pub template: PathBuf,
    pub data: PathBuf,
    pub assets_dir: PathBuf,
    pub dist_dir: PathBuf,
}

impl AppPaths {
    /// Build paths from an app root directory (e.g. `../../app/hello-world`).
    pub fn from_app_dir(app_dir: &Path) -> Self {
        Self {
            template: app_dir.join("templates").join("index.html"),
            data: app_dir.join("data").join("state.json"),
            assets_dir: app_dir.join("assets"),
            dist_dir: PathBuf::from("dist"),
        }
    }

    /// Directories that should be watched for file changes.
    pub fn watch_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.template
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf(),
            self.data.parent().unwrap_or(Path::new(".")).to_path_buf(),
            self.assets_dir.clone(),
        ]
    }
}
