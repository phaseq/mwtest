use crate::config;
use crate::report;
use crate::report::Reportable;
#[cfg(test)]
use crate::runnable::{ExecutionStyle, TestCommand};
use crate::runnable::{TestGroup, TestInstance, TestInstanceCreator};
use crate::OutputPaths;
use futures::future::{self, Either};
use futures::select;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
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
    test_groups: Vec<TestGroup>,
    output_paths: &crate::OutputPaths,
    run_config: &RunConfig,
) -> bool {
    let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
    if run_config.xge {
        runtime.block_on(async {
            run_report_xge(test_groups, &input_paths, &output_paths, &run_config).await
            //.unwrap_or(false)
        })
    } else {
        runtime.block_on(async {
            run_report_local(test_groups, &input_paths, &output_paths, &run_config).await
        })
    }
}

#[derive(Debug, Clone)]
pub struct TestCommandResult {
    pub exit_code: i32,
    pub stdout: String,
}

async fn run_report_local(
    test_groups: Vec<TestGroup>,
    input_paths: &config::InputPaths,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) -> bool {
    let report = Arc::new(Mutex::new(report::Report::new(
        &output_paths.out_dir,
        input_paths
            .testcases_dir
            .to_str()
            .expect("Couldn't convert path to string!"),
        run_config.verbose,
    )));
    run_local(test_groups, run_config, report).await
}

async fn run_local(
    test_groups: Vec<TestGroup>,
    run_config: &RunConfig,
    report: Arc<Mutex<dyn Reportable>>,
) -> bool {
    let n_tests: usize = test_groups.iter().map(|g| g.tests.len()).sum();
    let n = n_tests
        * match run_config.repeat {
            RepeatStrategy::Repeat(n) => n,
            RepeatStrategy::RepeatIfFailed(_) => 1,
        };
    {
        report.lock().unwrap().expect_additional_tests(n);
    }

    let n_workers = if run_config.parallel || run_config.xge {
        num_cpus::get()
    } else {
        1
    };

    let tests: Vec<(Arc<TestGroup>, TestInstanceCreator)> = test_groups
        .into_iter()
        .flat_map(|mut group| match group.gtest_generator.take() {
            Some(gen) => vec![(Arc::new(group), gen)],
            None => {
                let tests: Vec<TestInstanceCreator> = group.tests.drain(0..).collect();
                let group = Arc::new(group);
                tests
                    .into_iter()
                    .map(|t| (group.clone(), t))
                    .collect::<Vec<_>>()
            }
        })
        .collect();

    match run_config.repeat {
        RepeatStrategy::Repeat(repeat) => {
            // repeat each test exactly `repeat` times
            let instances = tests
                .iter()
                .flat_map(|(group, tic)| (0..repeat).map(move |_| (group, tic.instantiate())));
            futures::stream::iter(instances)
                .map(|(group, instance)| {
                    let app_name = group.app_name.clone();
                    let timeout = group.get_timeout_duration();
                    let report = report.clone();
                    async move {
                        if instance.is_g_multitest {
                            run_gtest(instance, &app_name, report).await
                        } else {
                            let result = instance.run_async(timeout).await;
                            report.lock().unwrap().add(&app_name, instance, &result);
                            result.exit_code == 0
                        }
                    }
                })
                .buffer_unordered(n_workers)
                .fold(true, |overall_success, success| {
                    future::ready(overall_success && success)
                })
                .await
        }
        RepeatStrategy::RepeatIfFailed(repeat_if_failed) => {
            // repeat each test up to `repeat_if_failed` times (or less, if it succeeds earlier)
            futures::stream::iter(tests)
                .map(|(group, tic)| {
                    let app_name = group.app_name.clone();
                    let timeout = group.get_timeout_duration();
                    async move {
                        assert_eq!(tic.is_g_multitest, false);
                        let mut results = vec![];
                        for _ in 0..=repeat_if_failed {
                            let instance = tic.instantiate();
                            let result = instance.run_async(timeout).await;
                            let success = result.exit_code == 0;
                            results.push((instance, result));
                            if success {
                                break;
                            }
                        }
                        (app_name, tic, results)
                    }
                })
                .buffer_unordered(n_workers)
                .fold(
                    (true, report),
                    |(mut success, report), (app_name, _tic, results)| {
                        for (test_instance, result) in results {
                            report
                                .lock()
                                .unwrap()
                                .add(&app_name, test_instance, &result);
                            success &= result.exit_code == 0;
                        }
                        future::ready((success, report))
                    },
                )
                .map(|(success, _)| success)
                .await
        }
    }
}

