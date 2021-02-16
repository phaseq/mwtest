use crate::config;
use crate::TestId;
use std::path::PathBuf;
use tokio::process::Command;
use uuid::Uuid;

pub fn create_run_commands(
    input_paths: &config::InputPaths,
    test_apps: &[crate::AppWithTests],
    output_paths: &crate::OutputPaths,
    no_timeout: bool,
) -> Vec<TestGroup> {
    let mut tests: Vec<TestGroup> = Vec::new();
    for app in test_apps {
        for group in &app.tests {
            let execution_style = match group.test_group.execution_style.as_ref() {
                "singlethreaded" => ExecutionStyle::Single,
                "parallel" => ExecutionStyle::Parallel,
                "xge" => ExecutionStyle::XGE,
                _ => panic!(
                    "Invalid execution style! Only 'singlethreaded', 'parallel', 'xge' allowed."
                ),
            };
            let timeout = if no_timeout {
                None
            } else {
                group.test_group.timeout
            };

            let gtest_generator = match &group.test_filter {
                Some(test_filter) => {
                    let test_id = TestId {
                        id: test_filter.clone(),
                        rel_path: None,
                    };
                    let (_input_str, cwd) = test_id_to_input(&test_id, &input_paths, &app.app);
                    Some(gtest_command_generator(&group.command, &test_filter, cwd))
                }
                None => None,
            };

            let mut test_generators = Vec::new();
            for test_id in &group.test_ids {
                let (input_str, cwd) = test_id_to_input(&test_id, &input_paths, &app.app);
                let generator = test_command_generator(
                    &group.command,
                    &input_str,
                    cwd,
                    output_paths.tmp_dir.clone(),
                );
                test_generators.push(TestInstanceCreator {
                    test_id: test_id.clone(),
                    command_generator: generator,
                    is_g_multitest: false,
                });
            }
            let gtest_generator = gtest_generator.map(|command_generator| TestInstanceCreator {
                test_id: TestId {
                    id: "<generator>".into(),
                    rel_path: None,
                },
                command_generator,
                is_g_multitest: true,
            });
            tests.push(TestGroup {
                app_name: app.name.clone(),
                gtest_generator,
                execution_style: execution_style.clone(),
                timeout,
                accepted_returncodes: group.test_group.accepted_returncodes.clone(),
                tests: test_generators,
            })
        }
    }
    tests
}

pub struct TestGroup {
    pub app_name: String,
    pub gtest_generator: Option<TestInstanceCreator>,
    pub execution_style: ExecutionStyle,
    pub timeout: Option<f32>,
    pub accepted_returncodes: Vec<i32>,
    pub tests: Vec<TestInstanceCreator>,
}
impl TestGroup {
    pub fn get_timeout_duration(&self) -> Option<std::time::Duration> {
        self.timeout
            .map(|t| std::time::Duration::from_millis((t * 1000.0) as u64))
    }
}

pub struct TestInstanceCreator {
    pub test_id: TestId,
    pub command_generator: Box<CommandGenerator>,
    pub is_g_multitest: bool,
}
unsafe impl Sync for TestInstanceCreator {}
impl TestInstanceCreator {
    pub fn instantiate(&self) -> TestInstance {
        TestInstance {
            test_id: self.test_id.clone(),
            command: (self.command_generator)(),
        }
    }
}

#[derive(Clone)]
pub enum ExecutionStyle {
    Single,
    Parallel,
    XGE,
}

#[derive(Clone)]
pub struct TestInstance {
    pub test_id: TestId,
    pub command: TestCommand,
}

