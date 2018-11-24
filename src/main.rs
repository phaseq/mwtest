mod config;
mod report;
extern crate clap;
extern crate htmlescape;
extern crate num_cpus;
extern crate scoped_threadpool;
extern crate term_size;
extern crate uuid;
extern crate xge_lib;
#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json;
use clap::{App, Arg, ArgGroup, SubCommand};
use config::CommandTemplate;
use scoped_threadpool::Pool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

#[global_allocator]
static GLOBAL: std::alloc::System = std::alloc::System;

fn main() {
    let test_app_arg = Arg::with_name("test_app").required(true).multiple(true);
    let filter_arg = Arg::with_name("filter")
        .short("f")
        .long("filter")
        .takes_value(true)
        .help("select ids that contain one of the given substrings")
        .multiple(true);
    let matches = App::new("MW Test")
        .arg(Arg::with_name("build-dir")
            .long("build-dir")
            .takes_value(true)
            .help("depends on build type, could be \"your-branch/dev\", a quickstart or a CMake build directory"))
        .arg(Arg::with_name("testcases-dir")
            .long("testcases-dir")
            .takes_value(true)
            .help("usually \"your-branch/testcases\""))
        .subcommand(
            SubCommand::with_name("build")
                .arg(test_app_arg.clone()),
        ).subcommand(
            SubCommand::with_name("list")
                .arg(test_app_arg.clone())
                .arg(filter_arg.clone()),
        ).subcommand(
            SubCommand::with_name("run")
                .arg(test_app_arg)
                .arg(filter_arg)
                .arg(Arg::with_name("verbose")
                    .short("v")
                    .long("verbose"))
                .arg(Arg::with_name("threadpool")
                    .short("p")
                    .long("parallel")
                    .help("run using local thread pool"))
                .arg(Arg::with_name("xge")
                    .long("xge"))
                .group(ArgGroup::with_name("parallel")
                    .args(&["threadpool", "xge"]))
                .arg(Arg::with_name("repeat")
                    .long("repeat")
                    .takes_value(true)
                    .default_value("1")),
        ).get_matches();

    let input_paths = config::InputPaths::from(
        &matches.value_of("build-dir"),
        &matches.value_of("testcases-dir"),
        &Some("dev-releaseunicode.json"),
        &Some("tests.json"),
    );

    if let Some(matches) = matches.subcommand_matches("build") {
        cmd_build(
            &matches.values_of("test_app").unwrap().collect(),
            &input_paths,
        );
        std::process::exit(0);
    }

    let root_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("../../");

    let test_group_file =
        config::read_test_group_file(&root_dir.join("ci.json").to_string_lossy()).unwrap();

    if let Some(matches) = matches.subcommand_matches("list") {
        let test_apps = test_apps_from_args(&matches, &input_paths, &test_group_file);
        cmd_list(&test_apps);
    } else if let Some(matches) = matches.subcommand_matches("run") {
        let test_apps = test_apps_from_args(&matches, &input_paths, &test_group_file);
        let out_dir = std::env::current_dir().unwrap();
        let output_paths = OutputPaths {
            out_dir: out_dir.clone(),
            tmp_dir: out_dir.join("tmp"),
        };
        if Path::exists(&output_paths.tmp_dir) {
            std::fs::remove_dir_all(&output_paths.tmp_dir)
                .expect("could not clean up tmp directory!");
        }
        std::fs::create_dir(&output_paths.tmp_dir).expect("could not create tmp directory!");
        let run_config = RunConfig {
            verbose: matches.is_present("verbose"),
            parallel: matches.is_present("threadpool"),
            xge: matches.is_present("xge"),
            repeat: matches
                .value_of("repeat")
                .unwrap()
                .parse()
                .expect("expected numeric value for repeat"),
        };
        cmd_run(&input_paths, &test_apps, &output_paths, &run_config);
    }
}

