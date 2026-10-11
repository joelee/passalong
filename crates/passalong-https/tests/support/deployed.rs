//! A deployed passalong-server, reached over the network: the one the
//! `INTEGRATION_TEST_*` variables of `.env` name. `just test-deployed`
//! loads them and runs the `deployed_*` tests (PLAN-00011).
//!
//! Nothing here starts a server. The tests use one workspace made for them,
//! and throwaway workspaces they create and delete over SSH; the operator's
//! other workspaces are never named.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use passalong_core::api_key::ApiKey;
use passalong_core::config::{self, Config};
use passalong_core::testing::MapEnv;

/// The recipe that sets the variables.
pub const RECIPE: &str = "just test-deployed";

/// The deployment as `.env` describes it.
#[derive(Debug, Clone)]
pub struct Deployed {
    /// `INTEGRATION_TEST_SERVER_URL`.
    pub url: String,
    /// `INTEGRATION_TEST_SERVER_ALT_URLS`, split on commas; may be empty.
    pub alt_urls: Vec<String>,
    /// `INTEGRATION_TEST_TLS_PIN`, as given (`sha256/` or `sha256//`).
    pub pin: String,
    /// `INTEGRATION_TEST_WORKSPACE`.
    pub workspace: String,
    /// `INTEGRATION_TEST_API_KEY`: read-write on [`Self::workspace`].
    pub key: ApiKey,
    /// `INTEGRATION_TEST_API_KEY_READ_ONLY`: read-only on the same.
    pub read_only_key: ApiKey,
    /// `INTEGRATION_TEST_SSH_HOST` and `INTEGRATION_TEST_SSH_COMPOSE_DIR`,
    /// when both are set.
    pub ssh: Option<Ssh>,
}

/// How to reach the server's admin commands.
#[derive(Debug, Clone)]
pub struct Ssh {
    /// `user@host` for `ssh`.
    pub host: String,
    /// The Compose project directory on that host.
    pub compose_dir: String,
}

impl Deployed {
    /// Reads the variables, failing with the name of a missing one.
    pub fn from_env() -> Self {
        let url = required("INTEGRATION_TEST_SERVER_URL");
        let alt_urls = optional("INTEGRATION_TEST_SERVER_ALT_URLS")
            .map(|list| {
                list.split(',')
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let pin = required("INTEGRATION_TEST_TLS_PIN");
        let workspace = required("INTEGRATION_TEST_WORKSPACE");
        let key = parse_key("INTEGRATION_TEST_API_KEY");
        let read_only_key = parse_key("INTEGRATION_TEST_API_KEY_READ_ONLY");
        let ssh = match (
            optional("INTEGRATION_TEST_SSH_HOST"),
            optional("INTEGRATION_TEST_SSH_COMPOSE_DIR"),
        ) {
            (Some(host), Some(compose_dir)) => Some(Ssh { host, compose_dir }),
            _ => None,
        };
        Self {
            url,
            alt_urls,
            pin,
            workspace,
            key,
            read_only_key,
            ssh,
        }
    }

    /// The configuration of a device in `dir` reaching `url` with `pin`
    /// (none means the operating system's trust store) under `key`, as
    /// text for `config.toml`. `extra` is appended, for `[serve]` settings.
    pub fn config_text(
        dir: &Path,
        url: &str,
        pin: Option<&str>,
        device_name: &str,
        extra: &str,
    ) -> String {
        let pin = pin.map_or_else(String::new, |pin| format!("tls_pin = \"{pin}\"\n"));
        format!(
            "[client]\ndevice_name = \"{device_name}\"\nkey_file = '{}'\ndownload_dir = '{}'\n\n[server]\nkind = \"https\"\n\n[server.https]\nurl = \"{url}\"\n{pin}api_key_file = '{}'\n{extra}",
            dir.join("store.key").display(),
            dir.join("downloads").display(),
            dir.join("api.key").display(),
        )
    }

    /// A parsed configuration for a device in `dir` reaching `url` with
    /// `pin`, keeping its keys there.
    pub fn config(&self, dir: &Path, url: &str, pin: Option<&str>) -> Config {
        let text = Self::config_text(dir, url, pin, "it-deployed", "");
        config::parse(
            &text,
            &dir.join("config.toml"),
            &MapEnv::new().with("HOME", dir.to_str().unwrap()),
        )
        .unwrap()
    }

    /// The configuration with this deployment's URL and pin.
    pub fn pinned(&self, dir: &Path) -> Config {
        self.config(dir, &self.url, Some(&self.pin))
    }

    /// The path of the API key file [`Self::config_text`] names.
    pub fn key_file(dir: &Path) -> PathBuf {
        dir.join("api.key")
    }
}

/// `text` with the secret of every API key in it replaced by `…`, so that a
/// panic or a log line can be shown.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("pal_") {
        let (before, from_key) = rest.split_at(at);
        out.push_str(before);
        let body = &from_key[4..];
        // `pal_<id>_<secret>`: keep the id, drop the secret.
        let key_chars = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
        let end = body.find(|c: char| !key_chars(c)).unwrap_or(body.len());
        let key = &body[..end];
        match key.find('_') {
            Some(sep) => {
                out.push_str("pal_");
                out.push_str(&key[..=sep]);
                out.push('…');
            }
            None => {
                out.push_str("pal_");
                out.push_str(key);
            }
        }
        rest = &body[end..];
    }
    out.push_str(rest);
    out
}

fn required(name: &str) -> String {
    optional(name).unwrap_or_else(|| panic!("`{name}` is not set: run these tests with `{RECIPE}`"))
}

fn optional(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn parse_key(name: &str) -> ApiKey {
    ApiKey::parse(&required(name))
        .unwrap_or_else(|err| panic!("`{name}` is not an API key (pal_<id>_<secret>): {err}"))
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn redact_keeps_ids_and_drops_secrets() {
        assert_eq!(
            redact("key pal_0123456789ab_s3cr3t-Value_x ended, pal_zz too"),
            "key pal_0123456789ab_… ended, pal_zz too"
        );
        assert_eq!(redact("no key here"), "no key here");
    }
}
