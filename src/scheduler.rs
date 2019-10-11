use crate::config;
use crate::report;
#[cfg(test)]
use crate::runnable::TestCommand;
use crate::runnable::{TestInstance, TestInstanceCreator};
use crate::OutputPaths;
use futures::future::{self, Either};
use futures::select;
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::codec::{Decoder, FramedRead, LinesCodec};
use tokio::prelude::*;
use tokio_process::Command;

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub verbose: bool,
    pub parallel: bool,
    pub xge: bool,
    pub repeat: RepeatStrategy,
}

#[derive(Debug, Clone)]
pub enum RepeatStrategy {
    Repeat(usize),
    RepeatIfFailed(usize),
}

pub fn run(
    input_paths: &config::InputPaths,
    tests: Vec<TestInstanceCreator>,
    output_paths: &crate::OutputPaths,
    run_config: &RunConfig,
) -> bool {
    let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
    if run_config.xge {
        runtime.block_on(async {
            run_report_xge(tests, &input_paths, &output_paths, &run_config).await
            //.unwrap_or(false)
        })
    } else {
        runtime.block_on(async {
            run_report_local(tests, &input_paths, &output_paths, &run_config).await
        })
    }
}

#[derive(Debug, Clone)]
pub struct TestCommandResult {
    pub exit_code: i32,
    pub stdout: String,
}

async fn run_report_local(
    tests: Vec<TestInstanceCreator>,
    input_paths: &config::InputPaths,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) -> bool {
    let mut report = report::Report::new(
        &output_paths.out_dir,
        input_paths
            .testcases_dir
            .to_str()
            .expect("Couldn't convert path to string!"),
        run_config.verbose,
    );
    run_local(tests, run_config, |i, n, test_instance, result| {
        report.add(i, n, test_instance, &result)
    })
    .await
}

async fn run_local<F: FnMut(usize, usize, TestInstance, &TestCommandResult)>(
    tests: Vec<TestInstanceCreator>,
    run_config: &RunConfig,
    callback: F,
) -> bool {
    let n = tests.len()
        * match run_config.repeat {
            RepeatStrategy::Repeat(n) => n,
            RepeatStrategy::RepeatIfFailed(_) => 1,
        };

    let n_workers = if run_config.parallel || run_config.xge {
        num_cpus::get()
    } else {
        1
    };

    match run_config.repeat {
        RepeatStrategy::Repeat(repeat) => {
            // repeat each test exactly `repeat` times
            let instances = tests
                .into_iter()
                .flat_map(|tic| (0..repeat).map(move |_| tic.instantiate()));
            futures::stream::iter(instances)
                .map(|instance| {
                    async {
                        let result = instance.run_async().await;
                        (instance, result)
                    }
                })
                .buffer_unordered(n_workers)
                .fold(
                    (true, callback, 0, repeat),
                    |(mut success, mut callback, i, repeat), (test_instance, result)| {
                        callback(i + 1, repeat, test_instance, &result);
                        success &= result.exit_code == 0;
                        future::ready((success, callback, i + 1, repeat))
                    },
                )
                .map(|(success, _, _, _)| success)
                .await
        }
        RepeatStrategy::RepeatIfFailed(repeat_if_failed) => {
            // repeat each test up to `repeat_if_failed` times (or less, if it succeeds earlier)
            futures::stream::iter(tests)
                .map(|tic| {
                    async {
                        let mut results = vec![];
                        for _ in 0..=repeat_if_failed {
                            let instance = tic.instantiate();
                            let result = instance.run_async().await;
                            let success = result.exit_code == 0;
                            results.push((instance, result));
                            if success {
                                break;
                            }
                        }
                        (tic, results)
                    }
                })
                .buffer_unordered(n_workers)
                .fold(
                    (true, callback, 0, n),
                    |(mut success, mut callback, i, n), (_tic, results)| {
                        for (test_instance, result) in results {
                            callback(i + 1, n, test_instance, &result);
                            success &= result.exit_code == 0;
                        }
                        future::ready((success, callback, i + 1, n))
                    },
                )
                .map(|(success, _, _, _)| success)
                .await
        }
    }
}

async fn run_report_xge(
    tests: Vec<TestInstanceCreator>,
    input_paths: &config::InputPaths,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) -> bool {
    let mut report = report::Report::new(
        &output_paths.out_dir,
        input_paths
            .testcases_dir
            .to_str()
            .expect("Couldn't convert path to string!"),
        run_config.verbose,
    );
    run_xge(
        tests,
        run_config,
        |i, n, test_instance, result| report.add(i, n, test_instance, &result),
        false,
    )
    .await
}

