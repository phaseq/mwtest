use crate::config;
use crate::report;
use crate::runnable::{TestInstance, TestInstanceCreator};
use crate::OutputPaths;
use futures::{try_ready, Future};
use std::boxed::Box;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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

    let n_workers = if run_config.parallel || run_config.xge {
        num_cpus::get()
    } else {
        1
    };

    let result_stream = TestStream::new(LocalStream::new(n_workers), tests, &run_config);
    report_async(&input_paths, &output_paths, &run_config, result_stream, n)
}

fn run_report_xge(
    tests: Vec<TestInstanceCreator>,
    input_paths: &config::InputPaths,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) -> impl Future<Item = bool, Error = ()> {
    let n = tests.len() * run_config.repeat;

    let (xge_tests, local_tests): (Vec<_>, Vec<_>) = tests.into_iter().partition(|t| t.allow_xge);

    let n_workers = if run_config.parallel || run_config.xge {
        num_cpus::get()
    } else {
        1
    };

    let local_result_stream =
        TestStream::new(LocalStream::new(n_workers), local_tests, &run_config);
    let xge_result_stream = TestStream::new(XGEStream::new(), xge_tests, run_config);

    let result_stream = local_result_stream.select(xge_result_stream);

    report_async(&input_paths, &output_paths, &run_config, result_stream, n)
}

fn report_async<S>(
    input_paths: &config::InputPaths,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
    stream: S,
    n: usize,
) -> impl Future<Item = bool, Error = ()>
where
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

    fn get_timeout_duration(&self) -> std::time::Duration {
        // TODO: is there a more elegant way to handle this?
        let timeout = self.timeout.unwrap_or((60 * 60 * 24) as f32);
        std::time::Duration::from_millis((timeout * 1000f32) as u64)
    }
}

struct RepeatableTest {
    creator: TestInstanceCreator,
    n_runs: AtomicUsize,
}

impl RepeatableTest {
    fn new(creator: TestInstanceCreator) -> RepeatableTest {
        RepeatableTest {
            creator,
            n_runs: AtomicUsize::new(0),
        }
    }

    fn increase_run_count(&self) {
        self.n_runs.fetch_add(1, Ordering::SeqCst);
    }

