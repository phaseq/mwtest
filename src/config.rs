extern crate glob;
extern crate regex;
//use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct TestConfig {
    pub command: Vec<String>,
    pub cwd: Option<String>,
}
pub type TestConfigFile = HashMap<String, TestConfig>;
pub fn read_test_config_file(path: &str) -> Result<TestConfigFile, Box<std::error::Error>> {
    let file = File::open(path)?;
    let content = serde_json::from_reader(file)?;
    Ok(content)
}

type BuildFile = HashMap<String, String>;
fn read_build_file(path: &str) -> Result<BuildFile, Box<std::error::Error>> {
    let file = File::open(path)?;
    let content = serde_json::from_reader(file)?;
    Ok(content)
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestGroup {
    pub find_glob: Option<String>,
    pub find_gtest: Option<Vec<String>>,
    pub id_pattern: String,
    pub matches_parent_dir: Option<bool>,
}
impl TestGroup {
    pub fn generate_test_inputs(&self, input_paths: &InputPaths) -> Vec<::TestInput> {
        let re = regex::Regex::new(&self.id_pattern).unwrap();
        if let Some(rel_path) = &self.find_glob {
            self.generate_paths(&rel_path, &input_paths.testcases_dir)
                .iter()
                .map(|rel_path| {
                    let id = re
                        .captures(&rel_path)
                        .expect("pattern did not match on one of the tests!")
                        .get(1)
                        .map_or("", |m| m.as_str());

                    ::TestInput {
                        id: id.to_string(),
                        rel_path: Some(PathBuf::from(rel_path)),
                    }
                }).collect()
        } else if let Some(filter) = &self.find_gtest {
            let cmd = &input_paths
                .exe_paths
                .get(filter[0].as_str())
                .expect("could not find find_gtest executable!");
            let output = std::process::Command::new(cmd)
                .args(filter[1..].iter())
                .output()
                .expect("failed to gather tests!");
            let output: &str = std::str::from_utf8(&output.stdout)
                .expect("could not decode find_gtest output as utf-8!");
            let mut group = String::new();
            let mut results = Vec::new();
            for line in output
                .lines()
                .filter(|l| !l.contains("DISABLED"))
                .map(|l| l.split('#').next().unwrap())
            {
                if !line.starts_with(' ') {
                    group = line.trim().to_string();
                } else {
                    let test_id = group.clone() + line.trim();
                    results.push(::TestInput {
                        id: test_id,
                        rel_path: None,
                    });
                }
            }
            results
        } else {
            panic!("no test generator defined!");
        }
    }
    fn generate_paths(&self, rel_path: &str, testcases_root: &Path) -> Vec<String> {
        let matches_parent_dir = self.matches_parent_dir.map_or(false, |b| b);
        let abs_path = testcases_root.join(rel_path).to_string_lossy().into_owned();
        glob::glob(&abs_path)
            .expect("failed to read glob pattern!")
            .map(|p| p.unwrap())
            .map(|p| {
                if matches_parent_dir {
                    PathBuf::from(p.parent().unwrap())
                } else {
                    p
                }
            }).map(|p| {
                let rel_path_buf = p.strip_prefix(testcases_root).unwrap().to_path_buf();
                rel_path_buf
                    .to_string_lossy()
                    .to_string()
                    .replace('\\', "/")
            }).collect()
    }
}

pub type TestGroupFile = HashMap<String, Vec<TestGroup>>;
pub fn read_test_group_file(path: &str) -> Result<TestGroupFile, Box<std::error::Error>> {
    let file = File::open(path)?;
    let content = serde_json::from_reader(file)?;
    Ok(content)
}

#[derive(Debug)]
pub struct InputPaths {
    pub build_dir: PathBuf,
    pub exe_paths: HashMap<String, String>,
    pub testcases_dir: PathBuf,
}
impl InputPaths {
    pub fn from(
        given_build_dir: &Option<&str>,
        build_file_path: &str,
        given_testcases_dir: &Option<&str>,
    ) -> InputPaths {
        let maybe_build_dir = given_build_dir
            .map_or_else(|| InputPaths::guess_build_dir(), |d| Some(PathBuf::from(d)));
        if maybe_build_dir.as_ref().map_or(true, |d| !d.exists()) {
            println!("couldn't find build-dir!");
            std::process::exit(-1);
        }
        let maybe_testcases_dir = given_testcases_dir.map_or_else(
            || InputPaths::guess_testcases_dir(),
            |d| Some(PathBuf::from(d)),
        );
        if maybe_testcases_dir.as_ref().map_or(true, |d| !d.exists()) {
            println!("couldn't find testcases-dir!");
            std::process::exit(-1);
        }
        let build_dir = maybe_build_dir.unwrap();
        let testcases_dir = maybe_testcases_dir.unwrap();

        let rel_exe_paths = read_build_file(&build_file_path);
        if !rel_exe_paths.is_ok() {
            println!(
                "failed to load build file \"{}\": {}",
                &build_file_path,
                rel_exe_paths.err().unwrap()
            );
            std::process::exit(-1);
        }

        let abs_exe_paths = rel_exe_paths
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), build_dir.join(v).to_string_lossy().to_string()))
            .collect();
        InputPaths {
            build_dir: PathBuf::from(build_dir),
            exe_paths: abs_exe_paths,
            testcases_dir: PathBuf::from(testcases_dir),
        }
    }

    fn find_root_dir() -> Option<PathBuf> {
        let cwd = std::env::current_dir().unwrap();
        let components = cwd.components();
        let dev_component = std::ffi::OsString::from("dev");
        let dev = components
            .into_iter()
            .take_while(|c| c.as_os_str() != dev_component);
        if dev.clone().next().is_none() {
            None
        } else {
            let root_components = dev.fold(PathBuf::from(""), |acc, c| acc.join(c));
            Some(root_components)
        }
    }

    fn guess_build_dir() -> Option<PathBuf> {
        InputPaths::find_root_dir().map(|p| p.join("dev"))
    }
    fn guess_testcases_dir() -> Option<PathBuf> {
        InputPaths::find_root_dir().map(|p| p.join("testcases"))
    }
}