async fn run_xge<F: FnMut(usize, usize, TestInstance, &TestCommandResult)>(
    tests: Vec<TestInstanceCreator>,
    run_config: &RunConfig,
    mut callback: F,
    mock: bool,
) -> bool {
    let n = tests.len()
        * match run_config.repeat {
            RepeatStrategy::Repeat(n) => n,
            RepeatStrategy::RepeatIfFailed(_) => 1,
        };
    let (mut child, xge_socket) = if mock {
        let (c, s) = xge_lib::xge_mock();
        (c, Either::Left(s))
    } else {
        let (c, s) = xge_lib::xge();
        (c, Either::Right(s))
    };
    let (mut writer, mut _reader) = LinesCodec::new()
        .framed(xge_socket.await.expect("remote client failed to connect"))
        .split();
    let mut reader = FramedRead::new(
        child.stdout().take().expect("failed to connect to stdout"),
        LinesCodec::new(),
    );
    match run_config.repeat {
        RepeatStrategy::Repeat(repeat) => {
            // repeat each test exactly `repeat` times
            let instances: Vec<_> = tests
                .into_iter()
                .flat_map(|tic| (0..repeat).map(move |_| (tic.allow_xge, tic.instantiate())))
                .collect();
            let sender = async {
                for (i, (allow_xge, instance)) in instances.iter().enumerate() {
                    let request = xge_lib::StreamRequest {
                        id: i as u64,
                        title: instance.test_id.id.clone(),
                        cwd: instance.command.cwd.clone(),
                        command: instance.command.command.clone(),
                        local: !allow_xge,
                    };
                    let message = serde_json::to_string(&request).unwrap();
                    writer.send(message).await.unwrap();
                }
            };
            let receiver = async {
                let mut i = 0;
                let mut success = true;

                while let Some(line) = reader.next().await {
                    let line = line.unwrap();
                    if line.starts_with("mwt ") {
                        if line.starts_with("mwt done") {
                            break;
                        }
                        let stream_result =
                            serde_json::from_str::<xge_lib::StreamResult>(&line[4..]).unwrap();
                        let result = TestCommandResult {
                            exit_code: stream_result.exit_code,
                            stdout: stream_result.stdout,
                        };
                        success &= stream_result.exit_code == 0;
                        i += 1;
                        callback(
                            i,
                            n,
                            instances[stream_result.id as usize].1.clone(),
                            &result,
                        );
                        if i == instances.len() {
                            break;
                        }
                    }
                }
                success
            };
            futures::future::join(sender, receiver).await.1
        }
        RepeatStrategy::RepeatIfFailed(repeat_if_failed) => {
            // repeat failed tests

            let queue = Mutex::new(TestQueue::new(tests));
            let mut i = 0;
            let mut done = false;
            let mut overall_success = true;
            while !done {
                let mut line = reader.next().map(|line| {
                    let line = line.unwrap().unwrap();
                    if line.starts_with("mwt ") {
                        if line.starts_with("mwt done") {
                            done = true;
                            return;
                        }
                        let stream_result =
                            serde_json::from_str::<xge_lib::StreamResult>(&line[4..]).unwrap();
                        let result = TestCommandResult {
                            exit_code: stream_result.exit_code,
                            stdout: stream_result.stdout,
                        };
                        let success = stream_result.exit_code == 0;
                        let (test_instance, is_done) = {
                            queue.lock().unwrap().return_response(
                                stream_result.id as usize,
                                success,
                                repeat_if_failed,
                            )
                        };
                        overall_success &= success;
                        i += 1;
                        callback(i, n, test_instance, &result);
                        done = is_done;
                    }
                });

                let next_request = { queue.lock().unwrap().next_request() };
                match next_request {
                    None => line.await,
                    Some(request) => {
                        let mut send_future = {
                            let message = serde_json::to_string(&request).unwrap();
                            writer.send(message).fuse()
                        };

                        loop {
                            select! {
                                _ = send_future => break,
                                _ = line => {}
                            };
                        }
                    }
                }
            }
            overall_success
        }
    }
}

