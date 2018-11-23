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

pub struct XGE {
    client_process: std::process::Child,
    writer: std::io::BufWriter<std::net::TcpStream>,
}
impl<'a> XGE {
    pub fn new() -> XGE {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let xge_exe = String::from(
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("xge.exe")
                .to_str()
                .unwrap(),
        );
        let client_process = std::process::Command::new("powershell.exe")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .arg(format!(
                r#"& 'xgConsole' @('/command="{}','client','127.0.0.1:{}"','/openmonitor')"#,
                xge_exe, port
            )).spawn()
            .expect("could not spawn XGE client!");
        let client_socket = listener.incoming().next().unwrap();

        let writer = std::io::BufWriter::new(client_socket.unwrap());

        XGE {
            client_process: client_process,
            writer: writer,
        }
    }

    pub fn run(&mut self, request: &StreamRequest) -> std::io::Result<()> {
        self.writer
            .write(serde_json::to_string(&request)?.as_bytes())?;
        self.writer.write(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn done(&mut self) -> std::io::Result<()> {
        self.writer.get_ref().shutdown(std::net::Shutdown::Both)?;
        Ok(())
    }

    pub fn results(&'a mut self) -> impl Iterator<Item = StreamResult> + 'a {
        let reader = std::io::BufReader::new(self.client_process.stdout.as_mut().unwrap());
        reader
            .lines()
            .map(|l| l.unwrap())
            .filter(|l| l.starts_with("mwt "))
            .map(|l| serde_json::from_str(&l[4..]).unwrap())
    }
}
