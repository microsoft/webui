// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub fn header(title: &str) {
    eprintln!(
        "\n  {} {}\n",
        console::style("⚡").cyan().bold(),
        console::style(title).cyan().bold()
    );
}

pub fn field(label: &str, value: &dyn std::fmt::Display) {
    eprintln!(
        "  {} {}",
        console::style(format!("▸ {label:<10}")).dim(),
        console::style(value).bold()
    );
}

pub fn success(message: &str) {
    eprintln!("  {} {message}", console::style("✔").green());
}

pub fn finish(message: &str) {
    eprintln!("\n  {} {message}\n", console::style("✨").green());
}

pub fn error(err: &anyhow::Error) {
    eprintln!(
        "\n  {} {}",
        console::style("✘").red().bold(),
        console::style(err).red().bold()
    );
    for cause in err.chain().skip(1) {
        eprintln!("  {} {cause}", console::style("caused by:").dim());
    }
}

pub fn hint(message: &str) {
    eprintln!("\n  {} {message}", console::style("hint:").dim());
}