fn cmd_build(test_names: &Vec<&str>, input_paths: &config::InputPaths) {
    let mut dependencies: HashMap<&str, Vec<&str>> = HashMap::new();
    for dep in test_names
        .iter()
        .map(|n| &input_paths.build_file.dependencies[*n])
    {
        let deps = dependencies.entry(&dep.solution).or_insert(vec![]);
        (*deps).push(&dep.project);
    }
    for (solution, projects) in dependencies {
        Command::new("buildConsole")
            .arg(solution)
            .arg("/build")
            .arg("/cfg=ReleaseUnicode|x64")
            .arg(format!("/prj={}", projects.join(",")))
            .arg("/openmonitor")
            .spawn()
            .expect("failed to launch buildConsole!")
            .wait()
            .expect("failed to build project!");
    }
}

fn cmd_list(test_apps: &Vec<AppWithTests>) {
    for test_app in test_apps {
        for group in &test_app.tests {
            for test_id in &group.test_ids {
                println!("{} --id {}", test_app.name, test_id.id);
            }
        }
    }
}

fn cmd_run(
    input_paths: &config::InputPaths,
    test_apps: &Vec<AppWithTests>,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) {
    let tests = create_run_commands(&input_paths, &test_apps, &output_paths);

    let n_workers = if run_config.parallel {
        num_cpus::get()
    } else {
        1
    };
    let mut pool = Pool::new(n_workers as u32);
    pool.scoped(|scoped| {
        let (tx, rx) = std::sync::mpsc::channel();
        let (xge_tx, xge_rx) = std::sync::mpsc::channel();
        if run_config.xge {
            let tx = tx.clone();
            scoped.execute(move || {
                run_xge_async(&xge_rx, &tx);
            });
        }
        for test_template in &tests {
            for _ in 0..run_config.repeat {
                let test_instance = test_template.instantiate();
                if run_config.xge {
                    xge_tx
                        .send(test_instance)
                        .expect("channel did not accept test input!");
                } else {
                    let tx = tx.clone();
                    scoped.execute(move || {
                        let output = test_instance.run();
                        tx.send((test_instance.app_name, test_instance.test_id, output))
                            .expect("channel did not accept test result!");
                    });
                }
            }
        }
        drop(xge_tx);

        let mut txt_report = report::FileLogger::create(&output_paths.out_dir);
        let mut xml_report = report::XmlReport::create(&output_paths.out_dir.join("results.xml"))
            .expect("could not create test report!");

        let n = tests.len() * run_config.repeat;
        let stdout_report = report::StdOut {
            verbose: run_config.verbose,
        };
        stdout_report.init(0, n);

        let mut i = 0;
        for (app_name, test_id, output) in rx.iter().take(n) {
            i += 1;
            txt_report.add(&app_name, &output.stdout);
            xml_report.add(&app_name, &test_id.id, &output);
            stdout_report.add(i, n, &app_name, &test_id.id, &output);
        }
        xml_report.write().expect("failed to write report!");
    });
}

struct RunConfig {
    verbose: bool,
    parallel: bool,
    xge: bool,
    repeat: usize,
}

#[derive(Debug)]
struct AppWithTests {
    name: String,
    config: config::TestConfig,
    tests: Vec<GroupWithTests>,
}

#[derive(Debug)]
struct GroupWithTests {
    test_group: config::TestGroup,
    test_ids: Vec<TestId>,
}

#[derive(Debug, Clone)]
pub struct TestId {
    pub id: String,
    pub rel_path: Option<PathBuf>,
}

#[derive(Debug)]
struct OutputPaths {
    out_dir: PathBuf,
    tmp_dir: PathBuf,
}

#[derive(Debug, Clone)]
struct TestCommand {
    command: Vec<String>,
    cwd: String,
    tmp_dir: Option<String>,
}
type CommandGenerator = Fn() -> TestCommand;

#[derive(Debug, Clone)]
pub struct TestCommandResult {
    exit_code: i32,
    stdout: String,
}