struct TestQueue {
    indices: VecDeque<usize>,
    creators: Vec<(TestInstanceCreator, Option<TestInstance>, usize)>,
    in_flight: usize,
}
impl TestQueue {
    fn new(tests: Vec<TestInstanceCreator>) -> TestQueue {
        TestQueue {
            indices: (0..tests.len()).collect(),
            creators: tests.into_iter().map(|tic| (tic, None, 0)).collect(),
            in_flight: 0,
        }
    }
    fn next_request(&mut self) -> Option<xge_lib::StreamRequest> {
        match self.indices.pop_front() {
            Some(send_idx) => {
                let (tic, ti, _count) = &mut self.creators[send_idx];
                let instance = tic.instantiate();
                *ti = Some(instance.clone());
                self.in_flight += 1;

                Some(xge_lib::StreamRequest {
                    id: send_idx as u64,
                    title: instance.test_id.id.clone(),
                    cwd: instance.command.cwd.clone(),
                    command: instance.command.command.clone(),
                    local: !tic.allow_xge,
                })
            }
            None => None,
        }
    }
    fn return_response(
        &mut self,
        id: usize,
        success: bool,
        repeat_if_failed: usize,
    ) -> (TestInstance, bool) {
        self.in_flight -= 1;
        if !success {
            let (_tic, _ti, count) = &mut self.creators[id];
            if *count < repeat_if_failed {
                *count += 1;
                self.indices.push_back(id);
            }
        }
        (self.creators[id].1.take().unwrap(), self.is_done())
    }
    fn is_done(&self) -> bool {
        self.in_flight == 0 && self.indices.is_empty()
    }
}

impl TestInstance {
    async fn run_async(&self) -> TestCommandResult {
        println!("{:?}", self.command.command);
        let output = Command::new(&self.command.command[0])
            .args(self.command.command[1..].iter())
            .current_dir(&self.command.cwd)
            .output()
            .timeout(self.get_timeout_duration())
            .await;
        let output = match output {
            Ok(output) => output,
            Err(_) => {
                return TestCommandResult {
                    exit_code: 1,
                    stdout: format!(
                        "[mwtest] terminated because {} second timeout was reached!",
                        self.get_timeout_duration().as_secs()
                    ),
                };
            }
        };
        let output = match output {
            Ok(output) => output,
            Err(e) => {
                return TestCommandResult {
                    exit_code: 1,
                    stdout: format!(
                        "[mwtest] error while trying to start test: {}",
                        e.to_string()
                    ),
                };
            }
        };
        let tmp_path = self.command.tmp_path.clone();
        let exit_code = output.status.code().unwrap_or(-7787);
        let stdout = std::str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
        let stderr = std::str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
        let output_str = stderr.to_owned() + stdout;

        // cleanup
        if let Some(tmp_path) = tmp_path {
            if tmp_path.is_dir() && std::fs::read_dir(&tmp_path).unwrap().next().is_none() {
                std::fs::remove_dir(&tmp_path).expect("failed to clean up temporary directory!");
            }
        }
        TestCommandResult {
            exit_code,
            stdout: output_str,
        }
    }

