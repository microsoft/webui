use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::render::render_to_index_html;

fn collect_file_times(dir_path: &Path) -> HashMap<PathBuf, SystemTime> {
    let mut file_times = HashMap::new();
    
    if let Ok(entries) = fs::read_dir(dir_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(metadata) = fs::metadata(&path) {
                    if let Ok(modified) = metadata.modified() {
                        file_times.insert(path, modified);
                    }
                }
            } else if path.is_dir() {
                // Recursively collect file times from subdirectories
                file_times.extend(collect_file_times(&path));
            }
        }
    }
    
    file_times
}

pub fn start_file_watcher() {
    thread::spawn(move || {
        let watch_dirs = ["src/templates", "src/data", "static"];
        let mut last_file_times: HashMap<PathBuf, SystemTime> = HashMap::new();

        // Initialize file times
        for dir in &watch_dirs {
            if Path::new(dir).exists() {
                last_file_times.extend(collect_file_times(Path::new(dir)));
            }
        }

        loop {
            let mut current_file_times: HashMap<PathBuf, SystemTime> = HashMap::new();
            
            // Collect current file modification times
            for dir in &watch_dirs {
                if Path::new(dir).exists() {
                    current_file_times.extend(collect_file_times(Path::new(dir)));
                }
            }

            // Check for changes
            let mut files_changed = false;
            
            // Check for modified or new files
            for (path, current_time) in &current_file_times {
                if let Some(&last_time) = last_file_times.get(path) {
                    if *current_time != last_time {
                        files_changed = true;
                        break;
                    }
                } else {
                    // New file
                    files_changed = true;
                    break;
                }
            }
            
            // Check for deleted files
            if !files_changed {
                for path in last_file_times.keys() {
                    if !current_file_times.contains_key(path) {
                        files_changed = true;
                        break;
                    }
                }
            }

            if files_changed {
                if let Err(err) = render_to_index_html() {
                    eprintln!("Failed to re-render index.html: {err}");
                }

                last_file_times = current_file_times;
            }

            thread::sleep(Duration::from_millis(500));
        }
    });
}
