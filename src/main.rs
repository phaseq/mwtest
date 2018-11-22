mod config;
extern crate clap;
extern crate htmlescape;
extern crate num_cpus;
extern crate term_size;
extern crate threadpool;
extern crate uuid;
extern crate xge_lib;
#[macro_use]
extern crate serde_derive;
extern crate serde;
#[macro_use]
extern crate serde_json;
use clap::{App, Arg, SubCommand};
use std::collections::{hash_map, HashMap};
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use threadpool::ThreadPool;
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
struct TestCommandResult {
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
                        let cwd;
                        if let Some(rel_path) = &test_input.rel_path {
                            input = rel_path.file_name().unwrap().to_string_lossy().into_owned();
                            let full_path = input_paths.testcases_dir.join(&rel_path);
                            cwd = full_path.parent().unwrap().to_string_lossy().to_string();
                        } else {
                            cwd = test_app.cwd.clone().expect("no cwd defined!");
                        }
                        (
                            test_input.clone(),
                            to_command(
                                &test_app.command_template,
                                input,
                                cwd,
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

struct XmlReport {
    file: File,
    results: HashMap<String, Vec<(String, TestCommandResult)>>,
}
impl XmlReport {
    fn create(path: &Path) -> std::io::Result<XmlReport> {
        Ok(XmlReport {
            file: File::create(&path)?,
            results: HashMap::new(),
        })
    }
    fn add(&mut self, test_name: &str, test_id: &str, test_result: &TestCommandResult) {
        match self.results.entry(test_name.to_string()) {
            hash_map::Entry::Vacant(e) => {
                e.insert(vec![(test_id.to_string(), test_result.clone())]);
            }
            hash_map::Entry::Occupied(mut e) => {
                e.get_mut().push((test_id.to_string(), test_result.clone()));
            }
        }
    }
    fn write(&mut self) -> std::io::Result<()> {
        let mut out = BufWriter::new(&self.file);
        out.write(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
        out.write(b"<testsuites>")?;
        for (test_name, test_results) in &self.results {
            out.write(
                format!(
                    "<testsuite name=\"{}\" test=\"{}\" failures=\"{}\">",
                    test_name,
                    test_results.len(),
                    -1
                ).as_bytes(),
            )?;
            for result in test_results.iter() {
                out.write(
                    format!(
                        "<testcase name=\"{}\">",
                        htmlescape::encode_attribute(&result.0)
                    ).as_bytes(),
                )?;
                out.write(format!("<exit-code>{}</exit_code>", result.1.exit_code).as_bytes())?;
                out.write(
                    format!(
                        "<system_out>{}</system_out>",
                        htmlescape::encode_minimal(&result.1.stdout)
                    ).as_bytes(),
                )?;
                out.write(b"</testcase>")?;
            }
            out.write(b"</testuite>")?;
        }
        out.write(b"</testsuites>")?;
        Ok(())
    }
}

fn run_xge(
    tests: &HashMap<&str, Vec<(TestInput, Box<CommandGenerator>)>>,
    tx: &std::sync::mpsc::Sender<(String, TestInput, TestCommandResult)>,
) -> std::io::Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let xge_exe = PathBuf::from(
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("xge"),
    );
    let mut xge_client = std::process::Command::new(xge_exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .arg("client")
        .arg(format!("127.0.0.1:{}", port))
        .spawn()
        .expect("could not spawn XGE client!");
    for stream in listener.incoming() {
        {
            let mut writer = std::io::BufWriter::new(stream?);
            for (test_name, commands) in tests {
                for (input, cmd_creator) in commands {
                    let cmd = cmd_creator();
                    let request = xge_lib::StreamRequest {
                        id: 1234,
                        title: input.id.clone(),
                        cwd: cmd.cwd,
                        command: cmd.command,
                        local: false,
                    };
                    writer.write(serde_json::to_string(&request)?.as_bytes())?;
                    writer.write(b"\n")?;
                    writer.flush()?;
                }
            }
        }
        println!("listening");
        let reader = std::io::BufReader::new(xge_client.stdout.as_mut().unwrap());
        for line in reader.lines().map(|l| l.unwrap()) {
            if line.starts_with("mwt ") {
                let stream_result: xge_lib::StreamResult =
                    serde_json::from_str(&line[4..]).unwrap();
                let ti = TestInput {
                    id: "todo".to_string(),
                    rel_path: None,
                };
                let result = TestCommandResult {
                    exit_code: stream_result.exit_code,
                    stdout: stream_result.stdout,
                };
                tx.send(("todo".to_string(), ti, result));
            }
        }
        println!("done");
        break;
    }
    Ok(())
}

struct RunConfig {
    verbose: bool,
    parallel: bool,
    xge: bool,
}
fn cmd_run(
    input_paths: &config::InputPaths,
    test_apps: &Vec<TestApp>,
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) {
    let tests = create_run_commands(&input_paths, &test_apps, &output_paths);

    let (width, _) = term_size::dimensions().unwrap();

    let n_workers = if run_config.parallel {
        num_cpus::get()
    } else {
        1
    };
    let pool = ThreadPool::new(n_workers);
    let (tx, rx) = std::sync::mpsc::channel();

    if run_config.xge {
        let ok = run_xge(&tests, &tx);
        if !ok.is_ok() {
            println!("XGE error: {}", ok.err().unwrap());
            std::process::exit(-1);
        }
    } else {
        for (test_name, commands) in &tests {
            for (input, cmd_creator) in commands {
                let test_name_copy = test_name.to_string();
                let input_copy = input.clone();
                let cmd = cmd_creator().clone();
                let tx = tx.clone();
                pool.execute(move || {
                    let output = run_command(&cmd);
                    tx.send((test_name_copy, input_copy, output))
                        .expect("channel did not accept test result!");
                });
            }
        }
    }

    let mut xml_report = XmlReport::create(&output_paths.out_dir.join("results.xml"))
        .expect("could not create test report!");

    let n = tests.values().map(|v| v.len()).sum();
    let mut i = 0;
    for (test_name, input, output) in rx.iter().take(n) {
        i += 1;
        xml_report.add(&test_name, &input.id, &output);
        if output.exit_code == 0 {
            if run_config.verbose {
                println!(
                    "[{}/{}] Ok: {} --id \"{}\"\n{}",
                    i, n, &test_name, &input.id, &output.stdout
                );
            } else {
                let line = format!("\r[{}/{}] Ok: {} --id \"{}\"", i, n, &test_name, &input.id);
                print!("{:width$}", line, width = width);
                io::stdout().flush().unwrap();
            }
        } else {
            println!(
                "[{}/{}] Failed: {} --id {}\n{}",
                i, n, &test_name, &input.id, &output.stdout
            );
        }
    }
    if !run_config.verbose {
        println!();
    }
    xml_report.write().expect("failed to write report!");
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
                    .long("xge")),
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
            out_dir: PathBuf::from("."),
            tmp_dir: PathBuf::from("tmp"),
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
        };
        cmd_run(&input_paths, &test_apps, &output_paths, &run_config);
    }
}
