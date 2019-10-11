use serde_derive::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct AppsConfig(pub HashMap<String, AppConfig>);
impl AppsConfig {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(serde_json::from_reader(File::open(
            InputPaths::mwtest_config_path(),
        )?)?)
    }

    pub fn app_names(self: &Self) -> Vec<String> {
        let mut names: Vec<_> = self.0.iter().map(|(n, _)| n.to_string()).collect();
        names.sort();
        names
    }

    pub fn select_build_and_preset(
        self: &Self,
        app_names: &Vec<&str>,
        input_paths: &InputPaths,
    ) -> Apps {
        let build_type = input_paths
            .build_type
            .as_ref()
            .expect("Please specify --build-type :)");
        let presets = [input_paths.preset.to_string()];
        Apps(
            self.0
                .iter()
                .filter(|(name, _config)| app_names.iter().any(|n| n == name || *n == "all"))
                .map(|(name, config)| {
                    let build_config = config
                        .builds
                        .get(build_type)
                        .unwrap_or_else(|| panic!("build '{}' not found in '{}'", build_type, name))
                        .clone();
                    let tests = presets
                        .iter()
                        .filter_map(|p| config.tests.get(p))
                        .cloned()
                        .collect();
                    (
                        name.to_string(),
                        App::from(
                            &input_paths,
                            config.command.clone(),
                            config.responsible.clone(),
                            build_config.clone(),
                            tests,
                            config.input_is_dir,
                        ),
                    )
                })
                .collect(),
        )
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub command: CommandTemplate,
    pub responsible: String,
    pub builds: HashMap<String, BuildConfig>,
    pub tests: HashMap<String, TestPresetConfig>,
    #[serde(default)]
    pub input_is_dir: bool,
}

#[derive(Debug)]
pub struct Apps(pub HashMap<String, App>);
impl Apps {
    pub fn app_names(self: &Self) -> Vec<String> {
        let mut names: Vec<_> = self.0.iter().map(|(n, _)| n.clone()).collect();
        names.sort();
        names
    }
}

