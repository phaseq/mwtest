use crate::config;
use crate::report;
#[cfg(test)]
use crate::runnable::TestCommand;
use crate::runnable::{TestInstance, TestInstanceCreator};
use crate::OutputPaths;
//use futures::io::AsyncBufReadExt;
use futures::stream::StreamExt;
//use std::io::Cursor;
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
        input_paths.testcases_root.to_str().unwrap(),
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
    let n = tests.len() * run_config.repeat;

    let n_workers = if run_config.parallel || run_config.xge {
        num_cpus::get()
    } else {
        1
    };

    if run_config.rerun_if_failed == 0 {
        // repeat each test exactly `repeat` times
        let instances = tests
            .into_iter()
            .flat_map(|tic| (0..run_config.repeat).map(move |_| tic.instantiate()));
        futures::stream::iter(instances)
            .map(|instance| {
                async {
                    let result = instance.run_async().await;
                    (instance, result)
                }
            })
            .buffer_unordered(n_workers)
            .fold(
                (true, callback, 0, n),
                |(mut success, mut callback, i, n), (test_instance, result)| {
                    callback(i + 1, n, test_instance, &result);
                    success &= result.exit_code == 0;
                    futures::future::ready((success, callback, i + 1, n))
                },
            )
            .map(|(success, _, _, _)| success)
            .await
    } else {
        // repeat each test up to `rerun_if_failed` times (or less, if it succeeds earlier)
        futures::stream::iter(tests)
            .map(|tic| {
                async {
                    let mut results = vec![];
                    for _ in 0..=run_config.rerun_if_failed {
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
                    futures::future::ready((success, callback, i + 1, n))
                },
            )
            .map(|(success, _, _, _)| success)
            .await
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
        input_paths.testcases_root.to_str().unwrap(),
        run_config.verbose,
    );
    run_xge(tests, run_config, |i, n, test_instance, result| {
        report.add(i, n, test_instance, &result)
    })
    .await
}

async fn run_xge<F: FnMut(usize, usize, TestInstance, &TestCommandResult)>(
    tests: Vec<TestInstanceCreator>,
    run_config: &RunConfig,
    mut callback: F,
) -> bool {
    let n = tests.len() * run_config.repeat;

    // TODO: non-parallelizable tests
    //let (xge_tests, local_tests): (Vec<_>, Vec<_>) = tests.into_iter().partition(|t| t.allow_xge);

    let (mut _child, xge_socket) = xge_lib::xge();
    let (mut writer, mut reader) = LinesCodec::new()
        .framed(xge_socket.await.expect("remote client failed to connect"))
        .split();
    if run_config.rerun_if_failed == 0 {
        // repeat each test exactly `repeat` times
        let instances: Vec<_> = tests
            .into_iter()
            .flat_map(|tic| (0..run_config.repeat).map(move |_| (tic.allow_xge, tic.instantiate())))
            .collect();
        let sender = async {
            for i in 0..instances.len() {
                let (allow_xge, instance) = &instances[i];
                let request = xge_lib::StreamRequest {
                    id: i as u64,
                    title: instance.test_id.id.clone(),
                    cwd: instance.command.cwd.clone(),
                    command: instance.command.command.clone(),
                    local: !allow_xge,
                };
                let message = serde_json::to_string(&request).unwrap();
                writer.send(message).await.expect("when does this fail?");
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
                    callback(
                        i + 1,
                        n,
                        instances[stream_result.id as usize].1.clone(),
                        &result,
                    );
                    i += 1;
                }
            }
            success
        };
        futures::future::join(sender, receiver).await.1
    } else {
        // TODO: repeat failed tests
        false
    }
}

impl TestInstance {
    async fn run_async(&self) -> TestCommandResult {
        let output = Command::new(&self.command.command[0])
            .args(self.command.command[1..].iter())
            .current_dir(&self.command.cwd)
            .output_async()
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
        std::time::Duration::from_millis((timeout * 1000f32) as u64)
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

    #[test]
    fn test_run_local_once() {
        let (success, count) = count_results(
            make_whoami_instance(),
            RunConfig {
                verbose: false,
                parallel: true,
                xge: false,
                repeat: 1,
                rerun_if_failed: 0,
            },
        );
        assert_eq!(success, true);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_run_local_repeat() {
        let (success, count) = count_results(
            make_whoami_instance(),
            RunConfig {
                verbose: false,
                parallel: true,
                xge: false,
                repeat: 10,
                rerun_if_failed: 0,
            },
        );
        assert_eq!(success, true);
        assert_eq!(count, 10);
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
                repeat: 10,
                rerun_if_failed: 0,
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
                repeat: 1,
                rerun_if_failed: 0,
            },
        );
        // tests should finish in the order of their expected duration
        assert_eq!(ids, vec!["Some(0.001)", "Some(0.05)", "Some(0.1)"]);
    }
}