impl TestInstance {
    pub async fn run_async(&self, timeout: Option<std::time::Duration>) -> TestCommandResult {
        let child = Command::new(&self.command.command[0])
            .args(self.command.command[1..].iter())
            .current_dir(&self.command.cwd)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
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

        // TODO: instead of reading lines of strings: should we read buffers of bytes, to guard against encoding errors?
        use tokio::io::AsyncBufReadExt;
        let stdout = child.stdout.take().unwrap();
        let stdout = tokio::io::BufReader::new(stdout);
        let stdout = stdout.lines();
        let stderr = child.stderr.take().unwrap();
        let stderr = tokio::io::BufReader::new(stderr);
        let stderr = stderr.lines();

        let status;
        let mut output_text = String::new();

        let timeout = timeout.unwrap_or_else(|| std::time::Duration::from_secs(356 * 24 * 60 * 60)); // TODO: more elegant solution for no_timeout?
        let timeout_future = tokio::time::sleep(timeout);
        tokio::pin!(timeout_future);
        tokio::pin!(stdout);
        tokio::pin!(stderr);
        loop {
            tokio::select! {
                line = stdout.next_line() => {
                    if let Ok(Some(line)) = line {
                        output_text.push_str(&line);
                        output_text.push('\n');
                    }
                },
                line = stderr.next_line() => {
                    if let Ok(Some(line)) = line {
                        output_text.push_str(&line);
                        output_text.push('\n');
                    }
                },
                // TODO: do we have to read lines after wait() or timeout finishes?
                output = child.wait() => {
                    status = output.map(|o| (o.success(), o.code()));
                    break;
                },
                _ = &mut timeout_future => {
                    child.kill().await.unwrap(); // TODO: when does this fail?
                    status = Ok((false, None));
                    output_text.push_str(&format!(
                        "[mwtest] terminated because {} second timeout was reached!",
                        timeout.as_secs()
                    ));
                    break;
                }
            }
        }

        let status = match status {
            Ok(status) => status,
            Err(e) => {
                return TestCommandResult {
                    exit_code: 1,
                    stdout: format!(
                        "[mwtest] error while trying to start test: {}",
                        e.to_string()
                    ),
                }
            }
        };

        let tmp_path = self.command.tmp_path.clone();
        let exit_code = status.1.unwrap_or(-7787);

        // cleanup
        if let Some(tmp_path) = tmp_path {
            if tmp_path.is_dir() && std::fs::read_dir(&tmp_path).unwrap().next().is_none() {
                std::fs::remove_dir(&tmp_path).expect("failed to clean up temporary directory!");
            }
        }
        TestCommandResult {
            exit_code,
            stdout: output_text,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestCommandResult {
    pub exit_code: i32,
    pub stdout: String,
}

#[derive(Debug, Clone)]
pub struct TestCommand {
    pub command: Vec<String>,
    pub cwd: String,
    pub tmp_path: Option<PathBuf>,
}
pub type CommandGenerator = dyn Fn() -> TestCommand + Sync + Send;

fn test_id_to_input(
    test_id: &TestId,
    input_paths: &config::InputPaths,
    app: &config::App,
) -> (String, String) {
    if let Some(rel_path) = &test_id.rel_path {
        let full_path = input_paths.testcases_dir.join(&rel_path);
        let full_path_str = full_path.to_str().unwrap().to_string();
        if let Some(cwd) = &app.build.cwd {
            // cncsim case
            (full_path_str, cwd.clone())
        } else if full_path.is_dir() {
            // machsim case
            (full_path_str.clone(), full_path_str)
        } else {
            // verifier case
            let parent_dir = full_path.parent().unwrap().to_str().unwrap().to_string();
            (full_path_str, parent_dir)
        }
    } else {
        // gtest case
        (
            test_id.id.clone(),
            app.build
                .cwd
                .clone()
                .expect("You need to specify a CWD for gtests (see preset)."),
        )
    }
}

fn test_command_generator(
    command_template: &config::CommandTemplate,
    input: &str,
    cwd: String,
    tmp_root: PathBuf,
) -> Box<CommandGenerator> {
    let command = command_template.apply("{{input}}", &input);
    if command.has_pattern("{{generate_output_dir}}") {
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_str().unwrap().to_string();
            let command = command.apply("{{generate_output_dir}}", &tmp_path);
            std::fs::create_dir(&tmp_path).expect("could not create tmp path!");
            TestCommand {
                command: command.0,
                cwd: cwd.to_string(),
                tmp_path: Some(tmp_dir),
            }
        })
    } else if command.has_pattern("{{generate_output_file}}") {
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_str().unwrap().to_string();
            let command = command.apply("{{generate_output_file}}", &tmp_path);
            TestCommand {
                command: command.0,
                cwd: cwd.to_string(),
                tmp_path: Some(tmp_dir),
            }
        })
    } else {
        Box::new(move || TestCommand {
            command: command.0.clone(),
            cwd: cwd.to_string(),
            tmp_path: None,
        })
    }
}

fn gtest_command_generator(
    command_template: &config::CommandTemplate,
    input: &str,
    cwd: String,
) -> Box<CommandGenerator> {
    let command = command_template.apply("{{input}}", &input);
    Box::new(move || TestCommand {
        command: command.0.clone(),
        cwd: cwd.to_string(),
        tmp_path: None,
    })
}
