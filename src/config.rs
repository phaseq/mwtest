use color_eyre::eyre::{eyre, ContextCompat, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufRead;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};

// The "*Config" structs in this module have exactly the same structure as apps.json.
// Instantiating them via Apps::select_build_and_preset creates the corresponding "*" structures.
// The difference between "*Config" and "*" structs:
//    * they are filtered
//    * keys that can be specified at different levels (like "command") are passed through to the lowest level

#[derive(Debug, Deserialize, Clone)]
pub struct AppsConfig(pub HashMap<String, AppConfig>);

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub command: CommandTemplate,
    pub responsible: String,
    #[serde(default)]
    pub alias: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_retcodes")]
    pub accepted_returncodes: Vec<i32>,
    #[serde(default)]
    pub disabled: bool,
    pub builds: HashMap<String, BuildConfig>,
    pub tests: HashMap<String, TestPresetConfig>,
    #[serde(default)]
    pub globber_matches_parent: bool,
    #[serde(default)]
    pub checkout_parent: bool, // TODO
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
    pub command: Option<CommandTemplate>,
    pub id_pattern: Option<String>,
    pub groups: Vec<TestGroupConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CommandTemplate(pub Vec<String>);

#[derive(Debug, Deserialize, Clone)]
pub struct TestGroupConfig {
    pub command: Option<CommandTemplate>,
    pub find_glob: Option<String>,
    pub find_gtest: Option<String>,
    pub timeout: Option<f32>,
    pub timeout_if_changed: Option<f32>, // TODO
    #[serde(default)]
    pub testcases_dependencies: Vec<String>,
    #[serde(default = "value_xge")]
    pub execution_style: String,
}

#[derive(Debug)]
pub struct Apps(pub HashMap<String, App>);

#[derive(Debug, Clone)]
pub struct App {
    pub responsible: String,
    pub build: Build,
    pub tests: Vec<TestPreset>,
    pub globber_matches_parent: bool,
    pub checkout_parent: bool,
}

#[derive(Debug, Clone)]
pub struct Build {
    pub exe: String,
    pub dll: Option<String>,
    pub cwd: Option<String>,
    pub solution: Option<String>,
    pub project: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TestPreset {
    pub id_pattern: Option<String>,
    pub groups: Vec<TestGroup>,
}

#[derive(Debug, Clone)]
pub struct TestGroup {
    pub command: CommandTemplate,
    pub find_glob: Option<String>,
    pub find_gtest: Option<String>,
    pub timeout: Option<f32>,
    pub timeout_if_changed: Option<f32>, // TODO
    pub accepted_returncodes: Vec<i32>,
    pub testcases_dependencies: Vec<String>,
    pub execution_style: String,
}

impl AppsConfig {
    pub fn load(dev_dir: &Option<PathBuf>, build_dir: &Path) -> Result<Self> {
        let apps_json_path = match (dev_dir, build_dir) {
            (Some(dev_dir), _) if dev_dir.join("tools/mwtest/apps.json").exists() => {
                dev_dir.join("tools/mwtest/apps.json")
            }
            (_, build_dir) if build_dir.join("mwtest/apps.json").exists() => {
                build_dir.join("mwtest/apps.json")
            }
            _ => return Err(eyre!("Could not find apps.json!")),
        };
        Ok(serde_json::from_reader(File::open(apps_json_path)?)?)
    }

    pub fn app_names(self: &Self) -> Vec<String> {
        let mut names: Vec<_> = self.0.iter().map(|(n, _)| n.to_string()).collect();
        names.sort();
        names
    }

