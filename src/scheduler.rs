use crate::config;
use crate::report;
use crate::runnable::{TestInstance, TestInstanceCreator};
use crate::{OutputPaths, TestUid};
use scoped_threadpool::Pool;
use std::collections::HashMap;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};

pub struct RunConfig {
    pub verbose: bool,
    pub parallel: bool,
    pub xge: bool,
    pub repeat: usize,
    pub rerun_if_failed: usize,
}

pub fn run<'a>(
    input_paths: &config::InputPaths,
    tests: &[TestInstanceCreator<'a>],
    output_paths: &crate::OutputPaths,
    run_config: &RunConfig,
) -> bool {
    let n_workers = if run_config.parallel {
        num_cpus::get()
    } else if run_config.xge {
        2 + num_cpus::get() // +2 for management threads
    } else {
        1
    };
    let mut pool = Pool::new(n_workers as u32);
    pool.scoped(|scope| run_in_scope(scope, &tests, &input_paths, &output_paths, &run_config))
}

#[derive(Debug, Clone)]
pub struct TestCommandResult {
    pub exit_code: i32,
    pub stdout: String,
}

fn run_in_scope<'scope>(
    scope: &scoped_threadpool::Scope<'_, 'scope>,
    tests: &'scope [TestInstanceCreator<'_>],
    input_paths: &'scope config::InputPaths,
    output_paths: &'scope OutputPaths,
    run_config: &'scope RunConfig,
) -> bool {
    let mut report = report::Report::new(
        &output_paths.out_dir,
        input_paths.testcases_root.to_str().unwrap(),
        run_config.verbose,
    );

    let mut n = tests.len() * run_config.repeat;

    let (tx, rx) = mpsc::channel();
    let (xge_tx, xge_rx) = mpsc::channel::<TestInstance<'_>>();
    if run_config.xge {
        launch_xge_management_threads(scope, xge_rx, &tx, &output_paths);
    }

    let mut run_counts = HashMap::new();
    for test_instance_generator in tests.iter() {
        run_counts.insert(test_instance_generator.get_uid(), RunCount::new());
        for _ in 0..run_config.repeat {
            run_test_instance(
                test_instance_generator.instantiate(),
                scope,
                &xge_tx,
                &tx,
                &output_paths,
                &run_config,
            );
        }
    }

    let mut i = 0;
    while i < n {
        let (test_instance, output) = match rx.recv_timeout(std::time::Duration::from_secs(6 * 60))
        {
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("test executor failed!"),
            Ok(result) => result,
        };
        let test_uid = test_instance.get_uid();
        let run_count = run_counts.get_mut(&test_uid).unwrap();
        run_count.n_runs += 1;
        i += 1;
        if output.exit_code == 0 {
            run_count.n_successes += 1;
        } else if run_count.n_runs <= run_config.rerun_if_failed {
            n += 1;
            let test_instance_generator = tests.iter().find(|t| t.get_uid() == test_uid).unwrap();

            run_test_instance(
                test_instance_generator.instantiate(),
                scope,
                &xge_tx,
                &tx,
                &output_paths,
                &run_config,
            );
        }
        report.add(i, n, test_instance, &output);
    }

    drop(report);

    report_and_check_runs(&run_counts)
}

pub fn run_test_instance<'scope>(
    test_instance: TestInstance<'scope>,
    scope: &scoped_threadpool::Scope<'_, 'scope>,
    xge_tx: &mpsc::Sender<TestInstance<'scope>>,
    tx: &mpsc::Sender<(TestInstance<'scope>, TestCommandResult)>,
    output_paths: &'scope OutputPaths,
    run_config: &'scope RunConfig,
) {
    if run_config.xge && test_instance.allow_xge {
        xge_tx
            .send(test_instance)
            .expect("channel did not accept test input!");
    } else {
        let tx = tx.clone();
        scope.execute(move || {
            let output = test_instance.run(&output_paths);
            tx.send((test_instance, output))
                .expect("channel did not accept test result!");
        });
    }
}

