// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::fs;

#[test]
fn fast_local_template_uses_component_filename_prefix() {
    let root = tempfile::TempDir::new().unwrap();
    fs::write(
        root.path().join("todo-item.template-webui.html"),
        "<template>item</template>",
    )
    .unwrap();
    fs::write(
        root.path().join("todo-item.template.html"),
        "<template>Alternate</template>",
    )
    .unwrap();

    let components = FastDiscoveryPlugin::new()
        .discover_local(root.path())
        .unwrap();

    assert_eq!(components.len(), 1);
    assert_eq!(components[0].tag_name, "todo-item");
}

#[test]
fn virtual_root_module_candidates_stay_inside_package() {
    let root = tempfile::TempDir::new().unwrap();
    let candidates = fast_template_candidates(root.path(), Path::new("index.js"), "MyButton");

    assert!(candidates
        .iter()
        .all(|candidate| candidate.starts_with(root.path())));
    assert!(candidates.contains(
        &root
            .path()
            .join("my-button")
            .join("my-button.template-webui.html")
    ));
    assert!(
        fast_template_candidates(root.path(), Path::new("../escape/index.js"), "Escape").is_empty()
    );
}
