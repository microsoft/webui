// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub(crate) fn snake(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().copied().enumerate() {
        if c == '_' {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            continue;
        }
        if c.is_ascii_uppercase() && i > 0 && !out.ends_with('_') {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || next_lower && prev.is_ascii_uppercase()
            {
                out.push('_');
            }
        }
        out.push(c.to_ascii_lowercase());
    }
    out.trim_end_matches('_').into()
}

pub(crate) fn pascal(name: &str) -> String {
    let mut result = String::new();
    for word in snake(name).split('_') {
        let mut chars = word.chars();
        if let Some(c) = chars.next() {
            result.push(c.to_ascii_uppercase());
            result.extend(chars);
        }
    }
    result
}

pub(crate) fn camel(name: &str) -> String {
    // ts-proto's snakeToCamel preserves existing capitalization.
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else {
            out.push(if out.is_empty() {
                c.to_ascii_lowercase()
            } else if upper {
                c.to_ascii_uppercase()
            } else {
                c
            });
            upper = false;
        }
    }
    out
}

pub(crate) fn ts_field(name: &str) -> String {
    if !name.contains('_') {
        return name.into();
    }
    let mixed = name.chars().any(|c| c.is_ascii_lowercase());
    let mut out = String::new();
    for (index, word) in name.split('_').enumerate() {
        let word = if mixed {
            word.to_owned()
        } else {
            word.to_ascii_lowercase()
        };
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.push(if index == 0 {
                first
            } else {
                first.to_ascii_uppercase()
            });
            out.extend(chars);
        }
    }
    out
}

pub(crate) fn rust_ident(name: &str) -> String {
    match name {
        "self" | "super" | "crate" | "Self" => format!("{name}_"),
        "as" | "async" | "await" | "break" | "const" | "continue" | "dyn" | "else" | "enum"
        | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match"
        | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "static" | "struct" | "trait"
        | "true" | "type" | "unsafe" | "use" | "where" | "while" | "abstract" | "become"
        | "box" | "do" | "final" | "macro" | "override" | "priv" | "typeof" | "unsized"
        | "virtual" | "yield" | "try" | "gen" => format!("r#{name}"),
        _ => name.into(),
    }
}
