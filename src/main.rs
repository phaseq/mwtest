mod config;
mod report;
mod runnable;
mod scheduler;
use clap::{App, AppSettings, Arg, ArgGroup, SubCommand};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[global_allocator]
static GLOBAL: std::alloc::System = std::alloc::System;

fn main() {
    let registered_tests = config::InputPaths::get_registered_tests();
    let registered_tests_str: Vec<&str> = registered_tests.iter().map(String::as_str).collect();
    let test_app_arg = Arg::with_name("test_app")
        .required(true)
        .multiple(true)
        .possible_values(&registered_tests_str[..]);
    let filter_arg = Arg::with_name("filter")
        .short("f")
        .long("filter")
        .takes_value(true)
        .help("select ids that contain one of the given substrings")
        .multiple(true);
    let matches = App::new("MW Test")
        .setting(AppSettings::SubcommandRequired)
        .arg(Arg::with_name("BUILD_ROOT")
            .long("build-dir")
            .takes_value(true)
            .help("depends on build type, could be \"your-branch/dev\", a quickstart or a CMake build directory"))
        .arg(Arg::with_name("TESTCASES_ROOT")
            .long("testcases-dir")
            .takes_value(true)
            .help("usually \"your-branch/testcases\""))
        .arg(Arg::with_name("OUT_DIR")
            .long("output-dir")
            .takes_value(true))
        .arg(Arg::with_name("BUILD_LAYOUT")
            .short("b")
            .long("--build")
            .takes_value(true)
            .help("specifies the layout of your build (like dev-releaseunicode, quickstart or cmake)"))
        .arg(Arg::with_name("PRESET")
            .long("--preset")
            .takes_value(true)
            .help("specifies which tests to run (like ci, nightly)"))
        .subcommand(
            SubCommand::with_name("build")
                .arg(test_app_arg.clone()),
        ).subcommand(
            SubCommand::with_name("list")
                .arg(test_app_arg.clone())
                .arg(filter_arg.clone()),
        ).subcommand(
            SubCommand::with_name("run")
                .arg(test_app_arg)
                .arg(filter_arg)
                .arg(Arg::with_name("id")
                    .long("id")
                    .takes_value(true))
                .group(ArgGroup::with_name("filter_group")
                    .args(&["filter", "id"]))
                .arg(Arg::with_name("verbose")
                    .short("v")
                    .long("verbose"))
                .arg(Arg::with_name("threadpool")
                    .short("p")
                    .long("parallel")
                    .help("run using local thread pool"))
                .arg(Arg::with_name("xge")
                    .long("xge"))
                .group(ArgGroup::with_name("parallel")
                    .args(&["threadpool", "xge"]))
                .arg(Arg::with_name("repeat")
                    .long("repeat")
                    .takes_value(true)
                    .default_value("1"))
                .arg(Arg::with_name("RERUN_IF_FAILED")
                    .long("repeat-if-failed")
                    .takes_value(true)
                    .default_value("0"))
        ).get_matches();

    let input_paths = config::InputPaths::from(
        &matches.value_of("BUILD_ROOT"),
        &matches.value_of("TESTCASES_ROOT"),
        &matches.value_of("BUILD_LAYOUT"),
        &matches.value_of("PRESET"),
    );

    if let Some(matches) = matches.subcommand_matches("build") {
        cmd_build(
            &matches
                .values_of("test_app")
                .unwrap()
                .collect::<Vec<&str>>(),
            &input_paths,
        );
        std::process::exit(0);
    }

    let test_group_file = match config::TestGroupFile::open(&input_paths.preset_path) {
        Ok(file) => file,
        Err(e) => {
            println!(
                "ERROR: failed to parse preset file at {:?}: {}",
                &input_paths.preset_path, e
            );
            std::process::exit(-1);
        }
    };

    if let Some(matches) = matches.subcommand_matches("list") {
        let test_apps = test_apps_from_args(&matches, &input_paths, &test_group_file);
        cmd_list(&test_apps);
    } else if let Some(matches) = matches.subcommand_matches("run") {
        let test_apps = test_apps_from_args(&matches, &input_paths, &test_group_file);
        let out_dir = matches
            .value_of("OUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap().join("test_output"));
        let output_paths = OutputPaths {
            out_dir: out_dir.clone(),
            tmp_dir: out_dir.join("tmp"),
        };
        let repeat = matches
            .value_of("repeat")
            .unwrap()
            .parse()
            .expect("expected numeric value for repeat");
        let repeat_if_failed = matches
            .value_of("RERUN_IF_FAILED")
            .unwrap()
            .parse()
            .expect("expected numeric value for repeat-if-failed");
        let repeat_strategy = if repeat_if_failed != 0 {
            scheduler::RepeatStrategy::RepeatIfFailed(repeat_if_failed)
        } else {
            scheduler::RepeatStrategy::Repeat(repeat)
        };
        let run_config = scheduler::RunConfig {
            verbose: matches.is_present("verbose"),
            parallel: matches.is_present("threadpool"),
            xge: matches.is_present("xge"),
            repeat: repeat_strategy,
        };
        let success = cmd_run(&input_paths, &test_apps, &output_paths, &run_config);
        if !success {
            std::process::exit(-1)
        }
    }
}

fn cmd_build(test_names: &[&str], input_paths: &config::InputPaths) {
    let mut dependencies: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, dep) in test_names
        .iter()
        .map(|n| (n, &input_paths.apps.get(*n).unwrap().layout))
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

fn cmd_list(test_apps: &[AppWithTests]) {
    for test_app in test_apps {
        for group in &test_app.tests {
            for test_id in &group.test_ids {
                println!("{} --id {}", test_app.name, test_id.id);
            }
        }
    }
}

fn cmd_run(
    input_paths: &config::InputPaths,
    test_apps: &[AppWithTests],
    output_paths: &OutputPaths,
    run_config: &scheduler::RunConfig,
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

    let tests = runnable::create_run_commands(&input_paths, &test_apps, &output_paths);
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

fn test_apps_from_args(
    args: &clap::ArgMatches<'_>,
    input_paths: &config::InputPaths,
    test_group_file: &config::TestGroupFile,
) -> Vec<AppWithTests> {
    let id_filter = id_filter_from_args(args);
    let apps: Vec<AppWithTests> = args
        .values_of("test_app")
        .unwrap()
        .map(|app_name| {
            // get app for string from command line
            let app = match input_paths.apps.get(app_name) {
                Some(app) => app,
                None => {
                    let app_names = input_paths.apps.app_names();
                    println!(
                        "ERROR: \"{}\" not found: must be one of {:?}",
                        app_name, app_names
                    );
                    std::process::exit(-1);
                }
            };

            // populate with tests
            let tests = test_group_file
                .get(app_name)
                .map(|test_groups| {
                    populate_test_groups(&app, input_paths, test_groups, &*id_filter)
                })
                .unwrap_or_else(|| vec![]);
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

fn id_filter_from_args(args: &clap::ArgMatches<'_>) -> Box<dyn Fn(&str) -> bool> {
    let filter_tokens: Option<Vec<String>> = args
        .values_of("filter")
        .map(|v| v.map(String::from).collect());
    let normalize = |input: &str| input.to_lowercase().replace('\\', "/");
    let id_token = args.value_of("id").map(|v| normalize(v).to_string());
    Box::new(move |input: &str| {
        let input = normalize(input);
        if let Some(filters) = &filter_tokens {
            filters
                .iter()
                .any(|f| input.contains(normalize(f).as_str()))
        } else if let Some(id) = &id_token {
            input == *id
        } else {
            true
        }
    })
}

fn populate_test_groups(
    app: &config::App,
    input_paths: &config::InputPaths,
    test_groups: &config::TestGroups,
    id_filter: &dyn Fn(&str) -> bool,
) -> Vec<GroupWithTests> {
    test_groups
        .groups
        .iter()
        .map(|test_group| GroupWithTests {
            test_group: test_group.clone(),
            test_ids: test_groups
                .generate_test_inputs(&test_group, &app, &input_paths)
                .into_iter()
                .filter(|f| id_filter(&f.id))
                .collect(),
        })
        .collect()
}
