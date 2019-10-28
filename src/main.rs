mod config;
mod report;
mod runnable;
mod scheduler;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use structopt::StructOpt;

#[global_allocator]
static GLOBAL: std::alloc::System = std::alloc::System;

#[derive(StructOpt)]
#[structopt(name = "mwtest", rename_all = "kebab-case")]
struct Args {
    /// path to a dev folder
    #[structopt(long)]
    dev_dir: Option<String>,

    /// depends on build type, could be a dev folder, a quickstart or a CMake build directory
    #[structopt(long)]
    build_dir: Option<String>,

    /// usually "your-branch/testcases"
    #[structopt(long)]
    testcases_dir: Option<String>,

    #[structopt(long)]
    output_dir: Option<String>,

    /// specifies the type of your build (like dev-releaseunicode, quickstart or cmake)
    #[structopt(long)]
    build_type: Option<String>,

    /// specifies which tests to run (like ci, nightly)
    #[structopt(long)]
    preset: Option<String>,

    /// specify build type (ReleaseUnicode, RelWithDebInfo, ...)
    #[structopt(long)]
    build_config: Option<String>,

    #[structopt(subcommand)]
    cmd: SubCommands,
}

#[derive(StructOpt)]
enum SubCommands {
    Build {
        app_names: Vec<String>,
    },
    List {
        app_names: Vec<String>,
    },
    Run {
        app_names: Vec<String>,

        #[structopt(long, conflicts_with = "filter")]
        id: Vec<String>,

        /// select ids that contain one of the given substrings
        #[structopt(short, long)]
        filter: Vec<String>,

        #[structopt(short, long)]
        verbose: bool,

        #[structopt(short, long, conflicts_with = "xge")]
        parallel: bool,
        #[structopt(long)]
        xge: bool,

        #[structopt(short, long, default_value = "1")]
        repeat: usize,
        #[structopt(long, default_value = "0", conflicts_with = "repeat")]
        repeat_if_failed: usize,

        #[structopt(long)]
        no_timeout: bool,
    },
}

fn main() {
    let args = Args::from_args();

    let input_paths = config::InputPaths::from(
        args.dev_dir,
        args.build_dir,
        args.testcases_dir,
        args.build_type,
        args.preset,
        args.build_config,
    );

    let apps_config = config::AppsConfig::load().expect("Failed to load apps.json!");

    match args.cmd {
        SubCommands::Build { app_names } => {
            let apps = apps_config.select_build_and_preset(&app_names, &input_paths);
            cmd_build(&app_names, &apps);
            std::process::exit(0);
        }
        SubCommands::List { app_names } => {
            if !app_names.is_empty() {
                let apps = apps_config.select_build_and_preset(&app_names, &input_paths);
                let app_tests = generate_app_tests(&app_names, vec![], vec![], &input_paths, &apps);
                cmd_list_tests(&app_tests);
            } else {
                cmd_list_apps(&apps_config);
            }
        }
        SubCommands::Run {
            app_names,
            id,
            filter,
            verbose,
            parallel,
            xge,
            repeat,
            repeat_if_failed,
            no_timeout,
        } => {
            let apps = apps_config.select_build_and_preset(&app_names, &input_paths);
            let app_tests = generate_app_tests(&app_names, filter, id, &input_paths, &apps);
            let out_dir = args
                .output_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap().join("test_output"));
            let output_paths = OutputPaths {
                out_dir: out_dir.clone(),
                tmp_dir: out_dir.join("tmp"),
            };
            let repeat_strategy = if repeat_if_failed != 0 {
                scheduler::RepeatStrategy::RepeatIfFailed(repeat_if_failed)
            } else {
                scheduler::RepeatStrategy::Repeat(repeat)
            };
            let run_config = scheduler::RunConfig {
                verbose,
                parallel,
                xge,
                repeat: repeat_strategy,
            };
            let success = cmd_run(
                &input_paths,
                &app_tests,
                &output_paths,
                &run_config,
                no_timeout,
            );
            if !success {
                std::process::exit(-1)
            }
        }
    }
}

fn cmd_build(test_names: &[String], apps: &config::Apps) {
    let mut dependencies: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, dep) in test_names
        .iter()
        .map(|n| (n, &apps.0.get(n).unwrap().build))
    {
        if dep.solution.is_none() || dep.project.is_none() {
            println!("ERROR: no solution/project defined for {}", name);
            std::process::exit(-1);
        }
        let deps = dependencies
            .entry(dep.solution.as_ref().unwrap())
            .or_insert_with(Vec::new);
        (*deps).push(dep.project.as_ref().unwrap());
    }
    for (solution, projects) in dependencies {
        let projects = projects.join(",");
        println!("building:\n  solution: {}\n  {}", &solution, &projects);
        Command::new("buildConsole")
            .arg(solution)
            .arg("/build")
            .arg("/silent")
            .arg("/cfg=ReleaseUnicode|x64")
            .arg(format!("/prj={}", projects))
            .arg("/openmonitor")
            .spawn()
            .expect("failed to launch buildConsole!")
            .wait()
            .expect("failed to build project!");
    }
}

