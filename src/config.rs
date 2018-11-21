extern crate glob;
extern crate regex;
//use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct TestConfig {
    pub command: Vec<String>,
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
    pub find: String,
    pub id_pattern: String,
    pub matches_parent_dir: Option<bool>,
}
impl TestGroup {
    pub fn generate_test_inputs(&self, testcases_root: &Path) -> Vec<::TestInput> {
        let re = regex::Regex::new(&self.id_pattern).unwrap();
        self.generate_paths(&testcases_root)
            .iter()
            .map(|p| {
                let rel_path_buf = p.strip_prefix(testcases_root).unwrap().to_path_buf();
                let rel_path = rel_path_buf
                    .to_string_lossy()
                    .to_string()
                    .replace('\\', "/");
                let id = re
                    .captures(&rel_path)
                    .expect("pattern did not match on one of the tests!")
                    .get(1)
                    .map_or("", |m| m.as_str());

                ::TestInput {
                    id: id.to_string(),
                    rel_path: rel_path_buf,
                }
            }).collect()
    }
    fn generate_paths(&self, testcases_root: &Path) -> Vec<PathBuf> {
        let pattern: Vec<&str> = self.find.split(':').collect();
        let rel_path = testcases_root
            .join(pattern[1])
            .to_string_lossy()
            .into_owned();
        let paths = glob::glob(&rel_path)
            .expect("failed to read glob pattern!")
            .map(|s| s.unwrap());
        if self.matches_parent_dir.map_or(false, |b| b) {
            paths.map(|s| PathBuf::from(s.parent().unwrap())).collect()
        } else {
            paths.collect()
        }
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