    pub fn select_build_and_preset(
        self: Self,
        app_names: &[String],
        input_paths: &InputPaths,
    ) -> Result<Apps> {
        let selected_app_configs = self.0.into_iter().filter(|(name, config)| {
            app_names.iter().any(|n| {
                let n = n.to_lowercase();
                n == name.to_lowercase()
                    || n == "all"
                    || config.alias.iter().any(|a| a.to_lowercase() == n)
                    || config.tags.iter().any(|t| t.to_lowercase() == n)
            })
        });

        let apps: Result<Vec<_>> = selected_app_configs
            .filter_map(|(name, config)| {
                config
                    .select_build_and_preset(&name, input_paths)
                    .map(|option_app| option_app.map(|app| (name, app)))
                    .transpose()
            })
            .collect();
        let apps = HashMap::from_iter(apps?.into_iter());
        Ok(Apps(apps))
    }
}

impl AppConfig {
    fn select_build_and_preset(
        mut self: Self,
        name: &str,
        input_paths: &InputPaths,
    ) -> Result<Option<App>> {
        let build_type: &str = match &input_paths.build_type {
            Some(b) => b,
            None => {
                println!("Please specify --build-type");
                std::process::exit(-1);
            }
        };
        let build_config = self
            .builds
            .remove(build_type)
            .wrap_err_with(|| format!("build '{}' not found in '{}'", build_type, name))?;
        if build_config.disabled {
            return Ok(None);
        }

        let build = Build::from(&build_config, &input_paths);
        let tests = input_paths
            .preset
            .split('+')
            .filter_map(|p| self.tests.get(p))
            .cloned()
            .collect();
        Ok(Some(App::from(
            input_paths,
            self.command,
            self.responsible,
            build,
            tests,
            self.globber_matches_parent,
            self.checkout_parent,
            &self.accepted_returncodes,
        )))
    }
}

impl App {
    fn from(
        input_paths: &InputPaths,
        command: CommandTemplate,
        responsible: String,
        build: Build,
        tests: Vec<TestPresetConfig>,
        globber_matches_parent: bool,
        checkout_parent: bool,
        accepted_returncodes: &[i32],
    ) -> Self {
        let patterns = [
            ("{{exe}}", Some(&build.exe)),
            ("{{dll}}", build.dll.as_ref()),
        ];
        let tests = tests
            .into_iter()
            .map(|p| {
                let command = p.command.unwrap_or_else(|| command.clone());

                let groups = p
                    .groups
                    .into_iter()
                    .map(|g| {
                        let mut command = g.command.unwrap_or_else(|| command.clone());
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
                        let command = command.apply_input_paths(input_paths);
                        TestGroup {
                            command,
                            find_glob: g.find_glob,
                            find_gtest: g.find_gtest,
                            timeout: g.timeout,
                            timeout_if_changed: g.timeout_if_changed,
                            accepted_returncodes: accepted_returncodes.to_vec(),
                            testcases_dependencies: g.testcases_dependencies,
                            execution_style: g.execution_style,
                        }
                    })
                    .collect();
                TestPreset {
                    id_pattern: p.id_pattern,
                    groups,
                }
            })
            .collect();
        App {
            responsible,
            build,
            tests,
            globber_matches_parent,
            checkout_parent,
        }
    }
}

impl Build {
    fn from(build: &BuildConfig, input_paths: &InputPaths) -> Build {
        let exe = Build::apply_config_string(&build.exe, &input_paths).unwrap();
        //if !PathBuf::from(&exe).exists() {
        //    println!("WARNING: exe not found: {:?}", exe);
        //}
        let dll = Build::apply_config_string(&build.dll, &input_paths);
        //if let Some(dll) = &dll {
        //    if !PathBuf::from(&dll).exists() {
        //        println!("WARNING: dll not found: {:?}", dll);
        //    }
        //}
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
            Some(s) => Some(input_paths.apply_to(s)),
            None => None,
        }
    }
}

fn default_retcodes() -> Vec<i32> {
    vec![0]
}

fn value_xge() -> String {
    "xge".to_string()
}

