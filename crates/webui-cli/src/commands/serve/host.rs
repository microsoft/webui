// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::http::header::HOST;
use actix_web::HttpRequest;

use crate::utils::error::CliError;

#[derive(Clone)]
struct ExactHost {
    name: String,
    port: Option<u16>,
}

#[derive(Clone)]
pub(super) struct HostPolicy {
    port: u16,
    additional: Vec<ExactHost>,
}

impl HostPolicy {
    pub(super) fn new(port: u16, hosts: &[String]) -> Result<Self, CliError> {
        if port == 0 {
            return Err(CliError::InvalidServePort);
        }
        let mut additional = Vec::with_capacity(hosts.len());
        for host in hosts {
            let Some((name, specified_port)) = parse_authority(host) else {
                return Err(CliError::InvalidAllowedHost { host: host.clone() });
            };
            if name.parse::<std::net::IpAddr>().is_ok() {
                return Err(CliError::InvalidAllowedHost { host: host.clone() });
            }
            additional.push(ExactHost {
                name: name.to_ascii_lowercase(),
                port: specified_port,
            });
        }
        Ok(Self { port, additional })
    }

    pub(super) fn allows(&self, req: &HttpRequest) -> bool {
        let mut values = req.headers().get_all(HOST);
        let Some(host) = values.next() else {
            return false;
        };
        if values.next().is_some() {
            return false;
        }
        let Some((name, port)) = host.to_str().ok().and_then(parse_authority) else {
            return false;
        };

        let default_port = port == Some(self.port) || (self.port == 80 && port.is_none());
        if default_port && is_loopback_name(name) {
            return true;
        }
        self.additional.iter().any(|allowed| {
            allowed.name.eq_ignore_ascii_case(name)
                && match allowed.port {
                    Some(required) => port == Some(required),
                    None => port.is_none() || port == Some(self.port),
                }
        })
    }
}

fn is_loopback_name(name: &str) -> bool {
    if name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" {
        return true;
    }
    let suffix = b".localhost";
    name.len() > suffix.len()
        && name.as_bytes()[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

fn parse_authority(value: &str) -> Option<(&str, Option<u16>)> {
    let (name, port) = if let Some((name, port)) = value.rsplit_once(':') {
        if port.is_empty() || !port.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let port: u16 = port.parse().ok()?;
        if port == 0 {
            return None;
        }
        (name, Some(port))
    } else {
        (value, None)
    };
    if !valid_hostname(name) {
        return None;
    }
    Some((name, port))
}

fn valid_hostname(name: &str) -> bool {
    if name.is_empty() || name.len() > 253 || !name.is_ascii() {
        return false;
    }
    name.split('.').all(|label| {
        let bytes = label.as_bytes();
        label.len() <= 63
            && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
            && bytes
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || *c == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

    #[test]
    fn allows_only_loopback_names_on_bound_port() {
        let policy = HostPolicy::new(3000, &[]).unwrap();
        for authority in [
            "localhost:3000",
            "LOCALHOST:3000",
            "127.0.0.1:3000",
            "play.xbox.localhost:3000",
            "PLAY.XBOX.LOCALHOST:3000",
        ] {
            let request = TestRequest::default()
                .insert_header((HOST, authority))
                .to_http_request();
            assert!(policy.allows(&request), "{authority}");
        }
        for authority in [
            "localhost",
            "localhost:3001",
            "127.0.0.2:3000",
            "localhost.evil.example:3000",
            "notlocalhost:3000",
            "play..localhost:3000",
            "evil.example:3000",
            "evil.example@localhost:3000",
            "localhost:0",
            "localhost:not-a-port",
            "[::1]:3000",
        ] {
            let request = TestRequest::default()
                .insert_header((HOST, authority))
                .to_http_request();
            assert!(!policy.allows(&request), "{authority}");
        }
    }

    #[test]
    fn rejects_missing_or_duplicate_host() {
        let policy = HostPolicy::new(3000, &[]).unwrap();
        let missing = TestRequest::default().to_http_request();
        assert!(!policy.allows(&missing));
        let duplicate = TestRequest::default()
            .append_header((HOST, "localhost:3000"))
            .append_header((HOST, "attacker.example:3000"))
            .to_http_request();
        assert!(!policy.allows(&duplicate));
    }

    #[test]
    fn exact_extra_hosts_require_an_opt_in() {
        let hosts = vec!["Dev.Example:443".into(), "project.example".into()];
        let policy = HostPolicy::new(3000, &hosts).unwrap();
        for authority in [
            "dev.example:443",
            "DEV.EXAMPLE:443",
            "project.example:3000",
            "project.example",
        ] {
            let request = TestRequest::default()
                .insert_header((HOST, authority))
                .to_http_request();
            assert!(policy.allows(&request), "{authority}");
        }
        for authority in [
            "dev.example:3000",
            "other.dev.example:443",
            "project.example:443",
        ] {
            let request = TestRequest::default()
                .insert_header((HOST, authority))
                .to_http_request();
            assert!(!policy.allows(&request), "{authority}");
        }
    }

    #[test]
    fn invalid_extra_hosts_fail_at_startup() {
        for invalid in [
            "",
            "*.example.com",
            ".example.com",
            "https://example.com",
            "example.com/path",
            "example.com:0",
            "example.com:65536",
            "example.com:",
            "example.com:443:80",
            "foo..example",
            "other.example.",
            "127.0.0.2",
        ] {
            assert!(
                HostPolicy::new(3000, &[invalid.into()]).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn rejects_zero_port_with_or_without_extra_hosts() {
        for hosts in [vec![], vec!["project.example".into()]] {
            assert!(matches!(
                HostPolicy::new(0, &hosts),
                Err(CliError::InvalidServePort)
            ));
        }
    }
}