async fn run_gtest(ti: TestInstance, app_name: &str, report: Arc<Mutex<dyn Reportable>>) -> bool {
    let mut child = Command::new(&ti.command.command[0])
        .args(ti.command.command[1..].iter())
        .current_dir(&ti.command.cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to launch command!");

    struct Pipe {
        line: String,
        active: bool,
    }
    let mut stdout = Pipe {
        line: String::new(),
        active: true,
    };
    let mut stderr = Pipe {
        line: String::new(),
        active: true,
    };

    let mut stdout_reader =
        tokio::io::BufReader::new(child.stdout().take().expect("Failed to open StdOut"));
    let mut stderr_reader =
        tokio::io::BufReader::new(child.stderr().take().expect("Failed to open StdErr"));

    let mut current_test = None;
    let mut current_output = String::new();
    let mut any_failed = false;
    loop {
        stdout.line.clear();
        stderr.line.clear();
        let mut stdout_fut = stdout_reader.read_line(&mut stdout.line).fuse();
        let mut stderr_fut = stderr_reader.read_line(&mut stderr.line).fuse();

        let (n_read, pipe) = match (stdout.active, stderr.active) {
            (true, true) => {
                select! {
                    n_read = stdout_fut => (n_read.unwrap(), &mut stdout),
                    n_read = stderr_fut => (n_read.unwrap(), &mut stderr),
                }
            }
            (true, false) => (stdout_fut.await.unwrap(), &mut stdout),
            (false, true) => (stderr_fut.await.unwrap(), &mut stderr),
            (false, false) => break,
        };

        if n_read == 0 {
            pipe.active = false;
            continue;
        }
        let line = &pipe.line;
        // [ RUN      ] RunLocal_OpenGLWrapper.GetVersionTwoContexts
        if line.starts_with("[ RUN      ]") {
            current_test = Some(line[13..].trim_end().to_string());
            current_output = line.clone();
        } else {
            current_output += &line;
            let mut success = None;
            // [       OK ] RunLocal_OpenGLWrapper.GetVersionTwoContexts (0 ms)
            if line.starts_with("[       OK ]") {
                success = Some(true);
            }
            // [  FAILED  ] RunLocal_OpenGLWrapper.GetVersionTwoContexts (0 ms)
            else if line.starts_with("[  FAILED  ]") {
                success = Some(false);
                any_failed = false;
            }
            if let Some(success) = success {
                let test_id = crate::TestId {
                    id: current_test.as_ref().unwrap().clone(),
                    rel_path: None,
                };
                let test_instance = TestInstance {
                    test_id,
                    command: crate::runnable::TestCommand {
                        command: vec![],
                        cwd: "".into(),
                        tmp_path: None,
                    },
                    is_g_multitest: false,
                };
                let exit_code = if success { 0 } else { 1 };
                let result = TestCommandResult {
                    exit_code,
                    stdout: current_output.clone(),
                };
                report.lock().unwrap().add(app_name, test_instance, &result);
            }
        }
    }
    any_failed
}

async fn run_report_xge(
    test_groups: Vec<TestGroup>,
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
    run_xge(test_groups, run_config, &mut report, false).await
}

async fn run_xge(
    test_groups: Vec<TestGroup>,
    run_config: &RunConfig,
    report: &mut dyn Reportable,
    mock: bool,
) -> bool {
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
    let (repeat, repeat_if_failed) = match run_config.repeat {
        RepeatStrategy::Repeat(repeat) => (repeat, 0),
        RepeatStrategy::RepeatIfFailed(repeat_if_failed) => (1, repeat_if_failed),
    };
    let n_tests: usize = test_groups.iter().map(|g| g.tests.len()).sum();
    report.expect_additional_tests(repeat * n_tests);

    let queue = Mutex::new(TestQueue::new(test_groups, repeat));
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
                let (group, test_instance, is_done) = {
                    queue.lock().unwrap().return_response(
                        stream_result.id as usize,
                        success,
                        repeat_if_failed,
                    )
                };
                overall_success &= success;
                report.add(&group.app_name, test_instance, &result);
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

struct TestQueue {
    indices: VecDeque<usize>,
    creators: Vec<(Arc<TestGroup>, TestInstanceCreator, Vec<TestInstance>)>,
    repeat: usize,
    in_flight: usize,
}
impl TestQueue {
    fn new(tests: Vec<TestGroup>, repeat: usize) -> TestQueue {
        let creators: Vec<(Arc<TestGroup>, TestInstanceCreator, Vec<TestInstance>)> = tests
            .into_iter()
            .flat_map(|mut group| {
                let tests: Vec<TestInstanceCreator> = group.tests.drain(0..).collect();
                let group = Arc::new(group);
                tests
                    .into_iter()
                    .map(|t| (group.clone(), t, vec![]))
                    .collect::<Vec<_>>()
            })
            .collect();
        TestQueue {
            indices: (0..creators.len())
                .map(|i| std::iter::repeat(i).take(repeat))
                .flatten()
                .collect(),
            creators,
            repeat,
            in_flight: 0,
        }
    }
    fn next_request(&mut self) -> Option<xge_lib::StreamRequest> {
        match self.indices.pop_front() {
            Some(send_idx) => {
                let (group, tic, tis) = &mut self.creators[send_idx];
                assert_eq!(tic.is_g_multitest, false);
                let instance = tic.instantiate();
                tis.push(instance.clone());
                self.in_flight += 1;

                Some(xge_lib::StreamRequest {
                    id: send_idx as u64,
                    title: instance.test_id.id.clone(),
                    cwd: instance.command.cwd.clone(),
                    command: instance.command.command.clone(),
                    local: !group.can_use_xge(),
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
    ) -> (Arc<TestGroup>, TestInstance, bool) {
        self.in_flight -= 1;
        if !success {
            if self.creators[id].2.len() <= repeat_if_failed {
                self.indices.push_back(id);
            }
        }
        (
            self.creators[id].0.clone(),
            self.creators[id].2[id % self.repeat].clone(),
            self.is_done(),
        )
    }
    fn is_done(&self) -> bool {
        self.in_flight == 0 && self.indices.is_empty()
    }
}

impl TestInstance {
    async fn run_async(&self, timeout: std::time::Duration) -> TestCommandResult {
        let output = Command::new(&self.command.command[0])
            .args(self.command.command[1..].iter())
            .current_dir(&self.command.cwd)
            .output()
            .timeout(timeout)
            .await;
        let output = match output {
            Ok(output) => output,
            Err(_) => {
                return TestCommandResult {
                    exit_code: 1,
                    stdout: format!(
                        "[mwtest] terminated because {} second timeout was reached!",
                        timeout.as_secs()
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
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let output_str = stderr + stdout;

        // cleanup
        if let Some(tmp_path) = tmp_path {
            if tmp_path.is_dir() && std::fs::read_dir(&tmp_path).unwrap().next().is_none() {
                std::fs::remove_dir(&tmp_path).expect("failed to clean up temporary directory!");
            }
        }
        TestCommandResult {
            exit_code,
            stdout: output_str.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_whoami_instance() -> Vec<TestGroup> {
        let command_generator = Box::new(move || TestCommand {
            command: vec!["whoami".to_owned()],
            cwd: ".".to_owned(),
            tmp_path: None,
        });
        let test = TestInstanceCreator {
            test_id: crate::TestId {
                id: "test_id".to_owned(),
                rel_path: None,
            },
            command_generator,
            is_g_multitest: false,
        };
        vec![TestGroup {
            app_name: "test".to_owned(),
            gtest_generator: None,
            execution_style: ExecutionStyle::Parallel,
            timeout: None,
            tests: vec![test],
        }]
    }

    fn make_failing_ls_instance() -> Vec<TestGroup> {
        let command_generator = Box::new(move || TestCommand {
            command: vec!["ls".to_string(), "/nonexistent-file".to_string()],
            cwd: ".".to_owned(),
            tmp_path: None,
        });
        let test = TestInstanceCreator {
            test_id: crate::TestId {
                id: "test_id".to_owned(),
                rel_path: None,
            },
            command_generator,
            is_g_multitest: false,
        };
        vec![TestGroup {
            app_name: "test".to_owned(),
            gtest_generator: None,
            execution_style: ExecutionStyle::Parallel,
            timeout: None,
            tests: vec![test],
        }]
    }

    fn make_echo_instance_for_gtest(output: &'static str) -> Vec<TestGroup> {
        let command_generator = Box::new(move || TestCommand {
            command: vec!["/bin/echo".into(), output.into()],
            cwd: ".".into(),
            tmp_path: None,
        });
        let test = TestInstanceCreator {
            test_id: crate::TestId {
                id: "test_id".to_owned(),
                rel_path: None,
            },
            command_generator,
            is_g_multitest: true,
        };
        vec![TestGroup {
            app_name: "test".to_owned(),
            gtest_generator: Some(test),
            execution_style: ExecutionStyle::Parallel,
            timeout: None,
            tests: vec![], // TODO
        }]
    }

    struct CountingReport {
        count: usize,
    }
    impl CountingReport {
        fn new() -> Self {
            Self { count: 0 }
        }
    }
    impl Reportable for CountingReport {
        fn expect_additional_tests(&mut self, _n: usize) {}
        fn add(
            &mut self,
            _app_name: &str,
            _test_instance: crate::runnable::TestInstance,
            _test_result: &crate::scheduler::TestCommandResult,
        ) {
            self.count += 1;
        }
    }

    fn count_results(tests: Vec<TestGroup>, run_config: RunConfig) -> (bool, usize) {
        let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
        let report = Arc::new(Mutex::new(CountingReport::new()));
        let success =
            runtime.block_on(async { run_local(tests, &run_config, report.clone()).await });
        let count = report.lock().unwrap().count;
        (success, count)
    }

    fn count_results_xge(tests: Vec<TestGroup>, run_config: RunConfig) -> (bool, usize) {
        let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
        let mut report = CountingReport::new();
        let success =
            runtime.block_on(async { run_xge(tests, &run_config, &mut report, true).await });
        (success, report.count)
    }

    struct CollectingReport {
        ids: Vec<String>,
    }
    impl CollectingReport {
        fn new() -> Self {
            Self { ids: vec![] }
        }
    }
    impl Reportable for CollectingReport {
        fn expect_additional_tests(&mut self, _n: usize) {}
        fn add(
            &mut self,
            _app_name: &str,
            test_instance: crate::runnable::TestInstance,
            _test_result: &crate::scheduler::TestCommandResult,
        ) {
            self.ids.push(test_instance.test_id.id);
        }
    }

    fn collect_results(tests: Vec<TestGroup>, run_config: RunConfig) -> (bool, Vec<String>) {
        let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
        let report = Arc::new(Mutex::new(CollectingReport::new()));
        let success =
            runtime.block_on(async { run_local(tests, &run_config, report.clone()).await });
        let ids = report.lock().unwrap().ids.clone();
        (success, ids)
    }

    fn collect_results_gtest(test: TestInstance) -> (bool, Vec<String>) {
        let runtime = tokio::runtime::Runtime::new().expect("Unable to create tokio runtime!");
        let report = Arc::new(Mutex::new(CollectingReport::new()));
        let success = runtime.block_on(async { run_gtest(test, "app_name", report.clone()).await });
        let ids = report.lock().unwrap().ids.clone();
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
                repeat: RepeatStrategy::Repeat(1),
            },
        );
        assert_eq!(success, true);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_run_gtest() {
        let mut tests = make_echo_instance_for_gtest(
            r#"
Some prefix
[ RUN      ] Sample.Succeed
[       OK ] Sample.Succeed
[ RUN      ] Sample.Failed
[  FAILED  ] Sample.Failed
"#,
        );
        let (success, ids) =
            collect_results_gtest(tests[0].gtest_generator.take().unwrap().instantiate());
        assert_eq!(success, false);
        assert_eq!(ids, ["Sample.Succeed", "Sample.Failed"]);
    }

    #[test]
    fn test_run_normal_and_gtest() {
        let mut tests = make_echo_instance_for_gtest(
            r#"
Some prefix
[ RUN      ] Sample.Succeed
[       OK ] Sample.Succeed
"#,
        );
        tests.push(make_whoami_instance().pop().unwrap());
        let (success, ids) = collect_results(
            tests,
            RunConfig {
                verbose: false,
                parallel: true,
                xge: false,
                repeat: RepeatStrategy::Repeat(2),
            },
        );
        assert_eq!(success, false);
        assert_eq!(
            ids,
            ["Sample.Succeed", "Sample.Succeed", "test_id", "test_id"]
        );
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

    fn make_sleep_instance(timeout: Option<f32>) -> TestGroup {
        let command_generator = Box::new(move || TestCommand {
            command: vec!["sleep".to_owned(), "1".to_owned()],
            cwd: ".".to_owned(),
            tmp_path: None,
        });
        TestGroup {
            app_name: "test".to_owned(),
            gtest_generator: None,
            execution_style: ExecutionStyle::Parallel,
            timeout,
            tests: vec![TestInstanceCreator {
                test_id: crate::TestId {
                    id: format!("{:?}", timeout),
                    rel_path: None,
                },
                command_generator,
                is_g_multitest: false,
            }],
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
