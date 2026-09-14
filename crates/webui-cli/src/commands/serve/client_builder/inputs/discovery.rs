// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[test]
fn nested_dist_and_target_component_contents_invalidate_ssr() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let components = fixture.sibling("components");
    fs::create_dir(&components)?;
    fixture
        .config
        .app_args
        .components
        .push(components.to_string_lossy().into_owned());
    let inputs = fixture.inputs()?;
    for root in [&fixture.config.app_dir, &components] {
        for name in ["dist", "target"] {
            let nested = root.join("nested").join(name);
            fs::create_dir_all(&nested)?;
            for extension in ["html", "css"] {
                let path = nested.join("nested-card").with_extension(extension);
                fs::write(&path, "before")?;
                let before = snapshot(&inputs)?;
                fs::write(&path, "after")?;
                assert_ne!(before, snapshot(&inputs)?, "{}", path.display());
            }
        }
    }
    Ok(())
}

#[test]
fn oversized_scripts_only_fingerprint_existence_and_resolved_target() -> Result<()> {
    let fixture = Fixture::new()?;
    let inputs = fixture.inputs()?;
    let script = fixture.config.app_dir.join("sample-card.ts");
    let before = snapshot(&inputs)?;
    File::create(&script)?.set_len(MAX_BYTES as u64 + 1)?;
    assert_eq!(before, snapshot(&inputs)?);
    File::create(&script)?.set_len(MAX_BYTES as u64 * 2)?;
    assert_eq!(before, snapshot(&inputs)?);
    fs::remove_file(&script)?;
    assert_ne!(before, snapshot(&inputs)?);
    Ok(())
}

#[test]
fn filename_only_script_never_reads_the_content_buffer() -> Result<()> {
    let fixture = Fixture::new()?;
    let mut capture = Capture {
        entries: BTreeMap::new(),
        directories: HashSet::new(),
        buffer: [0xA5; BUFFER_SIZE],
    };
    capture.file(&fixture.config.app_dir.join("sample-card.ts"), false)?;
    assert!(capture.buffer.iter().all(|byte| *byte == 0xA5));
    Ok(())
}

#[test]
fn canonical_file_paths_are_retained_once_instead_of_duplicated_as_targets() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.config.app_dir = fixture.config.app_dir.canonicalize()?;
    let snapshot = snapshot(&fixture.inputs()?)?;
    assert!(!snapshot.entries.is_empty());
    assert!(snapshot
        .entries
        .values()
        .all(|entry| entry.target.is_none()));
    Ok(())
}

#[test]
fn explicit_state_with_script_extension_still_requires_content_hashing() -> Result<()> {
    let mut fixture = Fixture::new()?;
    let state = fixture.sibling("state.ts");
    fixture.config.state_file = Some(state.clone());
    fs::write(&state, "{}")?;
    let inputs = fixture.inputs()?;
    let before = snapshot(&inputs)?;
    fs::write(&state, r#"{"value":1}"#)?;
    assert_ne!(before, snapshot(&inputs)?);
    File::create(&state)?.set_len(MAX_BYTES as u64 + 1)?;
    assert!(inputs.capture()?.is_none());
    Ok(())
}