    fn count_runs(&self) -> usize {
        self.n_runs.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
struct RepeatableTestInstance {
    creator: Arc<RepeatableTest>,
    instance: TestInstance,
}

impl RepeatableTestInstance {
    fn new(test: Arc<RepeatableTest>) -> RepeatableTestInstance {
        let instance = test.creator.instantiate();
        RepeatableTestInstance {
            creator: test,
            instance: instance,
        }
    }
}

trait RepeatableTestStream:
    Stream<Item = (RepeatableTestInstance, TestCommandResult), Error = ()>
{
    fn enqueue(&mut self, instance: RepeatableTestInstance);
}

struct TestStream<S>
where
    S: RepeatableTestStream,
{
    stream: S,
    n_retries: u64,
}

impl<S> TestStream<S>
where
    S: RepeatableTestStream,
{
    fn new(mut s: S, tests: Vec<TestInstanceCreator>, run_config: &RunConfig) -> TestStream<S> {
        tests.into_iter().for_each(|t| {
            s.enqueue(RepeatableTestInstance::new(Arc::new(RepeatableTest::new(
                t,
            ))))
        });

        TestStream {
            stream: s,
            n_retries: run_config.rerun_if_failed as u64,
        }
    }
}

impl<S> Stream for TestStream<S>
where
    S: RepeatableTestStream,
{
    type Item = (TestInstance, TestCommandResult);
    type Error = ();

    fn poll(&mut self) -> Poll<Option<(TestInstance, TestCommandResult)>, ()> {
        match self.stream.poll() {
            Ok(Async::Ready(result)) => {
                if let Some((instance, result)) = result {
                    instance.creator.increase_run_count();

                    // retry up to n_retries times
                    if result.exit_code != 0
                        && instance.creator.count_runs() < self.n_retries as usize
                    {
                        self.stream
                            .enqueue(RepeatableTestInstance::new(instance.creator.clone()));
                    }
                    Ok(Async::Ready(Some((instance.instance, result))))
                } else {
                    Ok(Async::Ready(None))
                }
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(_) => panic!("stream error"),
        }
    }
}

struct LocalStream {
    test_queue: std::collections::VecDeque<RepeatableTestInstance>,
    queue: stream::FuturesUnordered<AsyncTestInstance>,
    max: usize,
}

impl LocalStream {
    fn new(max_processes: usize) -> LocalStream {
        LocalStream {
            test_queue: std::collections::VecDeque::new(),
            queue: stream::FuturesUnordered::new(),
            max: max_processes,
        }
    }
}

impl Stream for LocalStream {
    type Item = (RepeatableTestInstance, TestCommandResult);
    type Error = ();

    fn poll(&mut self) -> Poll<Option<(RepeatableTestInstance, TestCommandResult)>, ()> {
        while self.queue.len() < self.max {
            if let Some(test) = self.test_queue.pop_front() {
                let timeout = test.instance.get_timeout_duration();
                let future = Box::new(test.instance.run_async().timeout(timeout).or_else(
                    move |_| {
                        future::ok(TestCommandResult {
                            exit_code: 1,
                            stdout: format!(
                                "(test was killed by {} second timeout)",
                                timeout.as_secs()
                            ),
                        })
                    },
                ));
                self.queue.push(AsyncTestInstance { test, future });
            } else {
                break;
            }
        }

        // Try polling a new future
        if let Some(val) = try_ready!(self.queue.poll()) {
            return Ok(Async::Ready(Some(val)));
        }

        // We're done
        Ok(Async::Ready(None))
    }
}

struct AsyncTestInstance {
    test: RepeatableTestInstance,
    future: Box<Future<Item = TestCommandResult, Error = ()> + Send>,
}

impl Future for AsyncTestInstance {
    type Item = (RepeatableTestInstance, TestCommandResult);
    type Error = ();

    fn poll(&mut self) -> Poll<(RepeatableTestInstance, TestCommandResult), ()> {
        match self.future.poll() {
            Ok(Async::Ready(r)) => {
                let res = (self.test.clone(), r);
                Ok(Async::Ready(res))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => Err(e),
        }
    }
}

impl RepeatableTestStream for LocalStream {
    fn enqueue(&mut self, instance: RepeatableTestInstance) {
        self.test_queue.push_back(instance);
    }
}

struct XGEStream {
    test_queue: Vec<RepeatableTestInstance>,
    #[allow(dead_code)]
    child: tokio_process::Child, // stored here to keep reader alive
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
    n_queued: u64,
}

impl XGEStream {
    fn new() -> XGEStream {
        let (mut child, xge_socket) = xge_lib::xge();
        let xge_socket = LinesCodec::new().framed(xge_socket.wait().unwrap());
        let (writer, _) = xge_socket.split();

        let stdout = child.stdout().take().unwrap();
        let reader = tokio::io::lines(std::io::BufReader::new(stdout));

        XGEStream {
            test_queue: vec![],
            test_idx: 0,
            child,
            writer: None,
            sink: Some(writer),
            reader,
            n_queued: 0,
        }
    }

    fn poll_send(&mut self) -> Poll<Option<()>, ()> {
        loop {
            if let Some(ref mut writer) = self.writer {
                match writer.poll() {
                    Ok(Async::Ready(sink)) => {
                        if let Some(message) = self.next_message() {
                            // use sink to send next message
                            self.writer = Some(sink.send(message));
                            self.n_queued += 1;
                        } else {
                            // no more messages to send: move sink to storage
                            self.writer = None;
                            self.sink = Some(sink);
                            return Ok(Async::Ready(None));
                        }
                    }
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Err(e) => panic!("XGE write error: {}", e),
                }
            } else if let Some(message) = self.next_message() {
                // consume sink to send new message
                self.writer = Some(self.sink.take().unwrap().send(message));
                self.n_queued += 1;
            } else {
                return Ok(Async::Ready(None));
            }
        }
    }

    fn next_message(&mut self) -> Option<String> {
        if self.test_idx < self.test_queue.len() {
            let test_instance = &self.test_queue[self.test_idx].instance;
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

    fn poll_receive(&mut self) -> Poll<Option<(RepeatableTestInstance, TestCommandResult)>, ()> {
        loop {
            match self.reader.poll() {
                Ok(Async::Ready(line)) => {
                    if let Some(line) = line {
                        if line != "mwt done" && line.starts_with("mwt ") {
                            let result = self.handle_received_message(line);
                            return Ok(Async::Ready(Some(result)));
                        }
                        continue;
                    } else {
                        // no more lines available (pipe has been closed)
                        return Ok(Async::Ready(None));
                    }
                }
                Ok(Async::NotReady) => {
                    if self.n_queued == 0 {
                        return Ok(Async::Ready(None));
                    } else {
                        return Ok(Async::NotReady);
                    }
                }
                Err(e) => panic!("XGE read error: {}", e),
            }
        }
    }

    fn handle_received_message(
        &mut self,
        line: String,
    ) -> (RepeatableTestInstance, TestCommandResult) {
        self.n_queued -= 1;
        let stream_result = serde_json::from_str::<xge_lib::StreamResult>(&line[4..]).unwrap();

        let test = self.test_queue[stream_result.id as usize].clone();

        let result = TestCommandResult {
            exit_code: stream_result.exit_code,
            stdout: stream_result.stdout,
        };
        (test, result)
    }
}

impl Stream for XGEStream {
    type Item = (RepeatableTestInstance, TestCommandResult);
    type Error = ();

    fn poll(&mut self) -> Poll<Option<(RepeatableTestInstance, TestCommandResult)>, ()> {
        self.poll_send().expect("XGE send failed");
        self.poll_receive()
    }
}

impl RepeatableTestStream for XGEStream {
    fn enqueue(&mut self, instance: RepeatableTestInstance) {
        self.test_queue.push(instance);
    }
}
