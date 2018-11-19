extern crate clap;
extern crate regex;
use clap::{App, Arg, SubCommand};
use regex::Regex;
use std::ffi::OsString;
use std::fs::{self, DirEntry};
use std::io;
use std::path::Path;

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
    rel_path: OsString,
}

fn test_inputs(paths: &Vec<DirEntry>, root_dir: &Path, pattern: &str) -> Vec<TestInput> {
    let re = Regex::new(pattern).unwrap();
    paths
        .iter()
        .map(|p| {
            let rel_path = OsString::from(p.path().strip_prefix(root_dir).unwrap());
            let rel_path_copy = rel_path.to_os_string().to_string_lossy().to_string();
            let id = re
                .captures(&rel_path_copy)
                .unwrap()
                .get(1)
                .map_or("", |m| m.as_str());

            TestInput {
                id: id.to_string(),
                rel_path: rel_path,
            }
        }).collect()
}

#[derive(Debug)]
struct InputPaths {
    testcases_dir: String,
}

#[derive(Debug)]
struct TestGroup {
    rel_dir: String,
    file_ext: String,
    id_pattern: String,
}

fn cmd_list(input_paths: &InputPaths, test_group: &TestGroup) {
    let root_dir = Path::new(&input_paths.testcases_dir);
    let paths = test_input_paths(root_dir, &test_group.file_ext);
    let inputs = test_inputs(&paths, root_dir, &test_group.id_pattern);
    for input in inputs {
        println!("verifier --id {}", input.id);
    }
}

fn main() {
    let matches = App::new("MW Test")
        .subcommand(SubCommand::with_name("list").arg(Arg::with_name("verifier")))
        .get_matches();

    if let Some(matches) = matches.subcommand_matches("list") {
        if matches.is_present("verifier") {
            let input_paths = InputPaths {
                testcases_dir: "/Users/fabian/Desktop/Moduleworks/testcases".to_string(),
            };
            let test_group = TestGroup {
                rel_dir: "cutsim/_servertest/verifier".to_string(),
                file_ext: "verytest.ini".to_string(),
                id_pattern: "cutsim/_servertest/verifier/(.*).verytest.ini".to_string(),
            };
            cmd_list(&input_paths, &test_group);
        }
    }
}