type ResultMessage<'a> = (TestInstance<'a>, TestCommandResult);
fn launch_xge_management_threads<'pool, 'scope>(
    scope: &scoped_threadpool::Scope<'pool, 'scope>,
    xge_rx: mpsc::Receiver<TestInstance<'scope>>,
    tx: &mpsc::Sender<ResultMessage<'scope>>,
    output_paths: &'scope OutputPaths,
) {
    let tx = tx.clone();
    let (mut xge_writer, xge_reader) = xge_lib::xge();
    let issued_commands: Arc<Mutex<Vec<TestInstance<'_>>>> = Arc::new(Mutex::new(Vec::new()));
    let issued_commands2 = issued_commands.clone();
    scope.execute(move || {
        for test_instance in xge_rx.iter() {
            let request = {
                let mut locked_issued_commands = issued_commands.lock().unwrap();
                let request = xge_lib::StreamRequest {
                    id: locked_issued_commands.len() as u64,
                    title: test_instance.test_id.id.clone(),
                    cwd: test_instance.command.cwd.clone(),
                    command: test_instance.command.command.clone(),
                    local: false,
                };
                locked_issued_commands.push(test_instance);
                request
            };

            xge_writer
                .run(&request)
                .expect("error in xge.run(): could not send command");
        }
        xge_writer
            .done()
            .expect("error in xge.done(): could not close socket");
    });
    scope.execute(move || {
        for stream_result in xge_reader {
            let result = TestCommandResult {
                exit_code: stream_result.exit_code,
                stdout: stream_result.stdout,
            };
            let success = result.exit_code == 0;
            let locked_issued_commands = issued_commands2.lock().unwrap();
            let test_instance = &locked_issued_commands[stream_result.id as usize];
            let message = (test_instance.clone(), result);
            tx.send(message)
                .expect("error in mpsc: could not send result");
            test_instance
                .cleanup(success, &output_paths)
                .expect("failed to clean up temporary output directory!");
        }
    });
}

struct RunCount {
    n_runs: usize,
    n_successes: usize,
}
impl RunCount {
    fn new() -> RunCount {
        RunCount {
            n_runs: 0,
            n_successes: 0,
        }
    }
}
fn report_and_check_runs(run_counts: &HashMap<TestUid<'_>, RunCount>) -> bool {
    let test_formatter = |(id, run_counts): (&TestUid<'_>, &RunCount)| {
        if run_counts.n_runs > 1 {
            format!(
                "  {} --id {} (succeeded {} out of {} runs)",
                id.0, id.1, run_counts.n_successes, run_counts.n_runs
            )
        } else {
            format!("  {} --id {}", id.0, id.1)
        }
    };
    let mut failed: Vec<String> = run_counts
        .iter()
        .filter(|(_id, run_counts)| run_counts.n_successes == 0)
        .map(test_formatter)
        .collect();
    failed.sort_unstable();
    let all_succeeded = failed.is_empty();

    let mut instable: Vec<String> = run_counts
        .iter()
        .filter(|(_id, run_counts)| {
            run_counts.n_successes > 0 && run_counts.n_successes < run_counts.n_runs
        })
        .map(test_formatter)
        .collect();
    instable.sort_unstable();
    let none_instable = instable.is_empty();

    if !none_instable {
        println!("Tests that are instable: ");
        for t in instable {
            println!("{}", t);
        }
    }

    if !all_succeeded {
        println!("Tests that failed: ");
        for t in failed {
            println!("{}", t);
        }
    }

    if all_succeeded && none_instable {
        println!("All tests succeeded!");
    }

    all_succeeded
}

impl<'a> TestInstance<'a> {
    fn run(&self, output_paths: &OutputPaths) -> TestCommandResult {
        let maybe_output = Command::new(&self.command.command[0])
            .args(self.command.command[1..].iter())
            .current_dir(&self.command.cwd)
            .output();
        let output = match maybe_output {
            Ok(output) => output,
            Err(e) => {
                println!(
                    "
ERROR: failed to run test command!
  command: {:?}
  cwd: {}
  error: {}

Did you forget to build?",
                    &self.command.command, &self.command.cwd, e
                );
                std::process::exit(-1);
            }
        };
        let exit_code = output.status.code().unwrap_or(-7787);
        let stdout = std::str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
        let stderr = std::str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
        let output_str = stderr.to_owned() + stdout;
        self.cleanup(exit_code == 0, &output_paths)
            .expect("failed to clean up temporary output directory!");
        TestCommandResult {
            exit_code,
            stdout: output_str,
        }
    }

    fn cleanup(&self, _equal: bool, _output_paths: &OutputPaths) -> std::io::Result<()> {
        if let Some(tmp_path) = &self.command.tmp_path {
            if tmp_path.is_dir() && std::fs::read_dir(tmp_path).unwrap().next().is_none() {
                std::fs::remove_dir(&tmp_path)?;
            }
        }
        Ok(())
    }

    fn get_uid(&self) -> TestUid<'a> {
        (self.app_name, &self.test_id.id)
    }
}
