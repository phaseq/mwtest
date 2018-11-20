extern crate clap;
extern crate regex;
extern crate termion;
extern crate uuid;
use clap::{App, Arg, SubCommand};
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, DirEntry, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json;

fn test_input_paths(path: &Path, file_ext: &str) -> Vec<DirEntry> {
    fn glob_recursive(path: &Path, file_ext: &str, cb: &mut FnMut(DirEntry)) -> io::Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                glob_recursive(&path, file_ext, cb)?;
            } else {
                let file_name = path.file_name();
                if let Some(file_name) = file_name {
                    if file_name.to_str().unwrap_or("").ends_with(file_ext) {
                        cb(entry);
                    }
                }
            }
        }
        Ok(())
    }

    let mut paths: Vec<DirEntry> = Vec::new();
    glob_recursive(path, file_ext, &mut |path| paths.push(path)).expect("Could not iterate path!");
    paths
}

#[derive(Debug, Clone)]
struct TestInput {
    id: String,
    rel_path: PathBuf,
}

fn test_inputs(paths: &Vec<DirEntry>, root_dir: &Path, pattern: &str) -> Vec<TestInput> {
    let re = Regex::new(pattern).unwrap();
    paths
        .iter()
        .map(|p| {
            let rel_path_buf = p.path().strip_prefix(root_dir).unwrap().to_path_buf();
            let rel_path = rel_path_buf.to_string_lossy().to_string();
            let id = re
                .captures(&rel_path)
                .unwrap()
                .get(1)
                .map_or("", |m| m.as_str());

            TestInput {
                id: id.to_string(),
                rel_path: rel_path_buf,
            }
        }).collect()
}

#[derive(Debug)]
struct InputPaths {
    exe_paths: HashMap<String, String>,
    testcases_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
struct TestGroup {
    rel_dir: PathBuf,
    file_ext: String,
    id_pattern: String,
}

fn to_inputs(input_paths: &InputPaths, test_group: &TestGroup) -> Vec<TestInput> {
    let paths = test_input_paths(&input_paths.testcases_dir, &test_group.file_ext);
    test_inputs(&paths, &input_paths.testcases_dir, &test_group.id_pattern)
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
struct TestTemplate {
    command: Vec<String>,
}
impl TestTemplate {
    fn to_command(
        &self,
        input_path: String,
        cwd: String,
        tmp_root: PathBuf,
    ) -> Box<CommandGenerator> {
        let mut command = self.command.clone();
        for e in command.iter_mut().filter(|e| *e == "{{input_path}}") {
            *e = input_path.clone();
        }
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_string_lossy().to_string();
            let mut command = command.clone();
            for e in command.iter_mut().filter(|e| *e == "{{tmp_path}}") {
                *e = tmp_path.clone();
            }
            //std::fs::create_dir(&tmp_path).expect("could not create tmp path!");
            TestCommand {
                command: command.clone(),
                cwd: cwd.to_string(),
                tmp_dir: tmp_path,
            }
        })
    }
}

fn command_template_verifier(input_paths: &InputPaths) -> TestTemplate {
    let verifier_exe = input_paths.exe_paths["verifier"].to_string();
    let verifier_dll = input_paths.exe_paths["verifier-dll"].to_string();
    TestTemplate {
        command: vec![
            verifier_exe,
            "--config".to_string(),
            "{{input_path}}".to_string(),
            "--verifier".to_string(),
            verifier_dll,
            "--out-dir".to_string(),
            "{{tmp_path}}".to_string(),
        ],
    }
}

#[derive(Debug)]
struct TestCommandResult {
    exit_code: i32,
    stdout: String,
}

fn run_command(_command: &TestCommand) -> TestCommandResult {
    TestCommandResult {
        exit_code: 0,
        stdout: "did not run".to_string(),
    }
}

#[derive(Debug)]
struct TestApp {
    name: String,
    command_template: TestTemplate,
    tests: Vec<(TestGroup, Vec<TestInput>)>,
}

