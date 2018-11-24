extern crate glob;
extern crate regex;
//use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct TestConfig {
    pub command_template: CommandTemplate,
    pub cwd: Option<String>,
    #[serde(default)]
    pub input_is_dir: bool,
}
impl TestConfig {
    fn apply_patterns(&mut self, patterns: &HashMap<String, String>) {
        self.command_template = self.command_template.apply_all(patterns);
        self.cwd = self.cwd.clone().map(|d| patterns[&d].clone());
    }
}
pub type TestConfigFile = HashMap<String, TestConfig>;
fn read_test_config_file(
    path: &Path,
    build_file: &BuildFile,
) -> Result<TestConfigFile, Box<std::error::Error>> {
    let file = File::open(path)?;
    let mut content: TestConfigFile = serde_json::from_reader(file)?;
    for test in content.values_mut() {
        (*test).apply_patterns(&build_file.exes);
    }
    Ok(content)
}

#[derive(Debug, Deserialize)]
pub struct BuildFile {
    pub dependencies: HashMap<String, BuildDependency>,
    pub exes: HashMap<String, String>,
}
impl BuildFile {
    pub fn from(path: &Path, build_dir: &Path) -> Result<BuildFile, Box<std::error::Error>> {
        let file = File::open(path)?;
        let mut content: BuildFile = serde_json::from_reader(file)?;
        for v in content.dependencies.values_mut() {
            *v = BuildDependency {
                solution: build_dir
                    .clone()
                    .join(v.solution.clone())
                    .to_string_lossy()
                    .into_owned(),
                project: v.project.clone(),
            };
        }
        for p in content.exes.values_mut() {
            *p = build_dir
                .clone()
                .join(p.clone())
                .to_string_lossy()
                .into_owned();
        }
        Ok(content)
    }
}

#[derive(Debug, Deserialize)]
pub struct BuildDependency {
    pub solution: String,
    pub project: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestGroup {
    pub find_glob: Option<String>,
    pub find_gtest: Option<Vec<String>>,
    pub id_pattern: String,
}
impl TestGroup {
    pub fn generate_test_inputs(
        &self,
        test_config: &TestConfig,
        input_paths: &InputPaths,
    ) -> Vec<::TestId> {
        if self.find_glob.is_some() {
            self.generate_path_inputs(&test_config, &input_paths)
        } else if self.find_gtest.is_some() {
            self.generate_gtest_inputs(&input_paths)
        } else {
            panic!("no test generator defined!");
        }
    }
    fn generate_path_inputs(
        &self,
        test_config: &TestConfig,
        input_paths: &InputPaths,
    ) -> Vec<::TestId> {
        let re = regex::Regex::new(&self.id_pattern).unwrap();
        let abs_path = input_paths
            .testcases_root
            .join(self.find_glob.clone().unwrap())
            .to_string_lossy()
            .into_owned();
        glob::glob(&abs_path)
            .expect("failed to read glob pattern!")
            .map(|p| p.unwrap())
            .map(|p| {
                if !test_config.input_is_dir {
                    PathBuf::from(p.parent().unwrap())
                } else {
                    p
                }
            }).map(|p| {
                let rel_path_buf = p
                    .strip_prefix(&input_paths.testcases_root)
                    .unwrap()
                    .to_path_buf();
                rel_path_buf
                    .to_string_lossy()
                    .to_string()
                    .replace('\\', "/")
            }).map(|rel_path: String| {
                let id = re
                    .captures(&rel_path)
                    .expect("pattern did not match on one of the tests!")
                    .get(1)
                    .map_or("", |m| m.as_str());

                ::TestId {
                    id: id.to_string(),
                    rel_path: Some(PathBuf::from(rel_path.clone())),
                }
            }).collect()
    }
    fn generate_gtest_inputs(&self, input_paths: &InputPaths) -> Vec<::TestId> {
        let filter = self.find_gtest.clone().unwrap();
        let cmd = &input_paths
            .build_file
            .exes
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
                results.push(::TestId {
                    id: test_id,
                    rel_path: None,
                });
            }
        }
        results
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
    pub test_config: TestConfigFile,
    pub build_file: BuildFile,
    pub testcases_root: PathBuf,
}
impl InputPaths {
    pub fn from(
        given_build_dir: &Option<&str>,
        given_testcases_root: &Option<&str>,
        given_build_file_path: &Option<&str>,
        given_test_config_path: &Option<&str>,
    ) -> InputPaths {
        let maybe_build_dir = given_build_dir
            .map_or_else(|| InputPaths::guess_build_dir(), |d| Some(PathBuf::from(d)));
        if maybe_build_dir.as_ref().map_or(true, |d| !d.exists()) {
            println!("couldn't find build-dir!");
            std::process::exit(-1);
        }
        let maybe_testcases_root = given_testcases_root.map_or_else(
            || InputPaths::guess_testcases_root(),
            |d| Some(PathBuf::from(d)),
        );
        if maybe_testcases_root.as_ref().map_or(true, |d| !d.exists()) {
            println!("couldn't find testcases-dir!");
            std::process::exit(-1);
        }
        let build_dir = maybe_build_dir.unwrap();
        let testcases_root = maybe_testcases_root.unwrap();

        let root_dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("../../");

        let build_file_path = root_dir.join(given_build_file_path.unwrap());
        let build_file = BuildFile::from(&build_file_path, &build_dir).unwrap();

        let test_config_path = root_dir.join(given_test_config_path.unwrap());
        let test_config = read_test_config_file(&test_config_path, &build_file).unwrap();

        InputPaths {
            test_config: test_config,
            build_file: build_file,
            testcases_root: PathBuf::from(testcases_root),
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
    fn guess_testcases_root() -> Option<PathBuf> {
        InputPaths::find_root_dir().map(|p| p.join("testcases"))
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CommandTemplate(pub Vec<String>);
impl CommandTemplate {
    pub fn apply(&self, from: &str, to: &str) -> CommandTemplate {
        CommandTemplate(
            self.0
                .iter()
                .map(|t| t.to_owned().replace(from, to))
                .collect(),
        )
    }
    pub fn apply_all(&self, patterns: &HashMap<String, String>) -> CommandTemplate {
        CommandTemplate(
            self.0
                .iter()
                .map(|t: &String| {
                    patterns
                        .iter()
                        .fold(t.to_owned(), |acc, (k, v)| acc.replace(k, v))
                }).collect(),
        )
    }
    pub fn has_pattern(&self, pattern: &str) -> bool {
        self.0.iter().any(|t| t == pattern)
    }
}
