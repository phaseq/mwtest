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

#[derive(Debug, Deserialize)]
pub struct TestGroupFile(HashMap<String, Vec<TestGroup>>);
impl TestGroupFile {
    pub fn open(path: &Path) -> Result<TestGroupFile, Box<std::error::Error>> {
        let file = File::open(path)?;
        let content = serde_json::from_reader(file)?;
        Ok(content)
    }

    pub fn get(&self, app_name: &str) -> Option<&Vec<TestGroup>> {
        self.0.get(app_name)
    }
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
                ::TestId {
                    id: id.to_string(),
                    rel_path: Some(PathBuf::from(&rel_path)),
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

fn true_value() -> bool {
    true
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
        let path = InputPaths::mwtest_config_root().join("tests.json");
        let file = File::open(path).unwrap();
        let content: AppPropertiesFile = serde_json::from_reader(file).unwrap();
        content.app_names()
    }

    pub fn from(
        given_build_dir: &Option<&str>,
        given_testcases_root: &Option<&str>,
        given_build_layout: &Option<&str>,
        given_preset: &Option<&str>,
    ) -> InputPaths {
        let mut build_dir: Option<PathBuf>;
        let mut build_layout: Option<&str>;
        match InputPaths::guess_build_layout() {
            BuildLayout::Dev(path) => {
                build_dir = Some(path);
                build_layout = Some("dev-releaseunicode");
            }
            BuildLayout::Quickstart(path) => {
                build_dir = Some(path);
                build_layout = Some("quickstart");
            }
            BuildLayout::None => {
                build_dir = None;
                build_layout = None;
            }
        }
        if let Some(given_build_dir) = given_build_dir {
            build_dir = Some(PathBuf::from(given_build_dir));
        }
        if let Some(given_build_layout) = given_build_layout {
            build_layout = Some(given_build_layout);
        }
        if build_dir.is_none() || !build_dir.as_ref().unwrap().exists() {
            println!("Could not determine build-dir! You may have to specify it explicitly!");
            std::process::exit(-1);
        }
        if build_layout.is_none() {
            println!("Could not determine build layout! You may have to specify it explicitly!");
            std::process::exit(-1);
        }

        let mut testcases_root: PathBuf;
        let mut preset: &str;
        match InputPaths::guess_testcases_layout() {
            TestcasesLayout::Testcases(path) => {
                testcases_root = path;
                preset = "ci";
            }
            TestcasesLayout::Custom(path) => {
                testcases_root = path;
                preset = "all";
            }
        }
        if let Some(given_testcases_root) = given_testcases_root {
            testcases_root = PathBuf::from(given_testcases_root);
        }
        if let Some(given_preset) = given_preset {
            preset = given_preset;
        }
        if !testcases_root.exists() {
            println!("Could not determine build-dir! You may have to specify it explicitly!");
            std::process::exit(-1);
        }

        let build_layout_file =
            InputPaths::build_layout_from(&build_layout.unwrap(), &build_dir.unwrap());
        let preset_path = InputPaths::preset_from(&preset);

        let root_dir = InputPaths::mwtest_config_root();
        let app_config_path = root_dir.join("tests.json");
        let app_properties = AppPropertiesFile::open(&app_config_path, &build_layout_file).unwrap();

        InputPaths {
            app_properties: app_properties,
            build_file: build_layout_file,
            preset_path: preset_path,
            testcases_root: PathBuf::from(testcases_root),
        }
    }

    fn build_layout_from(build_layout: &str, build_dir: &Path) -> BuildLayoutFile {
        let path = match InputPaths::mwtest_config_path(build_layout) {
            Some(path) => path,
            None => {
                println!("could not determine build layout! Please make sure that the path given via --build exists!");
                std::process::exit(-1);
            }
        };
        match BuildLayoutFile::from(&path, &build_dir) {
            Ok(content) => content,
            Err(e) => {
                println!("ERROR: failed to load build file {:?}:\n{}", path, e);
                std::process::exit(-1);
            }
        }
    }

    fn preset_from(preset: &str) -> PathBuf {
        match InputPaths::mwtest_config_path(preset) {
            Some(path) => path,
            None => {
                println!("ERROR: could not determine preset! Please make sure that the path given via --preset exists!");
                std::process::exit(-1);
            }
        }
    }

    fn mwtest_config_path(name: &str) -> Option<PathBuf> {
        let root_dir = InputPaths::mwtest_config_root();
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

    fn mwtest_config_root() -> PathBuf {
        let root = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        if root.join("config/tests.json").exists() {
            root.join("config")
        } else {
            // for "cargo run"
            root.join("../../config")
        }
    }

    fn guess_build_layout() -> BuildLayout {
        if let Some(dev_root) = InputPaths::find_dev_root() {
            BuildLayout::Dev(dev_root.join("dev"))
        } else if InputPaths::is_quickstart() {
            BuildLayout::Quickstart(std::env::current_dir().unwrap())
        } else {
            BuildLayout::None
        }
    }

    fn guess_testcases_layout() -> TestcasesLayout {
        if let Some(dev_root) = InputPaths::find_dev_root() {
            let path = dev_root.join("testcases");
            if path.exists() {
                return TestcasesLayout::Testcases(path);
            }
        }
        TestcasesLayout::Custom(std::env::current_dir().unwrap())
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

    fn is_quickstart() -> bool {
        let cwd = std::env::current_dir().unwrap();
        cwd.join("mwVerifier.dll").exists() && cwd.join("5axutil.dll").exists()
    }
}

enum BuildLayout {
    Dev(PathBuf),
    Quickstart(PathBuf),
    None,
}

enum TestcasesLayout {
    Testcases(PathBuf),
    Custom(PathBuf),
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
