// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Internal Windows test isolation; never changes application presentation.

mod stdio;
mod windows;

struct Request<'a> {
    command: &'a [String],
    timeout_ms: u32,
}

pub(crate) fn run(command: &[String]) -> Result<u32, String> {
    execute(request(command, 1_800_000)?)
}

pub(crate) fn is_current() -> Result<bool, String> {
    windows::is_current()
}

fn request(command: &[String], timeout_ms: u32) -> Result<Request<'_>, String> {
    if command.first().is_none_or(String::is_empty) {
        return Err("isolated test command requires an executable".into());
    }
    if timeout_ms == 0 || timeout_ms == u32::MAX {
        return Err("isolated test timeout must be a finite positive duration".into());
    }
    if command.iter().any(|arg| arg.contains('\0')) {
        return Err("command arguments must not contain NUL characters".into());
    }
    Ok(Request {
        command,
        timeout_ms,
    })
}

fn execute(request: Request<'_>) -> Result<u32, String> {
    windows::execute(request)
}

fn quote_argument(output: &mut Vec<u16>, value: impl Iterator<Item = u16> + Clone) {
    const QUOTE: u16 = b'"' as u16;
    const SLASH: u16 = b'\\' as u16;
    if !value
        .clone()
        .any(|unit| matches!(unit, 9 | 10 | 11 | 12 | 13 | 32 | QUOTE))
        && value.clone().next().is_some()
    {
        output.extend(value);
        return;
    }
    output.push(QUOTE);
    let mut slashes = 0;
    for unit in value {
        if unit == SLASH {
            slashes += 1;
            continue;
        }
        let count = if unit == QUOTE {
            slashes * 2 + 1
        } else {
            slashes
        };
        output.extend(std::iter::repeat_n(SLASH, count));
        output.push(unit);
        slashes = 0;
    }
    output.extend(std::iter::repeat_n(SLASH, slashes * 2));
    output.push(QUOTE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_child_args_and_default_timeout() {
        let args = ["cargo", "test", "--", "--ignored", ""].map(String::from);
        let request = request(&args, 1_800_000).unwrap();
        assert_eq!(request.timeout_ms, 1_800_000);
        assert_eq!(request.command, &args);
    }

    #[test]
    fn rejects_missing_commands_invalid_timeouts_and_nuls() {
        for args in [vec![], vec![""], vec!["cargo", "bad\0argument"]] {
            let args = args.into_iter().map(String::from).collect::<Vec<_>>();
            assert!(request(&args, 1_800_000).is_err(), "{args:?}");
        }
        let args = ["cargo"].map(String::from);
        for timeout in [0, u32::MAX] {
            assert!(request(&args, timeout).is_err());
        }
    }

    #[test]
    fn quotes_windows_arguments_including_empty_quotes_and_trailing_slashes() {
        for (input, expected) in [
            ("", "\"\""),
            ("test", "test"),
            ("two words", "\"two words\""),
            ("a\"b", "\"a\\\"b\""),
            ("C:\\path\\", "C:\\path\\"),
            ("C:\\two words\\", "\"C:\\two words\\\\\""),
            ("a\\\"b", "\"a\\\\\\\"b\""),
            ("& | < > % !", "\"& | < > % !\""),
            ("caf\u{e9}", "caf\u{e9}"),
        ] {
            let mut output = Vec::new();
            quote_argument(&mut output, input.encode_utf16());
            assert_eq!(String::from_utf16(&output).unwrap(), expected);
        }
    }
}
