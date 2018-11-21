mod config;

extern crate clap;
//extern crate termion;
extern crate uuid;
use clap::{App, Arg, SubCommand};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;
//use threadpool::ThreadPool;

#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json;

#[derive(Debug)]
struct TestApp {
    name: String,
    command_template: CommandTemplate,
    tests: Vec<PopulatedTestGroup>,
}

type PopulatedTestGroup = (config::TestGroup, Vec<TestInput>);

#[derive(Debug, Clone)]
pub struct TestInput {
    pub id: String,
    pub rel_path: PathBuf,
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
    tmp_dir: PathBuf,
}

#[derive(Debug)]
struct TestCommand {
    command: Vec<String>,
    cwd: String,
    tmp_dir: String,
}
type CommandGenerator = Fn() -> TestCommand;

#[derive(Debug)]
struct TestCommandResult {
    exit_code: i32,
    stdout: String,
}

fn run_command(command: &TestCommand) -> TestCommandResult {
    let maybe_output = Command::new(&command.command[0])
        .args(command.command[1..].iter())
        .current_dir(&command.cwd)
        .output();
    if maybe_output.is_err() {
        println!(
            "failed to run test command \"{}\": {}\nDid you forget to build?",
            &command.command[0],
            maybe_output.err().unwrap()
        );
        std::process::exit(-1);
    }
    let output = maybe_output.unwrap();
    let exit_code = output.status.code().unwrap_or(-7787);
    let stdout = std::str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
    let stderr = std::str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
    let output_str = stderr.to_owned() + stdout;
    if !std::fs::read_dir(&command.tmp_dir)
        .unwrap()
        .next()
        .is_some()
    {
        std::fs::remove_dir(&command.tmp_dir)
            .expect("could not remove test's empty tmp directory!");
    }
    TestCommandResult {
        exit_code: exit_code,
        stdout: output_str,
    }
}

fn create_run_commands(
    input_paths: &config::InputPaths,
    test_apps: &Vec<TestApp>,
    output_paths: &OutputPaths,
) -> Vec<(TestInput, Box<CommandGenerator>)> {
    test_apps
        .iter()
        .flat_map(|test_app: &TestApp| {
            test_app
                .tests
                .iter()
                .flat_map(|(_test_group, inputs)| inputs)
                .map(|input: &TestInput| {
                    let file_name = input
                        .rel_path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    let full_path = input_paths.testcases_dir.join(&input.rel_path);
                    let cwd = full_path.parent().unwrap().to_string_lossy().to_string();
                    (
                        input.clone(),
                        to_command(
                            &test_app.command_template,
                            file_name,
                            cwd,
                            output_paths.tmp_dir.clone(),
                        ),
                    )
                }).collect::<Vec<(TestInput, Box<CommandGenerator>)>>()
        }).collect()
}

fn cmd_run(input_paths: &config::InputPaths, test_apps: &Vec<TestApp>, output_paths: &OutputPaths) {
    let commands = create_run_commands(&input_paths, &test_apps, &output_paths);

    let (width, _) = (100, 0); //termion::terminal_size().unwrap();

    let mut i = 0;
    let n = commands.len();
    for (input, cmd) in commands {
        let output = run_command(&cmd());
        i += 1;
        if output.exit_code == 0 {
            let mut line = format!(
                "\r{}[{}/{}] Ok: {}",
                "", //termion::clear::AfterCursor,
                i,
                n,
                input.id
            );
            line.truncate(width as usize);
            print!("{}", line);
            io::stdout().flush().unwrap();
        } else {
            println!("Failed: {}\n{}", input.id, output.stdout);
        }
    }
    println!();
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
                    .generate_test_inputs(&input_paths.testcases_dir)
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
                .map(|t| if t == from { to.to_string() } else { t.clone() })
                .collect(),
        }
    }
    fn apply_all(&self, patterns: &HashMap<String, String>) -> CommandTemplate {
        CommandTemplate {
            tokens: self
                .tokens
                .iter()
                .map(|t| {
                    if let Some(value) = patterns.get(t) {
                        value.clone()
                    } else {
                        t.clone()
                    }
                }).collect(),
        }
    }
}

fn to_command(
    command_template: &CommandTemplate,
    input_path: String,
    cwd: String,
    tmp_root: PathBuf,
) -> Box<CommandGenerator> {
    let command = command_template.apply("{{input_path}}", &input_path);
    Box::new(move || {
        let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
        let tmp_path = tmp_dir.to_string_lossy().into_owned();
        let command = command.apply("{{tmp_path}}", &tmp_path);
        std::fs::create_dir(&tmp_path).expect("could not create tmp path!");
        TestCommand {
            command: command.tokens.clone(),
            cwd: cwd.to_string(),
            tmp_dir: tmp_path,
        }
    })
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
                .arg(filter_arg),
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
        let output_paths = OutputPaths {
            tmp_dir: PathBuf::from("tmp"),
        };
        if Path::exists(&output_paths.tmp_dir) {
            std::fs::remove_dir_all(&output_paths.tmp_dir)
                .expect("could not clean up tmp directory!");
        }
        std::fs::create_dir(&output_paths.tmp_dir).expect("could not create tmp directory!");
        cmd_run(&input_paths, &test_apps, &output_paths);
    }
}
