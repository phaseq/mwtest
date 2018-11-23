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
use clap::{App, Arg, SubCommand};
use scoped_threadpool::Pool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

#[derive(Debug)]
struct TestApp {
    name: String,
    command_template: CommandTemplate,
    cwd: Option<String>,
    tests: Vec<PopulatedTestGroup>,
}

type PopulatedTestGroup = (config::TestGroup, Vec<TestInput>);

#[derive(Debug, Clone)]
pub struct TestInput {
    pub id: String,
    pub rel_path: Option<PathBuf>,
}

fn cmd_list(test_apps: &Vec<TestApp>) {
    for test_app in test_apps {
        for (_test_group, test_inputs) in &test_app.tests {
            for input in test_inputs {
                println!("{} --id {}", test_app.name, input.id);
            }
        }
    }
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

fn run_command(command: &TestCommand) -> TestCommandResult {
    let maybe_output = Command::new(&command.command[0].replace('/', "\\"))
        .args(command.command[1..].iter())
        .current_dir(&command.cwd)
        .output();
    if maybe_output.is_err() {
        println!(
            "failed to run test command \"{:?}\": {}\nDid you forget to build?",
            &command.command,
            maybe_output.err().unwrap()
        );
        std::process::exit(-1);
    }
    let output = maybe_output.unwrap();
    let exit_code = output.status.code().unwrap_or(-7787);
    let stdout = std::str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
    let stderr = std::str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
    let output_str = stderr.to_owned() + stdout;
    if let Some(tmp_dir) = &command.tmp_dir {
        if !std::fs::read_dir(&tmp_dir).unwrap().next().is_some() {
            std::fs::remove_dir(&tmp_dir).expect("could not remove test's empty tmp directory!");
        }
    }
    TestCommandResult {
        exit_code: exit_code,
        stdout: output_str,
    }
}

fn create_run_commands<'a>(
    input_paths: &config::InputPaths,
    test_apps: &'a Vec<TestApp>,
    output_paths: &OutputPaths,
) -> HashMap<&'a str, Vec<(TestInput, Box<CommandGenerator>)>> {
    test_apps
        .iter()
        .map(|test_app: &TestApp| {
            (
                test_app.name.as_str(),
                test_app
                    .tests
                    .iter()
                    .flat_map(|(_test_group, inputs)| inputs)
                    .map(|test_input: &TestInput| {
                        let mut input = test_input.id.clone();
                        let mut cwd = test_app.cwd.clone();
                        if let Some(rel_path) = &test_input.rel_path {
                            let full_path = input_paths.testcases_dir.join(&rel_path);
                            if cwd.is_none() {
                                input =
                                    rel_path.file_name().unwrap().to_string_lossy().into_owned();
                                cwd =
                                    Some(full_path.parent().unwrap().to_string_lossy().to_string());
                            } else {
                                input = full_path.to_string_lossy().into_owned();
                            }
                        }
                        (
                            test_input.clone(),
                            to_command(
                                &test_app.command_template,
                                input,
                                cwd.expect("no cwd defined!"),
                                output_paths.tmp_dir.clone(),
                            ),
                        )
                    }).collect::<Vec<(TestInput, Box<CommandGenerator>)>>(),
            )
        }).collect()
}
fn to_command(
    command_template: &CommandTemplate,
    input: String,
    cwd: String,
    tmp_root: PathBuf,
) -> Box<CommandGenerator> {
    let command = command_template.apply("{{input_path}}", &input);
    if command.tokens.iter().any(|t| t == "{{tmp_path}}") {
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_string_lossy().into_owned();
            let command = command.apply("{{tmp_path}}", &tmp_path);
            std::fs::create_dir(&tmp_path).expect("could not create tmp path!");
            TestCommand {
                command: command.tokens.clone(),
                cwd: cwd.to_string(),
                tmp_dir: Some(tmp_path),
            }
        })
    } else {
        Box::new(move || TestCommand {
            command: command.tokens.clone(),
            cwd: cwd.to_string(),
            tmp_dir: None,
        })
    }
}

