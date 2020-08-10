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
            let mut args = request.command.iter();
            let mut cmd = std::process::Command::new(args.next().unwrap());
            for arg in args {
                cmd.arg(arg);
            }

            let maybe_output = cmd.output();
            match maybe_output {
                Ok(output) => {
                    let exit_code = output.status.code().unwrap_or(-7787);
                    let stdout =
                        str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
                    let stderr =
                        str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
                    let output_str = stderr.to_owned() + stdout;

                    report(request.id, exit_code, &output_str);
                }
                Err(e) => {
                    report(
                        request.id,
                        -7787,
                        &format!("XGE-Launcher: failed to execute process: {}", e),
                    );
                }
            }
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