fn cmd_list_apps(apps: &config::AppsConfig) {
    for name in apps.app_names() {
        println!("  {}", name);
    }
}

fn cmd_list_tests(apps: &[AppWithTests]) {
    for app in apps {
        for group in &app.tests {
            for test_id in &group.test_ids {
                println!("{} --id {}", app.name, test_id.id);
            }
        }
    }
}

fn cmd_run(
    input_paths: &config::InputPaths,
    test_apps: &[AppWithTests],
    output_paths: &OutputPaths,
    run_config: &scheduler::RunConfig,
    no_timeout: bool,
) -> bool {
    if Path::exists(&output_paths.out_dir) {
        if !Path::exists(&output_paths.out_dir.clone().join("results.xml")) {
            println!(
                "ERROR: can't reset the output directory: {:?}.\n It doesn't look like it \
                 was created by mwtest. Please select another one or delete it manually.",
                &output_paths.out_dir
            );
            std::process::exit(-1);
        }
        std::fs::remove_dir_all(&output_paths.out_dir).expect("could not clean up tmp directory!");
    }
    std::fs::create_dir_all(&output_paths.tmp_dir).expect("could not create tmp directory!");
    println!(
        "Test artifacts will be written to {}.",
        output_paths.out_dir.to_str().unwrap()
    );

    let tests = runnable::create_run_commands(&input_paths, &test_apps, &output_paths, no_timeout);
    if tests.is_empty() {
        println!("WARNING: No tests were selected.");
        std::process::exit(0); // counts as success
    }
    scheduler::run(&input_paths, tests, &output_paths, &run_config)
}

#[derive(Debug)]
pub struct AppWithTests {
    name: String,
    app: config::App,
    tests: Vec<GroupWithTests>,
}

#[derive(Debug)]
struct GroupWithTests {
    test_group: config::TestGroup,
    test_ids: Vec<TestId>,
    test_filter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TestId {
    pub id: String,
    pub rel_path: Option<PathBuf>,
}
impl std::hash::Hash for TestId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[derive(Debug, Clone)]
pub struct OutputPaths {
    out_dir: PathBuf,
    tmp_dir: PathBuf,
}

fn generate_app_tests(
    app_names: &[String],
    filter: Vec<String>,
    id: Vec<String>,
    input_paths: &config::InputPaths,
    apps_config: &config::Apps,
) -> Vec<AppWithTests> {
    let can_run_raw_gtest = filter.is_empty() && id.is_empty();
    let id_filter = id_filter_from_args(filter, id);
    let apps: Vec<AppWithTests> = app_names
        .iter()
        .map(|app_name| {
            // get app for string from command line
            let app = match apps_config.0.get(app_name) {
                Some(app) => app,
                None => {
                    let registered_tests = config::InputPaths::get_registered_tests();
                    println!(
                        "ERROR: \"{}\" not found: must be one of {:?}",
                        app_name, registered_tests
                    );
                    std::process::exit(-1);
                }
            };

            // populate with tests
            let tests: Vec<GroupWithTests> = app
                .tests
                .iter()
                .flat_map(|preset_config| {
                    preset_config
                        .groups
                        .iter()
                        .map(|test_group| {
                            let test_filter = if can_run_raw_gtest {
                                test_group.find_gtest.clone()
                            } else {
                                None
                            };
                            GroupWithTests {
                                test_group: test_group.clone(),
                                test_ids: test_group
                                    .generate_test_inputs(&app, &preset_config, &input_paths)
                                    .into_iter()
                                    .filter(|f| id_filter(&f.id))
                                    .collect(),
                                test_filter,
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            AppWithTests {
                name: app_name.to_string(),
                app: (*app).clone(),
                tests,
            }
        })
        .filter(|app_with_tests| !app_with_tests.tests.is_empty())
        .collect();
    if apps.is_empty() {
        println!("WARNING: you have not selected any tests.");
    }
    apps
}

fn id_filter_from_args(filter: Vec<String>, id: Vec<String>) -> Box<dyn Fn(&str) -> bool> {
    let normalize = |input: &str| input.to_lowercase().replace('\\', "/");
    Box::new(move |input: &str| {
        let input = normalize(input);
        if !filter.is_empty() {
            filter.iter().any(|f| input.contains(normalize(f).as_str()))
        } else if !id.is_empty() {
            id.iter().any(|i| normalize(i) == input)
        } else {
            true
        }
    })
}
