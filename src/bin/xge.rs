use serde_json;
use xge_lib;

use std::alloc::System;
use std::env;
use std::io::{self, BufRead};
use std::net::TcpStream;
use std::process::Command;
use std::str;

#[global_allocator]
static GLOBAL: System = System;

fn accept_commands(stream: TcpStream) {
    let reader = io::BufReader::new(stream);
    for cmd_str in reader.lines() {
        let request: xge_lib::StreamRequest = serde_json::from_str(&cmd_str.unwrap()).unwrap();
        let this_exe = env::current_exe().unwrap();

        let mut cmd = Command::new("xgSubmit");
        cmd.current_dir(request.cwd)
            .arg(format!("/caption={}", request.title.replace(' ', "_")));
        if request.local {
            cmd.arg("/allowremote=off");
        }
        cmd.arg("/command")
            .arg(this_exe)
            .arg("w")
            .arg(request.id.to_string())
            .args(request.command)
            .spawn()
            .expect("XGE-Launcher: failed to launch process!");
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
    ::std::process::exit(exit_code);
}

fn execute_wrapped(id: u64, exe: &str, args: Vec<&String>) {
    let mut cmd = Command::new(&exe);
    for arg in args {
        cmd.arg(arg);
    }

    let maybe_output = cmd.output();
    match maybe_output {
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-7787);
            let stdout = str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
            let stderr = str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
            let output_str = stderr.to_owned() + stdout;

            report(id, exit_code, &output_str);
        }
        Err(e) => {
            report(
                id,
                -7787,
                &format!("XGE-Launcher: failed to execute process: {}", e),
            );
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args[1] == "client" {
        let stream = TcpStream::connect(&args[2]).expect("could not connect to XGE server!");
        accept_commands(stream);
    } else if args[1] == "w" {
        execute_wrapped(
            args[2].parse().unwrap(),
            &args[3],
            args.iter().skip(4).collect(),
        );
    } else {
        panic!("unknown parameter!");
    }
}
