use crate::config;
use crate::TestId;
use std::path::PathBuf;
use uuid::Uuid;

pub fn create_run_commands<'a>(
    input_paths: &config::InputPaths,
    test_apps: &'a [crate::AppWithTests],
    output_paths: &crate::OutputPaths,
) -> Vec<TestInstanceCreator> {
    let mut tests: Vec<TestInstanceCreator> = Vec::new();
    for app in test_apps {
        for group in &app.tests {
            for test_id in &group.test_ids {
                let (input_str, cwd) = test_id_to_input(&test_id, &input_paths, &app.app);
                let generator = test_command_generator(
                    &app.app.properties.command_template,
                    &input_str,
                    cwd,
                    output_paths.tmp_dir.clone(),
                );
                tests.push(TestInstanceCreator {
                    app_name: app.name.clone(),
                    test_id: test_id.clone(),
                    allow_xge: group.test_group.xge,
                    timeout: group.test_group.timeout,
                    command_generator: generator,
                });
            }
        }
    }
    tests
}

pub struct TestInstanceCreator {
    app_name: String,
    test_id: TestId,
    pub allow_xge: bool,
    timeout: Option<f32>,
    command_generator: Box<CommandGenerator>,
}
unsafe impl Sync for TestInstanceCreator {}
impl TestInstanceCreator {
    pub fn instantiate(&self) -> TestInstance {
        TestInstance {
            app_name: self.app_name.clone(),
            test_id: self.test_id.clone(),
            allow_xge: self.allow_xge,
            timeout: self.timeout,
            command: (self.command_generator)(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestInstance {
    pub app_name: String,
    pub test_id: TestId,
    pub allow_xge: bool,
    pub timeout: Option<f32>,
    pub command: TestCommand,
}

#[derive(Debug, Clone)]
pub struct TestCommand {
    pub command: Vec<String>,
    pub cwd: String,
    pub tmp_path: Option<PathBuf>,
}
type CommandGenerator = dyn Fn() -> TestCommand + Sync + Send;

fn test_id_to_input(
    test_id: &TestId,
    input_paths: &config::InputPaths,
    app: &config::App,
) -> (String, String) {
    if let Some(rel_path) = &test_id.rel_path {
        let full_path = input_paths.testcases_root.join(&rel_path);
        if full_path.is_dir() {
            // machsim case
            let full_path = input_paths.testcases_root.join(&rel_path);
            (
                full_path.to_str().unwrap().to_string(),
                full_path.to_str().unwrap().to_string(),
            )
        } else if let Some(cwd) = &app.layout.cwd {
            // cncsim case
            (full_path.to_str().unwrap().to_string(), cwd.clone())
        } else {
            // verifier case
            let file_name = rel_path.file_name().unwrap().to_str().unwrap().to_string();
            let parent_dir = full_path.parent().unwrap().to_str().unwrap().to_string();
            (file_name, parent_dir)
        }
    } else {
        // gtest case
        (
            test_id.id.clone(),
            app.layout
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
    if command.has_pattern("{{tmp_path}}") {
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_str().unwrap().to_string();
            let command = command.apply("{{tmp_path}}", &tmp_path);
            std::fs::create_dir(&tmp_path).expect("could not create tmp path!");
            TestCommand {
                command: command.0.clone(),
                cwd: cwd.to_string(),
                tmp_path: Some(tmp_dir),
            }
        })
    } else if command.has_pattern("{{tmp_file}}") {
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_str().unwrap().to_string();
            let command = command.apply("{{tmp_file}}", &tmp_path);
            TestCommand {
                command: command.0.clone(),
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