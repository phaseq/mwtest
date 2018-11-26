extern crate glob;
extern crate regex;
//use regex::Regex;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct AppPropertiesFile(HashMap<String, AppProperties>);
impl AppPropertiesFile {
    fn open(
        path: &Path,
        build_layout: &BuildLayoutFile,
    ) -> Result<AppPropertiesFile, Box<std::error::Error>> {
        let file = File::open(path)?;
        let mut content: AppPropertiesFile = serde_json::from_reader(file)?;
        for test in content.0.values_mut() {
            (*test).apply_patterns(&build_layout.exes);
        }
        Ok(content)
    }

    pub fn get(&self, app_name: &str) -> Option<&AppProperties> {
        self.0.get(app_name)
    }

    pub fn app_names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppProperties {
    pub command_template: CommandTemplate,
    pub cwd: Option<String>,
    #[serde(default)]
    pub input_is_dir: bool,
}
impl AppProperties {
    fn apply_patterns(&mut self, patterns: &HashMap<String, String>) {
        self.command_template = self.command_template.apply_all(patterns);
        self.cwd = self.cwd.clone().map(|d| patterns[&d].clone());
    }
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
    #[serde(default = "true_value")]
    pub xge: bool,
}
impl TestGroup {
    pub fn generate_test_inputs(
        &self,
        test_config: &AppProperties,
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
        test_config: &AppProperties,
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
                p.strip_prefix(&input_paths.testcases_root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
                    .replace('\\', "/")
            }).map(|rel_path: String| {
                let id = match re.captures(&rel_path) {
                    Some(capture) => capture.get(1).map_or("", |m| m.as_str()),
                    None => {
                        println!(
                            "pattern did not match on one of the tests!\n pattern: {}\n test: {}",
                            &self.id_pattern, &rel_path
                        );
                        std::process::exit(-1);
                    }
                };
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

fn true_value() -> bool {
    true
}

pub type TestGroupFile = HashMap<String, Vec<TestGroup>>;
pub fn read_test_group_file(path: &Path) -> Result<TestGroupFile, Box<std::error::Error>> {
    let file = File::open(path)?;
    let content = serde_json::from_reader(file)?;
    Ok(content)
}

#[derive(Debug)]
pub struct InputPaths {
    pub app_properties: AppPropertiesFile,
    pub build_file: BuildLayoutFile,
    pub preset_path: PathBuf,
    pub testcases_root: PathBuf,
}
impl InputPaths {
    pub fn get_registered_tests() -> Vec<String> {
        let path = InputPaths::get_mwtest_root().join("tests.json");
        let file = File::open(path).unwrap();
        let content: AppPropertiesFile = serde_json::from_reader(file).unwrap();
        content.app_names()
    }

    pub fn from(
        given_build_dir: &Option<&str>,
        given_testcases_root: &Option<&str>,
        build_layout: &str,
        preset: &str,
    ) -> InputPaths {
        let build_dir = InputPaths::build_dir_from(&given_build_dir);
        let testcases_root = InputPaths::testcases_dir_from(&given_testcases_root);
        let preset_path = InputPaths::preset_from(&preset);
        let build_layout_path = InputPaths::build_layout_from(&build_layout);

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

        let root_dir = InputPaths::get_mwtest_root();
        let app_config_path = root_dir.join("tests.json");
        let app_properties = AppPropertiesFile::open(&app_config_path, &build_file).unwrap();

        InputPaths {
            app_properties: app_properties,
            build_file: build_file,
            preset_path: preset_path,
            testcases_root: PathBuf::from(testcases_root),
        }
    }

    fn build_dir_from(given_build_dir: &Option<&str>) -> PathBuf {
        let build_dir = given_build_dir
            .map_or_else(|| InputPaths::guess_build_dir(), |d| Some(PathBuf::from(d)));
        if build_dir.as_ref().map_or(true, |d| !d.exists()) {
            println!("couldn't find build-dir!");
            std::process::exit(-1);
        }
        build_dir.unwrap()
    }

    fn testcases_dir_from(given_testcases_root: &Option<&str>) -> PathBuf {
        let testcases_root = given_testcases_root.map_or_else(
            || InputPaths::guess_testcases_root(),
            |d| Some(PathBuf::from(d)),
        );
        if testcases_root.as_ref().map_or(true, |d| !d.exists()) {
            println!("couldn't find testcases-dir!");
            std::process::exit(-1);
        }
        testcases_root.unwrap()
    }

    fn build_layout_from(build_layout: &str) -> PathBuf {
        match InputPaths::mwtest_config_path(build_layout) {
            Some(path) => path,
            None => {
                println!("could not determine build layout! Please make sure that the path given via --build exists!");
                std::process::exit(-1);
            }
        }
    }

    fn preset_from(preset: &str) -> PathBuf {
        match InputPaths::mwtest_config_path(preset) {
            Some(path) => path,
            None => {
                println!("could not determine preset! Please make sure that the path given via --preset exists!");
                std::process::exit(-1);
            }
        }
    }

    fn mwtest_config_path(name: &str) -> Option<PathBuf> {
        let root_dir = InputPaths::get_mwtest_root();
        let path = root_dir.join(name.to_owned() + ".json");
        if path.exists() {
            return Some(path);
        }
        let path = PathBuf::from(name);
        if path.exists() {
            return Some(path);
        }
        None
    }

    fn get_mwtest_root() -> PathBuf {
        let root = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        if root.join("tests.json").exists() {
            root
        } else {
            // for "cargo run"
            root.join("../../")
        }
    }

    fn guess_build_dir() -> Option<PathBuf> {
        InputPaths::find_dev_root().map(|p| p.join("dev"))
    }
    fn guess_testcases_root() -> Option<PathBuf> {
        InputPaths::find_dev_root().map(|p| p.join("testcases"))
    }

    fn find_dev_root() -> Option<PathBuf> {
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
        self.0.iter().any(|t| t.contains(pattern))
    }
}
