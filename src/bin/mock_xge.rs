use serde_json;
use xge_lib;

use std::env;
use std::io::{self, BufRead};
use std::net::TcpStream;
use std::str;

fn accept_commands(stream: TcpStream) {
    let reader = io::BufReader::new(stream);
    for cmd_str in reader.lines() {
        let cmd_str = cmd_str.unwrap();
        if cmd_str.starts_with("mwt done") {
            break;
        }
        let request: serde_json::Result<xge_lib::StreamRequest> = serde_json::from_str(&cmd_str);
        if let Ok(request) = request {
            report(request.id, 0, "example output");
        }
    }
    println!("mwt done");
}

fn report(id: u64, exit_code: i32, output: &str) {
    let result = xge_lib::StreamResult {
        id,
        exit_code,
        stdout: output.to_string(),
    };
    println!("mwt {}", serde_json::to_string(&result).unwrap());
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let stream = TcpStream::connect(&args[1]).expect("could not connect to XGE server!");
    accept_commands(stream);
}
