use crate::config;
use crate::report;
use crate::runnable::{TestInstance, TestInstanceCreator};
use crate::OutputPaths;
use futures::Future;
use std::process::Command;
use tokio::codec::{Decoder, LinesCodec};
use tokio::prelude::*;
use tokio_process::CommandExt;

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub verbose: bool,
    pub parallel: bool,
    pub xge: bool,
    pub repeat: usize,
    pub rerun_if_failed: usize,
}

pub fn run(
    input_paths: &config::InputPaths,
    tests: Vec<TestInstanceCreator>,
    output_paths: &crate::OutputPaths,
    run_config: &RunConfig,
) -> bool {
    let mut runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
    if run_config.xge {
        runtime
            .block_on(run_report_xge(
                tests,
                &input_paths,
                &output_paths,
                &run_config,
            ))
            .unwrap()
    } else {
        runtime
            .block_on(run_report_local(
                tests,
                &input_paths,
                &output_paths,
                &run_config,
            ))
            .unwrap()
    }
}

#[derive(Debug, Clone)]
pub struct TestCommandResult {
    pub exit_code: i32,
    pub stdout: String,
}

fn run_report_local(
    tests: Vec<TestInstanceCreator>,
    input_paths: &config::InputPaths,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) -> impl Future<Item = bool, Error = ()> {
    let n = tests.len() * run_config.repeat;

    let result_stream = to_local_stream(tests, &run_config);
    report_async(
        &input_paths,
        &output_paths,
        &run_config,
        future::ok(()),
        result_stream,
        n,
    )
}

fn run_report_xge(
    tests: Vec<TestInstanceCreator>,
    input_paths: &config::InputPaths,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) -> impl Future<Item = bool, Error = ()> {
    let n = tests.len() * run_config.repeat;

    let (xge_tests, local_tests): (Vec<_>, Vec<_>) = tests.into_iter().partition(|t| t.allow_xge);
    let local_result_stream = to_local_stream(local_tests, &run_config);

    let xge_result_stream = XGEStream::new(xge_tests, run_config);
    //let (xge_future, xge_result_stream) = to_xge_stream(xge_tests, run_config);

    let result_stream = local_result_stream.select(xge_result_stream);
    report_async(
        &input_paths,
        &output_paths,
        &run_config,
        //xge_future,
        future::ok(()),
        result_stream,
        n,
    )
}

fn report_async<F, S>(
    input_paths: &config::InputPaths,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
    future: F,
    stream: S,
    n: usize,
) -> impl Future<Item = bool, Error = ()>
where
    F: Future<Item = (), Error = ()>,
    S: Stream<Item = (TestInstance, TestCommandResult), Error = ()>,
{
    let report = report::Report::new(
        &output_paths.out_dir,
        input_paths.testcases_root.to_str().unwrap(),
        run_config.verbose,
    );
    let result_stream = stream.fold(
        (true, report, 0, n),
        |(success, mut report, i, n), (test_instance, output)| {
            report.add(i + 1, n, test_instance, &output);
            let new_success = success && output.exit_code == 0;
            future::ok((new_success, report, i + 1, n))
        },
    );
    result_stream
        .map(|(success, _, _, _)| success)
        .join(future.map(|_| true))
        .map(|(success, _)| success)
}

