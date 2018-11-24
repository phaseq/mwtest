extern crate glob;
extern crate regex;
//use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub command_template: CommandTemplate,
    pub cwd: Option<String>,
    #[serde(default)]
    pub input_is_dir: bool,
}
impl AppConfig {
    fn apply_patterns(&mut self, patterns: &HashMap<String, String>) {
        self.command_template = self.command_template.apply_all(patterns);
        self.cwd = self.cwd.clone().map(|d| patterns[&d].clone());
    }
}
pub type AppConfigFile = HashMap<String, AppConfig>;
fn read_app_config_file(
    path: &Path,
    build_file: &BuildLayoutFile,
) -> Result<AppConfigFile, Box<std::error::Error>> {
    let file = File::open(path)?;
    let mut content: AppConfigFile = serde_json::from_reader(file)?;
    for test in content.values_mut() {
        (*test).apply_patterns(&build_file.exes);
    }
    Ok(content)
}

#[derive(Debug, Deserialize)]
pub struct BuildLayoutFile {
    pub dependencies: HashMap<String, BuildDependency>,
    pub exes: HashMap<String, String>,
}
impl BuildLayoutFile {
    pub fn from(path: &Path, build_dir: &Path) -> Result<BuildLayoutFile, Box<std::error::Error>> {
        let file = File::open(path)?;
        let mut content: BuildLayoutFile = serde_json::from_reader(file)?;
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
        test_config: &AppConfig,
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
        test_config: &AppConfig,
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
                if test_config.input_is_dir {
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
                let capture = re.captures(&rel_path);
                if capture.is_none() {
                    println!(
                        "pattern did not match on one of the tests!\n pattern: {}\n test: {}",
                        &self.id_pattern, &rel_path
                    );
                    std::process::exit(-1);
                }
                let id = capture.unwrap().get(1).map_or("", |m| m.as_str());
                let test_location = if test_config.input_is_dir {
                    ::RelTestLocation::Dir(PathBuf::from(&rel_path))
                } else {
                    ::RelTestLocation::File(PathBuf::from(&rel_path))
                };
                ::TestId {
                    id: id.to_string(),
                    rel_path: test_location,
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
                    rel_path: ::RelTestLocation::None,
                });
            }
        }
        results
    }
}

pub type TestGroupFile = HashMap<String, Vec<TestGroup>>;
pub fn read_test_group_file(path: &Path) -> Result<TestGroupFile, Box<std::error::Error>> {
    let file = File::open(path)?;
    let content = serde_json::from_reader(file)?;
    Ok(content)
}

#[derive(Debug)]
pub struct InputPaths {
    pub app_config: AppConfigFile,
    pub build_file: BuildLayoutFile,
    pub preset_path: PathBuf,
    pub testcases_root: PathBuf,
}
impl InputPaths {
    fn get_root_path() -> PathBuf {
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("../../")
    }

    pub fn get_registered_tests() -> Vec<String> {
        let path = InputPaths::get_root_path().join("tests.json");
        let file = File::open(path).unwrap();
        let content: AppConfigFile = serde_json::from_reader(file).unwrap();
        content.keys().cloned().collect()
    }

    pub fn from(
        given_build_dir: &Option<&str>,
        given_testcases_root: &Option<&str>,
        build_layout: &str,
        preset: &str,
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

        let root_dir = InputPaths::get_root_path();

        let mut build_layout_path = root_dir.join(build_layout.to_owned() + ".json");
        if !build_layout_path.exists() {
            build_layout_path = PathBuf::from(build_layout);
            if !build_layout_path.exists() {
                println!("could not determine build layout! Please make sure that the path given via --build exists!");
                std::process::exit(-1);
            }
        }
        let mut preset_path = root_dir.join(preset.to_owned() + ".json");
        if !preset_path.exists() {
            preset_path = PathBuf::from(preset);
            if !preset_path.exists() {
                println!("could not determine build layout! Please make sure that the path given via --build exists!");
                std::process::exit(-1);
            }
        }

        let build_layout_file = BuildLayoutFile::from(&build_layout_path, &build_dir);
        if build_layout_file.is_err() {
            println!(
                "failed to load build file {:?}:\n{:?}",
                build_layout_path,
                build_layout_file.unwrap_err()
            );
            std::process::exit(-1);
        }
        let build_file = build_layout_file.unwrap();

        let app_config_path = root_dir.join("tests.json");
        let app_config = read_app_config_file(&app_config_path, &build_file).unwrap();

        InputPaths {
            app_config: app_config,
            build_file: build_file,
            preset_path: preset_path,
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