fn create_run_commands<'a>(
    input_paths: &config::InputPaths,
    test_apps: &'a Vec<AppWithTests>,
    output_paths: &OutputPaths,
) -> Vec<TestInstanceCreator<'a>> {
    let mut tests: Vec<TestInstanceCreator<'a>> = Vec::new();
    for app in test_apps {
        for group in &app.tests {
            for test_id in &group.test_ids {
                let (input_str, cwd) = test_id_to_input(&test_id, &input_paths, &app.config);
                let generator = test_command_generator(
                    &app.config.command_template,
                    input_str,
                    cwd,
                    output_paths.tmp_dir.clone(),
                );
                tests.push(TestInstanceCreator {
                    app_name: &app.name,
                    test_id: &test_id,
                    command_generator: generator,
                });
            }
        }
    }
    tests
}

fn test_id_to_input(
    test_id: &TestId,
    input_paths: &config::InputPaths,
    app_config: &config::TestConfig,
) -> (String, String) {
    if let Some(rel_path) = &test_id.rel_path {
        let full_path = input_paths.testcases_root.join(&rel_path);
        if let Some(cwd) = &app_config.cwd {
            // cncsim case
            (full_path.to_string_lossy().into_owned(), cwd.clone())
        } else {
            let parent_dir = full_path.parent().unwrap().to_string_lossy().to_string();
            if app_config.input_is_dir {
                // machsim case
                (parent_dir.clone(), parent_dir)
            } else {
                // verifier case
                let file_name = rel_path.file_name().unwrap().to_string_lossy().into_owned();
                (file_name, parent_dir)
            }
        }
    } else {
        // gtest case
        (test_id.id.clone(), app_config.cwd.clone().unwrap())
    }
}

fn test_command_generator(
    command_template: &CommandTemplate,
    input: String,
    cwd: String,
    tmp_root: PathBuf,
) -> Box<CommandGenerator> {
    let command = command_template.apply("{{input_path}}", &input);
    if command.has_pattern("{{tmp_path}}") {
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_string_lossy().into_owned();
            let command = command.apply("{{tmp_path}}", &tmp_path);
            std::fs::create_dir(&tmp_path).expect("could not create tmp path!");
            TestCommand {
                command: command.0.clone(),
                cwd: cwd.to_string(),
                tmp_dir: Some(tmp_path),
            }
        })
    } else {
        Box::new(move || TestCommand {
            command: command.0.clone(),
            cwd: cwd.to_string(),
            tmp_dir: None,
        })
    }
}

/*struct TestRunner<'a> {
    tx: std::sync::mpsc::Receiver<(&'a str, &'a TestId, TestCommand)>,
    xge_rx: std::sync::mpsc::Sender<(&'a str, &'a TestId, TestCommandResult)>,
    xge: xge_lib::XGE,
}
impl TestRunner {
    fn new() -> TestRunner {
        let (xge_tx, xge_rx) = std::sync::mpsc::channel();
    }
    fn run_xge(test_instance: TestInstance) {

    }
}*/

struct TestInstanceCreator<'a> {
    app_name: &'a str,
    test_id: &'a TestId,
    command_generator: Box<CommandGenerator>,
}
impl<'a> TestInstanceCreator<'a> {
    fn instantiate(&self) -> TestInstance<'a> {
        TestInstance {
            app_name: self.app_name,
            test_id: self.test_id,
            command: (self.command_generator)(),
        }
    }
}
#[derive(Debug)]
struct TestInstance<'a> {
    app_name: &'a str,
    test_id: &'a TestId,
    command: TestCommand,
}
impl<'a> TestInstance<'a> {
    fn run(&self) -> TestCommandResult {
        let maybe_output = Command::new(&self.command.command[0])
            .args(self.command.command[1..].iter())
            .current_dir(&self.command.cwd)
            .output();
        if maybe_output.is_err() {
            println!(
                "failed to run test command \"{:?}\": {}\nDid you forget to build?",
                &self.command.command,
                maybe_output.err().unwrap()
            );
            std::process::exit(-1);
        }
        let output = maybe_output.unwrap();
        let exit_code = output.status.code().unwrap_or(-7787);
        let stdout = std::str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
        let stderr = std::str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
        let output_str = stderr.to_owned() + stdout;
        if let Some(tmp_dir) = &self.command.tmp_dir {
            if !std::fs::read_dir(&tmp_dir).unwrap().next().is_some() {
                std::fs::remove_dir(&tmp_dir)
                    .expect("could not remove test's empty tmp directory!");
            }
        }
        TestCommandResult {
            exit_code: exit_code,
            stdout: output_str,
        }
    }
}

