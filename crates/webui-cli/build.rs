// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

fn main() {
    println!("cargo:rerun-if-changed=src/commands/init/templates");
}
