extern crate clap;
extern crate indicatif;
extern crate regex;
extern crate uuid;
use clap::{App, Arg, SubCommand};
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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

#[derive(Debug)]
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
    testcases_dir: PathBuf,
}

#[derive(Debug)]
struct TestGroup {
    rel_dir: PathBuf,
    file_ext: String,
    id_pattern: String,
}

fn to_inputs(input_paths: &InputPaths, test_group: &TestGroup) -> Vec<TestInput> {
    let paths = test_input_paths(&input_paths.testcases_dir, &test_group.file_ext);
    test_inputs(&paths, &input_paths.testcases_dir, &test_group.id_pattern)
}

fn cmd_list(input_paths: &InputPaths, test_group: &TestGroup) {
    for input in to_inputs(input_paths, test_group) {
        println!("verifier --id {}", input.id);
    }
}

#[derive(Debug)]
struct BuildPaths {
    exe_paths: HashMap<String, String>,
}
#[derive(Debug)]
struct OutputPaths {
    tmp_dir: String,
}

#[derive(Debug)]
struct TestCommand {
    command: Vec<String>,
    cwd: String,
    tmp_dir: String,
}
type CommandGenerator = Fn() -> TestCommand;

fn generate_verifier_command(
    build_paths: &BuildPaths,
    tmp_root: PathBuf,
    input_path: String,
    cwd: String,
) -> Box<CommandGenerator> {
    let verifier_exe = build_paths.exe_paths["verifier"].clone();
    let verifier_dll = build_paths.exe_paths["verifier-dll"].clone();
    Box::new(move || {
        let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
        let tmp_path = tmp_dir.to_string_lossy().to_string();
        //std::fs::create_dir(&tmp_path).expect("could not create tmp path!");
        TestCommand {
            command: vec![
                verifier_exe.clone(),
                "--config".to_string(),
                input_path.clone(),
                "--verifier".to_string(),
                verifier_dll.clone(),
                "--out-dir".to_string(),
                tmp_path.clone(),
            ],
            cwd: cwd.to_string(),
            tmp_dir: tmp_path,
        }
    })
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

struct TestApp {
    generate_command: fn(&BuildPaths, PathBuf, String, String) -> Box<CommandGenerator>,
}

fn cmd_run(
    build_paths: &BuildPaths,
    input_paths: &InputPaths,
    test_app: &TestApp,
    test_group: &TestGroup,
    output_paths: &OutputPaths,
) {
    let inputs = to_inputs(input_paths, test_group);
    let commands = inputs.iter().map(|input| {
        let file_name = input.rel_path.file_name().unwrap();
        let full_path = input_paths.testcases_dir.join(&input.rel_path);
        let cwd = full_path.parent().unwrap();

        (
            input,
            (test_app.generate_command)(
                build_paths,
                PathBuf::from("tmp"),
                file_name.to_string_lossy().to_string(),
                cwd.to_string_lossy().to_string(),
            ),
        )
    });

    let bar = indicatif::ProgressBar::new(inputs.len() as u64);
    bar.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:30} {pos:>5}/{len:5} {wide_msg}")
            .progress_chars("##."),
    );
    for (input, cmd) in commands {
        let output = run_command(&cmd());
        if output.exit_code == 0 {
            bar.set_message(&format!("Ok: {}", input.id));
        } else {
            println!("Failed: {}\n{}", input.id, output.stdout);
        }
        bar.inc(1);
    }
    bar.finish();
    println!();
}

fn main() {
    let matches = App::new("MW Test")
        .subcommand(SubCommand::with_name("list").arg(Arg::with_name("verifier")))
        .subcommand(SubCommand::with_name("run").arg(Arg::with_name("verifier")))
        .get_matches();

    let input_paths = InputPaths {
        testcases_dir: PathBuf::from("/Users/fabian/Desktop/Moduleworks/testcases"),
    };

    let mut exe_paths: HashMap<String, String> = HashMap::new();
    exe_paths.insert(
        "verifier".to_string(),
        "/Users/fabian/Desktop/Moduleworks/dev/5axis/test/verifiertest/ReleaseUnicode/mwVerifierTest.exe".to_string());
    exe_paths.insert(
        "verifier-dll".to_string(),
        "/Users/fabian/Desktop/Moduleworks/dev/5axis/customer/quickstart/mwVerifier.dll"
            .to_string(),
    );
    let build_paths = BuildPaths {
        exe_paths: exe_paths,
    };

    let test_group = TestGroup {
        rel_dir: PathBuf::from("cutsim/_servertest/verifier"),
        file_ext: "verytest.ini".to_string(),
        id_pattern: "cutsim/_servertest/verifier/(.*).verytest.ini".to_string(),
    };

    let test_app = TestApp {
        generate_command: generate_verifier_command,
    };

    if let Some(matches) = matches.subcommand_matches("list") {
        if matches.is_present("verifier") {
            cmd_list(&input_paths, &test_group);
        }
    } else if let Some(matches) = matches.subcommand_matches("run") {
        if matches.is_present("verifier") {
            let output_paths = OutputPaths {
                tmp_dir: "tmp".to_string(),
            };
            std::fs::remove_dir_all(&output_paths.tmp_dir)
                .expect("could not clean up tmp directory!");
            std::fs::create_dir(&output_paths.tmp_dir).expect("could not create tmp directory!");
            cmd_run(
                &build_paths,
                &input_paths,
                &test_app,
                &test_group,
                &output_paths,
            );
        }
    }
}
