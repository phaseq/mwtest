use serde_derive::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct AppPropertiesFile(HashMap<String, AppProperties>);

#[derive(Debug, Deserialize, Clone)]
pub struct AppProperties {
    pub command_template: CommandTemplate,
    #[serde(default)]
    pub input_is_dir: bool,
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub struct BuildLayoutFile {
    pub apps: HashMap<String, AppLayout>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AppLayout {
    pub solution: Option<String>,
    pub project: Option<String>,
    pub exe: String,
    pub cwd: Option<String>,
    pub dll: Option<String>,
}

#[derive(Debug)]
pub struct Apps(HashMap<String, App>);
impl Apps {
    fn open(
        properties_path: &Path,
        build_layout_path: &Path,
        build_dir: &Path,
    ) -> Result<Apps, Box<dyn std::error::Error>> {
        let properties: AppPropertiesFile = serde_json::from_reader(File::open(properties_path)?)?;
        let layout: BuildLayoutFile = serde_json::from_reader(File::open(build_layout_path)?)?;
        let apps = layout
            .apps
            .iter()
            .map(|(k, v)| {
                let mut app_layout = v.clone();
                apply_build_dir(&mut app_layout, &build_dir);
                let mut app_props = properties.0[k].clone();
                apply_layout(&mut app_props, &app_layout);
                (
                    (*k).clone(),
                    App {
                        properties: app_props,
                        layout: app_layout,
                    },
                )
            })
            .collect();
        Ok(Apps(apps))
    }

    pub fn get(&self, app_name: &str) -> Option<&App> {
        self.0.get(app_name)
    }

    pub fn app_names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct App {
    pub properties: AppProperties,
    pub layout: AppLayout,
}
impl App {}
fn apply_layout(app_props: &mut AppProperties, app_layout: &AppLayout) {
    app_props.command_template = app_props
        .command_template
        .apply("{{exe}}", app_layout.exe.as_str());
    if let Some(dll) = &app_layout.dll {
        app_props.command_template = app_props.command_template.apply("{{dll}}", dll);
    }
}
fn apply_build_dir(app_layout: &mut AppLayout, build_dir: &Path) {
    if let Some(solution) = &app_layout.solution {
        app_layout.solution = Some(
            build_dir
                .join(solution.clone())
                .to_str()
                .unwrap()
                .to_string(),
        );
    }
    app_layout.exe = build_dir
        .join(app_layout.exe.clone())
        .to_str()
        .unwrap()
        .to_string();
    if let Some(dll) = &app_layout.dll {
        app_layout.dll = Some(build_dir.join(dll.clone()).to_str().unwrap().to_string());
    }
}

#[derive(Debug, Deserialize)]
pub struct TestGroupFile(HashMap<String, TestGroups>);
impl TestGroupFile {
    pub fn open(path: &Path) -> Result<TestGroupFile, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let content = serde_json::from_reader(file)?;
        Ok(content)
    }

    pub fn get(&self, app_name: &str) -> Option<&TestGroups> {
        self.0.get(app_name)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestGroups {
    pub id_pattern: String,
    pub groups: Vec<TestGroup>,
}
impl TestGroups {
    pub fn generate_test_inputs(
        &self,
        group: &TestGroup,
        app: &App,
        input_paths: &InputPaths,
    ) -> Vec<crate::TestId> {
        if group.find_glob.is_some() {
            group.generate_path_inputs(&app.properties, &input_paths, &self.id_pattern)
        } else if group.find_gtest.is_some() {
            group.generate_gtest_inputs(&app)
        } else {
            panic!("no test generator defined!");
        }
    }
}

/*
"verifier": {
  "id_pattern": "cutsim/_servertest/verifier/(.*).verytest.ini",
  "groups": [
    {
      "find_glob": "cutsim/_servertest/verifier/smoke/**/*.verytest.ini"
    },
    {
      "find_glob": "cutsim/_servertest/verifier/nightly/**/*.verytest.ini"
    }
  ]
},*/

#[derive(Debug, Clone, Deserialize)]
pub struct TestGroup {
    find_glob: Option<String>,
    find_gtest: Option<String>,
    #[serde(default = "true_value")]
    pub xge: bool,
    pub timeout: Option<f32>,
}
impl TestGroup {
    fn generate_path_inputs(
        &self,
        test_config: &AppProperties,
        input_paths: &InputPaths,
        id_pattern: &str,
    ) -> Vec<crate::TestId> {
        let re = regex::Regex::new(&id_pattern).unwrap();
        let abs_path = input_paths
            .testcases_root
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
                p.strip_prefix(&input_paths.testcases_root)
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
        if !PathBuf::from(&app.layout.exe).exists() {
            println!(
                "Could not find GTest executable at {}!\nDid you forget to build?",
                app.layout.exe
            );
            std::process::exit(-1);
        }
        let output = std::process::Command::new(&app.layout.exe)
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

fn true_value() -> bool {
    true
}

#[derive(Debug)]
pub struct InputPaths {
    pub apps: Apps,
    pub preset_path: PathBuf,
    pub testcases_root: PathBuf,
}
impl InputPaths {
    pub fn get_registered_tests() -> Vec<String> {
        let path = InputPaths::mwtest_config_root().join("apps.json");
        let file = File::open(path).unwrap();
        let content: AppPropertiesFile = serde_json::from_reader(file).unwrap();
        content.0.keys().cloned().collect()
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
        build_dir = given_build_dir.map(PathBuf::from).or(build_dir);
        build_layout = given_build_layout.or(build_layout);
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
        testcases_root = given_testcases_root
            .map(PathBuf::from)
            .unwrap_or(testcases_root);
        preset = given_preset.unwrap_or(preset);
        if !testcases_root.exists() {
            println!("Could not determine build-dir! You may have to specify it explicitly!");
            std::process::exit(-1);
        }

        let build_layout_path = match InputPaths::mwtest_config_path(build_layout.unwrap()) {
            Some(path) => path,
            None => {
                println!("could not determine build layout! Please make sure that the path given via --build exists!");
                std::process::exit(-1);
            }
        };
        let app_config_path = InputPaths::mwtest_config_root().join("apps.json");
        let apps = Apps::open(&app_config_path, &build_layout_path, &build_dir.unwrap())
            .expect("Failed to load config files!");

        let preset_path = InputPaths::preset_from(&preset);
        InputPaths {
            apps,
            preset_path,
            testcases_root,
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
        if root.join("config/apps.json").exists() {
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
    /*pub fn apply_all(&self, patterns: &HashMap<String, String>) -> CommandTemplate {
        CommandTemplate(
            self.0
                .iter()
                .map(|t: &String| {
                    patterns
                        .iter()
                        .fold(t.to_owned(), |acc, (k, v)| acc.replace(k, v))
                })
                .collect(),
        )
    }*/
    pub fn has_pattern(&self, pattern: &str) -> bool {
        self.0.iter().any(|t| t.contains(pattern))
    }
}