impl TestGroup {
    pub fn generate_test_inputs(
        &self,
        app: &App,
        preset: &TestPreset,
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
        // the glob module can't handle Windows' extended path syntax
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
        let args = self.command.clone().apply("{{input}}", &filter);
        let args = &args.0[1..];
        let cwd = app.build.cwd.as_ref().map(|s| s.as_ref()).unwrap_or(".");
        let output = std::process::Command::new(exe)
            .arg("--gtest_list_tests")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("failed to gather tests!");
        if !output.status.success() {
            println!("Failed to execute {} {:?}: {:?}", exe, args, output);
            std::process::exit(-1);
        }
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

impl CommandTemplate {
    pub fn apply(&self, from: &str, to: &str) -> CommandTemplate {
        CommandTemplate(
            self.0
                .iter()
                .map(|t| t.to_owned().replace(from, to))
                .collect(),
        )
    }
    pub fn apply_input_paths(&self, input_paths: &InputPaths) -> CommandTemplate {
        CommandTemplate(self.0.iter().map(|t| input_paths.apply_to(t)).collect())
    }
    pub fn has_pattern(&self, pattern: &str) -> bool {
        self.0.iter().any(|t| t.contains(pattern))
    }
}

#[derive(Debug)]
pub struct InputPaths {
    pub dev_dir: Option<PathBuf>,
    pub build_dir: PathBuf,
    pub testcases_dir: PathBuf,
    pub build_type: Option<String>,
    pub preset: String,
    pub build_config: String,
}
impl InputPaths {
    fn apply_to(&self, string: &str) -> String {
        let mut s = string.to_string();
        if s.contains("{{dev_dir}}") {
            let dev_dir = match self.dev_dir.as_ref() {
                Some(d) => d.to_str().unwrap(),
                None => {
                    println!("Please specify --dev-dir");
                    std::process::exit(-1);
                }
            };
            s = s.replace("{{dev_dir}}", dev_dir);
        }
        if s.contains("{{build_dir}}") {
            s = s.replace("{{build_dir}}", self.build_dir.to_str().unwrap());
        }
        if s.contains("{{testcases_dir}}") {
            s = s.replace("{{testcases_dir}}", self.testcases_dir.to_str().unwrap());
        }
        s = s.replace("{{build_config}}", &self.build_config);
        s = s.replace(
            "{{build_config_skipunicode}}",
            &self.build_config.replace("Unicode", ""),
        );
        s
    }

    pub fn from(
        given_dev_dir: Option<String>,
        given_build_dir: Option<String>,
        given_testcases_dir: Option<String>,
        given_build_type: Option<String>,
        given_preset: Option<String>,
        given_build_config: Option<String>,
    ) -> Result<InputPaths> {
        let dev_dir: Option<PathBuf>;
        let build_dir: Option<PathBuf>;
        let build_type: Option<&str>;
        match InputPaths::guess_build_type(&given_build_dir) {
            BuildType::CMake(build_path, dev_path) => {
                dev_dir = Some(dev_path);
                build_dir = Some(build_path);
                build_type = Some("cmake-windows");
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
        let dev_dir = given_dev_dir.map(PathBuf::from).or(dev_dir);
        let build_dir = given_build_dir
            .map(PathBuf::from)
            .or(build_dir)
            .wrap_err("Could not determine --build-dir. You may have to specify it explicitly.")?;
        let build_type = given_build_type.or_else(|| build_type.map(|s| s.to_string()));

        let testcases_dir = given_testcases_dir
            .map(PathBuf::from)
            .or_else(|| InputPaths::guess_testcases_layout(&build_dir))
            .wrap_err(
                "Could not determine --testcases-dir. You may have to specify it explicitly.",
            )?;

        let preset = given_preset.unwrap_or_else(|| "ci".to_string());

        // println!("dev_dir: {:?}", dev_dir);
        // println!("build_dir: {:?}", build_dir);
        // println!("build_type: {:?}", build_type);
        // println!("testcases_dir: {:?}", testcases_dir);

        let build_config = given_build_config.unwrap_or_else(|| "RelWithDebInfo".to_string());

        Ok(InputPaths {
            dev_dir,
            build_dir,
            testcases_dir,
            build_type,
            preset,
            build_config,
        })
    }

    fn guess_build_type(build_dir: &Option<String>) -> BuildType {
        if let Some(layout) = InputPaths::find_cmake_layout(build_dir) {
            layout
        } else if InputPaths::is_quickstart(build_dir) {
            BuildType::Quickstart(std::env::current_dir().unwrap())
        } else {
            BuildType::None
        }
    }

    fn guess_testcases_layout(build_dir: &PathBuf) -> Option<PathBuf> {
        let guessed_path = build_dir.parent()?.join("testcases");
        if guessed_path.exists() {
            Some(guessed_path)
        } else {
            None
        }
    }

    fn find_cmake_layout(build_dir: &Option<String>) -> Option<BuildType> {
        let path = PathBuf::from(build_dir.clone().unwrap_or_default()).join("CMakeCache.txt");
        if let Ok(f) = std::fs::File::open(path) {
            let mut reader = std::io::BufReader::new(f);
            let mut line = String::new();
            while let Ok(count) = reader.read_line(&mut line) {
                if count == 0 {
                    break;
                }
                if let Some(dev_path) = line.split("mwBuildAll_SOURCE_DIR:STATIC=").nth(1) {
                    return Some(BuildType::CMake(
                        std::env::current_dir().unwrap(),
                        PathBuf::from(dev_path.trim()),
                    ));
                }
            }
        }
        None
    }

    fn is_quickstart(build_dir: &Option<String>) -> bool {
        let cwd = build_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap());
        cwd.join("mwVerifier.dll").exists()
    }
}

enum BuildType {
    CMake(PathBuf, PathBuf),
    Quickstart(PathBuf),
    None,
}