fn cmd_run(input_paths: &InputPaths, test_apps: &Vec<TestApp>, output_paths: &OutputPaths) {
    let mut commands: Vec<(TestInput, Box<CommandGenerator>)> = Vec::new();
    for test_app in test_apps {
        for (_test_group, inputs) in &test_app.tests {
            for input in inputs {
                let file_name = input
                    .rel_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let full_path = input_paths.testcases_dir.join(&input.rel_path);
                let cwd = full_path.parent().unwrap().to_string_lossy().to_string();
                commands.push((
                    input.clone(),
                    test_app.command_template.to_command(
                        file_name,
                        cwd,
                        output_paths.tmp_dir.clone(),
                    ),
                ))
            }
        }
    }

    let (width, _) = termion::terminal_size().unwrap();

    let mut i = 0;
    let n = commands.len();
    for (input, cmd) in commands {
        let output = run_command(&cmd());
        i += 1;
        if output.exit_code == 0 {
            let mut line = format!(
                "\r{}[{}/{}] Ok: {}",
                termion::clear::AfterCursor,
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

fn read_build_file(path: &str) -> Result<HashMap<String, String>, Box<std::error::Error>> {
    let file = File::open(path)?;
    let content = serde_json::from_reader(file)?;
    Ok(content)
}

type TestGroupFile = HashMap<String, Vec<TestGroup>>;
fn read_test_group_file(path: &str) -> Result<TestGroupFile, Box<std::error::Error>> {
    let file = File::open(path)?;
    let content = serde_json::from_reader(file)?;
    Ok(content)
}

fn to_test_list(
    input_paths: &InputPaths,
    test_groups: &Vec<TestGroup>,
    id_filter: &Fn(&str) -> bool,
) -> Vec<(TestGroup, Vec<TestInput>)> {
    test_groups
        .iter()
        .map(|test_group| {
            (
                test_group.clone(),
                to_inputs(&input_paths, &test_group)
                    .into_iter()
                    .filter(|f| id_filter(&f.id))
                    .collect(),
            )
        }).collect()
}

fn to_test_apps(
    args: &clap::ArgMatches,
    input_paths: &InputPaths,
    test_group_file: &TestGroupFile,
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
            let command_template = match test_name {
                "verifier" => command_template_verifier(&input_paths),
                _ => panic!("not implemented"),
            };
            TestApp {
                name: test_name.to_string(),
                command_template: command_template,
                tests: to_test_list(input_paths, &test_group_file[test_name], &id_filter),
            }
        }).collect()
}

fn main() {
    let test_app_arg = Arg::with_name("test_app")
        .required(true)
        .multiple(true)
        .possible_values(&["verifier", "machsim"]);
    let filter_arg = Arg::with_name("filter")
        .short("f")
        .long("filter")
        .takes_value(true)
        .help("select ids that contain one of the given substrings")
        .multiple(true);
    let matches = App::new("MW Test")
        .subcommand(
            SubCommand::with_name("list")
                .arg(test_app_arg.clone())
                .arg(filter_arg.clone()),
        ).subcommand(
            SubCommand::with_name("run")
                .arg(test_app_arg)
                .arg(filter_arg),
        ).get_matches();

    let input_paths = InputPaths {
        exe_paths: read_build_file("dev-releaseunicode.json").unwrap(),
        testcases_dir: PathBuf::from("/Users/fabian/Desktop/Moduleworks/testcases"),
    };

    let test_group_file = read_test_group_file("ci.json").unwrap();

    if let Some(matches) = matches.subcommand_matches("list") {
        let test_apps = to_test_apps(&matches, &input_paths, &test_group_file);
        cmd_list(&test_apps);
    } else if let Some(matches) = matches.subcommand_matches("run") {
        let test_apps = to_test_apps(&matches, &input_paths, &test_group_file);
        let output_paths = OutputPaths {
            tmp_dir: PathBuf::from("tmp"),
        };
        std::fs::remove_dir_all(&output_paths.tmp_dir).expect("could not clean up tmp directory!");
        std::fs::create_dir(&output_paths.tmp_dir).expect("could not create tmp directory!");
        cmd_run(&input_paths, &test_apps, &output_paths);
    }
}
