#[macro_use]
extern crate serde_derive;
use std::io::{BufRead, Write};

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

pub fn xge() -> (XGEWriter, XGEReader) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let parent = std::env::current_exe().unwrap();
    let parent = parent.parent().unwrap();
    let profile_xml = String::from(parent.join("profile.xml").to_str().unwrap());
    let xge_exe = String::from(parent.join("xge.exe").to_str().unwrap());
    let client_process = std::process::Command::new("xgConsole")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .arg(format!("/profile={}", profile_xml))
        .arg("/openmonitor")
        .arg(format!("/command={} client 127.0.0.1:{}", xge_exe, port))
        .spawn()
        .expect("could not spawn XGE client!");
    let client_socket = listener.incoming().next().unwrap();

    let writer = std::io::BufWriter::new(client_socket.unwrap());
    (XGEWriter(writer), XGEReader::from(client_process))
}

pub struct XGEWriter(std::io::BufWriter<std::net::TcpStream>);
impl XGEWriter {
    pub fn run(&mut self, request: &StreamRequest) -> std::io::Result<()> {
        self.0.write(serde_json::to_string(&request)?.as_bytes())?;
        self.0.write(b"\n")?;
        self.0.flush()?;
        Ok(())
    }
    pub fn done(&mut self) -> std::io::Result<()> {
        self.0.get_ref().shutdown(std::net::Shutdown::Both)?;
        Ok(())
    }
}

pub struct XGEReader {
    reader: std::io::BufReader<std::process::ChildStdout>,
}
impl XGEReader {
    fn from(child: std::process::Child) -> XGEReader {
        XGEReader {
            reader: std::io::BufReader::new(child.stdout.unwrap()),
        }
    }
}
impl Iterator for XGEReader {
    type Item = StreamResult;
    fn next(&mut self) -> Option<StreamResult> {
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(_num_bytes) => {
                    if line.starts_with("mwt done") {
                        return None
                    } else if line.starts_with("mwt ") {
                        return Some(serde_json::from_str(&line[4..]).unwrap())
                    }
                }
                Err(_) => return None,
            }
        }
    }
}
