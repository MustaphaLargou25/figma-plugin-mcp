use std::{env, net::SocketAddr, time::Duration};

#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub bind: SocketAddr,
    pub command_timeout: Duration,
    pub max_queue_size: usize,
    pub max_message_bytes: usize,
    pub client_buffer_size: usize,
}

impl RelayConfig {
    pub fn from_env() -> Self {
        let host = env::var("FIGMA_MCP_BIND_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port = read_env_u16("FIGMA_MCP_SOCKET_PORT", 3055);
        let bind = env::var("FIGMA_MCP_BIND")
            .ok()
            .and_then(|raw| raw.parse::<SocketAddr>().ok())
            .unwrap_or_else(|| {
                format!("{host}:{port}")
                    .parse()
                    .expect("valid bind address")
            });

        Self {
            bind,
            command_timeout: Duration::from_millis(read_env_u64(
                "FIGMA_MCP_COMMAND_TIMEOUT_MS",
                120_000,
            )),
            max_queue_size: read_env_usize("FIGMA_MCP_MAX_QUEUE_SIZE", 100),
            max_message_bytes: read_env_usize("FIGMA_MCP_MAX_MESSAGE_BYTES", 25 * 1024 * 1024),
            client_buffer_size: read_env_usize("FIGMA_MCP_CLIENT_BUFFER_SIZE", 256),
        }
    }
}

fn read_env_u16(name: &str, fallback: u16) -> u16 {
    env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or(fallback)
}

fn read_env_u64(name: &str, fallback: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(fallback)
}

fn read_env_usize(name: &str, fallback: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(fallback)
}