    fn get_timeout_duration(&self) -> std::time::Duration {
        // TODO: is there a more elegant way to handle this?
        let timeout = self.timeout.unwrap_or((60 * 60 * 24) as f32);
        std::time::Duration::from_millis((timeout * 1000.0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_whoami_instance() -> Vec<TestInstanceCreator> {
        let command_generator = Box::new(move || TestCommand {
            command: vec!["whoami".to_owned()],
            cwd: ".".to_owned(),
            tmp_path: None,
        });
        let test = TestInstanceCreator {
            app_name: "test".to_owned(),
            test_id: crate::TestId {
                id: "test_id".to_owned(),
                rel_path: None,
            },
            allow_xge: false,
            timeout: None,
            command_generator,
        };
        vec![test]
    }

    fn make_failing_ls_instance() -> Vec<TestInstanceCreator> {
        let command_generator = Box::new(move || TestCommand {
            command: vec!["ls".to_string(), "/nonexistent-file".to_string()],
            cwd: ".".to_owned(),
            tmp_path: None,
        });
        let test = TestInstanceCreator {
            app_name: "test".to_owned(),
            test_id: crate::TestId {
                id: "test_id".to_owned(),
                rel_path: None,
            },
            allow_xge: false,
            timeout: None,
            command_generator,
        };
        vec![test]
    }

    fn count_results(tests: Vec<TestInstanceCreator>, run_config: RunConfig) -> (bool, usize) {
        let mut count = 0;
        let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
        let success = runtime.block_on(async {
            run_local(tests, &run_config, |_i, _n, _test_instance, _result| {
                count += 1;
            })
            .await
        });
        (success, count)
    }

    fn count_results_xge(tests: Vec<TestInstanceCreator>, run_config: RunConfig) -> (bool, usize) {
        let mut count = 0;
        let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
        let success = runtime.block_on(async {
            run_xge(
                tests,
                &run_config,
                |_i, _n, _test_instance, _result| {
                    count += 1;
                },
                true,
            )
            .await
        });
        (success, count)
    }

    fn collect_results(
        tests: Vec<TestInstanceCreator>,
        run_config: RunConfig,
    ) -> (bool, Vec<String>) {
        let mut ids = vec![];
        let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
        let success = runtime.block_on(async {
            run_local(tests, &run_config, |_i, _n, test_instance, _result| {
                ids.push(test_instance.test_id.id);
            })
            .await
        });
        (success, ids)
    }

    /*fn collect_results_xge(
        tests: Vec<TestInstanceCreator>,
        run_config: RunConfig,
    ) -> (bool, Vec<String>) {
        let mut ids = vec![];
        let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
        let success = runtime.block_on(async {
            run_xge(
                tests,
                &run_config,
                |_i, _n, test_instance, _result| {
                    ids.push(test_instance.test_id.id);
                },
                true,
            )
            .await
        });
        (success, ids)
    }*/

    #[test]
    fn test_run_local_once() {
        let (success, count) = count_results(
            make_whoami_instance(),
            RunConfig {
                verbose: false,
                parallel: true,
                xge: false,
                repeat: RepeatStrategy::Repeat(1),
            },
        );
        assert_eq!(success, true);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_run_local_once_xge() {
        let (success, count) = count_results_xge(
            make_whoami_instance(),
            RunConfig {
                verbose: false,
                parallel: true,
                xge: true,
                repeat: RepeatStrategy::Repeat(1),
            },
        );
        assert_eq!(success, true);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_run_repeat() {
        let (success, count) = count_results(
            make_whoami_instance(),
            RunConfig {
                verbose: false,
                parallel: true,
                xge: false,
                repeat: RepeatStrategy::Repeat(10),
            },
        );
        assert_eq!(success, true);
        assert_eq!(count, 10);
    }

    #[test]
    fn test_run_repeat_xge() {
        let (success, count) = count_results_xge(
            make_whoami_instance(),
            RunConfig {
                verbose: false,
                parallel: true,
                xge: true,
                repeat: RepeatStrategy::Repeat(10),
            },
        );
        assert_eq!(success, true);
        assert_eq!(count, 10);
    }

    #[test]
    fn test_run_repeat_if_failed() {
        let (success, count) = count_results(
            make_failing_ls_instance(),
            RunConfig {
                verbose: false,
                parallel: false,
                xge: false,
                repeat: RepeatStrategy::RepeatIfFailed(5),
            },
        );
        assert_eq!(success, false);
        assert_eq!(count, 6);
    }

    #[test]
    fn test_run_repeat_if_failed_xge() {
        let (success, count) = count_results_xge(
            make_failing_ls_instance(),
            RunConfig {
                verbose: false,
                parallel: false,
                xge: true,
                repeat: RepeatStrategy::RepeatIfFailed(5),
            },
        );
        assert_eq!(success, false);
        assert_eq!(count, 6);
    }

    fn make_sleep_instance(timeout: Option<f32>) -> TestInstanceCreator {
        let command_generator = Box::new(move || TestCommand {
            command: vec!["sleep".to_owned(), "1".to_owned()],
            cwd: ".".to_owned(),
            tmp_path: None,
        });
        TestInstanceCreator {
            app_name: "test".to_owned(),
            test_id: crate::TestId {
                id: format!("{:?}", timeout),
                rel_path: None,
            },
            allow_xge: false,
            timeout,
            command_generator,
        }
    }

    #[test]
    fn test_run_local_timeout_triggers() {
        let (success, count) = count_results(
            vec![make_sleep_instance(Some(0.001f32))],
            RunConfig {
                verbose: false,
                parallel: true,
                xge: false,
                repeat: RepeatStrategy::Repeat(10),
            },
        );
        assert_eq!(success, false);
        assert_eq!(count, 10);
    }

    #[test]
    fn test_run_local_out_of_order() {
        let tests = [0.1f32, 0.001f32, 0.05f32]
            .into_iter()
            .map(|t| make_sleep_instance(Some(*t)))
            .collect();

        let (_, ids) = collect_results(
            tests,
            RunConfig {
                verbose: false,
                parallel: true,
                xge: false,
                repeat: RepeatStrategy::Repeat(1),
            },
        );
        // tests should finish in the order of their expected duration
        assert_eq!(ids, vec!["Some(0.001)", "Some(0.05)", "Some(0.1)"]);
    }
}