fn run_xge_async<'a>(
    rx: &std::sync::mpsc::Receiver<(&'a str, &'a TestInput, TestCommand)>,
    tx: &std::sync::mpsc::Sender<(&'a str, &'a TestInput, TestCommandResult)>,
) {
    let mut xge = xge_lib::XGE::new();
    let mut issued_commands: Vec<(&str, &TestInput, Option<String>)> = Vec::new();
    for (test_name, input, cmd) in rx.iter() {
        let request = xge_lib::StreamRequest {
            id: issued_commands.len() as u64,
            title: input.id.clone(),
            cwd: cmd.cwd,
            command: cmd.command,
            local: false,
        };
        issued_commands.push((test_name, input, cmd.tmp_dir));
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

struct RunConfig {
    verbose: bool,
    parallel: bool,
    xge: bool,
    repeat: usize,
}
fn cmd_run(
    input_paths: &config::InputPaths,
    test_apps: &Vec<TestApp>,
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
        for (test_name, commands) in &tests {
            for (input, cmd_creator) in commands {
                for _ in 0..run_config.repeat {
                    let cmd = cmd_creator().clone();
                    if run_config.xge {
                        xge_tx
                            .send((test_name, input, cmd))
                            .expect("channel did not accept test input!");
                    } else {
                        let tx = tx.clone();
                        scoped.execute(move || {
                            let output = run_command(&cmd);
                            tx.send((test_name, input, output))
                                .expect("channel did not accept test result!");
                        });
                    }
                }
            }
        }
        drop(xge_tx);

        let mut txt_report = report::FileLogger::create(&output_paths.out_dir);
        let mut xml_report = report::XmlReport::create(&output_paths.out_dir.join("results.xml"))
            .expect("could not create test report!");

        let n = tests.values().map(|v| v.len()).sum::<usize>() * run_config.repeat;
        let stdout_report = report::StdOut {
            verbose: run_config.verbose,
        };
        stdout_report.init(0, n);

        let mut i = 0;
        for (test_name, input, output) in rx.iter().take(n) {
            i += 1;
            txt_report.add(&test_name, &output.stdout);
            xml_report.add(&test_name, &input.id, &output);
            stdout_report.add(i, n, &test_name, &input.id, &output);
        }
        xml_report.write().expect("failed to write report!");
    });
}

fn populate_test_groups(
    input_paths: &config::InputPaths,
    test_groups: &Vec<config::TestGroup>,
    id_filter: &Fn(&str) -> bool,
) -> Vec<PopulatedTestGroup> {
    test_groups
        .iter()
        .map(|test_group| {
            (
                test_group.clone(),
                test_group
                    .generate_test_inputs(&input_paths)
                    .into_iter()
                    .filter(|f| id_filter(&f.id))
                    .collect(),
            )
        }).collect()
}

#[derive(Debug)]
struct CommandTemplate {
    tokens: Vec<String>,
}
impl CommandTemplate {
    fn apply(&self, from: &str, to: &str) -> CommandTemplate {
        CommandTemplate {
            tokens: self
                .tokens
                .iter()
                .map(|t| t.to_owned().replace(from, to))
                .collect(),
        }
    }
    fn apply_all(&self, patterns: &HashMap<String, String>) -> CommandTemplate {
        CommandTemplate {
            tokens: self
                .tokens
                .iter()
                .map(|t: &String| {
                    patterns
                        .iter()
                        .fold(t.to_owned(), |acc, (k, v)| acc.replace(k, v))
                }).collect(),
        }
    }
}

fn test_apps_from_args(
    args: &clap::ArgMatches,
    test_config: &config::TestConfigFile,
    input_paths: &config::InputPaths,
    test_group_file: &config::TestGroupFile,
) -> Vec<TestApp> {
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
            let config = test_config.get(test_name);
            if config.is_none() {
                let test_names: Vec<&String> = test_config.keys().collect();
                println!(
                    "\"{}\" not found: must be one of {:?}",
                    test_name, test_names
                );
                std::process::exit(-1);
            }
            let command_template = CommandTemplate {
                tokens: config.unwrap().command.clone(),
            }.apply_all(&input_paths.exe_paths);
            TestApp {
                name: test_name.to_string(),
                command_template: command_template,
                cwd: config
                    .unwrap()
                    .cwd
                    .as_ref()
                    .map(|c| input_paths.exe_paths[c].clone())
                    .clone(),
                tests: populate_test_groups(input_paths, &test_group_file[test_name], &id_filter),
            }
        }).collect()
}

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
                .arg(Arg::with_name("repeat")
                    .long("repeat")
                    .takes_value(true)
                    .default_value("1")),
        ).get_matches();

    let root_dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("../../");
    let test_config =
        config::read_test_config_file(&root_dir.join("tests.json").to_string_lossy()).unwrap();

    let input_paths = config::InputPaths::from(
        &matches.value_of("build-dir"),
        &root_dir.join("dev-releaseunicode.json").to_string_lossy(),
        &matches.value_of("testcases-dir"),
    );

    let test_group_file =
        config::read_test_group_file(&root_dir.join("ci.json").to_string_lossy()).unwrap();

    if let Some(matches) = matches.subcommand_matches("list") {
        let test_apps = test_apps_from_args(&matches, &test_config, &input_paths, &test_group_file);
        cmd_list(&test_apps);
    } else if let Some(matches) = matches.subcommand_matches("run") {
        let test_apps = test_apps_from_args(&matches, &test_config, &input_paths, &test_group_file);
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
