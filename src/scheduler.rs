use crate::config;
use crate::report;
use crate::runnable::{TestInstance, TestInstanceCreator};
use crate::OutputPaths;
use futures::Future;
use std::process::Command;
use tokio::prelude::*;
use tokio_process::CommandExt;

pub struct RunConfig {
    pub verbose: bool,
    pub parallel: bool,
    pub xge: bool,
    pub repeat: usize,
    pub rerun_if_failed: usize,
}

pub fn run<'a>(
    input_paths: &config::InputPaths,
    tests: Vec<TestInstanceCreator>,
    output_paths: &crate::OutputPaths,
    run_config: &RunConfig,
) -> bool {
    let mut runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
    runtime
        .block_on(run_async(tests, &input_paths, &output_paths, &run_config))
        .unwrap()
}

#[derive(Debug, Clone)]
pub struct TestCommandResult {
    pub exit_code: i32,
    pub stdout: String,
}

fn run_async<'a>(
    tests: Vec<TestInstanceCreator>,
    input_paths: &'a config::InputPaths,
    output_paths: &'a OutputPaths,
    run_config: &'a RunConfig,
) -> impl Future<Item = bool, Error = ()> {
    let n = tests.len() * run_config.repeat;

    //let (xge_client_process, xge_socket) = xge_lib::xge();
    //let (xge_reader, mut xge_writer) = xge_socket.wait().unwrap().split();

    //let (xge_tests, local_tests): (Vec<_>, Vec<_>) = tests.into_iter().partition(|t| t.allow_xge);

    let n_workers = if run_config.parallel || run_config.xge {
        num_cpus::get()
    } else {
        1
    };

    let local_test_stream = stream::iter_ok::<_, ()>(/*local_tests*/ tests)
        .map(move |test_generator| {
            let test_instance = test_generator.instantiate();
            let timeout = test_instance.timeout.unwrap_or((60 * 60 * 24) as f32); // TODO: how to deal with no timeout?
            test_instance
                .run_async()
                .timeout(std::time::Duration::from_millis((timeout * 1000f32) as u64))
                .or_else(move |_| {
                    future::ok(TestCommandResult {
                        exit_code: 1,
                        stdout: format!("(test was killed by {} second timeout)", timeout),
                    })
                })
                .map(move |res| (test_instance, res))
        })
        .buffer_unordered(n_workers);

    //let running_tests: Vec<_> = xge_tests.iter().map(|t| t.instantiate()).collect();

    /*let xge_request_future = stream::iter_ok::<_, ()>(running_tests.clone())
        .fold(0u64, move |id, test_instance| {
            let request = xge_lib::StreamRequest {
                id,
                title: test_instance.test_id.id.clone(),
                cwd: test_instance.command.cwd.clone(),
                command: test_instance.command.command.clone(),
                local: false,
            };
            xge_writer
                .write((serde_json::to_string(&request).unwrap() + "\n").as_bytes())
                .unwrap();
            xge_writer.flush().unwrap();
            future::ok(id + 1)
        })
        .map(|_| {});
    tokio::spawn(xge_request_future);*/

    /*let stream_results = tokio::io::lines(std::io::BufReader::new(xge_reader))
    .filter_map(|line| {
        if line.starts_with("mwt ") {
            Some(serde_json::from_str::<xge_lib::StreamResult>(&line[4..]).unwrap())
        } else {
            None
        }
    })
    .map(|stream_result| {
        let result = TestCommandResult {
            exit_code: stream_result.exit_code,
            stdout: stream_result.stdout,
        };
        let test_instance = running_tests[stream_result.id as usize].clone();
        (test_instance, result)
    })
    .map_err(|_| ());*/

    let report = report::Report::new(
        &output_paths.out_dir,
        input_paths.testcases_root.to_str().unwrap(),
        run_config.verbose,
    );
    let result_stream = local_test_stream /*.select(stream_results)*/
        .fold(
            (true, report, 0, n),
            |(success, mut report, i, n), (test_instance, output)| {
                report.add(i + 1, n, test_instance, &output);
                let new_success = success && output.exit_code == 0;
                future::ok((new_success, report, i + 1, n))
            },
        );
    result_stream.map(|(success, _, _, _)| success)
}

impl TestInstance {
    fn run_async(&self) -> impl Future<Item = TestCommandResult, Error = ()> {
        let output = Command::new(&self.command.command[0])
            .args(self.command.command[1..].iter())
            .current_dir(&self.command.cwd)
            .output_async();
        let tmp_path = self.command.tmp_path.clone();
        output
            .map_err(|e| panic!("ERROR: failed to run test command: {}", e))
            .map(|output| {
                let exit_code = output.status.code().unwrap_or(-7787);
                let stdout =
                    std::str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
                let stderr =
                    std::str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
                let output_str = stderr.to_owned() + stdout;

                // cleanup
                if let Some(tmp_path) = tmp_path {
                    if tmp_path.is_dir() && std::fs::read_dir(&tmp_path).unwrap().next().is_none() {
                        std::fs::remove_dir(&tmp_path)
                            .expect("failed to clean up temporary directory!");
                    }
                }
                TestCommandResult {
                    exit_code,
                    stdout: output_str,
                }
            })
    }
}