#[derive(Debug, Clone)]
pub struct App {
    pub command: CommandTemplate,
    pub responsible: String,
    pub build: BuildConfig,
    pub tests: Vec<TestPresetConfig>,
    pub input_is_dir: bool,
}
impl App {
    fn from(
        input_paths: &InputPaths,
        mut command: CommandTemplate,
        responsible: String,
        build: BuildConfig,
        tests: Vec<TestPresetConfig>,
        input_is_dir: bool,
    ) -> Self {
        if command.has_pattern("{{exe}}") {
            let exe = build.exe.as_ref().unwrap();
            command = command.apply("{{exe}}", &exe);
        }
        if command.has_pattern("{{dll}}") {
            let dll = build.dll.as_ref().unwrap();
            command = command.apply("{{dll}}", &dll);
        }
        if command.has_pattern("{{dev_dir}}") {
            command = command.apply(
                "{{dev_dir}}",
                input_paths
                    .dev_dir
                    .as_ref()
                    .unwrap_or_else(|| panic!("Please specify --dev-dir :)"))
                    .to_str()
                    .unwrap(),
            );
        }
        if command.has_pattern("{{build_dir}}") {
            command = command.apply(
                "{{build_dir}}",
                input_paths
                    .dev_dir
                    .as_ref()
                    .unwrap_or_else(|| panic!("Please specify --build-dir :)"))
                    .to_str()
                    .unwrap(),
            );
        }
        if command.has_pattern("{{build_config}}") {
            command = command.apply("{{build_config}}", &input_paths.build_config);
        }
        App {
            command,
            responsible,
            build,
            tests,
            input_is_dir,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BuildConfig {
    pub exe: Option<String>,
    pub dll: Option<String>,
    pub cwd: Option<String>,
    pub solution: Option<String>,
    pub project: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TestPresetConfig {
    pub id_pattern: Option<String>,
    pub groups: Vec<TestGroup>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TestGroup {
    pub find_glob: Option<String>,
    pub find_gtest: Option<String>,
    #[serde(default = "true_value")]
    pub xge: bool,
    pub timeout: Option<f32>,
    pub execution_style: Option<String>,
}

impl TestGroup {
    pub fn generate_test_inputs(
        &self,
        app: &App,
        preset: &TestPresetConfig,
        input_paths: &InputPaths,
    ) -> Vec<crate::TestId> {
        if self.find_glob.is_some() {
            self.generate_path_inputs(
                &app,
                &preset.id_pattern.as_ref().unwrap_or(&"(.*)".to_string()),
                &input_paths,
            )
        } else if self.find_gtest.is_some() {
            self.generate_gtest_inputs(&app)
        } else {
            panic!("no test generator defined!");
        }
    }

    fn generate_path_inputs(
        &self,
        test_config: &App,
        id_pattern: &str,
        input_paths: &InputPaths,
    ) -> Vec<crate::TestId> {
        let re = regex::Regex::new(&id_pattern).unwrap();
        let abs_path = input_paths
            .testcases_dir
            .join(self.find_glob.clone().unwrap())
            .to_str()
            .unwrap()
            .to_string();
        glob::glob(&abs_path)
            .expect("failed to read glob pattern!")
            .map(Result::unwrap)
            .map(|p| {
                if test_config.input_is_dir {
                    PathBuf::from(p.parent().unwrap())
                } else {
                    p
                }
            })
            .map(|p| {
                p.strip_prefix(&input_paths.testcases_dir)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/")
            })
            .map(|rel_path: String| {
                let id = match re.captures(&rel_path) {
                    Some(capture) => capture.get(1).map_or("", |m| m.as_str()),
                    None => {
                        println!(
                            "pattern did not match on one of the tests!\n pattern: {}\n test: {}",
                            &id_pattern, &rel_path
                        );
                        std::process::exit(-1);
                    }
                };
                crate::TestId {
                    id: id.to_string(),
                    rel_path: Some(PathBuf::from(&rel_path)),
                }
            })
            .collect()
    }
    fn generate_gtest_inputs(&self, app: &App) -> Vec<crate::TestId> {
        let filter = self.find_gtest.clone().unwrap();
        let exe = &app.build.exe.as_ref().expect("Exe was not specified!");
        if !PathBuf::from(exe).exists() {
            println!(
                "Could not find GTest executable at {}!\nDid you forget to build?",
                exe
            );
            std::process::exit(-1);
        }
        let output = std::process::Command::new(exe)
            .arg("--gtest_list_tests")
            .arg(format!("--gtest_filter={}", filter))
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
                results.push(crate::TestId {
                    id: test_id,
                    rel_path: None,
                });
            }
        }
        results
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
    pub fn has_pattern(&self, pattern: &str) -> bool {
        self.0.iter().any(|t| t.contains(pattern))
    }
}

fn true_value() -> bool {
    true
}

#[derive(Debug)]
pub struct InputPaths {
    pub dev_dir: Option<PathBuf>,
    pub build_dir: Option<PathBuf>,
    pub testcases_dir: PathBuf,
    pub build_type: Option<String>,
    pub preset: String,
    pub build_config: String,
}
impl InputPaths {
    pub fn get_registered_tests() -> Vec<String> {
        let path = InputPaths::mwtest_config_root().join("apps.json");
        let file = File::open(path).expect("didn't find apps.json!");
        let content: AppsConfig = serde_json::from_reader(file).unwrap();
        content.0.keys().cloned().collect()
    }

    pub fn from(
        given_dev_dir: &Option<&str>,
        given_build_dir: &Option<&str>,
        given_testcases_dir: &Option<&str>,
        given_build_type: &Option<&str>,
        given_preset: &Option<&str>,
        given_build_config: &Option<&str>,
    ) -> InputPaths {
        let mut build_dir: Option<PathBuf>;
        let mut build_type: Option<&str>;
        match InputPaths::guess_build_type() {
            BuildType::Dev(path) => {
                build_dir = Some(path);
                build_type = Some("dev-windows");
            }
            BuildType::Quickstart(path) => {
                build_dir = Some(path);
                build_type = Some("quickstart");
            }
            BuildType::None => {
                build_dir = None;
                build_type = None;
            }
        }
        build_dir = given_build_dir.map(PathBuf::from).or(build_dir);
        build_type = given_build_type.or(build_type);

        let testcases_dir: PathBuf;
        let preset: &str;
        match InputPaths::guess_testcases_layout() {
            TestcasesLayout::Testcases(path) => {
                testcases_dir = path;
                preset = "ci";
            }
            TestcasesLayout::Custom(path) => {
                testcases_dir = path;
                preset = "all";
            }
        }
        let testcases_dir = given_testcases_dir
            .map(PathBuf::from)
            .unwrap_or(testcases_dir);
        let preset = given_preset.unwrap_or(preset).to_string();
        if !testcases_dir.exists() {
            println!("Could not determine build-dir! You may have to specify it explicitly!");
            std::process::exit(-1);
        }

        let build_config = given_build_config.unwrap_or("ReleaseUnicode").to_string();

        InputPaths {
            dev_dir: given_dev_dir.map(|s| PathBuf::from(s)),
            build_dir,
            testcases_dir,
            build_type: build_type.map(|s| s.to_string()),
            preset,
            build_config,
        }
    }

    fn mwtest_config_path() -> PathBuf {
        InputPaths::mwtest_config_root().join("apps.json")
    }

    fn mwtest_config_root() -> PathBuf {
        let root = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        if root.join("apps.json").exists() {
            root
        } else {
            // for "cargo run"
            root.join("../..")
        }
    }

    fn guess_build_type() -> BuildType {
        if let Some(dev_root) = InputPaths::find_dev_root() {
            BuildType::Dev(dev_root.join("dev"))
        } else if InputPaths::is_quickstart() {
            BuildType::Quickstart(std::env::current_dir().unwrap())
        } else {
            BuildType::None
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
        let dev_component = std::ffi::OsString::from("dev");
        let mut found = false;
        let dev: Vec<_> = cwd
            .components()
            .take_while(|c| {
                found = c.as_os_str() == dev_component;
                !found
            })
            .collect();
        if !found {
            None
        } else {
            let root_components = dev.iter().fold(PathBuf::from(""), |acc, c| acc.join(c));
            Some(root_components)
        }
    }

    fn is_quickstart() -> bool {
        let cwd = std::env::current_dir().unwrap();
        cwd.join("mwVerifier.dll").exists() && cwd.join("5axutil.dll").exists()
    }
}

enum BuildType {
    Dev(PathBuf),
    Quickstart(PathBuf),
    None,
}

enum TestcasesLayout {
    Testcases(PathBuf),
    Custom(PathBuf),
}
