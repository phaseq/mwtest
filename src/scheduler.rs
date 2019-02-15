use crate::config;
use crate::report;
use crate::runnable::{TestInstance, TestInstanceCreator};
use crate::OutputPaths;
use futures::Future;
use std::process::Command;
use tokio::codec::{Decoder, LinesCodec};
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

    let (xge_tests, local_tests): (Vec<_>, Vec<_>) = tests.into_iter().partition(|t| t.allow_xge);
    let local_result_stream = run_local_async(local_tests, &run_config);
    let (xge_future, xge_result_stream) = run_xge_async(xge_tests, run_config.repeat);

    let report = report::Report::new(
        &output_paths.out_dir,
        input_paths.testcases_root.to_str().unwrap(),
        run_config.verbose,
    );
    let result_stream = local_result_stream.select(xge_result_stream).fold(
        (true, report, 0, n),
        |(success, mut report, i, n), (test_instance, output)| {
            report.add(i + 1, n, test_instance, &output);
            let new_success = success && output.exit_code == 0;
            future::ok((new_success, report, i + 1, n))
        },
    );
    result_stream
        .map(|(success, _, _, _)| success)
        .join(xge_future.map(|_| true))
        .map(|(success, _)| success)
}

fn run_local_async<'a>(
    tests: Vec<TestInstanceCreator>,
    run_config: &'a RunConfig,
) -> impl Stream<Item = (TestInstance, TestCommandResult), Error = ()> {
    let n_workers = if run_config.parallel || run_config.xge {
        num_cpus::get()
    } else {
        1
    };

    //stream::iter_ok::<_, ()>(tests)
    TestIter::new(tests, run_config.repeat)
        .map(move |test_instance| {
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
        .buffer_unordered(n_workers)
}

fn run_xge_async<'a>(
    tests: Vec<TestInstanceCreator>,
    n_repeats: usize,
) -> (
    impl Future<Item = (), Error = ()>,
    impl Stream<Item = (TestInstance, TestCommandResult), Error = ()>,
) {
    let (xge_client_process, xge_socket) = xge_lib::xge();
    let xge_socket = LinesCodec::new().framed(xge_socket.wait().unwrap());
    let (xge_writer, xge_reader) = xge_socket.split();

    let running_tests = TestIter::new(tests, n_repeats).collect().wait().unwrap();

    let xge_requests_future = stream::iter_ok::<_, ()>(running_tests.clone())
        .zip(stream::iter_ok::<_, ()>(0..))
        .map(|(test_instance, id)| {
            let request = xge_lib::StreamRequest {
                id,
                title: test_instance.test_id.id.clone(),
                cwd: test_instance.command.cwd.clone(),
                command: test_instance.command.command.clone(),
                local: false,
            };
            serde_json::to_string(&request).unwrap() + "\n"
        })
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "oh no!"))
        .forward(xge_writer)
        .map(|_| ())
        .map_err(|e| panic!("error while sending to XGE server: {}", e));

    let xge_future = xge_requests_future
        .join(xge_client_process.map_err(|e| panic!("failed to run XGE server: {}", e)))
        .map(|(_, _)| ());

    let xge_result_stream = xge_reader
        .filter_map(|line| {
            if line == "mwt done" {
                None
            } else if line.starts_with("mwt ") {
                Some(serde_json::from_str::<xge_lib::StreamResult>(&line[4..]).unwrap())
            } else {
                None
            }
        })
        .map(move |stream_result| {
            let result = TestCommandResult {
                exit_code: stream_result.exit_code,
                stdout: stream_result.stdout,
            };
            let test_instance = running_tests[stream_result.id as usize].clone();
            (test_instance, result)
        })
        .map_err(|_| ());

    (xge_future, xge_result_stream)
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

struct TestIter {
    tests: Vec<TestInstanceCreator>,
    test_idx: usize,
    i: usize,
    n_repeats: usize,
}
impl TestIter {
    fn new(tests: Vec<TestInstanceCreator>, n_repeats: usize) -> TestIter {
        TestIter {
            tests,
            test_idx: 0,
            i: 0,
            n_repeats,
        }
    }
}
impl tokio::prelude::Stream for TestIter {
    type Item = TestInstance;
    type Error = ();

    fn poll(&mut self) -> Result<Async<Option<TestInstance>>, ()> {
        if self.test_idx >= self.tests.len() {
            Ok(Async::Ready(None))
        } else if self.i < self.n_repeats {
            let instance = self.tests[self.test_idx].instantiate();
            self.i += 1;
            Ok(Async::Ready(Some(instance)))
        } else {
            self.test_idx += 1;
            self.i = 0;
            self.poll()
        }
    }
}
