use tokio::net::TcpStream;

use crate::constants::network;

pub async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, String> {
    let stream = TcpStream::connect((host, port))
        .await
        .map_err(|error| error.to_string())?;
    stream.set_nodelay(true).map_err(|error| error.to_string())?;
    Ok(stream)
}

pub fn default_host() -> String {
    network::SERVER_HOST.to_string()
}

pub fn default_port() -> u16 {
    network::SERVER_PORT
}
