//! Network binding helpers.

use crate::config::Config;

pub fn bind_addr(config: &Config) -> String {
    format!("{}:{}", config.server.host, config.server.port)
}

#[cfg(test)]
mod tests {
    use super::bind_addr;
    use crate::config::{Config, ServerConfig};

    #[test]
    fn bind_addr_respects_config_host() {
        let config = Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 1234,
            },
            ..Default::default()
        };

        assert_eq!(bind_addr(&config), "127.0.0.1:1234");
    }
}
