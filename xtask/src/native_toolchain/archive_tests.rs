// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[test]
fn archive_inventory_rejects_links_escapes_duplicates_unknown_and_missing_files() {
    let required = BTreeSet::from(["LICENSE".to_owned(), "lib/tsc".to_owned()]);
    let valid = "-rw-r--r--  0 0 0 12 Nov 16 22:00 package/LICENSE\n-rwxr-xr-x  0 0 0 20 Nov 16 22:00 package/lib/tsc\n";
    assert!(validate_listing(valid.as_bytes(), &required).is_ok());
    for listing in [
        valid.replace("package/lib/tsc", "package/../tsc"),
        valid.replace("package/lib/tsc", "/absolute/tsc"),
        valid.replace("package/lib/tsc", "package/lib/tsc.js"),
        valid.replace("-rwxr-xr-x", "lrwxrwxrwx"),
        format!("{valid}{valid}"),
        valid.lines().take(1).collect::<String>(),
    ] {
        assert!(validate_listing(listing.as_bytes(), &required).is_err());
    }
}

#[test]
fn permits_only_matching_regular_upstream_signature_metadata() {
    let required = BTreeSet::from(["lib/tsc".to_owned()]);
    let binary = "-rwxr-xr-x  0 0 0 20 Nov 16 22:00 package/lib/tsc\n";
    let signature = "-rw-r--r--  0 0 0 12 Nov 16 22:00 package/lib/tsc.sig\n";
    assert!(validate_listing(format!("{binary}{signature}").as_bytes(), &required).is_ok());
    for metadata in [
        signature.replace("tsc.sig", "tsc.exe.sig"),
        signature.replace("tsc.sig", "unknown.sig"),
        signature.replace("-rw-r--r--", "lrwxrwxrwx"),
    ] {
        assert!(validate_listing(format!("{binary}{metadata}").as_bytes(), &required).is_err());
    }
}

#[test]
fn native_tar_listing_and_fixed_member_extraction_interoperate() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let source = temp.path().join("package");
    fs::create_dir(&source).map_err(|e| e.to_string())?;
    fs::write(
        source.join("LICENSE"),
        b"test fixture, not a native artifact",
    )
    .map_err(|e| e.to_string())?;
    let package = temp.path().join("source.tgz");
    let status = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .arg("-czf")
        .arg(&package)
        .arg("-C")
        .arg(temp.path())
        .arg("package/LICENSE")
        .status()
        .map_err(|e| e.to_string())?;
    assert!(status.success());
    let output = temp.path().join("unpacked");
    extract(&package, &output, &BTreeSet::from(["LICENSE".to_owned()]))?;
    assert_eq!(
        fs::read(output.join("LICENSE")).map_err(|e| e.to_string())?,
        b"test fixture, not a native artifact"
    );
    Ok(())
}
