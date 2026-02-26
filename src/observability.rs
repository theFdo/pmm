//! Shared logging configuration and initialization.

use std::env;
use std::net::SocketAddr;

use thiserror::Error;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Json,
    Pretty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingConfig {
    pub level: String,
    pub format: LogFormat,
    pub include_target: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: LogFormat::Pretty,
            include_target: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum LoggingInitError {
    #[error("logging already initialized: {0}")]
    AlreadyInitialized(#[from] tracing::subscriber::SetGlobalDefaultError),
}

pub fn logging_config_from_env() -> LoggingConfig {
    let mut config = LoggingConfig::default();

    if let Ok(level) = env::var("PMM_LOG_LEVEL") {
        let trimmed = level.trim();
        if !trimmed.is_empty() {
            config.level = trimmed.to_string();
        }
    }

    if let Ok(format) = env::var("PMM_LOG_FORMAT") {
        if let Some(parsed) = parse_log_format(&format) {
            config.format = parsed;
        }
    }

    if let Ok(include_target) = env::var("PMM_LOG_TARGET") {
        if let Some(parsed) = parse_bool(&include_target) {
            config.include_target = parsed;
        }
    }

    config
}

pub fn init_logging(config: &LoggingConfig) -> Result<(), LoggingInitError> {
    let env_filter =
        EnvFilter::try_new(config.level.clone()).unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(config.include_target)
        .with_ansi(matches!(config.format, LogFormat::Pretty));

    match config.format {
        LogFormat::Json => tracing::subscriber::set_global_default(builder.json().finish())?,
        LogFormat::Pretty => tracing::subscriber::set_global_default(builder.pretty().finish())?,
    }

    Ok(())
}

pub fn log_app_start(config: &LoggingConfig) {
    info!(
        component = "dashboard_server",
        event = "app.start",
        log_level = %config.level,
        log_format = ?config.format,
        include_target = config.include_target
    );
}

pub fn log_app_bind(bound_addr: SocketAddr) {
    info!(
        component = "dashboard_server",
        event = "app.bind",
        bind_addr = %bound_addr,
        route = "/dashboard"
    );
}

pub fn log_source_selected(source: &str, reason: Option<&str>, refresh_interval_ms: Option<u64>) {
    match (reason, refresh_interval_ms) {
        (Some(reason), Some(refresh_interval_ms)) => info!(
            component = "dashboard_server",
            event = "source.selected",
            source,
            reason,
            refresh_interval_ms
        ),
        (Some(reason), None) => info!(
            component = "dashboard_server",
            event = "source.selected",
            source,
            reason
        ),
        (None, Some(refresh_interval_ms)) => info!(
            component = "dashboard_server",
            event = "source.selected",
            source,
            refresh_interval_ms
        ),
        (None, None) => info!(
            component = "dashboard_server",
            event = "source.selected",
            source
        ),
    }
}

fn parse_log_format(raw: &str) -> Option<LogFormat> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "json" => Some(LogFormat::Json),
        "pretty" => Some(LogFormat::Pretty),
        _ => None,
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_env_vars<R>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> R) -> R {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let previous: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), env::var(key).ok()))
            .collect();

        for (key, value) in vars {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }

        let output = f();

        for (key, value) in previous {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }

        output
    }

    #[test]
    fn defaults_when_env_missing() {
        let cfg = with_env_vars(
            &[
                ("PMM_LOG_LEVEL", None),
                ("PMM_LOG_FORMAT", None),
                ("PMM_LOG_TARGET", None),
            ],
            logging_config_from_env,
        );

        assert_eq!(cfg, LoggingConfig::default());
    }

    #[test]
    fn parses_json_and_level_and_target_from_env() {
        let cfg = with_env_vars(
            &[
                ("PMM_LOG_LEVEL", Some("debug")),
                ("PMM_LOG_FORMAT", Some("json")),
                ("PMM_LOG_TARGET", Some("false")),
            ],
            logging_config_from_env,
        );

        assert_eq!(cfg.level, "debug");
        assert_eq!(cfg.format, LogFormat::Json);
        assert!(!cfg.include_target);
    }

    #[test]
    fn invalid_format_or_target_falls_back_to_defaults() {
        let cfg = with_env_vars(
            &[
                ("PMM_LOG_LEVEL", Some("trace")),
                ("PMM_LOG_FORMAT", Some("yaml")),
                ("PMM_LOG_TARGET", Some("maybe")),
            ],
            logging_config_from_env,
        );

        assert_eq!(cfg.level, "trace");
        assert_eq!(cfg.format, LogFormat::Pretty);
        assert!(cfg.include_target);
    }
}
