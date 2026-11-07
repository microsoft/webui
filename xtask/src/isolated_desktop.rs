// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Test-only Windows desktop isolation; never changes application presentation.

#[cfg(windows)]
mod stdio;
#[cfg(windows)]
mod windows;

use std::process::ExitCode;

struct Request<'a> {
    command: &'a [String],
    timeout_ms: u32,
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let result = parse(args).and_then(execute);
    match result {
        Ok(code) => {
            // Preserve Windows exception exit codes as well as ordinary test failures.
            std::process::exit(code.cast_signed());
        }
        Err(error) => {
            eprintln!(
                "{} isolated desktop: {error}",
                console::style("✘").red().bold()
            );
            ExitCode::FAILURE
        }
    }
}

fn parse(args: &[String]) -> Result<Request<'_>, String> {
    let (timeout_ms, rest) = if args.first().is_some_and(|arg| arg == "--timeout") {
        let seconds = args
            .get(1)
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| *value > 0)
            .ok_or("--timeout requires a positive whole number of seconds")?;
        let milliseconds = seconds
            .checked_mul(1000)
            .filter(|value| *value < u32::MAX)
            .ok_or("--timeout exceeds the supported Windows wait duration")?;
        (milliseconds, &args[2..])
    } else {
        (1_800_000, args)
    };
    if rest.first().is_none_or(|arg| arg != "--") || rest.get(1).is_none_or(String::is_empty) {
        return Err(
            "help: use cargo xtask isolated-desktop [--timeout SECONDS] -- <exe> [args...]".into(),
        );
    }
    if rest[1..].iter().any(|arg| arg.contains('\0')) {
        return Err("command arguments must not contain NUL characters".into());
    }
    Ok(Request {
        command: &rest[1..],
        timeout_ms,
    })
}

#[cfg(windows)]
fn execute(request: Request<'_>) -> Result<u32, String> {
    windows::execute(request)
}

#[cfg(not(windows))]
fn execute(request: Request<'_>) -> Result<u32, String> {
    let _ = (request.command, request.timeout_ms);
    Err(
        "requires Windows; help: run this command on your Windows desktop or Windows CI runner"
            .into(),
    )
}

#[cfg(any(windows, test))]
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
    fn parses_default_and_explicit_timeouts_without_changing_child_args() {
        let args = ["--", "cargo", "test", "--", "--ignored"].map(String::from);
        let request = parse(&args).unwrap();
        assert_eq!(request.timeout_ms, 1_800_000);
        assert_eq!(request.command, &args[1..]);
        let args = ["--timeout", "60", "--", "cargo", ""].map(String::from);
        let request = parse(&args).unwrap();
        assert_eq!(request.timeout_ms, 60_000);
        assert_eq!(request.command, &args[3..]);
    }

    #[test]
    fn rejects_missing_commands_invalid_timeouts_and_nuls() {
        for args in [
            vec![],
            vec!["--"],
            vec!["--", ""],
            vec!["cargo"],
            vec!["--timeout"],
            vec!["--timeout", "0", "--", "cargo"],
            vec!["--timeout", "-1", "--", "cargo"],
            vec!["--timeout", "4294968", "--", "cargo"],
            vec!["--", "cargo", "bad\0argument"],
        ] {
            let args = args.into_iter().map(String::from).collect::<Vec<_>>();
            assert!(parse(&args).is_err(), "{args:?}");
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

    #[cfg(not(windows))]
    #[test]
    fn refuses_to_run_unisolated_on_other_platforms() {
        let args = ["--", "cargo"].map(String::from);
        assert!(execute(parse(&args).unwrap())
            .unwrap_err()
            .contains("requires Windows"));
    }
}
