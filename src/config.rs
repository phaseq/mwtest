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
        let build_type: &str = match &input_paths.build_type {
            Some(b) => b,
            None => {
                println!("Please specify --build-type :)");
                std::process::exit(-1);
            }
        };
        let presets = [input_paths.preset.to_string()];
        Apps(
            self.0
                .iter()
                .filter(|(name, _config)| app_names.iter().any(|n| n == name || *n == "all"))
                .map(|(name, config)| {
                    let build_config = config.builds.get(build_type).unwrap_or_else(|| {
                        println!("build '{}' not found in '{}'", build_type, name);
                        std::process::exit(-1);
                    });
                    (name, config, build_config)
                })
                .filter(|(_name, _config, build_config)| !build_config.disabled)
                .map(|(name, config, build_config)| {
                    let build = Build::from(&build_config, &input_paths);
                    let tests = presets
                        .iter()
                        .filter_map(|p| config.tests.get(p))
                        .cloned()
                        .collect();
                    (
                        name.to_string(),
                        App::from(
                            config.command.clone(),
                            config.responsible.clone(),
                            build,
                            tests,
                            config.globber_matches_parent,
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
    pub responsible: String,                    // TODO
    pub alias: Option<Vec<String>>,             // TODO
    pub tags: Option<Vec<String>>,              // TODO
    pub accepted_returncodes: Option<Vec<u32>>, // TODO
    #[serde(default)]
    pub disabled: bool,   // TODO
    pub builds: HashMap<String, BuildConfig>,
    pub tests: HashMap<String, TestPresetConfig>,
    #[serde(default)]
    pub globber_matches_parent: bool,
    #[serde(default)]
    pub checkout_parent: bool, // TODO
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
    pub build: Build,
    pub tests: Vec<TestPresetConfig>,
    pub globber_matches_parent: bool,
}
impl App {
    fn from(
        mut command: CommandTemplate,
        responsible: String,
        build: Build,
        tests: Vec<TestPresetConfig>,
        globber_matches_parent: bool,
    ) -> Self {
        let patterns = [
            ("{{exe}}", Some(&build.exe)),
            ("{{dll}}", build.dll.as_ref()),
        ];
        for (p, r) in patterns.iter() {
            if command.has_pattern(p) {
                match r {
                    Some(r) => command = command.apply(p, r),
                    None => {
                        println!("Please specify {}", p);
                        std::process::exit(-1);
                    }
                }
            }
        }
        App {
            command,
            responsible,
            build,
            tests,
            globber_matches_parent,
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

#[derive(Debug, Clone)]
pub struct Build {
    pub exe: String,
    pub dll: Option<String>,
    pub cwd: Option<String>,
    pub solution: Option<String>,
    pub project: Option<String>,
}
impl Build {
    fn from(build: &BuildConfig, input_paths: &InputPaths) -> Build {
        let exe = Build::apply_config_string(&build.exe, &input_paths).unwrap();
        if !PathBuf::from(&exe).exists() {
            panic!("exe not found: {:?}", exe);
        }
        let dll = Build::apply_config_string(&build.dll, &input_paths);
        if let Some(dll) = &dll {
            if !PathBuf::from(&dll).exists() {
                panic!("dll not found: {:?}", dll);
            }
        }
        let cwd = Build::apply_config_string(&build.cwd, &input_paths);
        let solution = Build::apply_config_string(&build.solution, &input_paths);
        Build {
            exe,
            dll,
            cwd,
            solution,
            project: build.project.clone(),
        }
    }

    fn apply_config_string(string: &Option<String>, input_paths: &InputPaths) -> Option<String> {
        match &string {
            Some(s) => {
                let mut s = s.clone();
                if s.contains("{{dev_dir}}") {
                    let dev_dir = match input_paths.dev_dir.as_ref() {
                        Some(d) => d.to_str().unwrap(),
                        None => {
                            println!("Please specify --dev-dir :)");
                            std::process::exit(-1);
                        }
                    };
                    s = s.replace("{{dev_dir}}", dev_dir);
                }
                if s.contains("{{build_dir}}") {
                    let build_dir = match input_paths.build_dir.as_ref() {
                        Some(d) => d.to_str().unwrap(),
                        None => {
                            println!("Please specify --build-dir :)");
                            std::process::exit(-1);
                        }
                    };
                    s = s.replace("{{build_dir}}", build_dir);
                }
                if s.contains("{{testcases_dir}}") {
                    s = s.replace(
                        "{{testcases_dir}}",
                        input_paths.testcases_dir.to_str().unwrap(),
                    );
                }
                s = s.replace("{{build_config}}", &input_paths.build_config);
                Some(s)
            }
            None => None,
        }
    }
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
    pub timeout: Option<f32>,
    #[serde(default = "value_xge")]
    pub execution_style: String,
}

fn value_xge() -> String {
    "xge".to_string()
}

impl TestGroup {
    pub fn generate_test_inputs(
        &self,
        app: &App,
        preset: &TestPresetConfig,
        input_paths: &InputPaths,
    ) -> Vec<crate::TestId> {
        if self.find_glob.is_some() {
            let id_pattern = match &preset.id_pattern {
                Some(p) => &p,
                None => "(.*)",
            };
            self.generate_path_inputs(&app, id_pattern, &input_paths)
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
                if test_config.globber_matches_parent {
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
        let exe = &app.build.exe;
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
        let mut dev_dir: Option<PathBuf>;
        let mut build_dir: Option<PathBuf>;
        let mut build_type: Option<&str>;
        match InputPaths::guess_build_type() {
            BuildType::Dev(path) => {
                dev_dir = Some(path.clone());
                build_dir = Some(path);
                build_type = Some("dev-windows");
            }
            BuildType::Quickstart(path) => {
                dev_dir = None;
                build_dir = Some(path);
                build_type = Some("quickstart");
            }
            BuildType::None => {
                dev_dir = None;
                build_dir = None;
                build_type = None;
            }
        }
        dev_dir = given_dev_dir.map(PathBuf::from).or(dev_dir);
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
            dev_dir,
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