fn run_xge_async<'a>(
    rx: &std::sync::mpsc::Receiver<(TestInstance<'a>)>,
    tx: &std::sync::mpsc::Sender<(&'a str, &'a TestId, TestCommandResult)>,
) {
    let mut xge = xge_lib::XGE::new();
    let mut issued_commands: Vec<(&str, &TestId, Option<String>)> = Vec::new();
    for test_instance in rx.iter() {
        let request = xge_lib::StreamRequest {
            id: issued_commands.len() as u64,
            title: test_instance.test_id.id.clone(),
            cwd: test_instance.command.cwd.clone(),
            command: test_instance.command.command.clone(),
            local: false,
        };
        issued_commands.push((
            &test_instance.app_name,
            &test_instance.test_id,
            test_instance.command.tmp_dir.clone(),
        ));
        xge.run(&request)
            .expect("error in xge.run(): could not send command");
    }
    xge.done()
        .expect("error in xge.done(): could not close socket");
    for stream_result in xge.results() {
        let command = &issued_commands[stream_result.id as usize];
        let result = TestCommandResult {
            exit_code: stream_result.exit_code,
            stdout: stream_result.stdout,
        };
        tx.send((command.0, command.1, result))
            .expect("error in mpsc: could not send result");
        if let Some(tmp_dir) = &command.2 {
            if !std::fs::read_dir(&tmp_dir).unwrap().next().is_some() {
                std::fs::remove_dir(&tmp_dir)
                    .expect("could not remove test's empty tmp directory!");
            }
        }
    }
}

fn test_apps_from_args(
    args: &clap::ArgMatches,
    input_paths: &config::InputPaths,
    test_group_file: &config::TestGroupFile,
) -> Vec<AppWithTests> {
    let filter_tokens: Option<Vec<&str>> = args.values_of("filter").map(|v| v.collect());
    let id_filter = |input: &str| {
        if let Some(filters) = &filter_tokens {
            filters.iter().any(|f| input.contains(f))
        } else {
            true
        }
    };
    args.values_of("test_app")
        .unwrap()
        .map(|test_name| {
            let config = input_paths.test_config.get(test_name);
            if config.is_none() {
                let test_names: Vec<&String> = input_paths.test_config.keys().collect();
                println!(
                    "\"{}\" not found: must be one of {:?}",
                    test_name, test_names
                );
                std::process::exit(-1);
            }
            AppWithTests {
                name: test_name.to_string(),
                config: config.unwrap().clone(),
                tests: populate_test_groups(
                    config.unwrap(),
                    input_paths,
                    &test_group_file[test_name],
                    &id_filter,
                ),
            }
        }).collect()
}

fn populate_test_groups(
    test_config: &config::TestConfig,
    input_paths: &config::InputPaths,
    test_groups: &Vec<config::TestGroup>,
    id_filter: &Fn(&str) -> bool,
) -> Vec<GroupWithTests> {
    test_groups
        .iter()
        .map(|test_group| GroupWithTests {
            test_group: test_group.clone(),
            test_ids: test_group
                .generate_test_inputs(&test_config, &input_paths)
                .into_iter()
                .filter(|f| id_filter(&f.id))
                .collect(),
        }).collect()
}
