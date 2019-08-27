use serde_derive::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;
use tokio_process::Command;

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamRequest {
    pub id: u64,
    pub title: String,
    pub cwd: String,
    pub command: Vec<String>,
    pub local: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamResult {
    pub id: u64,
    pub exit_code: i32,
    pub stdout: String,
}

pub fn xge() -> (
    tokio_process::Child,
    impl Future<Output = std::io::Result<TcpStream>>,
) {
    let listener =
        TcpListener::bind(&"127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap()).unwrap();
    let port = listener.local_addr().unwrap().port();
    let parent = std::env::current_exe().unwrap();
    let parent = parent.parent().unwrap();
    let profile_xml = String::from(parent.join("profile.xml").to_str().unwrap());
    let xge_exe = String::from(parent.join("xge.exe").to_str().unwrap());
    let client_process = Command::new("xgConsole")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .arg(format!("/profile={}", profile_xml))
        .arg("/openmonitor")
        .arg(format!("/command={} client 127.0.0.1:{}", xge_exe, port))
        .spawn()
        .expect("could not spawn XGE client!");

    (
        client_process,
        listener
            .incoming()
            .take(1)
            .collect::<Vec<_>>()
            .map(|mut v| v.remove(0)),
    )
}