fn to_local_stream(
    tests: Vec<TestInstanceCreator>,
    run_config: &RunConfig,
) -> impl Stream<Item = (TestInstance, TestCommandResult), Error = ()> {
    let n_workers = if run_config.parallel || run_config.xge {
        num_cpus::get()
    } else {
        1
    };

    RepeatedTestStream::new(tests, run_config)
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

/*fn to_xge_stream(
    tests: Vec<TestInstanceCreator>,
    run_config: &RunConfig,
) -> (
    impl Future<Item = (), Error = ()>,
    impl Stream<Item = (TestInstance, TestCommandResult), Error = ()>,
) {
    let (mut xge_client_process, xge_socket) = xge_lib::xge();
    let xge_socket = LinesCodec::new().framed(xge_socket.wait().unwrap());
    let (xge_writer, _xge_reader) = xge_socket.split();

    let running_tests = RepeatedTestStream::new(tests, run_config)
        .collect()
        .wait()
        .unwrap();

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
            serde_json::to_string(&request).unwrap()
        })
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "oh no!"))
        .forward(xge_writer)
        .map_err(|e| panic!("error while sending to XGE server: {}", e))
        .map(|_| ());

    let stdout = xge_client_process.stdout().take().unwrap();
    let xge_result_stream = tokio::io::lines(std::io::BufReader::new(stdout))
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
        .map_err(|e| panic!("failed to get XGE stream: {}", e));

    let xge_future = xge_requests_future
        .join(xge_client_process.wait_with_output())
        .map(|(_, _)| ())
        .map_err(|e| panic!("xge_future: {}", e));
    (xge_future, xge_result_stream)
}*/

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

struct RepeatedTestStream {
    tests: Vec<TestInstanceCreator>,
    test_idx: usize,
    i: usize,
    run_config: RunConfig,
}
impl RepeatedTestStream {
    fn new(tests: Vec<TestInstanceCreator>, run_config: &RunConfig) -> RepeatedTestStream {
        RepeatedTestStream {
            tests,
            test_idx: 0,
            i: 0,
            run_config: run_config.clone(),
        }
    }
}
impl Stream for RepeatedTestStream {
    type Item = TestInstance;
    type Error = ();

    fn poll(&mut self) -> Result<Async<Option<TestInstance>>, ()> {
        if self.test_idx >= self.tests.len() {
            Ok(Async::Ready(None))
        } else if self.i < self.run_config.repeat {
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

struct XGEStream {
    test_creators: Vec<(TestInstanceCreator, u64)>,
    test_queue: Vec<(TestInstance, usize)>,
    child: tokio_process::Child,
    writer: Option<
        futures::sink::Send<
            futures::stream::SplitSink<
                tokio::codec::Framed<tokio::net::TcpStream, tokio::codec::LinesCodec>,
            >,
        >,
    >,
    sink: Option<
        futures::stream::SplitSink<
            tokio::codec::Framed<tokio::net::TcpStream, tokio::codec::LinesCodec>,
        >,
    >,
    reader: tokio::io::Lines<std::io::BufReader<tokio_process::ChildStdout>>,
    test_idx: usize,
    n_retries: u64,
}
impl XGEStream {
    fn new(tests: Vec<TestInstanceCreator>, run_config: &RunConfig) -> XGEStream {
        let (mut child, xge_socket) = xge_lib::xge();
        let xge_socket = LinesCodec::new().framed(xge_socket.wait().unwrap());
        let (writer, _) = xge_socket.split();

        let mut test_queue = vec![];
        let mut i = 0;
        for test_creator in &tests {
            for _ in 0..run_config.repeat {
                test_queue.push((test_creator.instantiate(), i));
            }
            i += 1;
        }

        let test_creators: Vec<(TestInstanceCreator, u64)> =
            tests.into_iter().map(move |t| (t, 0)).collect();

        let stdout = child.stdout().take().unwrap();
        let reader = tokio::io::lines(std::io::BufReader::new(stdout));

        XGEStream {
            test_creators,
            test_queue,
            test_idx: 0,
            child,
            writer: None,
            sink: Some(writer),
            reader,
            n_retries: run_config.rerun_if_failed as u64,
        }
    }

    fn next_message(&mut self) -> Option<String> {
        if self.test_idx < self.test_queue.len() {
            let test_instance = &self.test_queue[self.test_idx].0;
            let request = xge_lib::StreamRequest {
                id: self.test_idx as u64,
                title: test_instance.test_id.id.clone(),
                cwd: test_instance.command.cwd.clone(),
                command: test_instance.command.command.clone(),
                local: false,
            };
            let message = serde_json::to_string(&request).unwrap();
            self.test_idx += 1;
            Some(message)
        } else {
            None
        }
    }
}
impl Stream for XGEStream {
    type Item = (TestInstance, TestCommandResult);
    type Error = ();

    fn poll(&mut self) -> Poll<Option<(TestInstance, TestCommandResult)>, ()> {
        // try to send next message
        if let Some(ref mut writer) = self.writer {
            match writer.poll() {
                Ok(Async::Ready(sink)) => {
                    if let Some(message) = self.next_message() {
                        // use sink to send next message
                        self.writer = Some(sink.send(message));
                    } else {
                        // no more messages to send: move sink to storage
                        self.writer = None;
                        self.sink = Some(sink);
                    }
                }
                Ok(Async::NotReady) => {}
                Err(e) => panic!("XGE write error: {}", e),
            }
        } else if let Some(message) = self.next_message() {
            // consume sink to send new message
            self.writer = Some(self.sink.take().unwrap().send(message));
        }

        // try to get new response
        match self.reader.poll() {
            Ok(Async::Ready(line)) => {
                if let Some(line) = line {
                    if line != "mwt done" && line.starts_with("mwt ") {
                        let stream_result =
                            serde_json::from_str::<xge_lib::StreamResult>(&line[4..]).unwrap();

                        // increase the test's run counter
                        let test_instance = self.test_queue[stream_result.id as usize].clone();
                        let mut test_creator = &mut self.test_creators[test_instance.1];
                        test_creator.1 += 1;

                        // retry up to n_retries times
                        if stream_result.exit_code != 0 && test_creator.1 < self.n_retries {
                            self.test_queue
                                .push((test_creator.0.instantiate(), test_instance.1));
                        }

                        let result = TestCommandResult {
                            exit_code: stream_result.exit_code,
                            stdout: stream_result.stdout,
                        };
                        return Ok(Async::Ready(Some((test_instance.0, result))));
                    }
                } else {
                    // no more lines available (pipe has been closed)
                    return Ok(Async::Ready(None));
                }
            }
            Ok(Async::NotReady) => {}
            Err(e) => panic!("XGE read error: {}", e),
        }

        // poll client process
        match self.child.poll() {
            Ok(Async::Ready(_exit_status)) => return Ok(Async::Ready(None)),
            Ok(Async::NotReady) => {}
            Err(e) => panic!("XGE client error: {}", e),
        }
        Ok(Async::NotReady)
    }
}
